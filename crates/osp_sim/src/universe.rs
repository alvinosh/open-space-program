//! Top-level simulation state and time stepping.
//!
//! Two ways to advance time, which produce identical results for coasting vessels:
//! - [`Universe::step_physics`]: one fixed tick ([`PHYSICS_STEP`]). Integrated vessels are
//!   stepped numerically; coasting vessels resolve trajectory events inside the tick.
//! - [`Universe::advance_rails`]: jump straight to a target time, stopping at every
//!   trajectory event in order. Only allowed when no vessel needs physics.
//!
//! The game picks physics ticks for low warp and rails for high warp. Because coasting is
//! analytic and every event lands on an exact tick in both paths, switching warp rate never
//! changes the outcome. The same goes for a vessel's free rotation; with SAS on, rails hold
//! the attitude exactly where physics ticks would steer to it.

use std::sync::Arc;

use thiserror::Error;

use crate::{
    body::BodyId,
    data::{CraftDef, LaunchSiteDef, PartCatalog, ScenarioDef, StartDef, SystemDef},
    orbit::{Orbit, StateVector},
    rendezvous::Target,
    system::{SolarSystem, SystemError},
    terrain::{self, HeightSource},
    time::{SimTime, Ticks},
    vessel::{Vessel, VesselError, VesselEvent, VesselEventKind, VesselId},
};

/// Fixed physics tick.
pub const PHYSICS_STEP: Ticks = Ticks::from_millis(20);

#[derive(Debug, Error)]
pub enum UniverseError {
    #[error(transparent)]
    System(#[from] SystemError),
    #[error(transparent)]
    Vessel(#[from] VesselError),
    #[error("unknown launch site `{0}`")]
    UnknownSite(String),
    #[error("{0}")]
    Craft(String),
    #[error("scenario target `{0}` is neither a body nor a vessel")]
    UnknownTarget(String),
}

/// An event attributed to a vessel (by index).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimEvent {
    pub vessel: usize,
    pub event: VesselEvent,
}

/// Result of a rails advance.
#[derive(Debug, Default)]
pub struct RailsOutcome {
    pub events: Vec<SimEvent>,
    /// True if the advance stopped early (at an event needing physics or attention).
    pub interrupted: bool,
}

pub struct Universe {
    pub system: SolarSystem,
    pub vessels: Vec<Vessel>,
    time: SimTime,
    next_id: u64,
}

impl Universe {
    pub fn new(system: SolarSystem, vessels: Vec<Vessel>, time: SimTime) -> Self {
        let mut universe = Self {
            system,
            vessels: Vec::new(),
            time,
            next_id: 0,
        };
        for vessel in vessels {
            universe.add_vessel(vessel);
        }
        universe
    }

    /// Add a vessel, giving it a new id.
    pub fn add_vessel(&mut self, mut vessel: Vessel) -> VesselId {
        let id = VesselId(self.next_id);
        self.next_id += 1;
        vessel.set_id(id);
        self.vessels.push(vessel);
        id
    }

    /// Index of the vessel with `id`, if it is still in the game.
    pub fn find_vessel(&self, id: VesselId) -> Option<usize> {
        self.vessels.iter().position(|v| v.id() == id)
    }

    /// The conic `target` follows and the body it is relative to (a vessel's osculating
    /// orbit under power, its surface track's instantaneous conic when landed).
    pub fn target_orbit(&self, target: Target, time: SimTime) -> Option<(BodyId, Orbit)> {
        match target {
            Target::Body(body) => {
                let body = self.system.body(body);
                Some((body.parent?, body.orbit?))
            }
            Target::Vessel(id) => {
                let vessel = &self.vessels[self.find_vessel(id)?];
                Some((vessel.parent(), vessel.orbit(&self.system, time)))
            }
        }
    }

    /// State of vessel `index`'s target relative to that vessel's parent body.
    pub fn target_state(&self, index: usize, time: SimTime) -> Option<StateVector> {
        let vessel = &self.vessels[index];
        let (frame, state) = match vessel.target? {
            Target::Body(body) => (body, StateVector::default()),
            Target::Vessel(id) => {
                let other = self.vessels.get(self.find_vessel(id)?)?;
                (other.parent(), other.state_at(&self.system, time))
            }
        };
        Some(self.system.transform(state, frame, vessel.parent(), time))
    }

