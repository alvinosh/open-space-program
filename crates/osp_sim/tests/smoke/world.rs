//! The mission world: a universe flown the way a player flies it (throttle, stage key, SAS,
//! RCS, burns on the map), with everything that happens written down for the report.

use std::path::Path;

use osp_sim::{
    DVec3,
    attitude::SasMode,
    data::{CraftDef, LaunchSiteDef, PartCatalog, ScenarioDef, SystemDef},
    system::SolarSystem,
    time::SimTime,
    universe::{PHYSICS_STEP, Universe},
    vessel::{Vessel, VesselEventKind, VesselId},
};
use serde::Serialize;

use crate::record;

/// Sampled once per this many seconds of physics, and once per chunk on rails.
const SAMPLE_EVERY: f64 = 1.0;
/// Longest stretch of rails between samples, s.
const RAILS_CHUNK: f64 = 60.0;

/// What the sim was asked about, so the report can group findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    Build,
    Structure,
    Propulsion,
    Staging,
    Control,
    Aero,
    Orbit,
    Power,
    Avionics,
    Comms,
    Thermal,
    Recovery,
    Docking,
    Landing,
    Crew,
}

impl Area {
    pub fn name(self) -> &'static str {
        match self {
            Area::Build => "Build",
            Area::Structure => "Structure",
            Area::Propulsion => "Propulsion",
            Area::Staging => "Staging",
            Area::Control => "Guidance & control",
            Area::Aero => "Aerodynamics",
            Area::Orbit => "Orbits & navigation",
            Area::Power => "Electrical power",
            Area::Avionics => "Avionics",
            Area::Comms => "Communications",
            Area::Thermal => "Heating & entry",
            Area::Recovery => "Recovery",
            Area::Docking => "Docking",
            Area::Landing => "Landing",
            Area::Crew => "Crew & life support",
        }
    }
}

/// How a check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The sim did what the real vehicle did.
    Pass,
    /// It did not: a bug to fix. Fails the test.
    Fail,
    /// It did not, and the task tracker knows (`.tasks/`). Does not fail the test.
    Known,
    /// The sim has nothing for this yet (no model to check against).
    Gap,
    /// The script stepped over something the sim cannot do, to go on with the rest.
    Bridged,
    /// A measurement with no pass or fail.
    Info,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub phase: Option<String>,
    pub area: Area,
    pub status: Status,
    pub title: String,
    pub detail: String,
    /// Seconds since liftoff (negative before), if it happened at a moment.
    pub met: Option<f64>,
}

/// One row of telemetry.
#[derive(Debug, Clone, Serialize)]
pub struct Sample {
    /// Seconds since the scenario started.
    pub t: f64,
    pub body: String,
    /// Above the mean radius of the body it is near, m.
    pub altitude: f64,
    /// Relative to the ground (or the air), m/s.
    pub speed: f64,
    /// Relative to the body's centre, not rotating, m/s.
    pub orbital_speed: f64,
    pub vertical_speed: f64,
    pub mass: f64,
    pub thrust: f64,
    pub throttle: f64,
    /// Proper acceleration, in g.
    pub g: f64,
    /// Dynamic pressure, Pa.
    pub q: f64,
    pub mach: f64,
    /// Nose above the airflow, degrees.
    pub aoa: f64,
    /// Nose above the local horizon, degrees.
    pub pitch: f64,
    /// Apoapsis and periapsis altitudes above the body, m; no apoapsis on an escape.
    pub apoapsis: Option<f64>,
    pub periapsis: f64,
    pub inclination: f64,
    pub propellant: f64,
    pub rcs_propellant: f64,
    pub load: f64,
    pub engines: u32,
    /// Distance to the centres of the Earth and the Moon, m.
    pub earth: f64,
    pub moon: f64,
}

