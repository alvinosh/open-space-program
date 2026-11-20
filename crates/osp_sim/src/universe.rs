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

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use serde::{Deserialize, Serialize};

use crate::career::{Career, CraftSize};
use thiserror::Error;

use crate::{
    alarm::AlarmClock,
    attitude::SasMode,
    body::BodyId,
    collision::Colliders,
    comms::{self, Link, OnTheAir, Station, best_link},
    contact,
    contact::{Contacts, Member},
    data::{
        CraftDef, GroundStationDef, LaunchSiteDef, PartCatalog, ScenarioDef, StartDef, SystemDef,
    },
    docking::{
        Alignment, DockingPort, LATCH_ANGLE, LATCH_GAP, LATCH_TIME, PortApproach, PortState,
        RETRACT, roll_between,
    },
    mission::Mission,
    orbit::{Orbit, StateVector},
    propulsion::Port,
    rendezvous::Target,
    science::Returned,
    staging::{Activation, Sequences},
    system::{SolarSystem, SystemError},
    terrain::{self, HeightSource},
    time::{SimTime, Ticks},
    tutorial::Tutorial,
    vessel::{Situation, Vessel, VesselError, VesselEvent, VesselEventKind, VesselId},
};

/// Fixed physics tick.
pub const PHYSICS_STEP: Ticks = Ticks::from_millis(20);
/// Vessels further apart than this have no ports near each other, m.
const CAPTURE_SEARCH: f64 = 500.0;
/// Rails hand over to physics this far from a vessel the flown one is closing on, m, so that
/// nothing that happens between them is jumped over.
const CLOSE_CALL: f64 = 200.0;
/// Closing faster than this is passing by, not coming to meet, m/s, and slower is not closing.
const MEETING_SPEED: f64 = 100.0;
const NOT_CLOSING: f64 = 1e-3;
/// A vessel that closes more slowly than this gets no word of it in the log, m/s.
const QUIET_BUMP: f64 = 0.03;
/// How far apart two vessels must have been before they can bump, m. A stage just let go is flush
/// with what it left, and may yet drift back on its last thrust.
const DAYLIGHT: f64 = 5.0;
/// Vessels further apart than this are not worth a closer look, m.
const FAR_APART: f64 = 2_000.0;
/// How near two vessels' parts come, beyond how far they can go in a tick, before they are handed to
/// the contact solver, m.
const CONTACT_REACH: f64 = 1.0;
/// Closing faster than this, what hits and what is hit both break up, m/s.
const BREAK_SPEED: f64 = 15.0;

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
    #[error("{0}")]
    Mission(String),
}

/// Two ports that have caught, being drawn together.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SoftCapture {
    /// The vessel that stays the vessel, and its port.
    stay: VesselId,
    at: DockingPort,
    leave: VesselId,
    to: DockingPort,
    since: SimTime,
}

/// An event attributed to a vessel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimEvent {
    pub vessel: VesselId,
    pub event: VesselEvent,
}

/// Result of a rails advance.
/// The longest span of a jump on rails that data is sent over at one rate, s.
const SENDING_SPAN: f64 = 600.0;
/// Spans an orbit is cut into for sending, at least: some 15 s in a low orbit, a few short of a
/// pass over a station.
const SPANS_PER_ORBIT: f64 = 360.0;

/// What a vessel brought home ([`Universe::recover_vessel`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Recovered {
    pub name: String,
    /// Data on its drives, B.
    pub data: f64,
    /// Samples on its drives, kg.
    pub samples: f64,
    /// What it held of each subject.
    pub sets: BTreeMap<String, Returned>,
    /// What that earned, in a career.
    pub science: f64,
}

impl Recovered {
    /// What to tell the player.
    pub fn notice(&self) -> String {
        let brought = self.brought();
        if self.science > 0.0 {
            format!("{brought}: {:.1} science", self.science)
        } else {
            brought
        }
    }

    fn brought(&self) -> String {
        let name = &self.name;
        match (self.data > 0.0, self.samples > 0.0) {
            (false, false) => format!("Recovered {name}"),
            (true, false) => format!("Recovered {name} with {:.1} kB of data", self.data / 1e3),
            (false, true) => format!("Recovered {name} with {:.2} kg of samples", self.samples),
            (true, true) => format!(
                "Recovered {name} with {:.1} kB of data and {:.2} kg of samples",
                self.data / 1e3,
                self.samples
            ),
        }
    }
}

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
    /// What the scenario sets out to do, judged after every step (see [`crate::mission`]).
    pub mission: Option<Mission>,
    /// The dishes on the ground that vessels talk to.
    pub stations: Vec<Station>,
    /// Time warp stops where the focused vessel gains or loses its link.
    pub warp_stops_at_links: bool,
    /// Data goes home at the rate each vessel's link carries ([`comms::rate`]), as in a Realistic
    /// game; otherwise all of it at once whenever there is a link. The game's rules say which
    /// ([`crate::data::RulesDef::data_rates`]).
    pub data_rates: bool,
    /// Physics ticks until the links are looked at again.
    link_ticks: u32,
    /// The guided run the player is on, if any (see [`crate::tutorial`]). A restart ends it.
    pub tutorial: Option<Tutorial>,
    /// Pairs of vessels that have been [`DAYLIGHT`] apart. Only those can bump: a stage or a
    /// vessel that has let go starts out touching what it let go of.
    clear_of: BTreeSet<(VesselId, VesselId)>,
    /// What vessels collide with.
    pub colliders: Colliders,
    /// The contact solver's worlds. Not saved: what they hold comes back within a tick.
    contacts: Contacts,
    /// Ports that have caught and are closing.
    captures: Vec<SoftCapture>,
    /// What the player wants to be stopped for (see [`crate::alarm`]).
    pub alarms: AlarmClock,
    /// The career this is, if it is one: what it has unlocked and has to spend. A sandbox has none
    /// (see [`crate::career`]).
    pub career: Option<Career>,
    /// A vessel only on show, standing in for a place to look at (the space centre's view), and
    /// the vessel in focus before it: not part of the game, so a save leaves it out and keeps the
    /// one before it in focus (see [`Universe::put_on_show`]).
    on_show: Option<(VesselId, Option<VesselId>)>,
    time: SimTime,
    next_id: u64,
}