    /// Build the solar system with its terrain (`height_sources` by body id, plus the
    /// earthworks of every launch site) and place the scenario's craft.
    /// `craft_of` looks craft records up by id (the scenario's own and its other vessels').
    pub fn new_game(
        system: &SystemDef,
        height_sources: Vec<(String, Arc<dyn HeightSource>)>,
        sites: &[LaunchSiteDef],
        catalog: &PartCatalog,
        craft_of: impl Fn(&str) -> Result<CraftDef, String>,
        scenario: &ScenarioDef,
    ) -> Result<Self, UniverseError> {
        let mut system = SolarSystem::from_def(system)?;
        let sources = height_sources
            .into_iter()
            .filter_map(|(name, source)| Some((system.find(&name)?, source)))
            .collect();
        // Sites on unknown bodies only matter if a scenario starts there (reported below).
        terrain::install(&mut system, sources, sites);
        let time = SimTime::from_seconds(scenario.start_time);
        let spawn = |craft: &str, start: &StartDef| -> Result<Vessel, UniverseError> {
            let craft = craft_of(craft).map_err(UniverseError::Craft)?;
            Ok(match start {
                StartDef::LaunchSite(start) => {
                    let site = sites
                        .iter()
                        .find(|s| s.id == start.site)
                        .ok_or_else(|| UniverseError::UnknownSite(start.site.clone()))?;
                    Vessel::on_launch_site(&craft, catalog, &system, site, time)?
                }
                StartDef::Orbit(orbit) => Vessel::in_orbit(&craft, catalog, &system, orbit, time)?,
            })
        };
        let mut vessels = vec![spawn(&scenario.craft, &scenario.start)?];
        for other in &scenario.others {
            let mut vessel = spawn(&other.craft, &other.start)?;
            if let Some(name) = &other.name {
                vessel.name = name.clone();
            }
            vessels.push(vessel);
        }
        let mut universe = Self::new(system, vessels, time);
        if let Some(target) = &scenario.target {
            let found = universe
                .vessels
                .iter()
                .skip(1)
                .find(|v| &v.name == target)
                .map(|v| Target::Vessel(v.id()))
                .or_else(|| universe.system.find(target).map(Target::Body))
                .ok_or_else(|| UniverseError::UnknownTarget(target.clone()))?;
            universe.vessels[0].target = Some(found);
        }
        Ok(universe)
    }

    pub fn time(&self) -> SimTime {
        self.time
    }

    /// Rails warp is possible only when every vessel is landed or coasting with engines off,
    /// and none has reached the point where it needs physics again (a burn coming up).
    pub fn can_use_rails(&self) -> bool {
        !self
            .vessels
            .iter()
            .any(|v| v.needs_physics() || v.attention_time().is_some_and(|t| t <= self.time))
    }

    /// The earliest time some vessel needs physics again, if any does, and which vessel.
    fn attention(&self) -> Option<(usize, SimTime)> {
        self.vessels
            .iter()
            .enumerate()
            .filter_map(|(i, v)| Some((i, v.attention_time()?)))
            .min_by_key(|(_, t)| *t)
    }

    /// Advance one fixed physics tick.
    pub fn step_physics(&mut self) -> Vec<SimEvent> {
        let (t0, t1) = (self.time, self.time + PHYSICS_STEP);
        // Vessels steering towards a target need to know where it is.
        let targets: Vec<_> = (0..self.vessels.len())
            .map(|i| self.target_state(i, t0))
            .collect();
        for (vessel, state) in self.vessels.iter_mut().zip(targets) {
            vessel.set_target_state(state);
        }
        let mut events = Vec::new();
        for (index, vessel) in self.vessels.iter_mut().enumerate() {
            let mut local = Vec::new();
            vessel.step(&self.system, t0, t1, &mut local);
            events.extend(local.into_iter().map(|event| SimEvent {
                vessel: index,
                event,
            }));
        }
        self.time = t1;
        events
    }