/// A moment worth marking on the timeline.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub t: f64,
    pub phase: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The script never got this far.
    NotReached,
    Passed,
    /// Some of it ran; the rest was bridged, or the sim has gaps here.
    Partial,
    Failed,
}

/// A stretch of the mission, as the plan lists it.
#[derive(Debug, Clone, Copy)]
pub struct PhaseSpec {
    pub id: &'static str,
    pub title: &'static str,
    /// What the real mission did here.
    pub goal: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct Phase {
    pub id: String,
    pub title: String,
    pub goal: String,
    pub started: Option<f64>,
    pub ended: Option<f64>,
    pub outcome: Outcome,
    /// Mass at the start and end of the phase, kg.
    pub mass_start: f64,
    pub mass_end: f64,
    /// Time the engines gave thrust, s, and the speed that thrust gave, m/s (before gravity
    /// and drag take their share).
    pub burn_time: f64,
    pub delta_v: f64,
}

/// The script stops here: something it needs from the sim did not happen.
#[derive(Debug)]
pub struct Abort;

pub type Flight = Result<(), Abort>;

pub struct World {
    pub universe: Universe,
    pub focus: VesselId,
    pub catalog: PartCatalog,
    pub craft: CraftDef,
    pub scenario: ScenarioDef,
    pub start: SimTime,
    /// When the stack left the pad.
    pub liftoff: Option<SimTime>,
    pub samples: Vec<Sample>,
    pub events: Vec<Event>,
    pub findings: Vec<Finding>,
    pub phases: Vec<Phase>,
    /// What the craft is made of, once looked at.
    pub audit: Option<crate::audit::Audit>,
    current: Option<usize>,
    last_sample: f64,
    seen_log: Option<(SimTime, String)>,
    burn_time: f64,
    delta_v: f64,
    /// Where `burn_time` and `delta_v` stood when the phase began.
    phase_mark: (f64, f64),
    last_trace: f64,
}

impl World {
    /// Loads every mod and starts `scenario` as the game does.
    pub fn new(scenario: &str, plan: &[PhaseSpec]) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
        let registry = &content.registry;
        let catalog = record::catalog(registry);
        let mut diagnostics = Vec::new();
        let system = SystemDef {
            bodies: registry.all("bodies", &mut diagnostics),
        };
        let sites: Vec<LaunchSiteDef> = registry.all("launch_sites", &mut diagnostics);
        let scenario: ScenarioDef = registry.get("scenarios", scenario).unwrap();
        let craft_of = |id: &str| {
            registry
                .get::<CraftDef>("craft", id)
                .map_err(|e| format!("{e:?}"))
        };
        let craft = craft_of(&scenario.craft).unwrap();
        let universe =
            Universe::new_game(&system, Vec::new(), &sites, &catalog, craft_of, &scenario).unwrap();
        let focus = universe.focus.unwrap();
        let start = universe.time();
        let phases = plan
            .iter()
            .map(|p| Phase {
                id: p.id.into(),
                title: p.title.into(),
                goal: p.goal.into(),
                started: None,
                ended: None,
                outcome: Outcome::NotReached,
                mass_start: 0.0,
                mass_end: 0.0,
                burn_time: 0.0,
                delta_v: 0.0,
            })
            .collect();
        let mut world = Self {
            universe,
            focus,
            catalog,
            craft,
            scenario,
            start,
            liftoff: None,
            samples: Vec::new(),
            events: Vec::new(),
            findings: Vec::new(),
            phases,
            audit: None,
            current: None,
            last_sample: f64::NEG_INFINITY,
            seen_log: None,
            burn_time: 0.0,
            delta_v: 0.0,
            phase_mark: (0.0, 0.0),
            last_trace: f64::NEG_INFINITY,
        };
        world.sample();
        world
    }

    // --- the vessel and the clock ---------------------------------------------------------

    pub fn vessel(&self) -> &Vessel {
        let i = self
            .universe
            .find_vessel(self.focus)
            .expect("the vessel is gone");
        &self.universe.vessels[i]
    }

