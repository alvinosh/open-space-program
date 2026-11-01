//! Missions: the goals a scenario sets, checked against the simulation as it runs.
//!
//! A scenario can list `goals` and `fail` conditions. The universe hands its state to
//! [`Mission::observe`] after every physics tick and every jump on rails, so a mission is
//! judged on the simulation alone: no renderer, no UI, the same in the game and in a
//! headless script that flies with a player's controls.
//!
//! A [`Condition`] is a question about the vessel the player flies ("is it in an orbit
//! between these altitudes?", "has it landed within 500 m of the pad?"). A goal is a
//! condition, optionally held for a time; it stays met once it has been. The mission is
//! passed when every goal is met, and failed the moment a `fail` condition holds. Both end
//! it: the clock and the statistics stop there, for the results screen.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    body::Body,
    constants::G0,
    data::{LaunchSiteDef, ScenarioDef},
    system::SolarSystem,
    time::SimTime,
    universe::{SimEvent, Universe},
    vessel::{Situation, Vessel, VesselEventKind},
};

/// Closer than this to a site, m, is on it when a `Landed` condition names a site but not a
/// radius.
const SITE_RADIUS: f64 = 500.0;

/// A bound with either end left open.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Range {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

impl Range {
    pub fn new(min: Option<f64>, max: Option<f64>) -> Self {
        Self { min, max }
    }

    pub fn contains(&self, value: f64) -> bool {
        self.min.is_none_or(|min| value >= min) && self.max.is_none_or(|max| value <= max)
    }
}

/// A question about the player's vessel and the mission so far. Lengths are in metres, speeds
/// in m/s, angles in degrees, masses in kg and times in seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Condition {
    /// Height above sea level of the body it is near.
    Altitude {
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
    },
    /// Speed over the ground (the air), or with `orbital` relative to the body's centre.
    Speed {
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
        #[serde(default)]
        orbital: bool,
    },
    /// In a stable orbit (the lowest point clear of the air) round `body`, with the apsis
    /// altitudes and the inclination to its equator in range. An open range is any.
    Orbit {
        body: String,
        #[serde(default)]
        apoapsis: Range,
        #[serde(default)]
        periapsis: Range,
        #[serde(default)]
        inclination: Range,
    },
    /// Within `distance` of the vessel called `target` (and, with `speed`, slower than that
    /// relative to it). Hold it for a time with the goal's `hold`.
    Near {
        target: String,
        distance: f64,
        #[serde(default)]
        speed: Option<f64>,
    },
    /// On the ground, on `body` and within `radius` of `site` (a `launch_sites` id), if given.
    Landed {
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        site: Option<String>,
        #[serde(default)]
        radius: Option<f64>,
    },
    /// Back on the ground in one piece after flying, with the peak acceleration under
    /// `max_g`. (There is no crew model yet, so "crew alive" is "vessel intact".)
    Recovered {
        #[serde(default)]
        max_g: Option<f64>,
    },
    /// Propellant left in the main tanks (not the RCS).
    Propellant {
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
    },
    /// Broke up, or hit the ground too hard.
    Destroyed,
    /// This long since the mission began.
    Elapsed { seconds: f64 },
}

/// One goal, or one way to fail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalDef {
    pub id: String,
    /// What the panel says; for a `fail` entry, why the mission failed.
    pub text: String,
    pub when: Condition,
    /// The condition has to hold this long without a break, s.
    #[serde(default)]
    pub hold: f64,
}

/// What the player has done, for the results screen and for conditions that look back.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Stats {
    /// Mission time, s: the clock's, so it runs under time warp too.
    pub elapsed: f64,
    /// What the engines have given the vessel in physics ticks (thrust over mass, summed), m/s.
    pub delta_v: f64,
    /// Greatest acceleration the vessel's own thrust and the air gave it, g.
    pub peak_g: f64,
    pub max_altitude: f64,
    /// Has been off the ground (a mission that starts in the air counts).
    pub flown: bool,
    pub crashed: bool,
    pub destroyed: bool,
}