/// `craft` at `start`, at `time`.
fn spawn_vessel(
    craft: &CraftDef,
    catalog: &PartCatalog,
    system: &SolarSystem,
    sites: &[LaunchSiteDef],
    start: &StartDef,
    time: SimTime,
) -> Result<Vessel, UniverseError> {
    let site = |id: &str| {
        sites
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| UniverseError::UnknownSite(id.to_owned()))
    };
    Ok(match start {
        StartDef::LaunchSite(start) => {
            Vessel::on_launch_site(craft, catalog, system, site(&start.site)?, time)?
        }
        StartDef::Runway(start) => {
            Vessel::on_runway(craft, catalog, system, site(&start.site)?, time)?
        }
        StartDef::Orbit(orbit) => Vessel::in_orbit(craft, catalog, system, orbit, time)?,
        StartDef::Flight(flight) => Vessel::in_flight(craft, catalog, system, flight, time)?,
    })
}

/// Everything that happens in a universe, for saving: all of it but the solar system, the ground
/// stations and the colliders, which come from the records as they are loaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// The bodies it was saved among, in the order vessels' parents number them.
    bodies: Vec<String>,
    time: SimTime,
    next_id: u64,
    link_ticks: u32,
    vessels: Vec<Vessel>,
    focus: Option<VesselId>,
    mission: Option<Mission>,
    tutorial: Option<Tutorial>,
    clear_of: BTreeSet<(VesselId, VesselId)>,
    captures: Vec<SoftCapture>,
    /// Saves from before the alarm clock have none.
    #[serde(default)]
    alarms: AlarmClock,
    /// A sandbox, and saves from before careers, have none.
    #[serde(default)]
    career: Option<Career>,
}