    pub fn vessel_mut(&mut self) -> &mut Vessel {
        let i = self
            .universe
            .find_vessel(self.focus)
            .expect("the vessel is gone");
        &mut self.universe.vessels[i]
    }

    /// Runs `f` on the vessel with the solar system and the time, for what needs all three.
    pub fn with_vessel<R>(&mut self, f: impl FnOnce(&mut Vessel, &SolarSystem, SimTime) -> R) -> R {
        let i = self
            .universe
            .find_vessel(self.focus)
            .expect("the vessel is gone");
        let now = self.universe.time();
        f(&mut self.universe.vessels[i], &self.universe.system, now)
    }

    /// SAS on in `mode`, as the buttons next to the navball do.
    pub fn sas(&mut self, mode: SasMode) {
        self.with_vessel(|v, system, now| v.set_sas_mode(system, now, mode));
    }

    pub fn now(&self) -> SimTime {
        self.universe.time()
    }

    /// Seconds since the scenario started.
    pub fn t(&self) -> f64 {
        self.now().seconds_since(self.start)
    }

    /// Seconds since liftoff (since the start, before it).
    pub fn met(&self) -> f64 {
        self.now().seconds_since(self.liftoff.unwrap_or(self.start))
    }

    // --- flying --------------------------------------------------------------------------

    /// Steps physics ticks until `done` says so, giving `hands` (the pilot) a turn before
    /// each. `false` if `limit` seconds pass first.
    pub fn fly(&mut self, limit: f64, mut hands: impl FnMut(&mut World) -> bool) -> bool {
        let ticks = (limit / PHYSICS_STEP.as_seconds()).ceil() as u64;
        for _ in 0..ticks {
            if hands(self) {
                return true;
            }
            self.tick();
        }
        false
    }

    /// One physics tick for the whole universe.
    pub fn tick(&mut self) {
        let dt = PHYSICS_STEP.as_seconds();
        let (thrust, mass) = {
            let f = &self.vessel().flight;
            (f.thrust, f.mass)
        };
        if thrust > 0.0 && mass > 0.0 {
            self.burn_time += dt;
            self.delta_v += thrust / mass * dt;
        }
        let events = self.universe.step_physics();
        let focus = self.focus;
        for e in events.iter().filter(|e| e.vessel == focus) {
            self.on_event(e.event.kind, e.event.time);
        }
        self.after();
    }

    /// Jumps `seconds` ahead on rails, sampling along the way. `false` if something stopped
    /// it early (a descent, a burn that is due).
    pub fn warp(&mut self, seconds: f64) -> bool {
        let target = self.now().add_seconds(seconds);
        while self.now() < target {
            let next = (self.now().add_seconds(RAILS_CHUNK)).min(target);
            let outcome = self.universe.advance_rails(next);
            let focus = self.focus;
            for e in outcome.events.iter().filter(|e| e.vessel == focus) {
                self.on_event(e.event.kind, e.event.time);
            }
            self.after();
            if outcome.interrupted {
                return false;
            }
        }
        true
    }

    /// Passes up to `limit` seconds, on rails where the vessel allows it and under physics where
    /// it does not (an engine winding down), until `until` holds. `false` if it never did.
    pub fn coast(&mut self, limit: f64, mut until: impl FnMut(&World) -> bool) -> bool {
        let end = self.now().add_seconds(limit);
        while self.now() < end {
            if until(self) {
                return true;
            }
            if self.universe.can_use_rails() {
                self.warp(RAILS_CHUNK.min(end.seconds_since(self.now())));
            } else {
                self.fly(1.0, |_| false);
            }
        }
        until(self)
    }

    /// The stage key.
    pub fn stage(&mut self) {
        let index = self.universe.find_vessel(self.focus).unwrap();
        self.universe.activate_next_stage(index);
        self.after();
    }