/// How a mission stands.
#[derive(Debug, Clone, PartialEq)]
pub enum State {
    Running,
    Passed,
    /// A fail condition held: its text.
    Failed(String),
}

/// What just happened, for the game to tell the player once.
#[derive(Debug, Clone, PartialEq)]
pub enum MissionEvent {
    /// The goal at this index was met.
    GoalMet(usize),
    Passed,
    Failed(String),
}

/// A goal and how far it has got.
#[derive(Debug, Clone, PartialEq)]
pub struct Goal {
    pub def: GoalDef,
    /// Mission time it was met at, s.
    pub met_at: Option<f64>,
    /// How long its condition has held without a break, s.
    pub held: f64,
}

/// Where a launch site is, for `Landed`.
#[derive(Debug, Clone)]
struct Place {
    body: String,
    latitude: f64,
    longitude: f64,
}

#[derive(Debug, Clone)]
pub struct Mission {
    goals: Vec<Goal>,
    fail: Vec<Goal>,
    /// A goal is only looked at once the ones before it are met.
    ordered: bool,
    places: HashMap<String, Place>,
    state: State,
    stats: Stats,
    started: SimTime,
    /// When the mission was last looked at.
    last: SimTime,
    events: Vec<MissionEvent>,
}

impl Mission {
    /// The mission `scenario` sets, or none if it lists no goals. A goal that names something
    /// that is not there (a site, a body, a vessel) is an error.
    pub fn new(
        scenario: &ScenarioDef,
        sites: &[LaunchSiteDef],
        system: &SolarSystem,
    ) -> Result<Option<Self>, String> {
        if scenario.goals.is_empty() {
            return Ok(None);
        }
        let mut places = HashMap::new();
        for def in scenario.goals.iter().chain(&scenario.fail) {
            match &def.when {
                Condition::Orbit { body, .. } if system.find(body).is_none() => {
                    return Err(format!("goal `{}`: no body `{body}`", def.id));
                }
                Condition::Landed { body, site, .. } => {
                    if let Some(body) = body
                        && system.find(body).is_none()
                    {
                        return Err(format!("goal `{}`: no body `{body}`", def.id));
                    }
                    if let Some(id) = site {
                        let site = sites
                            .iter()
                            .find(|s| &s.id == id)
                            .ok_or_else(|| format!("goal `{}`: no launch site `{id}`", def.id))?;
                        places.insert(
                            id.clone(),
                            Place {
                                body: site.body.clone(),
                                latitude: site.latitude.to_radians(),
                                longitude: site.longitude.to_radians(),
                            },
                        );
                    }
                }
                Condition::Near { target, .. }
                    if !scenario
                        .others
                        .iter()
                        .any(|o| o.name.as_ref().unwrap_or(&o.craft) == target) =>
                {
                    return Err(format!(
                        "goal `{}`: the scenario starts no vessel called `{target}`",
                        def.id
                    ));
                }
                _ => {}
            }
        }
        let goal = |def: &GoalDef| Goal {
            def: def.clone(),
            met_at: None,
            held: 0.0,
        };
        let time = SimTime::from_seconds(scenario.start_time);
        Ok(Some(Self {
            goals: scenario.goals.iter().map(goal).collect(),
            fail: scenario.fail.iter().map(goal).collect(),
            ordered: scenario.ordered,
            places,
            state: State::Running,
            stats: Stats::default(),
            started: time,
            last: time,
            events: Vec::new(),
        }))
    }