impl Universe {
    pub fn new(system: SolarSystem, vessels: Vec<Vessel>, time: SimTime) -> Self {
        let mut universe = Self {
            system,
            vessels: Vec::new(),
            focus: None,
            stations: Vec::new(),
            warp_stops_at_links: false,
            data_rates: false,
            link_ticks: 0,
            mission: None,
            tutorial: None,
            clear_of: BTreeSet::new(),
            colliders: Colliders::default(),
            contacts: Contacts::default(),
            captures: Vec::new(),
            alarms: AlarmClock::default(),
            career: None,
            on_show: None,
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

    /// What a career's facilities limit of `id` as it stands: its mass with what is in its tanks,
    /// its height along its stack and its width the widest way across (the box round what it
    /// collides with), and its parts.
    pub fn craft_size(&mut self, id: VesselId) -> Option<CraftSize> {
        let index = self.find_vessel(id)?;
        let Self {
            colliders, vessels, ..
        } = self;
        let vessel = &vessels[index];
        let tree = vessel.propulsion.parts();
        let (height, width) = match colliders.of(tree).bounds() {
            Some(bounds) => {
                let span = bounds.max - bounds.min;
                (span.z, span.x.max(span.y))
            }
            None => (vessel.propulsion.length(), 0.0),
        };
        Some(CraftSize {
            mass: vessel.mass_properties().mass,
            height,
            width,
            parts: tree.parts().len(),
        })
    }

    /// Whether `id` can be fetched home: it is down, on the ground or afloat, on a world with one of
    /// `sites`.
    pub fn recoverable(&self, id: VesselId, sites: &[LaunchSiteDef]) -> bool {
        let Some(vessel) = self.vessel(id) else {
            return false;
        };
        let body = &self.system.body(vessel.parent()).name;
        vessel.situation(&self.system, self.time) == Situation::Landed
            && sites.iter().any(|site| &site.body == body)
    }

    /// Bring `id` home: it leaves, and in a career the data and samples on its drives count as
    /// having reached the ground. Whether it can be fetched where it is is the caller's to say
    /// ([`Self::recoverable`]).
    pub fn recover_vessel(&mut self, id: VesselId) -> Option<Recovered> {
        let vessel = self.vessel(id)?;
        let systems = &vessel.propulsion.systems;
        let held = |sample| -> f64 { systems.drives().map(|(_, d)| d.holds(sample)).sum() };
        let mut recovered = Recovered {
            name: vessel.name.clone(),
            data: held(false),
            samples: held(true),
            sets: systems.held(),
            science: 0.0,
        };
        if let Some(career) = &mut self.career {
            recovered.science = career.receive(&recovered.sets);
        }
        self.remove_vessel(id);
        Some(recovered)
    }

    /// Take a vessel out of the game; whoever targeted it loses the target.
    /// Put `vessel` in the game only to be looked from, in focus, in place of any put on show
    /// before: it is not part of the game, so a save leaves it out ([`Universe::snapshot`]) and
    /// keeps the vessel in focus before it. Its id.
    pub fn put_on_show(&mut self, vessel: Vessel) -> VesselId {
        let behind = match self.on_show.take() {
            Some((shown, behind)) => {
                self.remove_vessel(shown);
                behind
            }
            None => self.focus,
        };
        let id = self.add_vessel(vessel);
        self.on_show = Some((id, behind));
        self.focus = Some(id);
        id
    }

    /// Take the vessel on show away, if there is one: the vessel in focus before it is in focus
    /// again, unless another has been put in focus since.
    pub fn take_off_show(&mut self) {
        let Some((shown, behind)) = self.on_show.take() else {
            return;
        };
        self.remove_vessel(shown);
        if self.focus == Some(shown) {
            self.focus = behind.filter(|&id| self.vessel(id).is_some());
        }
    }

    /// The vessel on show, and the vessel in focus before it.
    pub fn on_show(&self) -> Option<(VesselId, Option<VesselId>)> {
        self.on_show
    }

    /// The vessels of the game: every one but the one on show.
    pub fn in_game(&self) -> impl Iterator<Item = &Vessel> {
        let shown = self.on_show.map(|(id, _)| id);
        self.vessels.iter().filter(move |v| Some(v.id()) != shown)
    }

    pub fn remove_vessel(&mut self, id: VesselId) {
        if self.on_show.is_some_and(|(shown, _)| shown == id) {
            self.on_show = None;
        }
        self.vessels.retain(|v| v.id() != id);
        for vessel in &mut self.vessels {
            if vessel.target.and_then(Target::vessel) == Some(id) {
                vessel.target = None;
            }
        }
    }

    /// The vessels standing on the part of `site` a launch needs clear: its runway for `runway`,
    /// otherwise its pad. Without the part it needs nothing stands on it.
    pub fn on_launch_ground(&self, site: &LaunchSiteDef, runway: bool) -> Vec<VesselId> {
        let size = if runway {
            site.runway.as_ref().map(|runway| runway.size)
        } else {
            let pad = site.pad.as_ref().map(|pad| pad.size);
            Some(pad.unwrap_or([BARE_SITE_SIZE; 2]))
        };
        let (Some(size), Some(parent)) = (size, self.system.find(&site.body)) else {
            return Vec::new();
        };
        let body = self.system.body(parent);
        let from_body = body.orientation(self.time).inverse();
        let (center, right, forward) = terrain::site_axes(site);
        let on_it = |vessel: &&Vessel| {
            let at = from_body * vessel.state_at(&self.system, self.time).position;
            let up = at.normalize_or_zero();
            let height = at.length() - body.radius - body.terrain_height(up);
            let along = (up - center) * body.radius;
            height < SITE_CLEARANCE
                && along.dot(forward).abs() <= size[0] / 2.0 + SITE_MARGIN
                && along.dot(right).abs() <= size[1] / 2.0 + SITE_MARGIN
        };
        (self.vessels.iter())
            .filter(|vessel| vessel.parent() == parent)
            .filter(on_it)
            .map(Vessel::id)
            .collect()
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

    /// Begin vessel `index`'s abort, if it has one; what it sheds flies on as debris.
    pub fn abort(&mut self, index: usize) -> Vec<VesselId> {
        let debris = self.vessels[index].abort(&self.system, self.time);
        debris.into_iter().map(|d| self.add_vessel(d)).collect()
    }

    /// Do one activation on vessel `id` by hand (see [`Vessel::perform`]); what it sheds
    /// flies on as debris. Whether the vessel's part offered it.
    pub fn perform(&mut self, id: VesselId, activation: Activation) -> bool {
        let Some(index) = self.find_vessel(id) else {
            return false;
        };
        let Some(debris) = self.vessels[index].perform(activation, &self.system, self.time) else {
            return false;
        };
        for d in debris {
            self.add_vessel(d);
        }
        true
    }

    /// Make vessel `b` part of vessel `a`, `b`'s node `to` joined to `a`'s `at`, the far node
    /// turned `roll` about the line they meet along. `b` is no longer in the game: whoever flew
    /// it flies `a`, and whoever targeted it loses the target. It comes back when the joint is let
    /// go ([`Self::perform`]). Nothing changes on an error.
    pub fn join(
        &mut self,
        a: VesselId,
        at: Port,
        b: VesselId,
        to: Port,
        roll: f64,
    ) -> Result<(), String> {
        if a == b {
            return Err("a vessel cannot join itself".into());
        }
        let (Some(first), Some(second)) = (self.find_vessel(a), self.find_vessel(b)) else {
            return Err("no such vessel".into());
        };
        self.vessels[first].can_join(&self.vessels[second], at, to, roll)?;
        let other = self.vessels.remove(second);
        let first = first - usize::from(second < first);
        let time = self.time;
        self.vessels[first].join(other, at, to, roll, &self.system, time)?;
        for vessel in &mut self.vessels {
            if vessel.target.and_then(Target::vessel) == Some(b) {
                vessel.target = None;
            }
        }
        if self.focus == Some(b) {
            self.focus = Some(a);
        }
        Ok(())
    }

    /// The ports vessel `index` would dock with its target by, if it has targeted a vessel or a
    /// part of one and they have ports that fit: the one it is flown from if that is a port, and
    /// the nearest otherwise, on its target's side the port it has targeted if it has.
    pub fn port_approach(&self, index: usize, time: SimTime) -> Option<PortApproach> {
        let own = self.vessels.get(index)?;
        let id = own.target?.vessel()?;
        let other = self.vessel(id)?;
        let flown = own.propulsion.control_part();
        let mine = own.propulsion.docking_ports();
        let from_it = mine.iter().any(|p| Some(p.part) == flown);
        let mine = mine.iter().filter(|p| !from_it || Some(p.part) == flown);
        let theirs = other.propulsion.docking_ports();
        let chosen = theirs
            .iter()
            .find(|p| own.target == Some(Target::Part(id, p.part)))
            .map(|p| p.part);
        let theirs: Vec<_> = (theirs.iter())
            .filter(|p| chosen.is_none_or(|part| p.part == part))
            .collect();
        mine.flat_map(|port| theirs.iter().map(move |&far| (port, far)))
            .filter(|(port, far)| port.fits(far))
            .map(|(port, far)| {
                let (near_state, far_state) = self.port_states(own, port, other, far, time);
                PortApproach {
                    alignment: Alignment::of(&near_state, &far_state),
                    near: near_state,
                    far: far_state,
                    port: port.clone(),
                    target: id,
                    target_port: far.clone(),
                }
            })
            .min_by(|a, b| a.alignment.distance().total_cmp(&b.alignment.distance()))
    }

    /// Put vessel `index` `gap` metres off the port its target would dock by, facing it, closing
    /// at `closing` m/s. For scenarios and captures: nothing a player can do. Whether it could.
    pub fn line_up_ports(&mut self, index: usize, gap: f64, closing: f64) -> bool {
        let time = self.time;
        let Some(approach) = self.port_approach(index, time) else {
            return false;
        };
        let Some(other) = self.vessel(approach.target) else {
            return false;
        };
        let far = other.port_state(&self.system, &approach.target_port, time);
        let place = StateVector::new(
            far.at.position + far.out * gap,
            far.at.velocity - far.out * closing,
        );
        let place =
            (self.system).transform(place, other.parent(), self.vessels[index].parent(), time);
        let vessel = &mut self.vessels[index];
        vessel.put_port(&self.system, &approach.port, place, -far.out, time);
        true
    }

    /// Where `own`'s `port` and `other`'s `far` are, in `own`'s parent's frame.
    fn port_states(
        &self,
        own: &Vessel,
        port: &DockingPort,
        other: &Vessel,
        far: &DockingPort,
        time: SimTime,
    ) -> (PortState, PortState) {
        let near = own.port_state(&self.system, port, time);
        let mut far = other.port_state(&self.system, far, time);
        far.at = (self.system).transform(far.at, other.parent(), own.parent(), time);
        (near, far)
    }

    /// How `own`'s `port` stands towards `other`'s `far`, in `own`'s parent's frame.
    fn alignment(
        &self,
        own: &Vessel,
        port: &DockingPort,
        other: &Vessel,
        far: &DockingPort,
        time: SimTime,
    ) -> Alignment {
        let (near, far) = self.port_states(own, port, other, far, time);
        Alignment::of(&near, &far)
    }

    /// Whether two vessels' ports have caught and are closing.
    fn capturing(&self, a: VesselId, b: VesselId) -> bool {
        self.captures
            .iter()
            .any(|c| (c.stay == a && c.leave == b) || (c.stay == b && c.leave == a))
    }

    /// Whether the vessel with `id` is in soft capture, its ports closing.
    pub fn in_soft_capture(&self, id: VesselId) -> bool {
        (self.captures.iter()).any(|c| c.stay == id || c.leave == id)
    }

    /// Start soft capture for the first two vessels whose free ports fit and catch now. The one
    /// being flown stays the vessel, or else the heavier.
    fn catch_ports(&mut self) {
        let time = self.time;
        let can = |v: &Vessel| {
            !v.is_landed()
                && !v.is_destroyed()
                && v.propulsion.docking_ports().iter().any(|p| p.free)
        };
        let ready: Vec<usize> = (0..self.vessels.len())
            .filter(|&i| can(&self.vessels[i]))
            .collect();
        let mut found = None;
        'pairs: for (k, &i) in ready.iter().enumerate() {
            for &j in &ready[k + 1..] {
                let (a, b) = (&self.vessels[i], &self.vessels[j]);
                if self.capturing(a.id(), b.id()) {
                    continue;
                }
                let theirs = self.system.transform(
                    b.state_at(&self.system, time),
                    b.parent(),
                    a.parent(),
                    time,
                );
                let apart = theirs.position - a.state_at(&self.system, time).position;
                if apart.length() > CAPTURE_SEARCH {
                    continue;
                }
                for port in a.propulsion.docking_ports() {
                    for far in b.propulsion.docking_ports() {
                        if port.fits(far) && self.alignment(a, port, b, far, time).captures() {
                            found = Some((i, port.clone(), j, far.clone()));
                            break 'pairs;
                        }
                    }
                }
            }
        }
        let Some((i, port, j, far)) = found else {
            return;
        };
        let (first, second) = (&self.vessels[i], &self.vessels[j]);
        let (stay, at, leave, to) = if self.focus == Some(second.id())
            || (self.focus != Some(first.id())
                && second.propulsion.mass() > first.propulsion.mass())
        {
            (j, far, i, port)
        } else {
            (i, port, j, far)
        };
        let (kept, taken) = (
            self.vessels[stay].name.clone(),
            self.vessels[leave].name.clone(),
        );
        self.vessels[stay].record(time, format!("Soft capture of {taken}"));
        self.vessels[leave].record(time, format!("Soft capture by {kept}"));
        self.captures.push(SoftCapture {
            stay: self.vessels[stay].id(),
            at,
            leave: self.vessels[leave].id(),
            to,
            since: time,
        });
    }

    /// Draw ports in soft capture together, with a critically damped spring between them that each
    /// vessel takes equally and oppositely, and close the latches when they are home. The vessels
    /// that docked.
    fn close_captures(&mut self) -> Vec<VesselId> {
        let time = self.time;
        let dt = PHYSICS_STEP.as_seconds();
        let mut docked = Vec::new();
        for capture in std::mem::take(&mut self.captures) {
            let (Some(i), Some(j)) = (
                self.find_vessel(capture.stay),
                self.find_vessel(capture.leave),
            ) else {
                continue;
            };
            let (near, far) = self.port_states(
                &self.vessels[i],
                &capture.at,
                &self.vessels[j],
                &capture.to,
                time,
            );
            let apart = far.at.position - near.at.position;
            let moving = far.at.velocity - near.at.velocity;
            let angle = far.out.angle_between(-near.out);
            let home = apart.length() < LATCH_GAP && angle < LATCH_ANGLE;
            if home || time.seconds_since(capture.since) >= LATCH_TIME {
                let roll = roll_between(
                    self.vessels[i].port_turn(&self.system, &capture.at, time),
                    self.vessels[j].port_turn(&self.system, &capture.to, time),
                );
                let joined = self.join(
                    capture.stay,
                    Port {
                        part: capture.at.part,
                        node: &capture.at.node,
                    },
                    capture.leave,
                    Port {
                        part: capture.to.part,
                        node: &capture.to.node,
                    },
                    roll,
                );
                if joined.is_ok() {
                    docked.push(capture.stay);
                }
                continue;
            }
            let mut stay = self.vessels[i].as_body(&self.system, time);
            let mut leave = self.vessels[j].as_body(&self.system, time);
            let pair_mass = stay.mass.mass * leave.mass.mass / (stay.mass.mass + leave.mass.mass);
            let pull = apart * (RETRACT * RETRACT) + moving * (2.0 * RETRACT);
            let impulse = pull * (pair_mass * dt);
            leave.push(-impulse, far.at.position);
            stay.push(impulse, near.at.position);
            // And turn them to face each other, and stop them turning against each other.
            let moment = |b: &contact::Body| b.mass.inertia.diagonal().max_element();
            let (ma, mb) = (moment(&stay), moment(&leave));
            let pair_moment = ma * mb / (ma + mb);
            let wrong = far.out.cross(-near.out);
            let rate = far.turning - near.turning;
            let twist = (wrong * (RETRACT * RETRACT) - rate * (2.0 * RETRACT)) * (pair_moment * dt);
            leave.twist(twist);
            stay.twist(-twist);
            self.vessels[i].set_body(&self.system, &stay, time);
            self.vessels[j].set_body(&self.system, &leave, time);
            self.captures.push(capture);
        }
        docked
    }

    /// Solve contact between every two vessels near enough to touch that have been clear of each
    /// other, at least one of them more than debris.
    fn collide_vessels(&mut self) -> Vec<SimEvent> {
        let time = self.time;
        let dt = PHYSICS_STEP.as_seconds();
        let now = time.seconds_since(SimTime::J2000);
        let mut near = BTreeSet::new();
        let mut events = Vec::new();
        for i in 0..self.vessels.len() {
            for j in (i + 1)..self.vessels.len() {
                let (a, b) = (&self.vessels[i], &self.vessels[j]);
                if (a.is_debris() && b.is_debris())
                    || (a.is_landed() && b.is_landed())
                    || a.parent() != b.parent()
                    || self.capturing(a.id(), b.id())
                {
                    continue;
                }
                let pair = (a.id().min(b.id()), a.id().max(b.id()));
                let (body_a, body_b) =
                    (a.as_body(&self.system, time), b.as_body(&self.system, time));
                let distance = body_a.center.distance(body_b.center);
                let rough = |v: &Vessel| {
                    let widest = (v.propulsion.sections.iter())
                        .map(|s| s.diameter)
                        .fold(0.0, f64::max);
                    v.propulsion.length() + widest + 30.0
                };
                let speed = (body_a.velocity - body_b.velocity).length()
                    + (body_a.turned * body_a.spin).length() * distance
                    + (body_b.turned * body_b.spin).length() * distance;
                let reach = speed * dt + CONTACT_REACH;
                if distance > FAR_APART.max(reach) || distance > rough(a) + rough(b) + reach {
                    self.clear_of.insert(pair);
                    continue;
                }
                // What it cannot dock with counts as clear once it is well away, with no need to
                // read any meshes for a stage falling off the stack.
                let clear = self.clear_of.contains(&pair);
                let fit = {
                    let theirs = b.propulsion.docking_ports();
                    (a.propulsion.docking_ports().iter())
                        .any(|p| theirs.iter().any(|far| p.fits(far)))
                };
                if !clear && (a.is_debris() || b.is_debris()) && !fit {
                    continue;
                }
                let mesh_a = self.colliders.of(a.propulsion.parts());
                let mesh_b = self.colliders.of(b.propulsion.parts());
                let frame_a = a.frame_of(&body_a);
                let frame_b = b.frame_of(&body_b);
                if !clear {
                    if !mesh_a.within(frame_a, &mesh_b, frame_b, DAYLIGHT) {
                        self.clear_of.insert(pair);
                    }
                    continue;
                }
                // They may yet meet: have their parts cut ready by the time they do.
                self.colliders.prepare(&mesh_a);
                self.colliders.prepare(&mesh_b);
                if !mesh_a.within(frame_a, &mesh_b, frame_b, reach) {
                    continue;
                }
                near.insert(pair);
                let member = |v: &Vessel, body, collider| Member {
                    id: v.id(),
                    body,
                    held: v.held(),
                    collider,
                };
                let outcome = self.contacts.step(
                    member(a, body_a, mesh_a),
                    member(b, body_b, mesh_b),
                    now,
                    dt,
                );
                // Said first: why the ports did not catch is how they were coming in.
                if let Some(closing) = outcome.met {
                    events.extend(self.vessels_met(i, j, closing));
                }
                for (id, body) in outcome.moved {
                    let k = if id == self.vessels[i].id() { i } else { j };
                    self.vessels[k].set_body(&self.system, &body, time);
                }
            }
        }
        self.contacts.keep_only(&near);
        events
    }

    /// Vessels `i` and `j` have just hit each other, closing at this speed, m/s: say so, and break
    /// them if that was too hard.
    fn vessels_met(&mut self, i: usize, j: usize, closing: f64) -> Vec<SimEvent> {
        let time = self.time;
        let mut events = Vec::new();
        let (a, b) = (&self.vessels[i], &self.vessels[j]);
        // Why the ports did not catch, if that is what they were trying to do.
        let theirs = b.propulsion.docking_ports();
        let why = (a.propulsion.docking_ports().iter())
            .flat_map(|p| theirs.iter().map(move |far| (p, far)))
            .filter(|(p, far)| p.fits(far))
            .map(|(p, far)| self.alignment(a, p, b, far, time))
            .min_by(|x, y| x.distance().total_cmp(&y.distance()))
            .and_then(|ports| ports.problem())
            .map_or(String::new(), |problem| format!(": {}", problem.label()));
        let (id_a, id_b) = (a.id(), b.id());
        let (name_a, name_b) = (a.name.clone(), b.name.clone());
        if closing >= QUIET_BUMP {
            let said = format!("Bumped {name_b} at {closing:.2} m/s{why}");
            self.vessels[i].record(time, said);
            let said = format!("Bumped {name_a} at {closing:.2} m/s{why}");
            self.vessels[j].record(time, said);
            for id in [id_a, id_b] {
                events.push(SimEvent {
                    vessel: id,
                    event: VesselEvent {
                        time,
                        kind: VesselEventKind::Bumped { speed: closing },
                    },
                });
            }
        }
        if closing >= BREAK_SPEED {
            let said = format!("Broke up, hitting at {closing:.0} m/s");
            for (k, id) in [(i, id_a), (j, id_b)] {
                if !self.vessels[k].is_destroyed() {
                    self.vessels[k].break_up(time, said.clone());
                    events.push(SimEvent {
                        vessel: id,
                        event: VesselEvent {
                            time,
                            kind: VesselEventKind::Destroyed,
                        },
                    });
                }
            }
        }
        events
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
            Target::Vessel(id) | Target::Part(id, _) => {
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
            Target::Part(id, part) => {
                let other = self.vessels.get(self.find_vessel(id)?)?;
                let state = other
                    .part_state(&self.system, part, time)
                    .unwrap_or_else(|| other.state_at(&self.system, time));
                (other.parent(), state)
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
            spawn_vessel(&craft, catalog, system, sites, start, time)
        };
        let mut vessels = vec![spawn(&scenario.craft, &scenario.start)?];
        for _ in 0..scenario.sequences_done {
            vessels[0].activate_next_stage(system, time);
        }
        vessels[0].set_rcs(scenario.rcs);
        for other in &scenario.others {
            let mut vessel = spawn(&other.craft, &other.start)?;
            if let Some(name) = &other.name {
                vessel.name = name.clone();
            }
            for _ in 0..other.sequences_done {
                vessel.activate_next_stage(system, time);
            }
            if other.holds_attitude {
                vessel.set_rcs(true);
                vessel.set_sas(system, time, true);
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
        let mission =
            Mission::new(scenario, sites, &self.system).map_err(UniverseError::Mission)?;
        self.vessels.clear();
        self.on_show = None;
        self.clear_of.clear();
        self.contacts.clear();
        self.captures.clear();
        self.alarms = AlarmClock::default();
        self.time = time;
        for mut vessel in vessels {
            // In a game, nothing flies without avionics or a crew to fly it.
            vessel.propulsion.require_control_under(&catalog.rules);
            self.add_vessel(vessel);
        }
        self.focus = Some(self.vessels[0].id());
        self.mission = mission;
        self.tutorial = None;
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

    /// Put `craft` at `start` and fly it, its sequences as `sequences` if given. The game carries
    /// on around it, or with `new_game` starts over at that time with nothing else in it. On an
    /// error nothing changes.
    #[allow(clippy::too_many_arguments)]
    pub fn launch(
        &mut self,
        sites: &[LaunchSiteDef],
        catalog: &PartCatalog,
        craft: &CraftDef,
        start: &StartDef,
        sequences: Option<Sequences>,
        new_game: Option<SimTime>,
    ) -> Result<VesselId, UniverseError> {
        let time = new_game.unwrap_or(self.time);
        let mut vessel = spawn_vessel(craft, catalog, &self.system, sites, start, time)?;
        if let Some(sequences) = sequences {
            vessel.propulsion.sequences = sequences;
        }
        // In a game, nothing flies without avionics or a crew to fly it.
        vessel.propulsion.require_control_under(&catalog.rules);
        if new_game.is_some() {
            self.vessels.clear();
            self.on_show = None;
            self.clear_of.clear();
            self.contacts.clear();
            self.contacts.clear();
            self.captures.clear();
            self.alarms = AlarmClock::default();
            self.mission = None;
            self.tutorial = None;
            self.time = time;
        }
        let id = self.add_vessel(vessel);
        self.focus = Some(id);
        Ok(id)
    }

    fn body_names(&self) -> Vec<String> {
        let bodies = self.system.bodies().iter();
        bodies.map(|body| body.name.clone()).collect()
    }

    /// Everything that happens here, for saving.
    pub fn snapshot(&self) -> Snapshot {
        // What is only on show is not part of the game.
        let focus = match self.on_show {
            Some((shown, behind)) if self.focus == Some(shown) => behind,
            _ => self.focus,
        };
        Snapshot {
            bodies: self.body_names(),
            time: self.time,
            next_id: self.next_id,
            link_ticks: self.link_ticks,
            vessels: self.in_game().cloned().collect(),
            focus,
            mission: self.mission.clone(),
            tutorial: self.tutorial.clone(),
            clear_of: self.clear_of.clone(),
            captures: self.captures.clone(),
            alarms: self.alarms.clone(),
            career: self.career.clone(),
        }
    }

    /// Carry on from `snapshot`, in this solar system. It must be among the same bodies as it was
    /// saved among, as vessels' parents number them; if not, nothing changes.
    pub fn restore(&mut self, snapshot: Snapshot) -> Result<(), String> {
        if snapshot.bodies != self.body_names() {
            return Err("it was saved with other bodies in the solar system".into());
        }
        if snapshot.vessels.is_empty() {
            return Err("it has no vessels".into());
        }
        self.time = snapshot.time;
        self.next_id = snapshot.next_id;
        self.link_ticks = snapshot.link_ticks;
        self.vessels = snapshot.vessels;
        self.on_show = None;
        self.focus = snapshot.focus;
        self.mission = snapshot.mission;
        self.tutorial = snapshot.tutorial;
        self.clear_of = snapshot.clear_of;
        self.contacts.clear();
        self.captures = snapshot.captures;
        self.alarms = snapshot.alarms;
        self.career = snapshot.career;
        Ok(())
    }

    /// Rails warp is possible only when every vessel is landed or coasting with engines off,
    /// and none has reached the point where it needs physics again (a burn coming up).
    /// Debris does not count: rails leave behind what of it needs physics.
    pub fn can_use_rails(&self) -> bool {
        let quiet = !self.vessels.iter().any(|v| {
            !self.expendable(v)
                && (v.needs_physics() || v.attention_time().is_some_and(|t| t <= self.time))
        });
        quiet && self.captures.is_empty() && !self.next_close_call().is_some_and(|wait| wait <= 0.0)
    }

    /// How long until the vessel being flown comes within [`CLOSE_CALL`] of a vessel it is slowly
    /// closing on, s, if it is: at once if it is there. Debris that it could not dock with does
    /// not count, as for time warp in general.
    fn next_close_call(&self) -> Option<f64> {
        let own = self.vessel(self.focus?)?;
        let here = own.state_at(&self.system, self.time);
        let ports = own.propulsion.docking_ports();
        (self.vessels.iter())
            .filter(|other| other.id() != own.id() && !other.is_landed())
            .filter(|other| {
                let theirs = other.propulsion.docking_ports();
                !self.expendable(other)
                    || ports.iter().any(|p| theirs.iter().any(|far| p.fits(far)))
            })
            .filter_map(|other| {
                let there = self.system.transform(
                    other.state_at(&self.system, self.time),
                    other.parent(),
                    own.parent(),
                    self.time,
                );
                let apart = there.position - here.position;
                let closing = -(there.velocity - here.velocity).dot(apart.normalize_or_zero());
                ((NOT_CLOSING..MEETING_SPEED).contains(&closing))
                    .then(|| (apart.length() - CLOSE_CALL) / closing)
            })
            .reduce(f64::min)
    }

    /// The earliest time some vessel needs physics again, if any does, and which vessel.
    fn attention(&self) -> Option<(usize, SimTime)> {
        self.vessels
            .iter()
            .enumerate()
            .filter_map(|(i, v)| Some((i, v.attention_time()?)))
            .min_by_key(|(_, t)| *t)
    }

    /// When the focused vessel next gains or loses its link, before `until`, on a coast.
    pub fn next_link_change(&self, until: SimTime) -> Option<SimTime> {
        let vessel = self.vessel(self.focus?)?;
        let radio = vessel.radio()?;
        let orbit = vessel.coasting()?;
        comms::next_link_change(
            &self.system,
            &self.stations,
            vessel.parent(),
            orbit,
            &radio,
            self.time,
            until,
        )
    }

    /// The ground stations vessels can talk to, from their records. Ones on no body there is are
    /// left out.
    pub fn set_stations(&mut self, defs: &[GroundStationDef]) {
        self.stations = (defs.iter())
            .filter_map(|def| Station::new(def, &self.system))
            .collect();
        self.update_links();
    }

    /// Work out each vessel's link: straight to the ground, or else through vessels with relay
    /// antennas that have a link of their own, by as many hops as it takes.
    pub fn update_links(&mut self) {
        let links = self.links_at(self.time);
        for (vessel, link) in self.vessels.iter_mut().zip(links) {
            vessel.set_link(link);
        }
    }

    /// Each vessel's link at `time`, with the radios they have now, where they are then on their
    /// way: within a jump on rails, where they all move as they are told.
    fn links_at(&self, time: SimTime) -> Vec<Option<Link>> {
        let root = self.system.root();
        let on_the_air: Vec<OnTheAir> = (self.vessels.iter())
            .map(|v| OnTheAir {
                position: self.system.relative_state(v.parent(), root, time).position
                    + v.state_at(&self.system, time).position,
                radio: v.radio(),
                relay: v.relay_radio(),
            })
            .collect();
        let direct: Vec<Option<Link>> = (self.vessels.iter().zip(&on_the_air))
            .map(|(v, air)| {
                let radio = air.radio.as_ref()?;
                let at = v.state_at(&self.system, time).position;
                best_link(&self.system, &self.stations, v.parent(), at, radio, time)
            })
            .collect();
        comms::spread_links(&self.system, root, &on_the_air, direct, time)
    }

    /// Which vessels have data on their drives and a radio on to send it with.
    fn senders(&self) -> Vec<bool> {
        (self.vessels.iter())
            .map(|v| v.radio().is_some() && v.propulsion.systems.sendable() > 0.0)
            .collect()
    }

    /// Send home what the vessels' drives hold over the `seconds` up to now, on the links they have
    /// now: all of it if there is a link, unless the game sends at the links' rates
    /// ([`Self::data_rates`]). What reaches the ground counts in a career.
    fn send_data(&mut self, seconds: f64) {
        let sending = self.senders();
        if !sending.contains(&true) {
            return;
        }
        let links: Vec<Option<Link>> = self.vessels.iter().map(|v| v.link().cloned()).collect();
        let bytes: Vec<f64> = if self.data_rates {
            (comms::shared_rates(&links, &sending).into_iter())
                .map(|bits| bits * seconds / 8.0)
                .collect()
        } else {
            (links.iter().zip(&sending))
                .map(|(link, sending)| {
                    if link.is_some() && *sending {
                        f64::INFINITY
                    } else {
                        0.0
                    }
                })
                .collect()
        };
        self.deliver(&bytes);
    }

    /// Send home what the vessels' drives hold over a jump on rails of `seconds` from `from`, at the
    /// rate each link carries as they move: the links are worked out at the middle of short spans
    /// of it ([`SENDING_SPAN`], or less than that of an orbit), so a pass over a station sends what
    /// it does at any warp. In a game that does not send at rates, as [`Self::send_data`].
    fn send_over(&mut self, from: SimTime, seconds: f64) {
        if !self.data_rates {
            self.send_data(seconds);
            return;
        }
        let sending = self.senders();
        if !sending.contains(&true) || seconds <= 0.0 {
            return;
        }
        let span = (self.vessels.iter().zip(&sending))
            .filter(|(_, sending)| **sending)
            .filter_map(|(v, _)| v.coasting()?.period())
            .filter(|p| p.is_finite() && *p > 0.0)
            .map(|p| p / SPANS_PER_ORBIT)
            .fold(SENDING_SPAN, f64::min);
        let spans = (seconds / span).ceil().max(1.0);
        let each = seconds / spans;
        let mut bytes = vec![0.0; self.vessels.len()];
        for k in 0..spans as usize {
            let links = self.links_at(from.add_seconds((k as f64 + 0.5) * each));
            for (all, bits) in bytes.iter_mut().zip(comms::shared_rates(&links, &sending)) {
                *all += bits * each / 8.0;
            }
        }
        self.deliver(&bytes);
    }

    /// Take `bytes` of data off each vessel's drives, as far as they hold it: it has reached the
    /// ground, and in a career earns its science. A vessel whose drives are clear says what its data
    /// earned since they last were.
    fn deliver(&mut self, bytes: &[f64]) {
        let time = self.time;
        for (vessel, &bytes) in self.vessels.iter_mut().zip(bytes) {
            if bytes <= 0.0 {
                continue;
            }
            let sent = vessel.propulsion.systems.send(bytes);
            if let Some(career) = &mut self.career {
                vessel.sent_science += career.receive(&sent);
            }
            if vessel.propulsion.systems.sendable() <= 0.0 && vessel.sent_science > 0.0 {
                let science = std::mem::take(&mut vessel.sent_science);
                vessel.record(time, format!("Data sent home: {science:.1} science"));
            }
        }
    }

    /// Advance one fixed physics tick.
    pub fn step_physics(&mut self) -> Vec<SimEvent> {
        // Once a second is often enough for a radio.
        if self.link_ticks == 0 {
            self.update_links();
            self.link_ticks = 50;
        }
        self.link_ticks -= 1;
        let (t0, t1) = (self.time, self.time + PHYSICS_STEP);
        // Vessels steering towards a target need to know where it is.
        let targets: Vec<_> = (0..self.vessels.len())
            .map(|i| self.target_state(i, t0))
            .collect();
        for (vessel, state) in self.vessels.iter_mut().zip(targets) {
            vessel.set_target_state(state);
        }
        // Those facing a port need to know which way it looks.
        let ports: Vec<_> = (0..self.vessels.len())
            .map(|i| {
                let sas = self.vessels[i].sas();
                (sas.enabled() && sas.mode() == SasMode::TargetPort)
                    .then(|| self.port_approach(i, t0))
                    .flatten()
                    .map(|approach| -approach.alignment.out)
            })
            .collect();
        for (vessel, port) in self.vessels.iter_mut().zip(ports) {
            vessel.set_target_port(port);
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
        // Aborts under way: what they let go of flies on as debris.
        for k in 0..self.vessels.len() {
            if self.vessels[k].propulsion.aborting() {
                let dt = PHYSICS_STEP.as_seconds();
                let debris = self.vessels[k].step_abort(&self.system, t1, dt);
                for d in debris {
                    self.add_vessel(d);
                }
            }
        }
        // Parts too hot to stay on come off; what hung from them flies on.
        for k in 0..self.vessels.len() {
            if !self.vessels[k]
                .propulsion
                .systems
                .burned_through()
                .is_empty()
            {
                let debris = self.vessels[k].burn_off(&self.system, t1);
                for d in debris {
                    self.add_vessel(d);
                }
            }
        }
        for event in &events {
            if matches!(
                event.event.kind,
                VesselEventKind::Crashed { .. } | VesselEventKind::Destroyed
            ) && self
                .vessel(event.vessel)
                .is_some_and(|v| self.expendable(v))
            {
                self.remove_vessel(event.vessel);
            }
        }
        self.send_data(PHYSICS_STEP.as_seconds());
        self.catch_ports();
        for vessel in self.close_captures() {
            events.push(SimEvent {
                vessel,
                event: VesselEvent {
                    time: t1,
                    kind: VesselEventKind::Docked,
                },
            });
        }
        events.extend(self.collide_vessels());
        self.observe_mission(&events, true);
        events
    }

    /// Let the mission and the tutorial look at the universe as it is now. A goal met in a career
    /// pays its reward into its funds.
    fn observe_mission(&mut self, events: &[SimEvent], physics: bool) {
        if let Some(mut mission) = self.mission.take() {
            let before: Vec<bool> = mission.goals().iter().map(|g| g.met_at.is_some()).collect();
            mission.observe(self, events, physics);
            if let Some(career) = &mut self.career {
                for (goal, was) in mission.goals().iter().zip(before) {
                    if goal.met_at.is_some() && !was {
                        career.earn(goal.def.reward);
                    }
                }
            }
            self.mission = Some(mission);
        }
        if let Some(mut career) = self.career.take() {
            if !career.contracts.taken.is_empty() {
                let earned = career.contracts.observe(self, events, physics);
                career.earn(earned);
            }
            self.career = Some(career);
        }
        if let Some(mut tutorial) = self.tutorial.take() {
            tutorial.observe(self, self.mission.as_ref());
            self.tutorial = Some(tutorial);
        }
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
        // Stop short of another vessel the flown one is closing on: physics takes it from there.
        let target = (self.next_close_call()).map_or(target, |wait| {
            target.min(self.time.add_seconds(wait.max(0.0)))
        });
        // Hand over to physics where a vessel needs it.
        let due = self
            .attention()
            .filter(|(_, t)| *t <= target)
            .map(|(i, t)| (self.vessels[i].id(), t));
        let target = due.map_or(target, |(_, t)| t);
        // A system that needs attention stops the jump there, as an event does.
        let started = self.time;
        let target = (self.vessels.iter())
            .filter_map(|v| v.next_system_event(&self.system, started, target))
            .fold(target, SimTime::min);
        // The player may have asked to stop where the focused vessel's link changes.
        let change = self
            .warp_stops_at_links
            .then(|| self.next_link_change(target))
            .flatten();
        let target = change.map_or(target, |at| target.min(at));
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
        let elapsed = self.time.seconds_since(started);
        if elapsed > 0.0 {
            for vessel in &mut self.vessels {
                vessel.step_systems(&self.system, started, elapsed);
            }
            self.update_links();
            self.send_over(started, elapsed);
            if change == Some(self.time) {
                outcome.interrupted = true;
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
        self.observe_mission(&outcome.events, false);
        outcome
    }
}

/// How high above a pad or runway a vessel stands in the way of a launch from it, m: as high as the
/// middle of the tallest rockets standing on it, and well below anything that has taken off.
const SITE_CLEARANCE: f64 = 150.0;
/// How far past the edge of a pad or runway it is in the way, m: half a big rocket's fins or wings.
const SITE_MARGIN: f64 = 20.0;
/// What a launch site without a pad needs clear, m.
const BARE_SITE_SIZE: f64 = 60.0;

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
            tests::{catalog, launch_sites, leo_vessel, pad_vessel, pathfinder},
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
            spin: [0.0; 3],
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
            Ok(crate::test_data::realistic(text, "craft").remove(0))
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
                .restart(&sites, &catalog(), craft_of, scenario("glider_runway"))
                .is_err()
        );
        assert_eq!(universe.vessels[0].name, "Comet");
    }

    #[test]
    fn a_vessel_on_the_pad_is_in_the_way_of_a_launch_from_it_and_of_nothing_else() {
        let system = rss();
        let (pad, t) = pad_vessel(&system);
        let id = pad.id();
        let mut universe = Universe::new(system, vec![pad], t);
        let sites = launch_sites();
        let cape = sites.iter().find(|s| s.pad.is_some()).unwrap();
        assert_eq!(universe.on_launch_ground(cape, false), [id]);
        // That site has no runway to be on, and another site is somewhere else.
        assert!(universe.on_launch_ground(cape, true).is_empty());
        let strip = sites.iter().find(|s| s.runway.is_some()).unwrap();
        assert!(universe.on_launch_ground(strip, true).is_empty());
        // It still stands there as the Earth turns.
        universe.time = t.add_seconds(30_000.0);
        assert_eq!(universe.on_launch_ground(cape, false), [id]);
        // Nothing on it, nothing in the way.
        universe.remove_vessel(id);
        assert!(universe.on_launch_ground(cape, false).is_empty());
    }

    #[test]
    fn a_vessel_anywhere_on_the_runway_is_in_the_way_of_a_launch_from_it() {
        let system = rss();
        let sites = launch_sites();
        let strip = sites.iter().find(|s| s.runway.is_some()).unwrap();
        let earth = system.find(&strip.body).unwrap();
        let t = SimTime::from_seconds(21_600.0);
        let (radius, turned) = {
            let body = system.body(earth);
            (body.radius, body.orientation(t))
        };
        let ground = |forward_m: f64, right_m: f64, height: f64| {
            let up = terrain::site_point(strip, radius, right_m, forward_m);
            let body = system.body(earth);
            let at = up * (radius + body.terrain_height(up) + height);
            StateVector::new(turned * at, DVec3::X * 100.0)
        };
        let vessel_at = |state: StateVector| vessel_with(&system, earth, state, t);
        let cases = [
            ("at its near end", ground(-2_200.0, 0.0, 3.0), true),
            ("half way along", ground(0.0, 0.0, 3.0), true),
            ("on its edge", ground(1_000.0, 40.0, 3.0), true),
            ("in the grass beside it", ground(0.0, 120.0, 3.0), false),
            ("past its end", ground(2_400.0, 0.0, 3.0), false),
            ("flying over it", ground(0.0, 0.0, 800.0), false),
        ];
        let mut universe = Universe::new(system.clone(), Vec::new(), t);
        for (what, state, wanted) in cases {
            let id = universe.add_vessel(vessel_at(state));
            let found = universe.on_launch_ground(strip, true).contains(&id);
            assert_eq!(found, wanted, "{what}");
            universe.remove_vessel(id);
        }
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
        // How far up the stack the upper stage stands: the booster and the ring on it.
        let sections = &u.vessels[0].propulsion.sections;
        let booster = sections[1].bottom.z - sections[0].bottom.z;
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

    /// On the pad, a vessel can be brought home; in orbit, or on a world with no launch site, it
    /// cannot; recovered, it is gone.
    #[test]
    fn a_vessel_can_be_recovered_only_when_it_is_down_on_a_world_with_a_launch_site() {
        let system = rss();
        let (pad, t) = pad_vessel(&system);
        let (leo, _) = leo_vessel(&system);
        let mut universe = Universe::new(system, vec![pad, leo], t);
        let (pad, leo) = (universe.vessels[0].id(), universe.vessels[1].id());
        let sites = launch_sites();
        assert!(universe.recoverable(pad, &sites));
        assert!(!universe.recoverable(leo, &sites));
        let elsewhere: Vec<LaunchSiteDef> = (sites.iter().cloned())
            .map(|site| LaunchSiteDef {
                body: "Moon".into(),
                ..site
            })
            .collect();
        assert!(!universe.recoverable(pad, &elsewhere));
        let recovered = universe.recover_vessel(pad).unwrap();
        assert!(
            recovered.notice().starts_with("Recovered "),
            "{recovered:?}"
        );
        assert!(!universe.recoverable(pad, &sites), "gone");
        assert_eq!(universe.vessels.len(), 1);
    }

    /// A rocket on the pad is measured as it stands: what it weighs with its propellant, its parts,
    /// and its height and width from the box round it.
    #[test]
    fn a_craft_is_measured_as_it_stands() {
        let system = rss();
        let (pad, t) = pad_vessel(&system);
        let mut universe = Universe::new(system, vec![pad], t);
        let id = universe.vessels[0].id();
        let size = universe.craft_size(id).unwrap();
        let vessel = &universe.vessels[0];
        assert_eq!(size.mass, vessel.mass_properties().mass);
        assert_eq!(size.parts, vessel.propulsion.parts().parts().len());
        let length = vessel.propulsion.length();
        assert!(
            (size.height - length).abs() < 0.2 * length,
            "{size:?} {length}"
        );
        let widest = (vessel.propulsion.sections.iter())
            .map(|s| s.diameter)
            .fold(0.0, f64::max);
        assert!(size.width >= 0.95 * widest, "{size:?} {widest}");
    }
}