    fn on_event(&mut self, kind: VesselEventKind, time: SimTime) {
        let met = time.seconds_since(self.liftoff.unwrap_or(self.start));
        let text = match kind {
            VesselEventKind::Liftoff => {
                self.liftoff = Some(time);
                "Liftoff".to_owned()
            }
            VesselEventKind::SoiChange { from, to } => {
                let system = &self.universe.system;
                format!(
                    "Left the {} sphere of influence for the {}'s",
                    system.body(from).name,
                    system.body(to).name
                )
            }
            VesselEventKind::Descent { body } => {
                format!("Descent towards {}", self.universe.system.body(body).name)
            }
            VesselEventKind::Landed { speed } => format!("Landed at {speed:.1} m/s"),
            VesselEventKind::Crashed { speed } => format!("Crashed at {speed:.0} m/s"),
            VesselEventKind::Destroyed => "Vessel destroyed".to_owned(),
            VesselEventKind::BurnDue => "Burn due".to_owned(),
            VesselEventKind::BurnEnded => return,
        };
        let _ = met;
        self.event(text);
    }

    /// Writes down the vessel's own log lines since the last look, and samples when due.
    fn after(&mut self) {
        let fresh: Vec<_> = {
            let log = &self.vessel().log;
            let from = self
                .seen_log
                .as_ref()
                .and_then(|seen| {
                    log.iter()
                        .rposition(|e| (e.time, &e.message) == (seen.0, &seen.1))
                })
                .map_or(0, |i| i + 1);
            log[from..]
                .iter()
                .map(|e| (e.time, e.message.clone()))
                .collect()
        };
        if let Some(last) = fresh.last() {
            self.seen_log = Some(last.clone());
        }
        for (time, message) in fresh {
            let t = time.seconds_since(self.start);
            self.events.push(Event {
                t,
                phase: self.current.map(|i| self.phases[i].id.clone()),
                text: message,
            });
        }
        let step = if self.vessel().is_coasting() {
            RAILS_CHUNK
        } else {
            SAMPLE_EVERY
        };
        if self.t() - self.last_sample >= step.min(RAILS_CHUNK) - 1e-9 {
            self.sample();
        }
    }

    /// Writes a moment onto the timeline.
    pub fn event(&mut self, text: impl Into<String>) {
        self.events.push(Event {
            t: self.t(),
            phase: self.current.map(|i| self.phases[i].id.clone()),
            text: text.into(),
        });
    }

    pub fn sample(&mut self) {
        let sample = self.measure();
        self.last_sample = sample.t;
        let every = if sample.thrust > 0.0 || sample.altitude < 300_000.0 {
            10.0
        } else {
            900.0
        };
        if trace() && sample.t - self.last_trace >= every - 1e-6 {
            self.last_trace = sample.t;
            eprintln!(
                "t={:8.0} {:5} alt={:9.0} v={:6.0} vz={:6.0} pitch={:6.1} aoa={:5.1} g={:4.2} q={:6.0} F={:9.0} m={:8.0} ap={:9.0} pe={:9.0} i={:5.1} rcs={:.0}",
                sample.t - self.liftoff.map_or(0.0, |l| l.seconds_since(self.start)),
                sample.body,
                sample.altitude,
                sample.orbital_speed,
                sample.vertical_speed,
                sample.pitch,
                sample.aoa,
                sample.g,
                sample.q,
                sample.thrust,
                sample.mass,
                sample.apoapsis.unwrap_or(f64::NAN),
                sample.periapsis,
                sample.inclination,
                sample.rcs_propellant,
            );
        }
        self.samples.push(sample);
    }