    pub fn goals(&self) -> &[Goal] {
        &self.goals
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    pub fn is_ordered(&self) -> bool {
        self.ordered
    }

    pub fn is_running(&self) -> bool {
        self.state == State::Running
    }

    /// The goal being worked on, in an ordered mission: the first not yet met.
    pub fn current(&self) -> Option<usize> {
        self.goals.iter().position(|g| g.met_at.is_none())
    }

    /// What has happened since this was last asked.
    pub fn take_events(&mut self) -> Vec<MissionEvent> {
        std::mem::take(&mut self.events)
    }

    /// Look at the universe as it is now, after a tick (`physics`) or a jump on rails. `events`
    /// are what that step reported.
    pub fn observe(&mut self, universe: &Universe, events: &[SimEvent], physics: bool) {
        if self.state != State::Running {
            return;
        }
        let Some(vessel) = universe.focus.and_then(|id| universe.vessel(id)) else {
            return;
        };
        let now = universe.time();
        let dt = now.seconds_since(self.last).max(0.0);
        self.last = now;
        self.record(universe, vessel, events, physics, dt);

        let view = View {
            universe,
            vessel,
            stats: &self.stats,
            places: &self.places,
        };
        let ordered = self.ordered;
        let elapsed = self.stats.elapsed;
        let mut met = Vec::new();
        for (i, goal) in self.goals.iter_mut().enumerate() {
            if goal.met_at.is_some() {
                continue;
            }
            if goal.advance(&view, dt, elapsed) {
                met.push(i);
            } else if ordered {
                break;
            }
        }
        let failed = self
            .fail
            .iter_mut()
            .find_map(|f| f.advance(&view, dt, elapsed).then(|| f.def.text.clone()));
        self.events
            .extend(met.into_iter().map(MissionEvent::GoalMet));
        if self.goals.iter().all(|g| g.met_at.is_some()) {
            self.state = State::Passed;
            self.events.push(MissionEvent::Passed);
        } else if let Some(reason) = failed {
            self.state = State::Failed(reason.clone());
            self.events.push(MissionEvent::Failed(reason));
        }
    }

    fn record(
        &mut self,
        universe: &Universe,
        vessel: &Vessel,
        events: &[SimEvent],
        physics: bool,
        dt: f64,
    ) {
        let stats = &mut self.stats;
        stats.elapsed = universe.time().seconds_since(self.started);
        let state = vessel.state_at(&universe.system, universe.time());
        let body = universe.system.body(vessel.parent());
        stats.max_altitude = stats
            .max_altitude
            .max(state.position.length() - body.radius);
        if physics {
            let flight = &vessel.flight;
            if flight.mass > 0.0 {
                stats.delta_v += flight.thrust / flight.mass * dt;
            }
            stats.peak_g = stats.peak_g.max(flight.proper_acceleration / G0);
        }
        if vessel.situation(&universe.system, universe.time()) != Situation::Landed {
            stats.flown = true;
        }
        for event in events.iter().filter(|e| e.vessel == vessel.id()) {
            match event.event.kind {
                VesselEventKind::Crashed { .. } => stats.crashed = true,
                VesselEventKind::Destroyed => stats.destroyed = true,
                _ => {}
            }
        }
        stats.destroyed |= vessel.is_destroyed();
    }
}

impl Goal {
    /// Look at the condition for `dt` more seconds; true the moment the goal is met.
    fn advance(&mut self, view: &View, dt: f64, elapsed: f64) -> bool {
        if !view.holds(&self.def.when) {
            self.held = 0.0;
            return false;
        }
        self.held += dt;
        if self.held >= self.def.hold {
            self.met_at = Some(elapsed);
            return true;
        }
        false
    }
}

/// Everything a condition looks at.
struct View<'a> {
    universe: &'a Universe,
    vessel: &'a Vessel,
    stats: &'a Stats,
    places: &'a HashMap<String, Place>,
}

