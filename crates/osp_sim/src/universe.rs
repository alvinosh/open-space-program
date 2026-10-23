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
//! a steady attitude exactly where physics ticks would keep it.
//!
//! Debris (what vessels shed when they stage) is simulated like any vessel, with two
//! exceptions unless it is the [`Universe::focus`]: it is removed when it crashes, and it
//! never holds up time warp. Debris that needs physics when rails warp starts, or enters an
//! atmosphere on rails, is left behind (removed), as Kerbal Space Program does.

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

/// An event attributed to a vessel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimEvent {
    pub vessel: VesselId,
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
    /// The vessel the player flies. The debris rules never apply to it.
    pub focus: Option<VesselId>,
    time: SimTime,
    next_id: u64,
}

impl Universe {
    pub fn new(system: SolarSystem, vessels: Vec<Vessel>, time: SimTime) -> Self {
        let mut universe = Self {
            system,
            vessels: Vec::new(),
            focus: None,
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

    /// The vessel with `id`, if it is still in the game.
    pub fn vessel(&self, id: VesselId) -> Option<&Vessel> {
        self.vessels.iter().find(|v| v.id() == id)
    }

    /// Take a vessel out of the game; whoever targeted it loses the target.
    pub fn remove_vessel(&mut self, id: VesselId) {
        self.vessels.retain(|v| v.id() != id);
        for vessel in &mut self.vessels {
            if vessel.target == Some(Target::Vessel(id)) {
                vessel.target = None;
            }
        }
    }

    /// Take every piece of debris out of the game except the focus; returns how many went.
    pub fn clear_debris(&mut self) -> usize {
        let debris: Vec<_> = self
            .vessels
            .iter()
            .filter(|v| self.expendable(v))
            .map(Vessel::id)
            .collect();
        for &id in &debris {
            self.remove_vessel(id);
        }
        debris.len()
    }

    /// Fire vessel `index`'s next activation sequence; what it sheds flies on as debris.
    /// Returns the debris' ids.
    pub fn activate_next_stage(&mut self, index: usize) -> Vec<VesselId> {
        let debris = self.vessels[index].activate_next_stage(&self.system, self.time);
        debris.into_iter().map(|d| self.add_vessel(d)).collect()
    }

    /// Debris other than the focus: removed when it crashes, never holds up warp.
    fn expendable(&self, vessel: &Vessel) -> bool {
        vessel.is_debris() && self.focus != Some(vessel.id())
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
        let mut universe = Self::new(system, Vec::new(), time);
        universe.restart(sites, catalog, craft_of, scenario)?;
        Ok(universe)
    }

    /// Start `scenario` afresh in this solar system: its craft replace every vessel and the
    /// clock goes to its start time. On an error nothing changes.
    pub fn restart(
        &mut self,
        sites: &[LaunchSiteDef],
        catalog: &PartCatalog,
        craft_of: impl Fn(&str) -> Result<CraftDef, String>,
        scenario: &ScenarioDef,
    ) -> Result<(), UniverseError> {
        let system = &self.system;
        let time = SimTime::from_seconds(scenario.start_time);
        let spawn = |craft: &str, start: &StartDef| -> Result<Vessel, UniverseError> {
            let craft = craft_of(craft).map_err(UniverseError::Craft)?;
            let site = |id: &str| {
                sites
                    .iter()
                    .find(|s| s.id == id)
                    .ok_or_else(|| UniverseError::UnknownSite(id.to_owned()))
            };
            Ok(match start {
                StartDef::LaunchSite(start) => {
                    Vessel::on_launch_site(&craft, catalog, system, site(&start.site)?, time)?
                }
                StartDef::Runway(start) => {
                    Vessel::on_runway(&craft, catalog, system, site(&start.site)?, time)?
                }
                StartDef::Orbit(orbit) => Vessel::in_orbit(&craft, catalog, system, orbit, time)?,
                StartDef::Flight(flight) => {
                    Vessel::in_flight(&craft, catalog, system, flight, time)?
                }
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
        // An unknown target fails the start before anything changes.
        if let Some(target) = &scenario.target
            && !vessels.iter().skip(1).any(|v| &v.name == target)
            && self.system.find(target).is_none()
        {
            return Err(UniverseError::UnknownTarget(target.clone()));
        }
        self.vessels.clear();
        self.time = time;
        for vessel in vessels {
            self.add_vessel(vessel);
        }
        self.focus = Some(self.vessels[0].id());
        if let Some(target) = &scenario.target {
            self.vessels[0].target = self
                .vessels
                .iter()
                .skip(1)
                .find(|v| &v.name == target)
                .map(|v| Target::Vessel(v.id()))
                .or_else(|| self.system.find(target).map(Target::Body));
        }
        Ok(())
    }

    pub fn time(&self) -> SimTime {
        self.time
    }

    /// Rails warp is possible only when every vessel is landed or coasting with engines off,
    /// and none has reached the point where it needs physics again (a burn coming up).
    /// Debris does not count: rails leave behind what of it needs physics.
    pub fn can_use_rails(&self) -> bool {
        !self.vessels.iter().any(|v| {
            !self.expendable(v)
                && (v.needs_physics() || v.attention_time().is_some_and(|t| t <= self.time))
        })
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
        for vessel in &mut self.vessels {
            let mut local = Vec::new();
            vessel.step(&self.system, t0, t1, &mut local);
            events.extend(local.into_iter().map(|event| SimEvent {
                vessel: vessel.id(),
                event,
            }));
        }
        self.time = t1;
        for event in &events {
            if matches!(event.event.kind, VesselEventKind::Crashed { .. })
                && self
                    .vessel(event.vessel)
                    .is_some_and(|v| self.expendable(v))
            {
                self.remove_vessel(event.vessel);
            }
        }
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
        let behind: Vec<_> = self
            .vessels
            .iter()
            .filter(|v| self.expendable(v) && v.needs_physics())
            .map(Vessel::id)
            .collect();
        for id in behind {
            self.remove_vessel(id);
        }
        for vessel in &mut self.vessels {
            vessel.enter_rails(&self.system, self.time);
        }
        // Hand over to physics where a vessel needs it.
        let due = self
            .attention()
            .filter(|(_, t)| *t <= target)
            .map(|(i, t)| (self.vessels[i].id(), t));
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
            let vessel = &mut self.vessels[index];
            vessel.apply_trajectory_event(&self.system, event, &mut local);
            let id = vessel.id();
            let stop = local.iter().any(|e| e.kind.stops_warp());
            if stop && self.expendable(&self.vessels[index]) {
                self.remove_vessel(id);
                continue;
            }
            outcome.events.extend(
                local
                    .into_iter()
                    .map(|event| SimEvent { vessel: id, event }),
            );
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
            tests::{catalog, leo_vessel, pad_vessel, pathfinder},
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

    #[test]
    fn a_restart_replaces_the_vessels_and_the_clock() {
        let scenarios: Vec<ScenarioDef> =
            crate::test_data::records(include_str!("../../../assets/core/scenarios/stock.json5"));
        let scenario = |id: &str| scenarios.iter().find(|s| s.id == id).unwrap();
        let sites = crate::vessel::tests::launch_sites();
        let craft_of = |id: &str| {
            let text = match id {
                "pathfinder" => include_str!("../../../assets/core/craft/pathfinder.json5"),
                "upper_stage" => include_str!("../../../assets/core/craft/upper_stage.json5"),
                "comet" => include_str!("../../../assets/core/craft/comet.json5"),
                _ => return Err(format!("no craft `{id}`")),
            };
            Ok(crate::test_data::records(text).remove(0))
        };
        let mut universe = Universe::new(rss(), Vec::new(), SimTime::J2000);
        universe
            .restart(&sites, &catalog(), craft_of, scenario("rendezvous"))
            .unwrap();
        assert_eq!(universe.vessels.len(), 2);
        assert!(universe.vessels[0].target.is_some());
        // Into another: everything from before is gone.
        universe
            .restart(&sites, &catalog(), craft_of, scenario("runway"))
            .unwrap();
        assert_eq!(universe.vessels.len(), 1);
        assert_eq!(universe.vessels[0].name, "Comet");
        assert!(universe.vessels[0].is_landed() && universe.vessels[0].target.is_none());
        assert_eq!(universe.time(), SimTime::from_seconds(21600.0));
        // A scenario that cannot start leaves things as they were.
        assert!(
            universe
                .restart(&sites, &catalog(), craft_of, scenario("glider"))
                .is_err()
        );
        assert_eq!(universe.vessels[0].name, "Comet");
    }

    /// The Pathfinder on the pad, staged three times with the throttle shut: the booster
    /// is left standing on the pad and the fairing halves fall from the top of the stack.
    fn staged_on_the_pad() -> Universe {
        let system = rss();
        let (pad, t) = pad_vessel(&system);
        let mut u = Universe::new(system, vec![pad], t);
        u.focus = Some(u.vessels[0].id());
        let top = |u: &Universe| u.vessels[0].base_at(&u.system, u.time());
        let base = top(&u);
        let booster = u.vessels[0].propulsion.stages[0].length;
        let debris: Vec<_> = (0..3).flat_map(|_| u.activate_next_stage(0)).collect();
        assert_eq!(debris.len(), 3);
        assert_eq!(u.vessels.len(), 4);
        // The upper stage stands on the booster, which stands where it stood.
        assert!(u.vessels[0].is_landed() && u.vessels[1].is_landed());
        assert!((u.vessels[1].base_at(&u.system, u.time()) - base).length() < 1e-6);
        let raised = top(&u) - base;
        assert!(
            (raised - base.normalize() * booster).length() < 1e-6,
            "{raised}"
        );
        u
    }

    #[test]
    fn crashed_debris_is_removed_unless_it_is_flown() {
        let mut u = staged_on_the_pad();
        let halves = [u.vessels[2].id(), u.vessels[3].id()];
        u.focus = Some(halves[1]);
        let mut crashes = 0;
        for _ in 0..(10 * 50) {
            crashes += u
                .step_physics()
                .iter()
                .filter(|e| matches!(e.event.kind, VesselEventKind::Crashed { .. }))
                .count();
        }
        assert_eq!(crashes, 2);
        assert!(u.vessel(halves[0]).is_none());
        assert!(u.vessel(halves[1]).is_some_and(Vessel::is_landed));
        assert_eq!(u.vessels.len(), 3);
    }

    #[test]
    fn clearing_debris_keeps_vessels_and_the_debris_being_flown() {
        let mut u = staged_on_the_pad();
        let flown = u.vessels[2].id();
        u.focus = Some(flown);
        u.vessels[0].target = Some(Target::Vessel(u.vessels[3].id()));
        assert_eq!(u.clear_debris(), 2);
        let left: Vec<_> = u.vessels.iter().map(Vessel::id).collect();
        assert_eq!(left.len(), 2);
        assert!(!u.vessels[0].is_debris() && left.contains(&flown));
        // Whoever targeted a cleared piece lost the target.
        assert_eq!(u.vessels[0].target, None);
    }

    #[test]
    fn warp_leaves_behind_debris_that_needs_physics() {
        let mut u = staged_on_the_pad();
        // The halves are falling, but that does not hold up warp.
        assert!(u.vessels[2].needs_physics() && u.vessels[3].needs_physics());
        assert!(u.can_use_rails());
        let end = u.time().add_seconds(60.0);
        assert!(!u.advance_rails(end).interrupted);
        assert_eq!(u.time(), end);
        assert_eq!(u.vessels.len(), 2);
        // Unless one is being flown.
        let mut u = staged_on_the_pad();
        u.focus = Some(u.vessels[3].id());
        assert!(!u.can_use_rails());
    }

    #[test]
    fn debris_entering_the_air_on_rails_is_left_behind() {
        let system = rss();
        let earth_id = system.find("Earth").unwrap();
        let earth = system.body(earth_id);
        // 400 x 100 km: periapsis inside the atmosphere.
        let (ra, rp) = (earth.radius + 400e3, earth.radius + 100e3);
        let va = (earth.gm * (2.0 / ra - 2.0 / (ra + rp))).sqrt();
        let low = StateVector::new(DVec3::new(ra, 0.0, 0.0), DVec3::new(0.0, va, 0.0));
        let high = leo_state(&system);
        let t0 = SimTime::J2000;
        let mut u = Universe::new(system, vec![], t0);
        let v = vessel_with(&u.system, earth_id, low, t0);
        u.add_vessel(v);
        u.focus = Some(u.vessels[0].id());
        // Fairing halves, and the upper stage under the payload.
        let debris: Vec<_> = (0..3).flat_map(|_| u.activate_next_stage(0)).collect();
        assert_eq!(debris.len(), 3);
        u.vessels[0].set_free_state(&u.system, earth_id, high, t0);
        let outcome = u.advance_rails(t0.add_seconds(86_400.0));
        assert!(!outcome.interrupted);
        assert!(outcome.events.is_empty());
        assert_eq!(u.vessels.len(), 1);
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