    /// Jump towards `target` analytically, processing trajectory events in time order.
    /// Stops early at events that need physics or attention (descent, landing), or
    /// immediately if any vessel needs physics.
    pub fn advance_rails(&mut self, target: SimTime) -> RailsOutcome {
        let mut outcome = RailsOutcome::default();
        if !self.can_use_rails() {
            outcome.interrupted = true;
            return outcome;
        }
        for vessel in &mut self.vessels {
            vessel.enter_rails(&self.system, self.time);
        }
        // Hand over to physics where a vessel needs it.
        let due = self.attention().filter(|(_, t)| *t <= target);
        let target = due.map_or(target, |(_, t)| t);
        while self.time < target {
            let next = self
                .vessels
                .iter()
                .enumerate()
                .filter_map(|(i, v)| {
                    v.next_trajectory_event(&self.system, self.time, target)
                        .map(|e| (i, e))
                })
                .min_by_key(|(_, e)| e.time);
            let Some((index, event)) = next else {
                self.time = target;
                break;
            };
            self.time = event.time;
            let mut local = Vec::new();
            self.vessels[index].apply_trajectory_event(&self.system, event, &mut local);
            let stop = local.iter().any(|e| e.kind.stops_warp());
            outcome
                .events
                .extend(local.into_iter().map(|event| SimEvent {
                    vessel: index,
                    event,
                }));
            if stop {
                outcome.interrupted = true;
                break;
            }
        }
        let targets: Vec<_> = (0..self.vessels.len())
            .map(|i| self.target_state(i, self.time))
            .collect();
        for (vessel, state) in self.vessels.iter_mut().zip(targets) {
            vessel.set_target_state(state);
        }
        if let Some((vessel, time)) = due
            && !outcome.interrupted
            && self.time == time
        {
            outcome.interrupted = true;
            outcome.events.push(SimEvent {
                vessel,
                event: VesselEvent {
                    time,
                    kind: VesselEventKind::BurnDue,
                },
            });
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::*;
    use crate::{
        events::tests::translunar,
        orbit::StateVector,
        system::tests::rss,
        vessel::{
            Motion, VesselEventKind,
            tests::{catalog, leo_vessel, pathfinder},
        },
    };

    fn vessel_with(
        system: &SolarSystem,
        frame: crate::body::BodyId,
        state: StateVector,
        t: SimTime,
    ) -> Vessel {
        let start = crate::data::OrbitStartDef {
            body: system.body(frame).name.clone(),
            periapsis_altitude: 400e3,
            apoapsis_altitude: 400e3,
            inclination: 0.0,
            longitude_of_ascending_node: 0.0,
            plane: None,
            argument_of_periapsis: 0.0,
            mean_anomaly: 0.0,
        };
        let mut v = Vessel::in_orbit(&pathfinder(), &catalog(), system, &start, t).unwrap();
        v.set_free_state(system, frame, state, t);
        v
    }

    fn universe_with(state: StateVector, frame_name: &str, t: SimTime) -> Universe {
        let system = rss();
        let frame = system.find(frame_name).unwrap();
        let v = vessel_with(&system, frame, state, t);
        Universe::new(system, vec![v], t)
    }

    fn leo_state(system: &SolarSystem) -> StateVector {
        let earth = system.body(system.find("Earth").unwrap());
        let r = earth.radius + 400e3;
        StateVector::new(
            DVec3::new(r, 0.0, 0.0),
            DVec3::new(0.0, (earth.gm / r).sqrt(), 0.0),
        )
    }

    #[test]
    fn warp_rate_does_not_change_coasting() {
        let t0 = SimTime::J2000;
        let state = leo_state(&rss());
        let mut physics = universe_with(state, "Earth", t0);
        let mut rails = universe_with(state, "Earth", t0);
        assert!(rails.can_use_rails());

        for _ in 0..(3_600 * 50) {
            physics.step_physics();
        }
        let outcome = rails.advance_rails(t0.add_seconds(3_600.0));
        assert!(!outcome.interrupted);
        assert_eq!(physics.time(), rails.time());
        let a = physics.vessels[0].state_at(&physics.system, physics.time());
        let b = rails.vessels[0].state_at(&rails.system, rails.time());
        // Bit-identical: both evaluate the same conic at the same tick.
        assert_eq!(a, b);
    }

    #[test]
    fn free_spin_is_identical_at_any_warp() {
        let system = rss();
        let (mut v, t0) = leo_vessel(&system);
        // Tumble it with the reaction wheels, then let go.
        v.rotation_input = DVec3::new(0.4, -0.2, 1.0);
        let mut physics = Universe::new(system, vec![v], t0);
        for _ in 0..50 {
            physics.step_physics();
        }
        physics.vessels[0].rotation_input = DVec3::ZERO;
        physics.step_physics();
        let mut rails = Universe::new(rss(), physics.vessels.clone(), physics.time());
        for _ in 0..(600 * 50) {
            physics.step_physics();
        }
        assert!(!rails.advance_rails(physics.time()).interrupted);
        let attitude = |u: &Universe| u.vessels[0].attitude_state(&u.system, u.time());
        assert_eq!(attitude(&physics), attitude(&rails));
        assert!(attitude(&rails).angular_velocity.length() > 0.01);
    }

    #[test]
    fn century_of_warp_keeps_the_orbit() {
        let t0 = SimTime::J2000;
        let state = leo_state(&rss());
        let mut u = universe_with(state, "Earth", t0);
        let before = u.vessels[0].orbit(&u.system, t0);
        // 100 years in 1-day frames, as 100 000x warp would do.
        for _ in 0..36_525 {
            let target = u.time().add_seconds(86_400.0);
            assert!(!u.advance_rails(target).interrupted);
        }
        let after = u.vessels[0].orbit(&u.system, u.time());
        assert_eq!(before.semi_major_axis(), after.semi_major_axis());
        assert_eq!(before.eccentricity(), after.eccentricity());
        // And the position is where the analytic orbit says, to well under a metre.
        let expected = before.state_at(u.time()).position;
        let got = u.vessels[0].state_at(&u.system, u.time()).position;
        assert!((expected - got).length() < 1e-6);
    }

    #[test]
    fn atmosphere_entry_stops_warp_at_the_boundary() {
        let system = rss();
        let earth_id = system.find("Earth").unwrap();
        let earth = system.body(earth_id);
        // 400 x 100 km orbit: periapsis inside the atmosphere.
        let ra = earth.radius + 400e3;
        let rp = earth.radius + 100e3;
        let a = (ra + rp) / 2.0;
        let va = (earth.gm * (2.0 / ra - 1.0 / a)).sqrt();
        let state = StateVector::new(DVec3::new(ra, 0.0, 0.0), DVec3::new(0.0, va, 0.0));
        let t0 = SimTime::J2000;
        let mut u = Universe::new(system, vec![], t0);
        let v = vessel_with(&u.system, earth_id, state, t0);
        u.vessels.push(v);

        let outcome = u.advance_rails(t0.add_seconds(86_400.0));
        assert!(outcome.interrupted);
        assert!(matches!(
            outcome.events[0].event.kind,
            VesselEventKind::Descent { .. }
        ));
        let r = u.vessels[0].state_at(&u.system, u.time()).position.length();
        let boundary = u.system.body(earth_id).coast_floor();
        assert!(r <= boundary && boundary - r < 1.0, "{}", boundary - r);
        assert!(matches!(u.vessels[0].motion(), Motion::Integrated(_)));
        assert!(!u.can_use_rails());
    }

    #[test]
    fn moon_transfer_is_identical_at_any_warp() {
        let system = rss();
        let (earth, orbit) = translunar(&system);
        let t0 = orbit.epoch();
        let state = orbit.epoch_state();
        let mut fast = Universe::new(system, vec![], t0);
        fast.vessels
            .push(vessel_with(&fast.system, earth, state, t0));
        let mut slow = Universe::new(rss(), vec![], t0);
        slow.vessels
            .push(vessel_with(&slow.system, earth, state, t0));

        let horizon = t0.add_seconds(6.0 * 86_400.0);
        // Fast: one big jump per event. Slow: 10-second frames.
        let mut fast_events = Vec::new();
        while fast.time() < horizon {
            let outcome = fast.advance_rails(horizon);
            fast_events.extend(outcome.events);
            if outcome.interrupted {
                break;
            }
        }
        let mut slow_events = Vec::new();
        while slow.time() < fast.time() {
            let target = slow.time().add_seconds(10.0).min(fast.time());
            let outcome = slow.advance_rails(target);
            slow_events.extend(outcome.events);
            if outcome.interrupted {
                break;
            }
        }
        assert!(!fast_events.is_empty());
        assert_eq!(fast_events, slow_events);
        assert_eq!(fast.time(), slow.time());
        let a = fast.vessels[0].state_at(&fast.system, fast.time());
        let b = slow.vessels[0].state_at(&slow.system, slow.time());
        assert_eq!(a, b);
        assert_ne!(
            fast.vessels[0].parent(),
            earth,
            "should have reached the Moon"
        );
    }
}