impl View<'_> {
    fn system(&self) -> &SolarSystem {
        &self.universe.system
    }

    fn body(&self) -> &Body {
        self.system().body(self.vessel.parent())
    }

    fn holds(&self, condition: &Condition) -> bool {
        let time = self.universe.time();
        let state = self.vessel.state_at(self.system(), time);
        let body = self.body();
        match condition {
            Condition::Altitude { min, max } => {
                Range::new(*min, *max).contains(state.position.length() - body.radius)
            }
            Condition::Speed { min, max, orbital } => {
                let velocity = if *orbital {
                    state.velocity
                } else {
                    state.velocity - body.surface_velocity(state.position)
                };
                Range::new(*min, *max).contains(velocity.length())
            }
            Condition::Orbit {
                body: name,
                apoapsis,
                periapsis,
                inclination,
            } => {
                if &body.name != name
                    || self.vessel.situation(self.system(), time) != Situation::Orbiting
                {
                    return false;
                }
                let orbit = self.vessel.orbit(self.system(), time);
                let Some(apo) = orbit.apoapsis() else {
                    return false;
                };
                apoapsis.contains(apo - body.radius)
                    && periapsis.contains(orbit.periapsis() - body.radius)
                    && inclination.contains(orbit.inclination_to(body.pole()).to_degrees())
            }
            Condition::Near {
                target,
                distance,
                speed,
            } => {
                let other = self
                    .universe
                    .vessels
                    .iter()
                    .find(|v| &v.name == target && !v.is_debris());
                let Some(other) = other else {
                    return false;
                };
                let theirs = self.system().transform(
                    other.state_at(self.system(), time),
                    other.parent(),
                    self.vessel.parent(),
                    time,
                );
                (theirs.position - state.position).length() <= *distance
                    && speed
                        .is_none_or(|limit| (theirs.velocity - state.velocity).length() <= limit)
            }
            Condition::Landed {
                body: on,
                site,
                radius,
            } => {
                if !self.on_the_ground() {
                    return false;
                }
                if on.as_ref().is_some_and(|name| &body.name != name) {
                    return false;
                }
                let Some(place) = site.as_ref().and_then(|id| self.places.get(id)) else {
                    return true;
                };
                let here = body.lat_lon(state.position, time);
                let angle = angle_between(here, (place.latitude, place.longitude));
                body.name == place.body && angle * body.radius <= radius.unwrap_or(SITE_RADIUS)
            }
            Condition::Recovered { max_g } => {
                self.on_the_ground()
                    && self.stats.flown
                    && max_g.is_none_or(|limit| self.stats.peak_g <= limit)
            }
            Condition::Propellant { min, max } => {
                let p = &self.vessel.propulsion;
                let left: f64 = p.sections.iter().map(|s| p.propellant_mass(s.id)).sum();
                Range::new(*min, *max).contains(left)
            }
            Condition::Destroyed => self.stats.destroyed || self.stats.crashed,
            Condition::Elapsed { seconds } => self.stats.elapsed >= *seconds,
        }
    }

    /// Standing on the ground in one piece.
    fn on_the_ground(&self) -> bool {
        !(self.stats.crashed || self.stats.destroyed)
            && self.vessel.situation(self.system(), self.universe.time()) == Situation::Landed
    }
}