    fn measure(&self) -> Sample {
        let system = &self.universe.system;
        let now = self.now();
        let v = self.vessel();
        let body = system.body(v.parent());
        let state = v.state_at(system, now);
        let up = state.position.normalize_or_zero();
        let ground = state.velocity - body.surface_velocity(state.position);
        let orbit = v.orbit(system, now);
        let nose = v.attitude_at(system, now) * DVec3::Z;
        let earth = system.find("Earth").unwrap();
        let moon = system.find("Moon").unwrap();
        let here = |of| {
            system
                .transform(state, v.parent(), of, now)
                .position
                .length()
        };
        let p = &v.propulsion;
        let f = &v.flight;
        Sample {
            t: now.seconds_since(self.start),
            body: body.name.clone(),
            altitude: state.position.length() - body.radius,
            speed: ground.length(),
            orbital_speed: state.velocity.length(),
            vertical_speed: ground.dot(up),
            mass: f.mass,
            thrust: f.thrust,
            throttle: p.throttle(),
            g: f.proper_acceleration / 9.80665,
            q: f.dynamic_pressure,
            mach: f.mach,
            aoa: f.angle_of_attack.to_degrees(),
            pitch: nose.dot(up).clamp(-1.0, 1.0).asin().to_degrees(),
            apoapsis: orbit.apoapsis().map(|a| a - body.radius),
            periapsis: orbit.periapsis() - body.radius,
            inclination: orbit.inclination_to(body.pole()).to_degrees(),
            propellant: p.sections.iter().map(|s| p.propellant_mass(s.id)).sum(),
            rcs_propellant: p.rcs_propellant_mass(),
            load: f.structure_load,
            engines: p.engines.iter().filter(|e| e.running).count() as u32,
            earth: here(earth),
            moon: here(moon),
        }
    }

    // --- phases --------------------------------------------------------------------------

    /// Ends the phase under way and begins `id`, which the plan must list.
    pub fn phase(&mut self, id: &str) {
        self.end_phase();
        let i = self
            .phases
            .iter()
            .position(|p| p.id == id)
            .unwrap_or_else(|| panic!("the plan has no phase `{id}`"));
        let t = self.t();
        let mass = self.vessel().flight.mass;
        let phase = &mut self.phases[i];
        phase.started = Some(t);
        phase.mass_start = mass;
        phase.outcome = Outcome::Passed;
        self.current = Some(i);
        self.phase_mark = (self.burn_time, self.delta_v);
        let rcs = self.vessel().propulsion.rcs_propellant_mass();
        self.event(format!(
            "— {} — ({rcs:.0} kg of RCS propellant)",
            self.phases[i].title
        ));
    }

    /// Time the engines have given thrust in the phase under way, s, and the speed it gave.
    pub fn progress(&self) -> (f64, f64) {
        (
            self.burn_time - self.phase_mark.0,
            self.delta_v - self.phase_mark.1,
        )
    }

    pub fn end_phase(&mut self) {
        let Some(i) = self.current.take() else { return };
        let (t, mass) = (self.t(), self.vessel().flight.mass);
        let (burn, dv) = (
            self.burn_time - self.phase_mark.0,
            self.delta_v - self.phase_mark.1,
        );
        let phase = &mut self.phases[i];
        phase.ended = Some(t);
        phase.mass_end = mass;
        phase.burn_time = burn;
        phase.delta_v = dv;
    }