/// The angle at the centre between two (latitude, longitude) points, rad.
fn angle_between(a: (f64, f64), b: (f64, f64)) -> f64 {
    let unit = |(lat, lon): (f64, f64)| Body::surface_normal_fixed(lat, lon);
    unit(a).angle_between(unit(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        data::OrbitStartDef,
        system::tests::rss,
        test_data::records,
        vessel::tests::{catalog, launch_sites, leo_vessel, pad_vessel, upper_stage},
    };

    fn scenario(goals: &str) -> ScenarioDef {
        let text = format!(
            r#"{{ id: "t", start_time: 0, craft: "x", start: {{ type: "LaunchSite", site: "cape_canaveral" }}, {goals} }}"#
        );
        records(&text).remove(0)
    }

    /// The mission that starts now, in `universe`.
    fn mission(goals: &str, universe: &Universe) -> Mission {
        let mut scenario = scenario(goals);
        scenario.start_time = universe.time().as_seconds();
        Mission::new(&scenario, &launch_sites(), &universe.system)
            .unwrap()
            .unwrap()
    }

    /// The Pathfinder on the pad, and an upper stage in a 300 km orbit.
    fn on_the_pad() -> Universe {
        let (pad, t) = pad_vessel(&rss());
        let mut universe = Universe::new(rss(), vec![pad], t);
        universe.focus = Some(universe.vessels[0].id());
        universe
    }

    fn in_orbit() -> Universe {
        let (leo, t) = leo_vessel(&rss());
        let mut universe = Universe::new(rss(), vec![leo], t);
        universe.focus = Some(universe.vessels[0].id());
        universe
    }

    /// Whether `when` holds for the focus vessel, with the mission's statistics `stats`.
    fn holds(universe: &Universe, stats: Stats, when: &str) -> bool {
        let condition: Condition = serde_json::from_value(json5::from_str(when).unwrap()).unwrap();
        let places = [(
            "cape_canaveral".to_owned(),
            Place {
                body: "Earth".into(),
                latitude: 28.6082f64.to_radians(),
                longitude: (-80.6041f64).to_radians(),
            },
        )]
        .into();
        View {
            universe,
            vessel: universe.vessel(universe.focus.unwrap()).unwrap(),
            stats: &stats,
            places: &places,
        }
        .holds(&condition)
    }

    fn now(universe: &Universe, when: &str) -> bool {
        holds(universe, Stats::default(), when)
    }

    #[test]
    fn a_range_may_leave_either_end_open() {
        let range = Range::new(Some(2.0), None);
        assert!(range.contains(2.0) && range.contains(1e9) && !range.contains(1.9));
        let range = Range::new(None, Some(2.0));
        assert!(range.contains(-1e9) && range.contains(2.0) && !range.contains(2.1));
        assert!(Range::default().contains(0.0));
    }

    #[test]
    fn altitude_and_speed_look_at_where_and_how_fast() {
        let orbit = in_orbit();
        assert!(now(
            &orbit,
            "{ type: 'Altitude', min: 250000, max: 350000 }"
        ));
        assert!(!now(&orbit, "{ type: 'Altitude', max: 100000 }"));
        assert!(now(
            &orbit,
            "{ type: 'Speed', min: 7000, max: 8000, orbital: true }"
        ));
        assert!(!now(&orbit, "{ type: 'Speed', max: 100, orbital: true }"));
        // Over the ground, the pad stands still and an orbit does not.
        let pad = on_the_pad();
        assert!(now(&pad, "{ type: 'Altitude', max: 100 }"));
        assert!(now(&pad, "{ type: 'Speed', max: 1 }"));
        assert!(!now(&pad, "{ type: 'Altitude', min: 1000 }"));
        assert!(!now(&orbit, "{ type: 'Speed', max: 1 }"));
    }

    #[test]
    fn an_orbit_has_a_body_apsides_and_an_inclination() {
        let orbit = in_orbit();
        let when = |extra: &str| format!("{{ type: 'Orbit', body: 'Earth', {extra} }}");
        assert!(now(&orbit, &when("")));
        assert!(now(
            &orbit,
            &when("periapsis: { min: 250000 }, apoapsis: { max: 350000 }")
        ));
        assert!(now(&orbit, &when("inclination: { min: 28, max: 29 }")));
        assert!(!now(&orbit, &when("periapsis: { min: 400000 }")));
        assert!(!now(&orbit, &when("apoapsis: { max: 200000 }")));
        assert!(!now(&orbit, &when("inclination: { min: 50 }")));
        assert!(!now(&orbit, "{ type: 'Orbit', body: 'Moon' }"));
        // Standing on the pad is not an orbit.
        assert!(!now(&on_the_pad(), &when("")));
    }

    #[test]
    fn near_a_vessel_is_a_distance_and_a_relative_speed() {
        let system = rss();
        let (mut chaser, t) = leo_vessel(&system);
        let start = |anomaly: f64| OrbitStartDef {
            body: "Earth".into(),
            periapsis_altitude: 300e3,
            apoapsis_altitude: 300e3,
            inclination: 28.6,
            longitude_of_ascending_node: 0.0,
            plane: None,
            argument_of_periapsis: 0.0,
            mean_anomaly: anomaly,
            spin: [0.0; 3],
        };
        chaser.name = "Chaser".into();
        // A hundredth of a degree ahead on the same orbit: about 1.2 km, 1.3 m/s.
        let mut station =
            Vessel::in_orbit(&upper_stage(), &catalog(), &system, &start(0.01), t).unwrap();
        station.name = "Station".into();
        let mut universe = Universe::new(system, vec![chaser, station], t);
        universe.focus = Some(universe.vessels[0].id());
        let near = |distance: f64, speed: &str| {
            format!("{{ type: 'Near', target: 'Station', distance: {distance}{speed} }}")
        };
        assert!(now(&universe, &near(2000.0, "")));
        assert!(now(&universe, &near(2000.0, ", speed: 5")));
        assert!(!now(&universe, &near(500.0, "")));
        assert!(!now(&universe, &near(2000.0, ", speed: 0.5")));
        assert!(!now(
            &universe,
            "{ type: 'Near', target: 'Nobody', distance: 1e9 }"
        ));
    }

    #[test]
    fn landed_is_on_the_ground_at_a_place() {
        let pad = on_the_pad();
        assert!(now(&pad, "{ type: 'Landed' }"));
        assert!(now(
            &pad,
            "{ type: 'Landed', body: 'Earth', site: 'cape_canaveral' }"
        ));
        assert!(now(
            &pad,
            "{ type: 'Landed', site: 'cape_canaveral', radius: 50 }"
        ));
        assert!(!now(&pad, "{ type: 'Landed', body: 'Moon' }"));
        // Another place on the other side of the country is not here.
        let elsewhere = holds_at(&pad, "white_sands_lc33", 8.0, 33.0);
        assert!(!elsewhere);
        assert!(!now(&in_orbit(), "{ type: 'Landed' }"));
        // A crash is not a landing.
        let crashed = Stats {
            crashed: true,
            ..Stats::default()
        };
        assert!(!holds(&pad, crashed, "{ type: 'Landed' }"));
    }

    /// `Landed` at a site at (`latitude`, `longitude`) degrees, as a vessel on the pad sees it.
    fn holds_at(universe: &Universe, id: &str, latitude: f64, longitude: f64) -> bool {
        let condition = Condition::Landed {
            body: None,
            site: Some(id.into()),
            radius: None,
        };
        let places = [(
            id.to_owned(),
            Place {
                body: "Earth".into(),
                latitude: latitude.to_radians(),
                longitude: longitude.to_radians(),
            },
        )]
        .into();
        View {
            universe,
            vessel: universe.vessel(universe.focus.unwrap()).unwrap(),
            stats: &Stats::default(),
            places: &places,
        }
        .holds(&condition)
    }

    #[test]
    fn recovered_is_back_down_in_one_piece_after_flying_gently() {
        let pad = on_the_pad();
        let flown = |peak_g| Stats {
            flown: true,
            peak_g,
            ..Stats::default()
        };
        let recovered = "{ type: 'Recovered', max_g: 8 }";
        assert!(holds(&pad, flown(4.0), recovered));
        assert!(holds(&pad, flown(4.0), "{ type: 'Recovered' }"));
        // Too hard on the crew, or never left the ground, or in pieces, or still up there.
        assert!(!holds(&pad, flown(9.0), recovered));
        assert!(!holds(&pad, Stats::default(), recovered));
        let broken = Stats {
            destroyed: true,
            ..flown(4.0)
        };
        assert!(!holds(&pad, broken, recovered));
        assert!(!holds(&in_orbit(), flown(4.0), recovered));
    }

    #[test]
    fn propellant_left_is_a_mass() {
        let pad = on_the_pad();
        assert!(now(&pad, "{ type: 'Propellant', min: 100000 }"));
        assert!(!now(&pad, "{ type: 'Propellant', max: 100 }"));
        // An upper stage has little compared with the whole rocket.
        assert!(now(&in_orbit(), "{ type: 'Propellant', max: 100000 }"));
    }

    #[test]
    fn destroyed_and_elapsed_look_at_what_happened() {
        let pad = on_the_pad();
        assert!(!now(&pad, "{ type: 'Destroyed' }"));
        let crashed = Stats {
            crashed: true,
            ..Stats::default()
        };
        assert!(holds(&pad, crashed, "{ type: 'Destroyed' }"));
        let torn = Stats {
            destroyed: true,
            ..Stats::default()
        };
        assert!(holds(&pad, torn, "{ type: 'Destroyed' }"));
        let ten = Stats {
            elapsed: 10.0,
            ..Stats::default()
        };
        assert!(holds(&pad, ten, "{ type: 'Elapsed', seconds: 10 }"));
        assert!(!holds(&pad, ten, "{ type: 'Elapsed', seconds: 11 }"));
    }

    /// `goal` looks at `universe` for `dt` more seconds.
    fn advance(goal: &mut Goal, universe: &Universe, dt: f64, elapsed: f64) -> bool {
        let (stats, places) = (Stats::default(), HashMap::new());
        let view = View {
            universe,
            vessel: universe.vessel(universe.focus.unwrap()).unwrap(),
            stats: &stats,
            places: &places,
        };
        goal.advance(&view, dt, elapsed)
    }

    #[test]
    fn a_goal_held_for_a_time_starts_over_when_it_lets_go() {
        let (high, low) = (in_orbit(), on_the_pad());
        let mut goal = Goal {
            def: GoalDef {
                id: "high".into(),
                text: "Stay high".into(),
                when: Condition::Altitude {
                    min: Some(200e3),
                    max: None,
                },
                hold: 1.0,
            },
            met_at: None,
            held: 0.0,
        };
        assert!(!advance(&mut goal, &high, 0.5, 1.0));
        assert!(!advance(&mut goal, &low, 0.5, 1.5));
        assert_eq!(goal.held, 0.0);
        assert!(!advance(&mut goal, &high, 0.5, 2.0));
        assert!(advance(&mut goal, &high, 0.5, 2.5));
        assert_eq!(goal.met_at, Some(2.5));
    }

    /// Steps `universe` for `seconds` and returns what the mission said.
    fn run(universe: &mut Universe, seconds: f64) -> Vec<MissionEvent> {
        for _ in 0..(seconds * 50.0).round() as usize {
            universe.step_physics();
        }
        universe.mission.as_mut().unwrap().take_events()
    }

    const TWO_GOALS: &str = r#"goals: [
        { id: "wait", text: "Wait", when: { type: "Elapsed", seconds: 2 } },
        { id: "high", text: "Be high", when: { type: "Altitude", min: 200000 } },
    ]"#;

    #[test]
    fn goals_are_met_in_any_order_unless_the_scenario_says_otherwise() {
        let mut universe = in_orbit();
        universe.mission = Some(mission(TWO_GOALS, &universe));
        // The second is true from the start; the first takes two seconds.
        assert_eq!(run(&mut universe, 1.0), [MissionEvent::GoalMet(1)]);
        assert_eq!(
            run(&mut universe, 1.5),
            [MissionEvent::GoalMet(0), MissionEvent::Passed]
        );
        let mission = universe.mission.as_ref().unwrap();
        assert_eq!(*mission.state(), State::Passed);
        assert_eq!(mission.goals()[1].met_at.map(|t| t < 0.1), Some(true));

        let mut universe = in_orbit();
        universe.mission = Some(mission_ordered(&universe));
        // The second waits for the first, then is met on the same tick.
        assert!(run(&mut universe, 1.0).is_empty());
        assert_eq!(universe.mission.as_ref().unwrap().current(), Some(0));
        assert_eq!(
            run(&mut universe, 1.5),
            [
                MissionEvent::GoalMet(0),
                MissionEvent::GoalMet(1),
                MissionEvent::Passed
            ]
        );
    }

    fn mission_ordered(universe: &Universe) -> Mission {
        mission(&format!("ordered: true, {TWO_GOALS}"), universe)
    }

    #[test]
    fn a_fail_condition_ends_the_mission_and_stops_the_clock() {
        let mut universe = in_orbit();
        universe.mission = Some(mission(
            r#"goals: [{ id: "moon", text: "Reach the Moon", when: { type: "Altitude", min: 3e8 } }],
               fail: [{ id: "late", text: "Out of time", when: { type: "Elapsed", seconds: 1 } }]"#,
            &universe,
        ));
        assert_eq!(
            run(&mut universe, 2.0),
            [MissionEvent::Failed("Out of time".into())]
        );
        let ended = *universe.mission.as_ref().unwrap().stats();
        assert_eq!(
            *universe.mission.as_ref().unwrap().state(),
            State::Failed("Out of time".into())
        );
        assert!((1.0..1.1).contains(&ended.elapsed), "{}", ended.elapsed);
        // Nothing moves it any more.
        assert!(run(&mut universe, 1.0).is_empty());
        assert_eq!(*universe.mission.as_ref().unwrap().stats(), ended);
    }

    #[test]
    fn the_stats_follow_the_flight() {
        let mut universe = on_the_pad();
        universe.mission = Some(mission(
            r#"goals: [{ id: "up", text: "Climb", when: { type: "Altitude", min: 1e6 } }]"#,
            &universe,
        ));
        universe.vessels[0].set_throttle(1.0);
        universe.activate_next_stage(0);
        run(&mut universe, 8.0);
        let stats = *universe.mission.as_ref().unwrap().stats();
        assert!((7.9..8.1).contains(&stats.elapsed));
        assert!(stats.delta_v > 20.0, "{stats:?}");
        assert!(stats.peak_g > 1.2, "{stats:?}");
        assert!(stats.max_altitude > 10.0, "{stats:?}");
        assert!(
            stats.flown && !stats.crashed && !stats.destroyed,
            "{stats:?}"
        );
    }

    #[test]
    fn a_jump_on_rails_counts_for_the_clock_but_not_the_engines() {
        let mut universe = in_orbit();
        universe.mission = Some(mission(
            r#"goals: [{ id: "wait", text: "Wait a minute", when: { type: "Elapsed", seconds: 60 } }]"#,
            &universe,
        ));
        let target = universe.time().add_seconds(100.0);
        universe.advance_rails(target);
        let mission = universe.mission.as_ref().unwrap();
        assert_eq!(*mission.state(), State::Passed);
        assert!((99.9..100.1).contains(&mission.stats().elapsed));
        assert_eq!(mission.stats().delta_v, 0.0);
    }

    #[test]
    fn a_goal_naming_something_that_is_not_there_stops_the_scenario_starting() {
        let universe = in_orbit();
        let sites = launch_sites();
        let error =
            |goals: &str| Mission::new(&scenario(goals), &sites, &universe.system).unwrap_err();
        let goal = |when: &str| format!(r#"goals: [{{ id: "g", text: "G", when: {when} }}]"#);
        assert!(error(&goal("{ type: 'Landed', site: 'nowhere' }")).contains("nowhere"));
        assert!(error(&goal("{ type: 'Landed', body: 'Vulcan' }")).contains("Vulcan"));
        assert!(error(&goal("{ type: 'Orbit', body: 'Vulcan' }")).contains("Vulcan"));
        assert!(
            error(&goal("{ type: 'Near', target: 'Station', distance: 10 }")).contains("Station")
        );
        // No goals, no mission.
        assert!(
            Mission::new(&scenario(""), &sites, &universe.system)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_scenario_record_lists_goals_in_the_documented_syntax() {
        let scenario = scenario(
            r#"
            ordered: true,
            goals: [
              { id: "liftoff", text: "Leave the pad", when: { type: "Altitude", min: 100 } },
              { id: "orbit", text: "Reach orbit", hold: 5,
                when: { type: "Orbit", body: "Earth", periapsis: { min: 150000 }, apoapsis: { max: 600000 } } },
            ],
            fail: [{ id: "lost", text: "The rocket was lost", when: { type: "Destroyed" } }]"#,
        );
        assert!(scenario.ordered && scenario.goals.len() == 2 && scenario.fail.len() == 1);
        assert_eq!(scenario.goals[1].hold, 5.0);
        assert_eq!(
            scenario.goals[1].when,
            Condition::Orbit {
                body: "Earth".into(),
                apoapsis: Range::new(None, Some(600_000.0)),
                periapsis: Range::new(Some(150_000.0), None),
                inclination: Range::default(),
            }
        );
        assert_eq!(scenario.fail[0].when, Condition::Destroyed);
    }
}