    /// The largest `f` over the samples taken in phase `id`, and when (MET).
    pub fn peak(&self, id: &str, f: impl Fn(&Sample) -> f64) -> Option<(f64, f64)> {
        let phase = self.phases.iter().find(|p| p.id == id)?;
        let (from, to) = (phase.started?, phase.ended.unwrap_or(f64::INFINITY));
        let liftoff = self.liftoff.map_or(0.0, |l| l.seconds_since(self.start));
        self.samples
            .iter()
            .filter(|s| s.t >= from && s.t <= to)
            .map(|s| (f(s), s.t - liftoff))
            .max_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// Seconds since liftoff at which phase `id` began and how long it lasted.
    pub fn span(&self, id: &str) -> Option<(f64, f64)> {
        let phase = self.phases.iter().find(|p| p.id == id)?;
        let liftoff = self.liftoff.map_or(0.0, |l| l.seconds_since(self.start));
        let from = phase.started?;
        Some((from - liftoff, phase.ended.unwrap_or(self.t()) - from))
    }

    pub fn phase_id(&self) -> Option<&str> {
        self.current.map(|i| self.phases[i].id.as_str())
    }

    // --- findings ------------------------------------------------------------------------

    fn add(&mut self, area: Area, status: Status, title: &str, detail: String) {
        let met = Some(self.met());
        let phase = self.phase_id().map(str::to_owned);
        if let Some(i) = self.current {
            let outcome = &mut self.phases[i].outcome;
            *outcome = match (status, *outcome) {
                (Status::Fail, _) | (_, Outcome::Failed) => Outcome::Failed,
                (Status::Known | Status::Gap | Status::Bridged, _) | (_, Outcome::Partial) => {
                    Outcome::Partial
                }
                _ => Outcome::Passed,
            };
        }
        self.findings.push(Finding {
            phase,
            area,
            status,
            title: title.to_owned(),
            detail,
            met,
        });
    }

    /// The sim did what it should (or did not).
    pub fn check(&mut self, area: Area, title: &str, ok: bool, detail: impl Into<String>) -> bool {
        let status = if ok { Status::Pass } else { Status::Fail };
        self.add(area, status, title, detail.into());
        ok
    }

    /// A check the script cannot go on without.
    pub fn must(
        &mut self,
        area: Area,
        title: &str,
        ok: bool,
        detail: impl Into<String>,
    ) -> Result<(), Abort> {
        if self.check(area, title, ok, detail) {
            Ok(())
        } else {
            self.end_phase();
            Err(Abort)
        }
    }

    /// `value` should lie in `range`; `real` says what the real mission measured.
    pub fn within(
        &mut self,
        area: Area,
        title: &str,
        value: f64,
        range: std::ops::RangeInclusive<f64>,
        unit: &str,
        real: &str,
    ) -> bool {
        let ok = range.contains(&value);
        let detail = format!(
            "{} {unit} (expected {} to {} {unit}; {real})",
            round(value),
            round(*range.start()),
            round(*range.end())
        );
        self.check(area, title, ok, detail)
    }

    /// Something the sim gets wrong that a task in `.tasks/` already covers (`task` is its id).
    pub fn known(&mut self, area: Area, title: &str, task: &str, detail: impl Into<String>) {
        self.add(
            area,
            Status::Known,
            title,
            format!("{} ({task})", detail.into()),
        );
    }

    /// The sim has no model of this.
    pub fn gap(&mut self, area: Area, title: &str, detail: impl Into<String>) {
        self.add(area, Status::Gap, title, detail.into());
    }

    /// The script went on without the sim doing this.
    pub fn bridged(&mut self, area: Area, title: &str, detail: impl Into<String>) {
        self.add(area, Status::Bridged, title, detail.into());
    }

    pub fn info(&mut self, area: Area, title: &str, detail: impl Into<String>) {
        self.add(area, Status::Info, title, detail.into());
    }

    pub fn failures(&self) -> Vec<&Finding> {
        self.findings
            .iter()
            .filter(|f| f.status == Status::Fail)
            .collect()
    }

    /// Closes the last phase and the books.
    pub fn finish(&mut self) {
        self.end_phase();
        self.sample();
    }
}

/// Three significant figures, no trailing noise.
pub fn round(x: f64) -> String {
    if x == 0.0 {
        return "0".into();
    }
    let digits = (2 - x.abs().log10().floor() as i32).clamp(0, 6) as usize;
    format!("{x:.digits$}")
}

/// `OSP_SMOKE_TRACE=1` prints a line of telemetry every ten seconds, for calibrating a script.
fn trace() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| std::env::var_os("OSP_SMOKE_TRACE").is_some())
}
