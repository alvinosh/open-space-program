//! Vessels and how they move.
//!
//! A vessel is always in exactly one [`Motion`] mode:
//! - **Landed**: fixed in the body's rotating frame.
//! - **Coasting**: gravity only, so its trajectory is an exact conic. It is evaluated
//!   analytically at any time and only changes at [`TrajectoryEvent`]s. This is the same at
//!   1x and at 100 000x, so time warp cannot change where a coasting vessel ends up.
//! - **Integrated**: thrust, the air or terrain proximity, integrated with Encke's method on
//!   a fixed tick grid.
//!
//! Transitions happen at exact ticks: Coasting → Integrated at a descent event or when
//! thrust starts, Integrated → Coasting once forces stop above the coast floor.
//!
//! A vessel's position is its centre of mass, which it rotates about. Rotation follows the
//! same idea as translation ([`crate::attitude`]): without torque it is exact at any time,
//! and torque from the engine gimbal, reaction wheels, RCS or the air ([`crate::aero`]) is
//! integrated on the physics tick. During time warp on rails nothing steers: a turning
//! vessel keeps turning, and with SAS on a steady one holds its attitude exactly.
//!
//! Engines that need ullage only light while the propellant is settled, i.e. while
//! something pushes the vessel forward: thrust still running (hot staging), or RCS
//! translating forward ([`crate::rcs`]) after a coast.

use glam::{DMat3, DQuat, DVec3};
use thiserror::Error;

use crate::{
    aero::{Flow, Loads, Panel, Shape},
    atmosphere::AtmosphereSample,
    attitude::{self, Attitude, Inertia, MassProperties, NavBasis, NavFrame, Sas, SasMode},
    autopilot::{self, BurnExecutor, BurnTiming, Capability, Finish},
    body::{Body, BodyId},
    data::{CraftDef, FlightStartDef, LaunchSiteDef, OrbitStartDef, PartCatalog},
    events::{EventKind, TrajectoryEvent, apply_event, next_event},
    gear::{self, Contact, Leg},
    integrator::Encke,
    orbit::{Elements, Orbit, StateVector},
    plan::{Burn, FlightPlan},
    parts::PartId,
    propulsion::{CraftError, Propulsion, Released, Staged, placed},
    rcs,
    rendezvous::Target,
    staging::Activation,
    structure::{self, Conditions, Part, PartLoad},
    system::SolarSystem,
    terrain,
    time::{SimTime, Ticks},
};

/// Impacts slower than this (relative to the ground) count as a landing, not a crash.
const SAFE_LANDING_SPEED: f64 = 10.0;
/// Proper acceleration along the nose needed to keep propellant settled, m/s².
const ULLAGE_ACCELERATION: f64 = 1e-3;
/// A vessel may only start coasting if it won't hit the coast floor within this many
/// seconds; prevents flip-flopping between modes on the boundary.
const COAST_HYSTERESIS: f64 = 0.1;
/// Substep for a vessel on or near the ground on landing gear, whose struts are stiff.
const GEAR_SUBSTEP: Ticks = Ticks::from_millis(2);
/// Height above its gear's reach within which a vessel rolls (see `Vessel::roll`), m.
const GEAR_MARGIN: f64 = 30.0;
/// Hitting the ground with anything but its wheels faster than this is a crash, m/s.
const HULL_IMPACT_SPEED: f64 = 5.0;
/// On the ground with no thrust and slower than these (m/s, rad/s), a vessel comes to rest.
const REST_SPEED: f64 = 0.05;
const REST_RATE: f64 = 0.02;
/// How far in from the end of a runway a vessel starts, m.
const RUNWAY_START: f64 = 60.0;
/// Speed at which a decoupler without an impulse of its own pushes the stages either side of
/// it apart, m/s.
const SEPARATION_SPEED: f64 = 1.0;
/// A jettisoned fairing half swings open about its base at this rate, rad/s, and its base
/// moves out at this speed, m/s.
const FAIRING_SWING: f64 = 0.3;
const FAIRING_PUSH: f64 = 0.5;
const MAX_LOG: usize = 64;

#[derive(Debug, Error)]
pub enum VesselError {
    #[error(transparent)]
    Craft(#[from] CraftError),
    #[error("unknown body `{0}`")]
    UnknownBody(String),
    #[error("`{0}` does not orbit `{1}`, so its plane cannot be used")]
    NotOrbiting(String, String),
    #[error("{0} has no runway")]
    NoRunway(String),
}

#[derive(Debug, Clone)]
pub enum Motion {
    /// Standing on the ground: the base of the stack and the attitude, both in the
    /// body-fixed (rotating) frame.
    Landed {
        base: DVec3,
        attitude: DQuat,
    },
    Coasting(Orbit),
    Integrated(Encke),
}

/// Stable identity of a vessel, unchanged as vessels come and go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct VesselId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Situation {
    Landed,
    Flying,
    SubOrbital,
    Orbiting,
    Escaping,
}

/// Something that happened to a vessel, for the game to react to (e.g. stop warp).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VesselEventKind {
    SoiChange {
        from: BodyId,
        to: BodyId,
    },
    /// Entered an atmosphere or came close to terrain: physics now required.
    Descent {
        body: BodyId,
    },
    Liftoff,
    Landed {
        speed: f64,
    },
    Crashed {
        speed: f64,
    },
    /// A tank buckled or burst: the vessel broke up.
    Destroyed,
    /// The burn executor needs physics ticks from here on (time warp hands over).
    BurnDue,
    /// The burn executor finished a burn, or gave up on it.
    BurnEnded,
}

impl VesselEventKind {
    /// Events that need the player's attention at normal speed.
    pub fn stops_warp(&self) -> bool {
        !matches!(self, VesselEventKind::SoiChange { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VesselEvent {
    pub time: SimTime,
    pub kind: VesselEventKind,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub time: SimTime,
    pub message: String,
}

/// Instrument readings from the last physics step.
#[derive(Debug, Clone, Copy, Default)]
pub struct FlightData {
    pub mass: f64,
    pub thrust: f64,
    pub dynamic_pressure: f64,
    pub mach: f64,
    /// Nose above the airflow (pitch up) and the airflow from the right, rad.
    pub angle_of_attack: f64,
    pub sideslip: f64,
    /// The air's force across and against the airflow, N.
    pub lift: f64,
    pub drag: f64,
    pub static_pressure: f64,
    /// Non-gravitational acceleration (what an accelerometer reads), m/s².
    pub proper_acceleration: f64,
    /// Rotation command applied (pitch up, yaw right, roll right), −1..1.
    pub control: DVec3,
    /// Largest torque available about each vessel axis, N·m.
    pub torque_authority: DVec3,
    /// The most loaded part's load over its strength (1 = at its limit).
    pub structure_load: f64,
}

/// The vessel's rotation: exact at `epoch`, and how it continues from there until a
/// physics step changes it.
#[derive(Debug, Clone, Copy)]
struct AttitudeTrack {
    epoch: SimTime,
    state: Attitude,
    inertia: Inertia,
    hold: Hold,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Hold {
    /// Torque-free rotation.
    Free,
    /// SAS holding a fixed attitude through time warp.
    Inertial,
    /// SAS holding an attitude relative to the navigation frame through time warp
    /// (prograde stays prograde); `basis` is that frame at the epoch.
    Tracking { frame: NavFrame, basis: DMat3 },
}

#[derive(Debug, Clone)]
pub struct Vessel {
    pub name: String,
    /// Assigned by the universe the vessel is in.
    id: VesselId,
    parent: BodyId,
    motion: Motion,
    attitude: AttitudeTrack,
    /// The pilot's rotation command (pitch up, yaw right, roll right), −1..1. Overrides
    /// SAS while non-zero.
    pub rotation_input: DVec3,
    /// The pilot's translation command (right, down, forward), −1..1, for the RCS.
    pub translation_input: DVec3,
    /// Wheel brakes on.
    pub brakes: bool,
    /// How far into its travel each gear leg's strut is, m, as of the last physics step.
    gear: Vec<f64>,
    /// Some wheel was on the ground at the last physics step.
    wheels_down: bool,
    sas: Sas,
    /// RCS armed: it translates on the pilot's command and helps with every rotation.
    rcs: bool,
    nav_frame: NavFrame,
    /// Inside the atmosphere or low over the ground, where the surface frame applies.
    near_surface: bool,
    pub propulsion: Propulsion,
    mass: MassProperties,
    propellant_settled: bool,
    /// Broke up: nothing works any more and it falls as wreckage.
    destroyed: bool,
    /// Came loose from another vessel (a spent stage, a fairing half).
    debris: bool,
    pub flight: FlightData,
    /// Load on every tank and fin set as of the last physics step.
    pub structure: Vec<PartLoad>,
    /// Axial load at the joint each stage hangs from as of the last physics step, N,
    /// compression positive (see [`structure::joint_loads`]).
    pub joint_loads: Vec<f64>,
    pub log: Vec<LogEntry>,
    /// Burns the pilot has planned.
    pub plan: FlightPlan,
    /// Flies the first planned burn when engaged.
    executor: BurnExecutor,
    /// The executor wants the RCS pushing forward this tick (ullage).
    ullage_push: bool,
    /// What the pilot navigates towards (navball, SAS target modes, closest approach).
    pub target: Option<Target>,
    /// The target's state relative to this vessel's parent, as of the last physics tick
    /// (set by the universe, which can see other vessels).
    target_state: Option<StateVector>,
}

impl Vessel {
    fn new(
        craft: &CraftDef,
        catalog: &PartCatalog,
        parent: BodyId,
        motion: Motion,
        orientation: DQuat,
        time: SimTime,
    ) -> Result<Self, VesselError> {
        let name = if craft.name.is_empty() {
            craft.id.clone()
        } else {
            craft.name.clone()
        };
        let propulsion = Propulsion::from_craft(craft, catalog)?;
        Ok(Self::assemble(
            name,
            propulsion,
            parent,
            motion,
            orientation,
            time,
        ))
    }

    fn assemble(
        name: String,
        propulsion: Propulsion,
        parent: BodyId,
        motion: Motion,
        orientation: DQuat,
        time: SimTime,
    ) -> Self {
        let mass = attitude::mass_properties(&propulsion);
        Self {
            name,
            parent,
            motion,
            attitude: AttitudeTrack {
                epoch: time,
                state: Attitude {
                    orientation,
                    angular_velocity: DVec3::ZERO,
                },
                inertia: mass.inertia,
                hold: Hold::Free,
            },
            rotation_input: DVec3::ZERO,
            translation_input: DVec3::ZERO,
            sas: Sas::default(),
            rcs: false,
            nav_frame: NavFrame::Surface,
            near_surface: true,
            flight: FlightData {
                mass: mass.mass,
                ..Default::default()
            },
            mass,
            propulsion,
            propellant_settled: false,
            destroyed: false,
            debris: false,
            structure: Vec::new(),
            joint_loads: Vec::new(),
            log: Vec::new(),
            plan: FlightPlan::default(),
            executor: BurnExecutor::default(),
            ullage_push: false,
            brakes: false,
            gear: Vec::new(),
            wheels_down: false,
            id: VesselId::default(),
            target: None,
            target_state: None,
        }
    }

    pub fn id(&self) -> VesselId {
        self.id
    }

    pub(crate) fn set_id(&mut self, id: VesselId) {
        self.id = id;
    }

    pub(crate) fn set_target_state(&mut self, state: Option<StateVector>) {
        self.target_state = state;
    }

    /// Standing on the surface at a launch site, nose up, its feet (+Y) to the east so
    /// that pitching down heads east.
    pub fn on_launch_site(
        craft: &CraftDef,
        catalog: &PartCatalog,
        system: &SolarSystem,
        site: &LaunchSiteDef,
        time: SimTime,
    ) -> Result<Self, VesselError> {
        let parent = system
            .find(&site.body)
            .ok_or_else(|| VesselError::UnknownBody(site.body.clone()))?;
        let body = system.body(parent);
        let (lat, lon) = (site.latitude.to_radians(), site.longitude.to_radians());
        let up = Body::surface_normal_fixed(lat, lon);
        let north = DVec3::new(-lat.sin() * lon.cos(), -lat.sin() * lon.sin(), lat.cos());
        let east = north.cross(up);
        let attitude = DQuat::from_mat3(&DMat3::from_cols(east.cross(up), east, up));
        let mount = site.pad.as_ref().map_or(0.0, |p| p.mount_height);
        let base = up * (body.radius + body.terrain_height(up) + mount);

        let mut vessel = Self::new(
            craft,
            catalog,
            parent,
            Motion::Landed { base, attitude },
            body.orientation(time) * attitude,
            time,
        )?;
        vessel.propellant_settled = true;
        vessel.record(
            time,
            format!("{} on the pad at {}", vessel.name, site.display_name()),
        );
        Ok(vessel)
    }

    /// At the near end of a launch site's runway, lined up along it, standing level on its
    /// gear (or on its belly without any), brakes on.
    pub fn on_runway(
        craft: &CraftDef,
        catalog: &PartCatalog,
        system: &SolarSystem,
        site: &LaunchSiteDef,
        time: SimTime,
    ) -> Result<Self, VesselError> {
        let runway = site
            .runway
            .as_ref()
            .ok_or_else(|| VesselError::NoRunway(site.display_name().to_owned()))?;
        let parent = system
            .find(&site.body)
            .ok_or_else(|| VesselError::UnknownBody(site.body.clone()))?;
        let body = system.body(parent);
        let along = -(runway.size[0] / 2.0 - RUNWAY_START);
        let up = terrain::site_point(site, body.radius, 0.0, along);
        let (_, _, forward) = terrain::site_axes(site);
        let nose = forward.reject_from(up).normalize();
        let attitude = DQuat::from_mat3(&DMat3::from_cols((-up).cross(nose), -up, nose));
        let mut vessel = Self::new(
            craft,
            catalog,
            parent,
            Motion::Landed {
                base: up * body.radius,
                attitude,
            },
            body.orientation(time) * attitude,
            time,
        )?;
        let legs = gear::legs(&vessel.propulsion.sections);
        let height = gear::rest_height(&legs)
            .unwrap_or_else(|| vessel.propulsion.sections[0].diameter / 2.0);
        let ground = body.radius + body.terrain_height(up);
        vessel.motion = Motion::Landed {
            base: up * (ground + height),
            attitude,
        };
        let contact = gear::contact(
            &legs,
            body,
            time,
            &vessel.state_at(system, time),
            &vessel.attitude_state(system, time),
            vessel.mass.center_of_mass,
            false,
            0.0,
        );
        vessel.wheels_down = contact.touching();
        vessel.gear = contact.compression;
        vessel.brakes = true;
        vessel.propellant_settled = true;
        vessel.record(
            time,
            format!("{} on the runway at {}", vessel.name, site.display_name()),
        );
        Ok(vessel)
    }

    /// Flying through the air, as if just dropped from a carrier aircraft: nose along the
    /// flight path, feet towards the ground, wings level.
    pub fn in_flight(
        craft: &CraftDef,
        catalog: &PartCatalog,
        system: &SolarSystem,
        start: &FlightStartDef,
        time: SimTime,
    ) -> Result<Self, VesselError> {
        let parent = system
            .find(&start.body)
            .ok_or_else(|| VesselError::UnknownBody(start.body.clone()))?;
        let body = system.body(parent);
        let (lat, lon) = (start.latitude.to_radians(), start.longitude.to_radians());
        let up = Body::surface_normal_fixed(lat, lon);
        let north = DVec3::new(-lat.sin() * lon.cos(), -lat.sin() * lon.sin(), lat.cos());
        let east = north.cross(up);
        let (heading, climb) = (start.heading.to_radians(), start.climb.to_radians());
        let level = north * heading.cos() + east * heading.sin();
        let to_inertial = body.orientation(time);
        let nose = to_inertial * (level * climb.cos() + up * climb.sin());
        let feet = (to_inertial * -up).reject_from(nose).normalize();
        let position = to_inertial * up * (body.radius + start.altitude);
        let velocity = body.surface_velocity(position) + nose * start.speed;
        let attitude = DQuat::from_mat3(&DMat3::from_cols(feet.cross(nose), feet, nose));
        let state = StateVector::new(position, velocity);

        let mut vessel = Self::new(
            craft,
            catalog,
            parent,
            free_motion(body, state, time),
            attitude,
            time,
        )?;
        vessel.record(time, format!("{} flying over {}", vessel.name, body.name));
        Ok(vessel)
    }

    /// In orbit, from elements relative to the body's equator (or in the plane of a body
    /// that orbits it). Nose prograde, feet towards the body.
    pub fn in_orbit(
        craft: &CraftDef,
        catalog: &PartCatalog,
        system: &SolarSystem,
        start: &OrbitStartDef,
        time: SimTime,
    ) -> Result<Self, VesselError> {
        let parent = system
            .find(&start.body)
            .ok_or_else(|| VesselError::UnknownBody(start.body.clone()))?;
        let body = system.body(parent);
        let frame = body.rotation.equatorial_frame();
        let (inclination, node) = match &start.plane {
            Some(name) => {
                let other = system
                    .find(name)
                    .map(|id| system.body(id))
                    .ok_or_else(|| VesselError::UnknownBody(name.clone()))?;
                let orbit = other
                    .orbit
                    .filter(|_| other.parent == Some(parent))
                    .ok_or_else(|| VesselError::NotOrbiting(name.clone(), body.name.clone()))?;
                // The plane's normal in the equatorial frame gives its tilt and node.
                let normal = frame.inverse() * orbit.normal();
                (normal.z.clamp(-1.0, 1.0).acos(), normal.x.atan2(-normal.y))
            }
            None => (
                start.inclination.to_radians(),
                start.longitude_of_ascending_node.to_radians(),
            ),
        };
        let rp = body.radius + start.periapsis_altitude;
        let ra = body.radius + start.apoapsis_altitude.max(start.periapsis_altitude);
        let elements = Elements {
            semi_major_axis: (rp + ra) / 2.0,
            eccentricity: (ra - rp) / (ra + rp),
            inclination,
            longitude_of_ascending_node: node,
            argument_of_periapsis: start.argument_of_periapsis.to_radians(),
            true_anomaly: 0.0,
        };
        let equatorial =
            Orbit::from_elements(body.gm, &elements, start.mean_anomaly.to_radians(), time);
        let local = equatorial.epoch_state();
        let state = StateVector::new(frame * local.position, frame * local.velocity);
        let nose = state.velocity.normalize();
        let feet = (-state.position).reject_from(nose).normalize();
        let attitude = DQuat::from_mat3(&DMat3::from_cols(feet.cross(nose), feet, nose));

        let mut vessel = Self::new(
            craft,
            catalog,
            parent,
            Motion::Coasting(Orbit::new(body.gm, time, state)),
            attitude,
            time,
        )?;
        vessel.set_free_state(system, parent, state, time);
        vessel.attitude.state.angular_velocity = DVec3::from_array(start.spin.map(f64::to_radians));
        vessel.near_surface = state.position.length() < body.coast_floor();
        vessel.nav_frame = if vessel.near_surface {
            NavFrame::Surface
        } else {
            NavFrame::Orbit
        };
        vessel.record(
            time,
            format!("{} in orbit around {}", vessel.name, body.name),
        );
        Ok(vessel)
    }

    /// Place the vessel in free flight (debug / "set orbit" cheat). Chooses the right SOI
    /// and motion mode.
    pub fn set_free_state(
        &mut self,
        system: &SolarSystem,
        frame: BodyId,
        state: StateVector,
        time: SimTime,
    ) {
        let (frame, state) = system.locate(frame, state, time);
        self.parent = frame;
        self.motion = free_motion(system.body(frame), state, time);
    }

    pub fn parent(&self) -> BodyId {
        self.parent
    }

    pub fn motion(&self) -> &Motion {
        &self.motion
    }

    pub fn is_landed(&self) -> bool {
        matches!(self.motion, Motion::Landed { .. })
    }

    pub fn is_coasting(&self) -> bool {
        matches!(self.motion, Motion::Coasting(_))
    }

    /// Broke up when a tank failed; it no longer responds to anything.
    pub fn is_destroyed(&self) -> bool {
        self.destroyed
    }

    /// Came loose from another vessel when it staged.
    pub fn is_debris(&self) -> bool {
        self.debris
    }

    /// Whether the next step needs numerical integration (rails warp is impossible).
    pub fn needs_physics(&self) -> bool {
        matches!(self.motion, Motion::Integrated(_))
            || self.propulsion.wants_thrust()
            || (self.rcs && self.translation_input != DVec3::ZERO)
            || self.executor.burning()
            || self.ullage_push
    }

    /// Mass, centre of mass and inertia as of the last physics step.
    pub fn mass_properties(&self) -> MassProperties {
        self.mass
    }

    /// How far into its travel each gear leg's strut is (see [`gear::legs`]), m.
    pub fn gear_compression(&self) -> &[f64] {
        &self.gear
    }

    /// State of the centre of mass relative to [`Self::parent`] at `time`. Exact for
    /// landed and coasting vessels; integrated vessels are extrapolated linearly from their
    /// last step, which is only meant for rendering between physics ticks.
    pub fn state_at(&self, system: &SolarSystem, time: SimTime) -> StateVector {
        let body = system.body(self.parent);
        match &self.motion {
            Motion::Landed { base, attitude } => {
                let fixed = *base + *attitude * self.mass.center_of_mass;
                let position = body.orientation(time) * fixed;
                StateVector::new(position, body.surface_velocity(position))
            }
            Motion::Coasting(orbit) => orbit.state_at(time),
            Motion::Integrated(encke) => {
                let s = encke.state();
                let dt = time.seconds_since(encke.time());
                StateVector::new(s.position + s.velocity * dt, s.velocity)
            }
        }
    }

    /// Position of the middle of the vessel's base relative to the parent.
    pub fn base_at(&self, system: &SolarSystem, time: SimTime) -> DVec3 {
        let attitude = self.attitude_at(system, time);
        self.state_at(system, time).position - attitude * self.mass.center_of_mass
    }

    pub fn attitude_at(&self, system: &SolarSystem, time: SimTime) -> DQuat {
        self.attitude_state(system, time).orientation
    }

    /// Orientation and angular velocity at `time`.
    pub fn attitude_state(&self, system: &SolarSystem, time: SimTime) -> Attitude {
        let body = system.body(self.parent);
        if let Motion::Landed { attitude, .. } = &self.motion {
            let orientation = body.orientation(time) * *attitude;
            return Attitude {
                orientation,
                angular_velocity: orientation.inverse() * body.angular_velocity(),
            };
        }
        let track = &self.attitude;
        match track.hold {
            Hold::Free => {
                attitude::coast(track.state, track.inertia, time.seconds_since(track.epoch))
            }
            Hold::Inertial => track.state,
            Hold::Tracking { frame, basis } => {
                match NavBasis::new(frame, body, &self.state_at(system, time)) {
                    Some(now) => {
                        let turn = DQuat::from_mat3(&(now.matrix() * basis.transpose()));
                        Attitude {
                            orientation: (turn * track.state.orientation).normalize(),
                            angular_velocity: DVec3::ZERO,
                        }
                    }
                    None => track.state,
                }
            }
        }
    }

    /// Restart the attitude track at `time` from wherever it is, rotating freely.
    fn rebase_attitude(&mut self, system: &SolarSystem, time: SimTime) {
        if self.is_landed() {
            return;
        }
        self.attitude = AttitudeTrack {
            epoch: time,
            state: self.attitude_state(system, time),
            inertia: self.mass.inertia,
            hold: Hold::Free,
        };
    }

    pub fn sas(&self) -> &Sas {
        &self.sas
    }

    pub fn set_sas(&mut self, system: &SolarSystem, time: SimTime, enabled: bool) {
        self.rebase_attitude(system, time);
        self.sas.set_enabled(enabled);
    }

    /// Select a SAS mode (switching SAS on).
    pub fn set_sas_mode(&mut self, system: &SolarSystem, time: SimTime, mode: SasMode) {
        self.rebase_attitude(system, time);
        self.sas.set_mode(mode);
    }

    pub fn rcs(&self) -> bool {
        self.rcs
    }

    pub fn set_rcs(&mut self, enabled: bool) {
        self.rcs = enabled && !self.destroyed;
    }

    /// Reference frame for prograde, SAS and speed. Switches to surface when entering the
    /// atmosphere or descending close to the ground, and to orbit when leaving; the player
    /// can choose in between.
    pub fn nav_frame(&self) -> NavFrame {
        self.nav_frame
    }

    /// Switch the navigation frame (the target frame only with a target).
    pub fn set_nav_frame(&mut self, system: &SolarSystem, time: SimTime, frame: NavFrame) {
        if frame == NavFrame::Target && self.target.is_none() {
            return;
        }
        self.rebase_attitude(system, time);
        self.nav_frame = frame;
    }

    /// Prograde, normal and radial directions in the current navigation frame.
    pub fn nav_basis(&self, system: &SolarSystem, time: SimTime) -> Option<NavBasis> {
        let body = system.body(self.parent);
        self.basis_at(body, &self.state_at(system, time))
    }

    fn basis_at(&self, body: &Body, state: &StateVector) -> Option<NavBasis> {
        match self.nav_frame {
            NavFrame::Target => NavBasis::relative_to(state, &self.target_state?),
            frame => NavBasis::new(frame, body, state),
        }
    }

    /// Unit vector from the vessel to its target, from the last known target state.
    fn toward_target(&self, state: &StateVector) -> Option<DVec3> {
        (self.target_state?.position - state.position).try_normalize()
    }

    /// The target's state relative to this vessel's parent as of the last update.
    pub fn target_state(&self) -> Option<StateVector> {
        self.target_state
    }

    /// The burn executor (flies the first planned burn).
    pub fn executor(&self) -> &BurnExecutor {
        &self.executor
    }

    /// Fly the first planned burn: point along it, light the engines when it starts, cut
    /// them when it is done.
    pub fn engage_burn(&mut self, system: &SolarSystem, time: SimTime) {
        if self.plan.is_empty() || self.destroyed || self.executor.engaged() {
            return;
        }
        // Arm the bottom stage if its engines are not armed yet (never fires a decoupler).
        if let Some(message) = self.propulsion.arm_bottom() {
            self.record(time, message);
        }
        self.executor.engage();
        self.set_sas_mode(system, time, SasMode::Burn);
        self.record(time, "Burn executor engaged".into());
    }

    /// Hand control back to the pilot, cutting the engines if a burn was running.
    pub fn disengage_burn(&mut self, time: SimTime) {
        if !self.executor.engaged() {
            return;
        }
        if self.executor.burning() {
            self.propulsion.set_throttle(0.0);
        }
        self.executor.disengage();
        self.ullage_push = false;
        self.record(time, "Burn executor off".into());
    }

    /// The first planned burn's Δv in inertial axes, on the trajectory as it is now; during
    /// the burn, what is still to give.
    pub fn burn_vector(&self, system: &SolarSystem, time: SimTime) -> Option<DVec3> {
        if let Some(remaining) = self.executor.remaining() {
            return Some(remaining);
        }
        let burn = self.plan.burns().first()?;
        burn.inertial(&self.orbit(system, time).state_at(burn.time))
    }

    /// When a planned burn would start and how long it would take with the bottom stage
    /// (armed or not), if it can burn at all.
    pub fn burn_timing(&self, burn: &Burn, pressure: f64) -> Option<BurnTiming> {
        let capability = Capability::estimate(&self.propulsion, pressure)?;
        Some(BurnTiming::of(burn, &capability, self.mass.mass))
    }

    /// The burns ahead of `time`, for prediction: the plan, with a burn in progress
    /// replaced by what is left of it, given now.
    pub fn burns_ahead(&self, system: &SolarSystem, time: SimTime) -> Vec<Burn> {
        let mut burns = self.plan.burns().to_vec();
        if let Some(remaining) = self.executor.remaining()
            && !burns.is_empty()
        {
            let soon = time.add_ticks(1);
            let state = self.orbit(system, time).state_at(soon);
            match Burn::from_inertial(soon, &state, remaining) {
                Some(rest) => burns[0] = rest,
                None => {
                    burns.remove(0);
                }
            }
        }
        burns
    }

    /// The earliest time the vessel needs physics ticks again although it could coast on
    /// rails until then (the burn executor waiting for its burn).
    pub fn attention_time(&self) -> Option<SimTime> {
        if !self.executor.engaged() || self.executor.burning() {
            return None;
        }
        let burn = self.plan.burns().first()?;
        let start = self.burn_timing(burn, 0.0).map_or(burn.time, |t| t.start);
        Some(start.add_seconds(-(autopilot::WARP_MARGIN + autopilot::ULLAGE_LEAD)))
    }

    /// Run the burn executor for the tick starting at `time`; returns where it wants the
    /// nose.
    fn fly_burn(
        &mut self,
        system: &SolarSystem,
        time: SimTime,
        pressure: f64,
        out: &mut Vec<VesselEvent>,
    ) -> Option<DVec3> {
        self.ullage_push = false;
        if !self.executor.engaged() {
            return None;
        }
        let Some(burn) = self.plan.burns().first().copied() else {
            self.executor.disengage();
            return None;
        };
        // The executor steers through SAS; the pilot disengages it to take SAS back.
        if !self.sas.enabled() || self.sas.mode() != SasMode::Burn {
            self.sas.set_mode(SasMode::Burn);
        }
        let delta_v = self.burn_vector(system, time).unwrap_or(DVec3::ZERO);
        let has_rcs = self
            .propulsion
            .sections
            .iter()
            .any(|s| self.propulsion.rcs_fraction(s.id).is_some_and(|f| f > 0.0));
        let situation = autopilot::Situation {
            time,
            burn: &burn,
            delta_v,
            capability: Capability::of(&self.propulsion, pressure),
            mass: self.mass.mass,
            nose: self.attitude_state(system, time).nose(),
            settled: self.propellant_settled,
            has_rcs,
            running: self.propulsion.any_engine_running(),
        };
        let command = self.executor.command(&situation);
        if let Some(throttle) = command.throttle {
            self.propulsion.set_throttle(throttle);
        }
        self.ullage_push = command.ullage && !self.is_landed();
        if let Some(finish) = command.finished {
            self.end_burn(time, finish, out);
        }
        command.direction
    }

    fn end_burn(&mut self, time: SimTime, finish: Finish, out: &mut Vec<VesselEvent>) {
        self.ullage_push = false;
        self.propulsion.set_throttle(0.0);
        let message = match finish {
            Finish::Done { residual } => {
                self.plan.remove(0);
                self.sas.set_mode(SasMode::Stability);
                format!("Burn complete, {residual:.2} m/s left over")
            }
            Finish::Aborted(why) => format!("Burn aborted: {why}"),
        };
        self.record(time, message);
        out.push(VesselEvent {
            time,
            kind: VesselEventKind::BurnEnded,
        });
    }

    /// Time warp is about to skip physics from `time`. Nothing steers on rails, so a vessel
    /// that is turning keeps turning freely, SAS or not. With SAS on, a steady vessel holds
    /// the attitude it has (relative to the navigation frame for direction modes, so
    /// prograde stays prograde) where physics ticks would keep it.
    pub fn enter_rails(&mut self, system: &SolarSystem, time: SimTime) {
        self.flight.control = DVec3::ZERO;
        rcs::idle(&mut self.propulsion);
        self.ullage_push = false;
        if self.is_landed() || !self.sas.enabled() || self.attitude.hold != Hold::Free {
            return;
        }
        let state = self.attitude_state(system, time);
        if state.angular_velocity.length() >= attitude::CAPTURE_RATE {
            return;
        }
        // A burn points a fixed inertial way and the target hardly moves across the sky
        // during a warp, so those (and stability) hold still.
        let hold = match self.nav_basis(system, time) {
            Some(basis)
                if self.nav_frame != NavFrame::Target
                    && self.sas.mode().direction(&basis).is_some() =>
            {
                Hold::Tracking {
                    frame: self.nav_frame,
                    basis: basis.matrix(),
                }
            }
            _ => Hold::Inertial,
        };
        self.attitude = AttitudeTrack {
            epoch: time,
            state: Attitude {
                orientation: state.orientation,
                angular_velocity: DVec3::ZERO,
            },
            inertia: self.mass.inertia,
            hold,
        };
    }

    /// The conic the vessel follows (or would follow if all forces stopped now).
    pub fn orbit(&self, system: &SolarSystem, time: SimTime) -> Orbit {
        match &self.motion {
            Motion::Coasting(orbit) => *orbit,
            Motion::Integrated(encke) => encke.osculating(),
            Motion::Landed { .. } => Orbit::new(
                system.body(self.parent).gm,
                time,
                self.state_at(system, time),
            ),
        }
    }

    pub fn situation(&self, system: &SolarSystem, time: SimTime) -> Situation {
        if self.is_landed() || self.wheels_down {
            return Situation::Landed;
        }
        let body = system.body(self.parent);
        let state = self.state_at(system, time);
        if body.has_atmosphere_at(state.position.length() - body.radius) {
            return Situation::Flying;
        }
        let orbit = self.orbit(system, time);
        match orbit.apoapsis() {
            None => Situation::Escaping,
            Some(ap) if ap > body.soi => Situation::Escaping,
            Some(_) if orbit.periapsis() < body.coast_floor() => Situation::SubOrbital,
            Some(_) => Situation::Orbiting,
        }
    }

    pub fn set_throttle(&mut self, throttle: f64) {
        if !self.destroyed {
            self.propulsion.set_throttle(throttle);
        }
    }

    /// Fire the next activation sequence (see [`crate::staging`]). What is left keeps
    /// moving as it was, so when stages fall away or a fairing goes the centre of mass (the
    /// vessel's position) moves along the stack. What fell away comes back as debris for
    /// the universe to add (see [`Self::shed`]).
    pub fn activate_next_stage(&mut self, system: &SolarSystem, time: SimTime) -> Vec<Vessel> {
        if self.destroyed {
            return Vec::new();
        }
        let Some(staged) = self.propulsion.activate_next_sequence() else {
            return Vec::new();
        };
        self.staged(staged, system, time)
    }

    /// The actions the player can take on `part` now (see
    /// [`Propulsion::part_actions`]); none once the vessel is destroyed.
    pub fn part_actions(&self, part: PartId) -> Vec<Activation> {
        if self.destroyed {
            return Vec::new();
        }
        self.propulsion.part_actions(part, self.rcs)
    }

    /// Do one activation by hand, outside the sequences (it leaves the one that listed it).
    /// What it releases comes back as debris, as from [`Self::activate_next_stage`]; `None`
    /// if the part does not offer it.
    pub fn perform(
        &mut self,
        activation: Activation,
        system: &SolarSystem,
        time: SimTime,
    ) -> Option<Vec<Vessel>> {
        if self.destroyed {
            return None;
        }
        let staged = self.propulsion.perform(activation, self.rcs)?;
        Some(self.staged(staged, system, time))
    }

    /// What the vessel does once `staged` happened to its parts: log it, switch RCS on, and
    /// move the centre of mass and shed the debris if mass left.
    fn staged(&mut self, staged: Staged, system: &SolarSystem, time: SimTime) -> Vec<Vessel> {
        for message in staged.messages {
            self.record(time, message);
        }
        if staged.rcs_on {
            self.rcs = true;
        }
        if !staged.mass_changed {
            return Vec::new();
        }
        let dropped = staged.dropped_length;
        let state = self.state_at(system, time);
        let attitude = self.attitude_state(system, time);
        let rest = attitude::mass_properties(&self.propulsion);
        // Off the ground, decouplers push what is left and what fell away apart, keeping
        // their momentum.
        let mut recoil = DVec3::ZERO;
        let debris = staged
            .released
            .into_iter()
            .map(|released| {
                let fairing = released.sections[0].fairing.as_ref();
                let push = if self.is_landed() || fairing.is_some_and(|f| f.half.is_some()) {
                    DVec3::ZERO
                } else {
                    // The decouplers' impulse on each side, or the usual parting speed.
                    let mass = released.mass();
                    let away = release_direction(&released, &self.propulsion);
                    let (gone, kept) = if released.impulse > 0.0 {
                        (released.impulse / mass, released.impulse / rest.mass)
                    } else {
                        let total = mass + rest.mass;
                        (
                            SEPARATION_SPEED * rest.mass / total,
                            SEPARATION_SPEED * mass / total,
                        )
                    };
                    recoil -= away * kept;
                    away * gone
                };
                self.shed(system, time, released, state, attitude, push)
            })
            .collect();
        let before = self.mass.center_of_mass;
        self.mass = rest;
        let offset =
            attitude.orientation * (DVec3::Z * dropped + self.mass.center_of_mass - before);
        let shift = StateVector::new(
            offset,
            attitude.angular_velocity_inertial().cross(offset) + attitude.orientation * recoil,
        );
        match &mut self.motion {
            // It now stands on what fell away.
            Motion::Landed { base, attitude } => *base += *attitude * DVec3::Z * dropped,
            Motion::Coasting(orbit) => {
                *orbit = Orbit::new(orbit.mu(), time, orbit.state_at(time) + shift);
            }
            Motion::Integrated(encke) => {
                *encke = Encke::new(encke.osculating().mu(), encke.time(), encke.state() + shift);
            }
        }
        if !self.is_landed() {
            self.attitude = AttitudeTrack {
                epoch: time,
                state: attitude,
                inertia: self.mass.inertia,
                hold: Hold::Free,
            };
        }
        debris
    }

    /// What `released` from this vessel as a vessel of its own, `state` and `attitude` being
    /// this vessel's just before. It starts where it was on the vessel, moving and turning
    /// with it, pushed `push` (m/s, vessel axes); a fairing half also swings open about its
    /// base. On the ground, fallen stages stay standing where they were.
    fn shed(
        &self,
        system: &SolarSystem,
        time: SimTime,
        released: Released,
        state: StateVector,
        attitude: Attitude,
        push: DVec3,
    ) -> Vessel {
        let name = format!("{} ({})", released.sections[0].name, self.name);
        let half = released.sections[0].fairing.as_ref().and_then(|f| f.half);
        let height = released.base;
        let propulsion = Propulsion::from_released(released);
        let mut debris = Vessel::assemble(
            name,
            propulsion,
            self.parent,
            self.motion.clone(),
            attitude.orientation,
            time,
        );
        let com = debris.mass.center_of_mass;
        let arm = attitude.orientation * (DVec3::Z * height + com - self.mass.center_of_mass);
        let mut velocity = state.velocity
            + attitude.angular_velocity_inertial().cross(arm)
            + attitude.orientation * push;
        let mut spin = attitude.angular_velocity;
        if let Some(angle) = half {
            let out = DVec3::new(angle.cos(), angle.sin(), 0.0);
            spin += DVec3::Z.cross(out) * FAIRING_SWING;
            velocity += attitude.orientation * out * (FAIRING_SWING * com.z + FAIRING_PUSH);
        }
        debris.motion = match self.motion {
            Motion::Landed { base, attitude } if half.is_none() => Motion::Landed {
                base: base + attitude * DVec3::Z * height,
                attitude,
            },
            _ => {
                let state = StateVector::new(state.position + arm, velocity);
                free_motion(system.body(self.parent), state, time)
            }
        };
        debris.attitude.state.angular_velocity = spin;
        debris.debris = true;
        debris.near_surface = self.near_surface;
        debris.nav_frame = self.nav_frame;
        debris.propellant_settled = self.propellant_settled;
        debris.record(time, format!("Separated from {}", self.name));
        debris
    }

    /// Next trajectory event in `(after, until]`, if coasting.
    pub fn next_trajectory_event(
        &self,
        system: &SolarSystem,
        after: SimTime,
        until: SimTime,
    ) -> Option<TrajectoryEvent> {
        match &self.motion {
            Motion::Coasting(orbit) => next_event(system, self.parent, orbit, after, until),
            _ => None,
        }
    }

    /// Apply a trajectory event found by [`Self::next_trajectory_event`].
    pub fn apply_trajectory_event(
        &mut self,
        system: &SolarSystem,
        event: TrajectoryEvent,
        out: &mut Vec<VesselEvent>,
    ) {
        let Motion::Coasting(orbit) = self.motion else {
            return;
        };
        let from = self.parent;
        match event.kind {
            EventKind::Descent => {
                let state = orbit.state_at(event.time);
                self.motion = Motion::Integrated(Encke::new(orbit.mu(), event.time, state));
                let body = system.body(from);
                let what = if body.atmosphere.is_some() {
                    "Entering atmosphere of"
                } else {
                    "Low altitude over"
                };
                self.record(event.time, format!("{what} {}", body.name));
                out.push(VesselEvent {
                    time: event.time,
                    kind: VesselEventKind::Descent { body: from },
                });
            }
            EventKind::SoiExit | EventKind::SoiEntry(_) => {
                let (to, orbit) = apply_event(system, from, &orbit, event);
                self.parent = to;
                self.motion = Motion::Coasting(orbit);
                self.record(
                    event.time,
                    format!("Entered {} sphere of influence", system.body(to).name),
                );
                out.push(VesselEvent {
                    time: event.time,
                    kind: VesselEventKind::SoiChange { from, to },
                });
            }
        }
    }

    /// One physics tick from `t0` to `t1`.
    pub fn step(
        &mut self,
        system: &SolarSystem,
        t0: SimTime,
        t1: SimTime,
        out: &mut Vec<VesselEvent>,
    ) {
        let dt = t1.seconds_since(t0);
        let body = system.body(self.parent);
        let state = self.state_at(system, t0);
        let air = air_at(body, &state);
        let aim = self.fly_burn(system, t0, air.pressure, out);
        let mut messages = Vec::new();
        self.propulsion
            .burn(air.pressure, dt, self.propellant_settled, &mut messages);
        for message in messages {
            self.record(t0, message);
        }
        self.mass = attitude::mass_properties(&self.propulsion);
        let mass = self.mass.mass;

        let near_surface = self.is_landed() || state.position.length() < body.coast_floor();
        if near_surface != self.near_surface {
            self.near_surface = near_surface;
            self.nav_frame = if near_surface {
                NavFrame::Surface
            } else {
                NavFrame::Orbit
            };
        }

        // Rotation over this tick, under the torque commanded at its start.
        let start = self.attitude_state(system, t0);
        let com = self.mass.center_of_mass;

        // The air, felt from the attitude at the start of the tick and again as the vessel
        // turns within it, with the control surfaces where the command puts them.
        let air_velocity = state.velocity - body.surface_velocity(state.position);
        let shape = (!self.is_landed() && air.density > 0.0).then(|| self.propulsion.shape());
        let plume = self.propulsion.engines.iter().any(|e| e.thrust > 0.0);
        let spin = body.angular_velocity();
        let flow_at = |attitude: Attitude, controls: DVec3| {
            flow(attitude, air_velocity, spin, air, plume, controls)
        };

        let rcs = (self.rcs || self.ullage_push) && !self.is_landed();
        let mut authority = attitude::torque_authority(&self.propulsion, com);
        if rcs {
            authority += rcs::torque_authority(&self.propulsion, com.z, air.pressure);
        }
        if let Some(shape) = &shape {
            authority += shape.control_authority(&flow_at(start, DVec3::ZERO), com);
        }
        let command = if self.is_landed() || self.destroyed {
            DVec3::ZERO
        } else {
            let target = aim.or_else(|| self.sas_direction(system, body, &state, t0));
            self.rotation_command(start, authority, target)
        };
        let (mut force, mut torque) = attitude::actuate(&mut self.propulsion, com, command);
        if rcs {
            let push = if self.ullage_push {
                DVec3::Z
            } else {
                DVec3::ZERO
            };
            let translation = (self.translation_input + push).clamp(DVec3::NEG_ONE, DVec3::ONE);
            let (f, t) = rcs::fire(
                &mut self.propulsion,
                com.z,
                command,
                translation,
                air.pressure,
                dt,
            );
            // Its torque about the centre of mass where that is off the axis.
            force += f;
            torque += t - (com * DVec3::new(1.0, 1.0, 0.0)).cross(f);
        } else {
            rcs::idle(&mut self.propulsion);
        }
        let air_loads = |attitude: Attitude| {
            shape.as_ref().map_or(Loads::default(), |shape| {
                shape.loads_about(&flow_at(attitude, command), com)
            })
        };
        let air_now = air_loads(start);

        // On or near the ground on landing gear, rotation and translation are integrated
        // together in short substeps (`roll`). Parked on its wheels, a vessel rolls away as
        // soon as anything pushes it; come to rest on anything else, it stays put.
        let legs = gear::legs(&self.propulsion.sections);
        let rolling = !legs.is_empty()
            && match &self.motion {
                Motion::Integrated(encke) => near_ground(body, &encke.state(), t0, &legs, com),
                Motion::Landed { .. } => force != DVec3::ZERO && self.wheels_down,
                Motion::Coasting(_) => false,
            };

        let track = &self.attitude;
        if !self.is_landed()
            && !rolling
            && (torque != DVec3::ZERO
                || shape.is_some()
                || track.inertia != self.mass.inertia
                || track.hold != Hold::Free)
        {
            let inertia = self.mass.inertia;
            let end = if shape.is_some() {
                attitude::integrate_with(start, inertia, dt, |a| torque + air_loads(a).torque)
            } else if torque == DVec3::ZERO {
                attitude::coast(start, inertia, dt)
            } else {
                attitude::integrate(start, inertia, torque, dt)
            };
            self.attitude = AttitudeTrack {
                epoch: t1,
                state: end,
                inertia,
                hold: Hold::Free,
            };
        }

        let thrust = force.length();
        let thrust_accel = start.orientation * force / mass;
        let tail_off = self.propulsion.tail_off_impulse(air.pressure) / mass;
        if let Some(finish) = self.executor.account(
            thrust_accel * dt,
            self.propulsion.any_engine_running(),
            tail_off,
        ) {
            self.end_burn(t0, finish, out);
        }
        let air_accel = start.orientation * air_now.force / mass;
        let proper = thrust_accel + air_accel;
        let air_speed = air_velocity.length();
        let airflow = start.orientation.inverse() * air_velocity;
        let along = air_now.force.dot(airflow.normalize_or_zero());
        let structure_load = self.flight.structure_load;
        self.flight = FlightData {
            mass,
            thrust,
            dynamic_pressure: 0.5 * air.density * air_speed * air_speed,
            mach: if air.speed_of_sound > 0.0 {
                air_speed / air.speed_of_sound
            } else {
                0.0
            },
            angle_of_attack: if air_speed > 1.0 {
                airflow.y.atan2(airflow.z)
            } else {
                0.0
            },
            sideslip: if air_speed > 1.0 {
                (airflow.x / air_speed).clamp(-1.0, 1.0).asin()
            } else {
                0.0
            },
            lift: (air_now.force - airflow.normalize_or_zero() * along).length(),
            drag: -along,
            static_pressure: air.pressure,
            proper_acceleration: proper.length(),
            control: command,
            torque_authority: authority,
            structure_load,
        };
        let nose = start.nose();
        self.propellant_settled = self.is_landed() || proper.dot(nose) > ULLAGE_ACCELERATION;

        let conditions = Conditions {
            // On the ground, the ground holds the stack up, until the engines lift more
            // than its weight: it pushes, but never holds it down.
            acceleration: if self.is_landed() {
                let gravity = body.gm / state.position.length_squared()
                    * state.position.normalize().dot(nose);
                gravity.max(proper.dot(nose))
            } else {
                proper.dot(nose)
            },
            drag: (-air_now.force.z).max(0.0),
            dynamic_pressure: self.flight.dynamic_pressure,
            mach: self.flight.mach,
            angle_of_attack: if air_speed > 1.0 {
                nose.angle_between(air_velocity)
            } else {
                0.0
            },
            landed: self.is_landed(),
        };
        self.check_structure(&conditions, t0, out);

        let rolled = Rolled {
            thrust: force,
            torque,
            legs: &legs,
            shape: shape.as_deref(),
            air,
            plume,
            command,
        };
        match self.motion {
            Motion::Landed { .. } if rolling => {
                self.motion = Motion::Integrated(Encke::new(body.gm, t0, state));
                self.attitude = AttitudeTrack {
                    epoch: t0,
                    state: start,
                    inertia: self.mass.inertia,
                    hold: Hold::Free,
                };
                self.roll(system, t0, t1, &rolled, out);
            }
            // Parked on its gear.
            Motion::Landed { .. } if !legs.is_empty() => {}
            Motion::Landed { .. } => {
                let up = state.position.normalize();
                let weight = body.gm / state.position.length_squared();
                if thrust_accel.dot(up) <= weight {
                    return;
                }
                self.motion = Motion::Integrated(Encke::new(body.gm, t0, state));
                self.attitude = AttitudeTrack {
                    epoch: t0,
                    state: start,
                    inertia: self.mass.inertia,
                    hold: Hold::Free,
                };
                self.record(t0, "Liftoff!".into());
                out.push(VesselEvent {
                    time: t0,
                    kind: VesselEventKind::Liftoff,
                });
                self.integrate(system, t1, thrust_accel, air_accel, out);
            }
            Motion::Coasting(orbit) if thrust > 0.0 => {
                self.motion = Motion::Integrated(Encke::new(orbit.mu(), t0, orbit.state_at(t0)));
                self.integrate(system, t1, thrust_accel, air_accel, out);
            }
            Motion::Coasting(_) => {
                let mut t = t0;
                while let Some(event) = self.next_trajectory_event(system, t, t1) {
                    self.apply_trajectory_event(system, event, out);
                    t = event.time;
                    if event.kind == EventKind::Descent {
                        // Finish this tick under physics from the exact event time.
                        self.integrate(system, t1, DVec3::ZERO, DVec3::ZERO, out);
                        break;
                    }
                }
            }
            Motion::Integrated(_) if rolling => self.roll(system, t0, t1, &rolled, out),
            Motion::Integrated(_) => self.integrate(system, t1, thrust_accel, air_accel, out),
        }
    }

    /// Integrate from `t0` to `t1` on or near the ground on landing gear: rotation and
    /// translation together, in substeps short enough for the stiff struts, with the
    /// wheels', the skidding hull's and the air's forces worked out afresh for each. Then
    /// note touching down and leaving the ground, stop if the hull hit the ground hard (a
    /// crash), and park once at rest.
    fn roll(
        &mut self,
        system: &SolarSystem,
        t0: SimTime,
        t1: SimTime,
        rolled: &Rolled,
        out: &mut Vec<VesselEvent>,
    ) {
        let body = system.body(self.parent);
        let (mass, com, inertia) = (self.mass.mass, self.mass.center_of_mass, self.mass.inertia);
        let spin = body.angular_velocity();
        let mut attitude = self.attitude_state(system, t0);
        let brakes = self.brakes;
        let hull = hard_points(&self.propulsion);
        let Motion::Integrated(encke) = &mut self.motion else {
            return;
        };
        let mut contact = Contact::default();
        let mut impact: f64 = 0.0;
        let mut t = t0;
        while t < t1 {
            let next = (t + GEAR_SUBSTEP).min(t1);
            let h = next.seconds_since(t);
            let state = encke.state();
            contact = gear::contact(
                rolled.legs,
                body,
                t,
                &state,
                &attitude,
                com,
                brakes,
                rolled.command.y,
            );
            let points = hull.world(&state, &attitude, com);
            impact = impact.max(gear::skid(
                &points,
                body,
                t,
                &state,
                &attitude,
                mass,
                &mut contact,
            ));
            let air_velocity = state.velocity - body.surface_velocity(state.position);
            let air_loads = |a: Attitude| {
                rolled.shape.map_or(Loads::default(), |shape| {
                    let flow = flow(
                        a,
                        air_velocity,
                        spin,
                        rolled.air,
                        rolled.plume,
                        rolled.command,
                    );
                    shape.loads_about(&flow, com)
                })
            };
            let push = attitude.orientation * (rolled.thrust + air_loads(attitude).force);
            let accel = (push + contact.force) / mass;
            let turning = rolled.torque + contact.torque;
            attitude =
                attitude::integrate_with(attitude, inertia, h, |a| turning + air_loads(a).torque);
            encke.step(next, |_, _| accel);
            t = next;
        }
        let state = encke.state();
        self.attitude = AttitudeTrack {
            epoch: t1,
            state: attitude,
            inertia,
            hold: Hold::Free,
        };

        let up = state.position.normalize();
        let ground_velocity = state.velocity - body.surface_velocity(state.position);
        let touching = contact.touching();
        if touching && !self.wheels_down {
            let sink = (-ground_velocity.dot(up)).max(0.0);
            self.record(t1, format!("Touched down at {sink:.1} m/s"));
            out.push(VesselEvent {
                time: t1,
                kind: VesselEventKind::Landed { speed: sink },
            });
        } else if !touching && self.wheels_down {
            self.record(t1, "Airborne".into());
            out.push(VesselEvent {
                time: t1,
                kind: VesselEventKind::Liftoff,
            });
        }
        self.wheels_down = touching;
        self.gear = contact.compression;

        let to_fixed = body.orientation(t1).inverse();
        let base = state.position - attitude.orientation * com;
        let parked = Motion::Landed {
            base: to_fixed * base,
            attitude: to_fixed * attitude.orientation,
        };
        if impact > HULL_IMPACT_SPEED {
            self.record(t1, format!("Crashed into {} at {impact:.0} m/s", body.name));
            out.push(VesselEvent {
                time: t1,
                kind: VesselEventKind::Crashed { speed: impact },
            });
            self.motion = parked;
            self.wheels_down = false;
            return;
        }
        let rate = attitude.angular_velocity - attitude.orientation.inverse() * spin;
        if (touching || contact.force != DVec3::ZERO)
            && rolled.thrust == DVec3::ZERO
            && ground_velocity.length() < REST_SPEED
            && rate.length() < REST_RATE
        {
            self.motion = parked;
        }
    }

    /// Check every tank and fin set against its strength. Overloaded fins tear off; an
    /// overloaded tank breaks the vessel up.
    fn check_structure(
        &mut self,
        conditions: &Conditions,
        time: SimTime,
        out: &mut Vec<VesselEvent>,
    ) {
        if self.destroyed {
            return;
        }
        self.joint_loads = structure::joint_loads(&self.propulsion, conditions);
        self.structure = structure::loads(&self.propulsion, conditions);
        self.flight.structure_load = self
            .structure
            .iter()
            .map(|l| l.fraction)
            .fold(0.0, f64::max);
        let failed: Vec<PartLoad> = self
            .structure
            .iter()
            .filter(|l| l.fraction > 1.0)
            .copied()
            .collect();
        for load in failed {
            let name = self.propulsion.sections[load.stage].name.clone();
            let percent = load.fraction * 100.0;
            match load.part {
                Part::Fins => {
                    self.propulsion.tear_off_fins(load.stage);
                    self.record(
                        time,
                        format!("{name}: fins torn off at {percent:.0}% of their strength"),
                    );
                }
                Part::Tank(_) if !self.destroyed => {
                    let what = match load.mode {
                        structure::Mode::Burst => "burst",
                        _ => "buckled",
                    };
                    self.destroyed = true;
                    self.propulsion.shut_down();
                    self.rcs = false;
                    self.sas.set_enabled(false);
                    self.record(
                        time,
                        format!(
                            "{name}: tank {what} at {percent:.0}% of its strength. Vessel destroyed"
                        ),
                    );
                    out.push(VesselEvent {
                        time,
                        kind: VesselEventKind::Destroyed,
                    });
                }
                Part::Tank(_) => {}
            }
        }
    }

    /// Where the SAS mode wants the nose (inertial), if that is defined right now.
    fn sas_direction(
        &self,
        system: &SolarSystem,
        body: &Body,
        state: &StateVector,
        time: SimTime,
    ) -> Option<DVec3> {
        match self.sas.mode() {
            SasMode::Burn => self
                .burn_vector(system, time)
                .and_then(DVec3::try_normalize),
            SasMode::Target => self.toward_target(state),
            SasMode::AntiTarget => self.toward_target(state).map(|d| -d),
            mode => self.basis_at(body, state).and_then(|b| mode.direction(&b)),
        }
    }

    /// The pilot's input if any, otherwise what SAS wants (`target`: where to point).
    fn rotation_command(
        &mut self,
        attitude: Attitude,
        authority: DVec3,
        target: Option<DVec3>,
    ) -> DVec3 {
        if self.rotation_input != DVec3::ZERO {
            self.sas.release();
            return self.rotation_input.clamp(DVec3::NEG_ONE, DVec3::ONE);
        }
        if !self.sas.enabled() && target.is_none() {
            return DVec3::ZERO;
        }
        let alpha = authority / self.mass.inertia.diagonal().max(DVec3::splat(1e-9));
        self.sas.steer(attitude, alpha, target)
    }

    /// Integrate to `end`, then resolve ground contact, SOI changes and a return to coasting.
    fn integrate(
        &mut self,
        system: &SolarSystem,
        end: SimTime,
        thrust_accel: DVec3,
        air_accel: DVec3,
        out: &mut Vec<VesselEvent>,
    ) {
        let Motion::Integrated(encke) = &mut self.motion else {
            return;
        };
        let body = system.body(self.parent);
        encke.step(end, |_, _| thrust_accel + air_accel);
        let state = encke.state();

        // Ground contact at the base of the stack, while moving towards the ground.
        let attitude = self.attitude_state(system, end);
        let base = state.position - attitude.orientation * self.mass.center_of_mass;
        let orientation = body.orientation(end);
        let fixed_dir = orientation.inverse() * base.normalize();
        let surface = body.radius + body.terrain_height(fixed_dir);
        let ground_velocity = state.velocity - body.surface_velocity(state.position);
        if base.length() < surface && ground_velocity.dot(base) < 0.0 {
            let speed = ground_velocity.length();
            let kind = if speed > SAFE_LANDING_SPEED {
                // TODO: destroy parts based on impact tolerance.
                self.record(end, format!("Crashed into {} at {speed:.0} m/s", body.name));
                VesselEventKind::Crashed { speed }
            } else {
                self.record(end, format!("Landed on {} at {speed:.1} m/s", body.name));
                VesselEventKind::Landed { speed }
            };
            out.push(VesselEvent { time: end, kind });
            self.motion = Motion::Landed {
                base: fixed_dir * surface,
                attitude: orientation.inverse() * attitude.orientation,
            };
            return;
        }

        // Sphere-of-influence changes (state-based: at most one tick late under thrust).
        let (frame, local) = system.locate(self.parent, state, end);
        if frame != self.parent {
            let from = self.parent;
            self.parent = frame;
            self.motion = Motion::Integrated(Encke::new(system.body(frame).gm, end, local));
            self.record(
                end,
                format!("Entered {} sphere of influence", system.body(frame).name),
            );
            out.push(VesselEvent {
                time: end,
                kind: VesselEventKind::SoiChange { from, to: frame },
            });
        }

        // Back to exact coasting once only gravity acts.
        if thrust_accel == DVec3::ZERO
            && !self.propulsion.any_engine_running()
            && let Motion::Integrated(encke) = &self.motion
        {
            let body = system.body(self.parent);
            let state = encke.state();
            if can_coast(body, &state) {
                self.motion = Motion::Coasting(Orbit::new(body.gm, end, state));
            }
        }
    }

    fn record(&mut self, time: SimTime, message: String) {
        if self.log.len() == MAX_LOG {
            self.log.remove(0);
        }
        self.log.push(LogEntry { time, message });
    }
}

/// Which way a decoupler pushes what it `released` from `rest`, vessel axes: straight out
/// from the stage it hung beside, or back down the axis from the one it hung under.
fn release_direction(released: &Released, rest: &Propulsion) -> DVec3 {
    let gone = |id: Option<usize>| released.sections.iter().any(|s| Some(s.id) == id);
    let top = released.sections.iter().find(|s| !gone(s.parent));
    let holder = top.and_then(|top| rest.sections.iter().find(|h| Some(h.id) == top.parent));
    match (top, holder) {
        (Some(top), Some(holder)) if top.beside => (top.bottom - holder.bottom)
            .truncate()
            .try_normalize()
            .map_or(DVec3::NEG_Z, |out| out.extend(0.0)),
        _ => DVec3::NEG_Z,
    }
}

/// The points of a vessel that can touch the ground besides its wheels: rings round its
/// body (the middle of each, stack coordinates, and how far it reaches from there) and its
/// wing tips (stack coordinates).
struct HardPoints {
    rings: Vec<(DVec3, f64)>,
    tips: Vec<DVec3>,
}

fn hard_points(propulsion: &Propulsion) -> HardPoints {
    let mut tips = Vec::new();
    for (base, stage) in placed(&propulsion.sections) {
        let axis = stage.bottom.truncate().extend(0.0);
        for wing in &stage.wings {
            for panel in Panel::of(wing, stage.diameter / 2.0, base) {
                let tip = panel.tip + axis;
                tips.extend([tip, tip - DVec3::Z * panel.tip_chord]);
            }
        }
    }
    HardPoints {
        rings: propulsion.shape().rings().collect(),
        tips,
    }
}

impl HardPoints {
    /// Inertial positions for a vessel at `state` turned to `attitude`, its centre of mass
    /// at `com` (see [`MassProperties::center_of_mass`]): the lowest point of each ring of
    /// the outline, and the wing tips.
    fn world(&self, state: &StateVector, attitude: &Attitude, com: DVec3) -> Vec<DVec3> {
        let nose = attitude.nose();
        let below = (-state.position).reject_from(nose).normalize_or_zero();
        let to_world = |p: DVec3| state.position + attitude.orientation * (p - com);
        self.rings
            .iter()
            .map(|&(middle, r)| to_world(middle) + below * r)
            .chain(self.tips.iter().map(|&p| to_world(p)))
            .collect()
    }
}

/// What drives a vessel rolling on its gear over one tick (see `Vessel::roll`).
struct Rolled<'a> {
    /// Thrust (engines and RCS) and actuator torque, vessel axes.
    thrust: DVec3,
    torque: DVec3,
    legs: &'a [Leg],
    shape: Option<&'a Shape>,
    air: AtmosphereSample,
    plume: bool,
    /// The rotation command, which the control surfaces and steering follow.
    command: DVec3,
}

/// How a vessel turned to `attitude` meets the air, flying at `air_velocity` (inertial)
/// through air that turns with its body at `spin`.
fn flow(
    attitude: Attitude,
    air_velocity: DVec3,
    spin: DVec3,
    air: AtmosphereSample,
    plume: bool,
    controls: DVec3,
) -> Flow {
    let to_vessel = attitude.orientation.inverse();
    Flow {
        velocity: to_vessel * air_velocity,
        angular_velocity: attitude.angular_velocity - to_vessel * spin,
        air,
        plume,
        controls,
    }
}

/// Close enough to the ground that its gear might touch within the tick.
fn near_ground(body: &Body, state: &StateVector, time: SimTime, legs: &[Leg], com: DVec3) -> bool {
    let reach = legs
        .iter()
        .map(|l| (l.axle(0.0) - com).length() + l.wheel_radius)
        .fold(0.0, f64::max);
    let fixed = body.orientation(time).inverse() * state.position.normalize();
    let height = state.position.length() - body.radius - body.terrain_height(fixed);
    height < reach + GEAR_MARGIN
}

fn free_motion(body: &Body, state: StateVector, time: SimTime) -> Motion {
    if can_coast(body, &state) {
        Motion::Coasting(Orbit::new(body.gm, time, state))
    } else {
        Motion::Integrated(Encke::new(body.gm, time, state))
    }
}

/// Above the coast floor and not about to cross it.
fn can_coast(body: &Body, state: &StateVector) -> bool {
    let floor = body.coast_floor();
    let r = state.position.length();
    let descent_rate = (-state.radial_speed()).max(0.0);
    r - floor > descent_rate * COAST_HYSTERESIS
}

fn air_at(body: &Body, state: &StateVector) -> AtmosphereSample {
    let altitude = state.position.length() - body.radius;
    body.atmosphere
        .as_ref()
        .map_or(AtmosphereSample::VACUUM, |a| a.sample(altitude))
}

#[cfg(test)]
mod glider;
#[cfg(test)]
mod radial;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        data::ScenarioDef, data::StagedCraftDef, data::StartDef, parts::PartKind,
        propulsion::FAIRING_HALVES, system::tests::rss, time::Ticks,
    };

    use crate::test_data::records;

    pub fn catalog() -> PartCatalog {
        PartCatalog {
            engines: records(include_str!("../../../assets/core/engines/stock.json5")),
            resources: records(include_str!("../../../assets/core/resources/stock.json5")),
            materials: records(include_str!("../../../assets/core/materials/stock.json5")),
            tank_types: records(include_str!("../../../assets/core/tank_types/stock.json5")),
            reaction_wheels: records(include_str!(
                "../../../assets/core/reaction_wheels/stock.json5"
            )),
            rcs: records(include_str!("../../../assets/core/rcs/stock.json5")),
            pods: records(include_str!("../../../assets/core/pods/stock.json5")),
            batteries: records(include_str!(
                "../../../mods/realism/batteries/batteries.json5"
            )),
            liveries: Vec::new(),
        }
    }

    pub fn pathfinder() -> CraftDef {
        records(include_str!("../../../assets/core/craft/pathfinder.json5")).remove(0)
    }

    /// Pathfinder as the stack of stages it was converted from, for tests of the stage
    /// form.
    pub fn staged_pathfinder() -> StagedCraftDef {
        records(STAGED_PATHFINDER).remove(0)
    }

    const STAGED_PATHFINDER: &str = r#"{
  id: "pathfinder",
  name: "Pathfinder",
  stages: [
    {
      name: "Booster",
      engines: [
        "K-1 Booster", "K-1 Booster", "K-1 Booster",
        "K-1 Booster", "K-1 Booster", "K-1 Booster",
        "K-1 Booster", "K-1 Booster", "K-1 Booster",
      ],
      tanks: [
        { type: "Isogrid Al-Li", length: 38, propellants: { Kerosene: 1, LqdOxygen: 2.36 } },
      ],
      structure_mass: 9400, // thrust structure, interstage, avionics
      diameter: 3.7,
      length: 42,
      // The interstage is the 4 m above the tank, around the upper stage's engine.
      appearance: { legs: 4, grid_fins: 4, raceway: true },
    },
    {
      name: "Upper Stage",
      engines: ["K-1 Vacuum"],
      tanks: [
        // Full while the booster pulls 6 g, so built for 6.5 g.
        {
          type: "Isogrid Al-Li",
          length: 9.6,
          design_load: 6.5,
          propellants: { Kerosene: 1, LqdOxygen: 2.36 },
        },
        // RCS nitrogen in a bottle inside the interstage.
        { type: "COPV", shape: "sphere", diameter: 1.35, propellants: { Nitrogen: 1 } },
      ],
      reaction_wheels: ["Attitude Control Unit"],
      rcs: ["Cold Gas Quad", "Cold Gas Quad", "Cold Gas Quad", "Cold Gas Quad"],
      structure_mass: 1830, // interstage, avionics, payload adapter
      diameter: 3.7,
      length: 14,
      appearance: { raceway: true },
    },
    {
      name: "Payload",
      structure_mass: 15000,
      // Two composite halves and the nose, jettisoned once out of the thick air.
      fairing: { mass: 1900 },
      diameter: 3.7,
      length: 11,
      // The fairing's nose: a tangent ogive 1.3 diameters long.
      shells: [{ length: 4.81, top_diameter: 0, profile: "ogive" }],
    },
  ],
}"#;

    pub fn upper_stage() -> CraftDef {
        records(include_str!("../../../assets/core/craft/upper_stage.json5")).remove(0)
    }

    pub fn lunar_upper() -> CraftDef {
        records(include_str!("../../../assets/core/craft/lunar_upper.json5")).remove(0)
    }

    pub fn launch_sites() -> Vec<LaunchSiteDef> {
        records(include_str!(
            "../../../assets/core/launch_sites/stock.json5"
        ))
    }

    pub fn pad_vessel(system: &SolarSystem) -> (Vessel, SimTime) {
        let scenarios: Vec<ScenarioDef> =
            records(include_str!("../../../assets/core/scenarios/stock.json5"));
        let scenario = scenarios.into_iter().find(|s| s.id == "pad").unwrap();
        let StartDef::LaunchSite(start) = &scenario.start else {
            panic!("pad scenario")
        };
        let site = launch_sites()
            .into_iter()
            .find(|s| s.id == start.site)
            .unwrap();
        let t = SimTime::from_seconds(scenario.start_time);
        (
            Vessel::on_launch_site(&pathfinder(), &catalog(), system, &site, t).unwrap(),
            t,
        )
    }

    /// The upper stage alone in a 300 km circular orbit.
    pub fn leo_vessel(system: &SolarSystem) -> (Vessel, SimTime) {
        let start = OrbitStartDef {
            body: "Earth".into(),
            periapsis_altitude: 300e3,
            apoapsis_altitude: 300e3,
            inclination: 28.6,
            longitude_of_ascending_node: 0.0,
            plane: None,
            argument_of_periapsis: 0.0,
            mean_anomaly: 0.0,
            spin: [0.0; 3],
        };
        let t = SimTime::J2000;
        (
            Vessel::in_orbit(&upper_stage(), &catalog(), system, &start, t).unwrap(),
            t,
        )
    }

    const DT: Ticks = Ticks::from_millis(20);

    pub fn run(v: &mut Vessel, system: &SolarSystem, t: &mut SimTime, seconds: f64) {
        let mut events = Vec::new();
        for _ in 0..(seconds * 50.0).round() as usize {
            v.step(system, *t, *t + DT, &mut events);
            *t = *t + DT;
        }
    }

    pub(super) fn nose(v: &Vessel, system: &SolarSystem, t: SimTime) -> DVec3 {
        v.attitude_at(system, t) * DVec3::Z
    }

    #[test]
    fn pad_vessel_sits_on_rotating_surface() {
        let system = rss();
        let (mut v, start) = pad_vessel(&system);
        let earth = system.body(v.parent());
        // On the launch mount, above the (smooth) ground.
        let mount = launch_sites()[0].pad.as_ref().unwrap().mount_height;
        let ground = earth.radius + mount;
        assert!((v.base_at(&system, start).length() - ground).abs() < 1e-6);
        let s = v.state_at(&system, start);
        // The centre of mass is up the stack; Kennedy at 28.6°N moves east at ~408 m/s.
        let com = v.mass_properties().center_of_mass.z;
        assert!(com > 20.0 && com < 40.0, "{com}");
        assert!(((s.position.length() - ground) - com).abs() < 1e-6);
        assert!(
            (s.velocity.length() - 408.0).abs() < 2.0,
            "{}",
            s.velocity.length()
        );
        assert!(nose(&v, &system, start).dot(s.position.normalize()) > 0.9999);
        let mut t = start;
        run(&mut v, &system, &mut t, 10.0);
        assert!(v.is_landed());
        // A year later it is still exactly on the ground.
        let later = t.add_seconds(365.25 * 86_400.0);
        assert!((v.base_at(&system, later).length() - ground).abs() < 1e-6);
    }

    #[test]
    fn pad_vessel_stands_on_the_pad_deck() {
        let mut system = rss();
        let sites = launch_sites();
        crate::terrain::install(&mut system, Vec::new(), &sites);
        let (mut v, start) = pad_vessel(&system);
        let earth = system.body(v.parent());
        let pad = sites[0].pad.as_ref().unwrap();
        let deck = pad.elevation.unwrap() + pad.mount_height;
        let altitude = |v: &Vessel, t| v.base_at(&system, t).length() - earth.radius;
        assert!((altitude(&v, start) - deck).abs() < 1e-6);
        // It stays put on the raised deck rather than sinking or bouncing.
        let mut t = start;
        run(&mut v, &system, &mut t, 4.0);
        assert!(v.is_landed());
        assert!((altitude(&v, t) - deck).abs() < 1e-6);
    }

    #[test]
    fn stack_has_orbital_delta_v() {
        let system = rss();
        let (v, _) = pad_vessel(&system);
        let total: f64 = v
            .propulsion
            .stage_info()
            .iter()
            .map(|s| s.delta_v_vac)
            .sum();
        assert!(total > 9_400.0, "{total}");
    }

    fn launch(system: &SolarSystem) -> (Vessel, SimTime) {
        let (mut v, t) = pad_vessel(system);
        v.set_throttle(1.0);
        v.activate_next_stage(system, t);
        (v, t)
    }

    /// The Pathfinder's sequences, as `kind:stage name`, once for each stage's parts.
    fn sequence_names(v: &Vessel) -> Vec<Vec<String>> {
        let stages = craft_stage_names();
        let parts = v.propulsion.parts();
        v.propulsion
            .sequences
            .list()
            .iter()
            .map(|seq| {
                let mut names: Vec<String> = seq
                    .iter()
                    .map(|a| {
                        let stage = parts.get(a.part()).expect("on the vessel").stage;
                        format!("{}:{}", a.kind(), stages[stage])
                    })
                    .collect();
                names.dedup();
                names
            })
            .collect()
    }

    fn craft_stage_names() -> Vec<String> {
        let propulsion = Propulsion::from_craft(&pathfinder(), &catalog()).unwrap();
        propulsion.sections.into_iter().map(|s| s.name).collect()
    }

    #[test]
    fn default_sequences_light_separate_jettison_and_release() {
        let system = rss();
        let (v, _) = pad_vessel(&system);
        assert_eq!(
            sequence_names(&v),
            [
                vec!["engines:Booster"],
                vec![
                    "decoupler:Booster",
                    "engines:Upper Stage",
                    "rcs:Upper Stage"
                ],
                vec!["fairing:Payload"],
                vec!["decoupler:Upper Stage"],
            ]
        );
    }

    #[test]
    fn staging_follows_the_sequences_and_the_fairing_takes_its_mass() {
        let system = rss();
        let (mut v, mut t) = launch(&system);
        assert!(
            v.propulsion.armed(v.propulsion.sections[0].id) && v.propulsion.sections.len() == 3
        );
        run(&mut v, &system, &mut t, 20.0);
        // Separation, upper stage ignition and RCS on in one press.
        v.activate_next_stage(&system, t);
        assert_eq!(v.propulsion.sections.len(), 2);
        assert!(v.propulsion.armed(v.propulsion.sections[0].id));
        assert!(v.rcs());
        // The fairing goes with its mass; the payload stays.
        let mass = v.propulsion.mass();
        v.activate_next_stage(&system, t);
        assert_eq!(v.propulsion.sections.len(), 2);
        assert!((mass - v.propulsion.mass() - 1900.0).abs() < 1e-6);
        assert!(v.log.iter().any(|e| e.message == "Fairing jettisoned"));
        // Payload separation leaves the payload alone; then nothing is left to stage.
        v.activate_next_stage(&system, t);
        assert_eq!(v.propulsion.sections.len(), 1);
        assert!(v.propulsion.sequences.is_empty());
        let log = v.log.len();
        v.activate_next_stage(&system, t);
        assert_eq!(v.log.len(), log);
    }

    fn momentum(vessels: &[&Vessel], system: &SolarSystem, t: SimTime) -> DVec3 {
        vessels
            .iter()
            .map(|v| v.state_at(system, t).velocity * v.mass_properties().mass)
            .sum()
    }

    #[test]
    fn a_released_stage_flies_on_from_where_it_was() {
        let system = rss();
        let (mut v, mut t) = launch(&system);
        run(&mut v, &system, &mut t, 10.0);
        // Turning, so the stages move apart at different speeds.
        v.rotation_input = DVec3::new(0.0, 0.3, 0.2);
        run(&mut v, &system, &mut t, 2.0);
        v.rotation_input = DVec3::ZERO;
        let (base, spin) = (v.base_at(&system, t), v.attitude_state(&system, t));
        let before = momentum(&[&v], &system, t);
        let debris = v.activate_next_stage(&system, t);
        let [booster] = &debris[..] else {
            panic!("{} pieces", debris.len())
        };
        assert!(booster.is_debris() && !v.is_debris());
        assert_eq!(booster.name, "Booster (Pathfinder)");
        assert_eq!(booster.propulsion.sections.len(), 1);
        assert!(booster.propulsion.sequences.is_empty());
        assert!((booster.base_at(&system, t) - base).length() < 1e-6);
        let turning = booster.attitude_state(&system, t);
        assert_eq!(turning.orientation, spin.orientation);
        assert_eq!(turning.angular_velocity, spin.angular_velocity);
        // Pushed apart along the stack, momentum kept.
        let apart = v.state_at(&system, t).velocity - booster.state_at(&system, t).velocity;
        let along = apart.dot(nose(&v, &system, t));
        assert!((along - SEPARATION_SPEED).abs() < 1e-9, "{along}");
        let after = momentum(&[&v, booster], &system, t);
        assert!((after - before).length() < 1e-9 * before.length());

        // Its engine winds down while the upper stage lights and pulls away.
        let mut booster = booster.clone();
        let mut tb = t;
        run(&mut booster, &system, &mut tb, 5.0);
        run(&mut v, &system, &mut t, 5.0);
        assert_eq!(booster.flight.thrust, 0.0);
        assert!(v.flight.thrust > 0.0);
        let gap = v.base_at(&system, t) - booster.base_at(&system, t);
        assert!(gap.length() > booster.propulsion.length() + 10.0, "{gap}");
    }

    #[test]
    fn fairing_halves_swing_open_away_from_the_payload() {
        let system = rss();
        let (mut v, t) = leo_vessel(&system);
        v.activate_next_stage(&system, t);
        let before = momentum(&[&v], &system, t);
        let halves = v.activate_next_stage(&system, t);
        assert_eq!(halves.len(), 2);
        let velocity = v.state_at(&system, t).velocity;
        let side = v.attitude_at(&system, t) * DVec3::X;
        let apart: Vec<_> = halves
            .iter()
            .map(|h| h.state_at(&system, t).velocity - velocity)
            .collect();
        for (h, angle) in halves.iter().zip(FAIRING_HALVES) {
            assert!(h.is_debris());
            assert_eq!(h.mass_properties().mass, 950.0);
            assert_eq!(
                h.propulsion.sections[0].fairing.as_ref().unwrap().half,
                Some(angle)
            );
        }
        // Out to either side, the pushes cancelling.
        assert!(apart[0].dot(side) > 2.0 && apart[1].dot(side) < -2.0);
        let after = momentum(&[&v, &halves[0], &halves[1]], &system, t);
        assert!((after - before).length() < 1e-9 * before.length());
        // Top first: a second later each nose leans out its own way, and its base has
        // cleared the payload.
        for (h, out) in halves.iter().zip([side, -side]) {
            let (mut h, mut th) = (h.clone(), t);
            run(&mut h, &system, &mut th, 1.0);
            let lean = nose(&h, &system, th).dot(out);
            assert!((lean - FAIRING_SWING.sin()).abs() < 0.01, "{lean}");
            let base = h.base_at(&system, th) - v.base_at(&system, th);
            assert!(base.dot(out) > FAIRING_PUSH * 0.9, "{}", base.dot(out));
        }
    }

    #[test]
    fn engines_armed_early_light_when_their_stage_is_at_the_bottom() {
        use crate::staging::Target;
        let system = rss();
        let (mut v, mut t) = launch(&system);
        run(&mut v, &system, &mut t, 10.0);
        // Light the upper stage in its own sequence before separating: it does not burn
        // while the booster is below it, and lights the moment it is at the bottom.
        let seqs = &mut v.propulsion.sequences;
        assert!(seqs.move_activation(0, 1, Target::New(0)));
        assert_eq!(sequence_names(&v)[0], ["engines:Upper Stage"]);
        v.activate_next_stage(&system, t);
        assert!(
            v.propulsion.armed(v.propulsion.sections[1].id) && v.propulsion.sections.len() == 3
        );
        // The log says why.
        let said = |v: &Vessel, text: &str| v.log.iter().any(|e| e.message.contains(text));
        assert!(said(&v, "Upper Stage waits for Booster to fall away"));
        v.step(&system, t, t + DT, &mut Vec::new());
        t = t + DT;
        assert!(
            !v.propulsion
                .engines_of(v.propulsion.sections[1].id)
                .next()
                .unwrap()
                .running
        );
        v.activate_next_stage(&system, t);
        v.step(&system, t, t + DT, &mut Vec::new());
        assert!(
            v.propulsion
                .engines_of(v.propulsion.sections[0].id)
                .next()
                .unwrap()
                .running,
            "{:?}",
            v.log
        );
    }

    #[test]
    fn a_cluster_can_light_one_engine_first() {
        let mut p = Propulsion::from_craft(&pathfinder(), &catalog()).unwrap();
        // The booster's engines are one group in the first sequence.
        let groups = p.groups(&p.sequences.list()[0]);
        let [group] = groups.as_slice() else {
            panic!("{groups:?}")
        };
        let n = group.members.len();
        assert!(group.kind == "engines" && n > 1, "{group:?}");
        // One of them, moved into a sequence of its own ahead of the rest, lights alone.
        let one = group.members[0];
        let (first, rest) = (group.parts[0], group.parts[1..].to_vec());
        assert!(
            p.sequences
                .move_activations(0, &[one], crate::staging::Target::New(0))
        );
        assert_eq!(p.groups(&p.sequences.list()[1])[0].members.len(), n - 1);
        p.set_throttle(1.0);
        p.activate_next_sequence();
        assert_eq!(p.firing().into_iter().collect::<Vec<_>>(), [first]);
        let thrust = p.burn(crate::constants::ATM, 2.0, true, &mut Vec::new());
        let single = p.engines.iter().find(|e| e.part == first).unwrap().thrust;
        assert!(single > 0.0 && (thrust - single).abs() < 1e-9 * thrust);
        // The rest follow with the next press.
        p.activate_next_sequence();
        assert!(rest.iter().all(|&e| p.engine_armed(e)));
        assert_eq!(p.firing().len(), n);
    }

    #[test]
    fn the_burn_executor_arms_engines_without_firing_their_sequence() {
        let system = rss();
        let (mut v, _) = pad_vessel(&system);
        // Put the booster's engines with its decoupler: arming them must not separate.
        let seqs = &mut v.propulsion.sequences;
        let all: Vec<usize> = (0..seqs.list()[0].len()).collect();
        assert!(seqs.move_activations(0, &all, crate::staging::Target::Into(1)));
        assert!(seqs.remove_empty(0));
        assert_eq!(
            v.propulsion.arm_bottom().as_deref(),
            Some("Staged: Booster")
        );
        assert_eq!(v.propulsion.sections.len(), 3);
        assert_eq!(
            sequence_names(&v)[0],
            [
                "decoupler:Booster",
                "engines:Upper Stage",
                "rcs:Upper Stage"
            ]
        );
    }

    #[test]
    fn craft_files_can_set_their_own_sequences() {
        let mut craft = pathfinder();
        craft.sequences = Some(vec![
            vec!["engines:booster".into()],
            vec![
                "fairing:payload".into(),
                "decoupler:booster_engine_1".into(),
            ],
            vec!["engines:upper_stage".into()],
        ]);
        let p = Propulsion::from_craft(&craft, &catalog()).unwrap();
        assert_eq!(p.sequences.list().len(), 3);
        craft.sequences = Some(vec![vec!["fairing:booster".into()]]);
        let err = Propulsion::from_craft(&craft, &catalog()).unwrap_err();
        assert!(err.to_string().contains("nothing to activate"), "{err}");
    }

    #[test]
    fn liftoff_and_ullage() {
        let system = rss();
        let (mut v, mut t) = launch(&system);
        let mut events = Vec::new();
        for _ in 0..(20 * 50) {
            v.step(&system, t, t + DT, &mut events);
            t = t + DT;
        }
        assert!(events.iter().any(|e| e.kind == VesselEventKind::Liftoff));
        // Lifting off does not touch down again on the next tick.
        assert!(
            !events
                .iter()
                .any(|e| matches!(e.kind, VesselEventKind::Landed { .. }))
        );
        let altitude = v.base_at(&system, t).length() - system.body(v.parent()).radius;
        assert!(altitude > 500.0, "{altitude}");

        // Cut throttle and let the thrust tail off (drag then decelerates us: propellant
        // floats forward), then try to light the upper stage: it needs ullage and fails.
        v.set_throttle(0.0);
        for _ in 0..50 {
            v.step(&system, t, t + DT, &mut events);
            t = t + DT;
        }
        assert_eq!(v.propulsion.engine_level(), 0.0);
        v.activate_next_stage(&system, t);
        v.set_throttle(1.0);
        v.step(&system, t, t + DT, &mut events);
        assert!(!v.propulsion.any_engine_running());
        assert!(v.log.iter().any(|e| e.message.contains("ullage")));
    }

    #[test]
    fn gimbal_pitches_the_booster_east_and_sas_stops_it() {
        let system = rss();
        let (mut v, mut t) = launch(&system);
        v.set_sas(&system, t, true);
        run(&mut v, &system, &mut t, 10.0);
        // Nothing disturbs the climb: still vertical, barely rotating.
        let up = v.state_at(&system, t).position.normalize();
        assert!(nose(&v, &system, t).dot(up) > 0.9999);

        // Pitch down (towards the vessel's feet, east) for two seconds.
        v.rotation_input = DVec3::new(-1.0, 0.0, 0.0);
        run(&mut v, &system, &mut t, 2.0);
        assert!(v.flight.control.x == -1.0 && v.flight.torque_authority.x > 1e6);
        let w = v.attitude_state(&system, t).angular_velocity;
        assert!(w.x < -0.05, "{w}");
        v.rotation_input = DVec3::ZERO;
        run(&mut v, &system, &mut t, 10.0);
        let held = v.attitude_state(&system, t);
        assert!(
            held.angular_velocity.length() < 2e-3,
            "{}",
            held.angular_velocity
        );

        let state = v.state_at(&system, t);
        let earth = system.body(v.parent());
        let up = state.position.normalize();
        let east = earth.pole().cross(up).normalize();
        let tilt = held.nose().angle_between(up).to_degrees();
        assert!(tilt > 3.0 && tilt < 45.0, "{tilt}");
        assert!(held.nose().dot(east) > 0.9 * held.nose().reject_from(up).length());
    }

    #[test]
    fn staging_keeps_the_upper_stage_where_it_was() {
        let system = rss();
        let (mut v, mut t) = launch(&system);
        run(&mut v, &system, &mut t, 10.0);
        v.rotation_input = DVec3::new(0.0, 0.3, 0.2);
        run(&mut v, &system, &mut t, 2.0);
        v.rotation_input = DVec3::ZERO;
        let booster = v.propulsion.sections[0].length;
        let upper_base =
            |v: &Vessel, extra: f64| v.base_at(&system, t) + nose(v, &system, t) * extra;
        let before = upper_base(&v, booster);
        let com_before = v.mass_properties().center_of_mass;
        v.activate_next_stage(&system, t);
        assert_eq!(v.propulsion.sections.len(), 2);
        let after = upper_base(&v, 0.0);
        assert!(
            (before - after).length() < 1e-6,
            "{}",
            (before - after).length()
        );
        assert!(v.mass_properties().center_of_mass.z < com_before.z);
    }

    #[test]
    fn reaction_wheels_turn_the_stage_in_orbit() {
        let system = rss();
        let (mut v, mut t) = leo_vessel(&system);
        assert_eq!(v.nav_frame(), NavFrame::Orbit);
        let retro = |v: &Vessel, t| -v.nav_basis(&system, t).unwrap().prograde;
        assert!(nose(&v, &system, t).dot(retro(&v, t)) < -0.999);
        v.set_sas_mode(&system, t, SasMode::Retrograde);
        run(&mut v, &system, &mut t, 120.0);
        let error = nose(&v, &system, t)
            .angle_between(retro(&v, t))
            .to_degrees();
        assert!(error < 0.2, "{error}");
        assert!(v.is_coasting());
    }

    #[test]
    fn rcs_ullage_relights_the_upper_stage_in_orbit() {
        let system = rss();
        let (mut v, mut t) = leo_vessel(&system);
        v.activate_next_stage(&system, t);
        // In free fall the propellant floats: the engine cannot light.
        v.set_throttle(1.0);
        run(&mut v, &system, &mut t, 0.1);
        assert!(!v.propulsion.any_engine_running());
        assert!(v.log.iter().any(|e| e.message.contains("ullage")));

        // Translating forward on RCS settles it; light the engine while still pushing.
        v.set_throttle(0.0);
        v.set_rcs(true);
        v.translation_input = DVec3::Z;
        assert!(v.needs_physics());
        run(&mut v, &system, &mut t, 3.0);
        assert!(
            v.propulsion
                .rcs_fraction(v.propulsion.sections[0].id)
                .unwrap()
                < 1.0
        );
        v.set_throttle(1.0);
        run(&mut v, &system, &mut t, 1.0);
        assert!(v.propulsion.any_engine_running());
    }

    #[test]
    fn executor_flies_a_planned_burn_like_the_impulse() {
        let system = rss();
        // The stage starts unarmed: engaging the executor arms it.
        let (mut v, mut t) = leo_vessel(&system);
        assert!(!v.propulsion.armed(v.propulsion.sections[0].id));
        let burn = Burn {
            time: t.add_seconds(120.0),
            delta_v: DVec3::new(100.0, 0.0, 0.0),
        };
        v.plan.add(burn);
        // What the plan predicts: the impulse at the burn's time.
        let before = v.orbit(&system, t);
        let state = before.state_at(burn.time);
        let impulse = burn.inertial(&state).unwrap();
        let expected = Orbit::new(
            before.mu(),
            burn.time,
            StateVector::new(state.position, state.velocity + impulse),
        );
        let ignitions = v
            .propulsion
            .engines_of(v.propulsion.sections[0].id)
            .next()
            .unwrap()
            .ignitions_left;

        v.engage_burn(&system, t);
        assert_eq!(v.sas().mode(), SasMode::Burn);
        assert!(v.propulsion.armed(v.propulsion.sections[0].id));
        assert_eq!(v.propulsion.sections.len(), 2);
        run(&mut v, &system, &mut t, 200.0);

        assert!(v.plan.is_empty(), "{:?}", v.log);
        assert!(!v.executor().engaged());
        assert!(!v.propulsion.any_engine_running());
        assert!(v.log.iter().any(|e| e.message.starts_with("Burn complete")));
        // One ignition, lit settled thanks to the RCS push.
        let used = ignitions.unwrap()
            - v.propulsion
                .engines_of(v.propulsion.sections[0].id)
                .next()
                .unwrap()
                .ignitions_left
                .unwrap();
        assert_eq!(used, 1, "{:?}", v.log);
        // A finite burn centred on the impulse lands on nearly the same orbit.
        let after = v.orbit(&system, t);
        let ap = |o: &Orbit| o.apoapsis().unwrap();
        assert!(
            (ap(&after) - ap(&expected)).abs() < 1e3,
            "{}",
            ap(&after) - ap(&expected)
        );
        assert!((after.periapsis() - expected.periapsis()).abs() < 1e3);
        assert!(after.normal().angle_between(expected.normal()) < 1e-4);
    }

    #[test]
    fn rails_warp_hands_over_before_an_engaged_burn() {
        let system = rss();
        let (mut v, t) = leo_vessel(&system);
        v.activate_next_stage(&system, t);
        v.plan.add(Burn {
            time: t.add_seconds(3_600.0),
            delta_v: DVec3::new(50.0, 0.0, 0.0),
        });
        v.engage_burn(&system, t);
        let mut universe = crate::universe::Universe::new(system, vec![v], t);
        assert!(universe.can_use_rails());
        let outcome = universe.advance_rails(t.add_seconds(7_200.0));
        assert!(outcome.interrupted);
        assert!(
            outcome
                .events
                .iter()
                .any(|e| e.event.kind == VesselEventKind::BurnDue)
        );
        let left = t.add_seconds(3_600.0).seconds_since(universe.time());
        assert!(left > autopilot::WARP_MARGIN && left < 60.0, "{left}");
        assert!(!universe.can_use_rails());
    }

    #[test]
    fn sas_points_at_a_target_vessel() {
        let system = rss();
        let (chaser, t) = leo_vessel(&system);
        let start = OrbitStartDef {
            body: "Earth".into(),
            periapsis_altitude: 320e3,
            apoapsis_altitude: 320e3,
            inclination: 28.6,
            longitude_of_ascending_node: 0.0,
            plane: None,
            argument_of_periapsis: 0.0,
            mean_anomaly: 1.0,
            spin: [0.0; 3],
        };
        let station = Vessel::in_orbit(&upper_stage(), &catalog(), &system, &start, t).unwrap();
        let mut universe = crate::universe::Universe::new(system, vec![chaser, station], t);
        let station = universe.vessels[1].id();
        universe.vessels[0].target = Some(Target::Vessel(station));
        let ut = universe.time();
        let (system, vessels) = (&universe.system, &mut universe.vessels);
        vessels[0].set_sas_mode(system, ut, SasMode::Target);
        for _ in 0..(90.0 * 50.0) as usize {
            universe.step_physics();
        }
        let t = universe.time();
        let v = &universe.vessels[0];
        let own = v.state_at(&universe.system, t);
        let target = universe.target_state(0, t).unwrap();
        let toward = (target.position - own.position).normalize();
        let nose = v.attitude_at(&universe.system, t) * DVec3::Z;
        let off = nose.angle_between(toward).to_degrees();
        assert!(off < 1.0, "{off}");
        // The target frame's prograde is the velocity relative to the station.
        let mut v = universe.vessels[0].clone();
        v.set_nav_frame(&universe.system, t, NavFrame::Target);
        let basis = v.nav_basis(&universe.system, t).unwrap();
        let relative = own.velocity - target.velocity;
        assert!(basis.prograde.angle_between(relative) < 1e-3);
    }

    #[test]
    fn rcs_turns_the_stage_without_leaving_its_orbit() {
        let system = rss();
        let (mut v, mut t) = leo_vessel(&system);
        for stage in &mut v.propulsion.sections {
            stage.wheel_torque = 0.0;
        }
        // Without wheels, and with the RCS off, nothing turns it.
        v.rotation_input = DVec3::X;
        run(&mut v, &system, &mut t, 1.0);
        assert_eq!(v.attitude_state(&system, t).angular_velocity, DVec3::ZERO);

        // RCS couples pitch it up without pushing: it stays on the same exact conic.
        v.set_rcs(true);
        let Motion::Coasting(orbit) = *v.motion() else {
            panic!("coasting")
        };
        run(&mut v, &system, &mut t, 5.0);
        assert!(v.attitude_state(&system, t).angular_velocity.x > 0.0);
        assert!(v.is_coasting());
        assert_eq!(v.state_at(&system, t).position, orbit.state_at(t).position);
    }

    #[test]
    fn an_ascent_stays_within_the_tanks_strength() {
        let system = rss();
        let (mut v, mut t) = launch(&system);
        v.set_sas(&system, t, true);
        let mut peak: f64 = 0.0;
        let mut fly = |v: &mut Vessel, t: &mut SimTime, seconds: f64| {
            for _ in 0..(seconds * 50.0) as usize {
                run(v, &system, t, 0.02);
                peak = peak.max(v.flight.structure_load);
            }
        };
        fly(&mut v, &mut t, 10.0);
        v.rotation_input = DVec3::new(-0.3, 0.0, 0.0);
        fly(&mut v, &mut t, 2.0);
        v.rotation_input = DVec3::ZERO;
        // To just before the booster runs dry, then hot-stage onto the upper stage.
        fly(&mut v, &mut t, 130.0);
        assert_eq!(v.propulsion.sections.len(), 3);
        v.activate_next_stage(&system, t);
        fly(&mut v, &mut t, 60.0);
        let log: Vec<_> = v.log.iter().map(|e| &e.message).collect();
        assert!(v.propulsion.any_engine_running(), "{log:?}");
        assert!(!v.is_destroyed());
        // The tanks work hard (full tanks at liftoff, high g near burnout) but hold.
        assert!(peak > 0.3 && peak < 1.0, "{peak}");
    }

    #[test]
    fn the_pad_lets_go_of_the_stack_without_a_jolt() {
        let system = rss();
        let (mut v, mut t) = launch(&system);
        let mut events = Vec::new();
        let mut joints = Vec::new();
        for _ in 0..(5 * 50) {
            v.step(&system, t, t + DT, &mut events);
            t = t + DT;
            joints.push(v.joint_loads.clone());
        }
        assert!(events.iter().any(|e| e.kind == VesselEventKind::Liftoff));
        // The joints carry what speeding the stages above them up takes, which grows
        // with the thrust; the last tick on the pad is no different.
        let last = joints.last().unwrap();
        for pair in joints.windows(2) {
            for (k, (a, b)) in pair[0].iter().zip(&pair[1]).enumerate() {
                let jump = (b - a).abs() / last[k].abs().max(1.0);
                assert!(
                    jump < 0.05,
                    "joint {k}: {a} then {b}, {} at the end",
                    last[k]
                );
            }
        }
    }

    #[test]
    fn an_overloaded_stack_collapses_on_the_pad() {
        let system = rss();
        let mut craft = pathfinder();
        let payload = craft.parts.iter_mut().find(|p| p.id == "payload").unwrap();
        payload.structure.mass = payload.structure.mass.map(|m| m * 100.0);
        let site = &launch_sites()[0];
        let t = SimTime::J2000;
        let mut v = Vessel::on_launch_site(&craft, &catalog(), &system, site, t).unwrap();
        let mut events = Vec::new();
        v.step(&system, t, t + DT, &mut events);
        assert!(v.is_destroyed());
        assert!(events.iter().any(|e| e.kind == VesselEventKind::Destroyed));
        // The upper stage's tank, right under the payload, gives first.
        assert!(
            v.log
                .iter()
                .any(|e| e.message.contains("Upper Stage: tank buckled"))
        );
        // Wreckage does not fly.
        v.set_throttle(1.0);
        v.activate_next_stage(&system, t);
        assert_eq!(v.propulsion.throttle(), 0.0);
        assert!(!v.propulsion.armed(v.propulsion.sections[0].id));
    }

    #[test]
    fn fins_tear_off_in_a_hard_turn_at_speed() {
        let system = rss();
        let craft: CraftDef =
            records(include_str!("../../../assets/core/craft/sounder.json5")).remove(0);
        let start = OrbitStartDef {
            body: "Earth".into(),
            periapsis_altitude: 300e3,
            apoapsis_altitude: 300e3,
            inclination: 0.0,
            longitude_of_ascending_node: 0.0,
            plane: None,
            argument_of_periapsis: 0.0,
            mean_anomaly: 0.0,
            spin: [0.0; 3],
        };
        let t = SimTime::J2000;
        let mut v = Vessel::in_orbit(&craft, &catalog(), &system, &start, t).unwrap();
        let fly_at = |v: &mut Vessel, speed: f64, angle_of_attack: f64| {
            let earth = system.body(v.parent());
            let nose = nose(v, &system, t);
            let up = nose.any_orthonormal_vector();
            let position = up * (earth.radius + 8_000.0);
            let air = (nose * angle_of_attack.cos() + up * angle_of_attack.sin()) * speed;
            let state = StateVector::new(position, earth.surface_velocity(position) + air);
            v.set_free_state(&system, v.parent(), state, t);
            let mut events = Vec::new();
            v.step(&system, t, t + DT, &mut events);
        };
        // Mach 2 at 8 km, nose into the wind: fine.
        fly_at(&mut v, 600.0, 0.0);
        assert!(v.propulsion.sections[0].fins.is_some());
        assert!(v.flight.structure_load < 1.0);
        // Thirty degrees off: the fins go, the rest of the rocket holds.
        fly_at(&mut v, 600.0, 30f64.to_radians());
        assert!(v.propulsion.sections[0].fins.is_none());
        let parts = v.propulsion.parts().parts();
        assert!(!parts.iter().any(|p| matches!(p.kind, PartKind::Fins(_))));
        assert!(v.log.iter().any(|e| e.message.contains("fins torn off")));
        assert!(!v.is_destroyed());
    }

    /// The sounder (minus the stages in `drop`) flying through the air at `altitude`,
    /// `speed` along `direction` (+1 up, −1 down), its nose `angle` off the airflow.
    fn sounder_in_the_air(
        system: &SolarSystem,
        drop: usize,
        altitude: f64,
        speed: f64,
        direction: f64,
        angle: f64,
    ) -> (Vessel, SimTime) {
        let mut craft: CraftDef =
            records(include_str!("../../../assets/core/craft/sounder.json5")).remove(0);
        // Its stages' bodies from the bottom, each with its parts named after it.
        for stage in ["sounder", "capsule"].into_iter().take(drop) {
            let prefix = format!("{stage}_");
            craft
                .parts
                .retain(|p| p.id != stage && !p.id.starts_with(&prefix));
        }
        let start = OrbitStartDef {
            body: "Earth".into(),
            periapsis_altitude: 300e3,
            apoapsis_altitude: 300e3,
            inclination: 0.0,
            longitude_of_ascending_node: 0.0,
            plane: None,
            argument_of_periapsis: 0.0,
            mean_anomaly: 0.0,
            spin: [0.0; 3],
        };
        let t = SimTime::J2000;
        let mut v = Vessel::in_orbit(&craft, &catalog(), system, &start, t).unwrap();
        let earth = system.body(v.parent());
        let nose = nose(&v, system, t);
        let side = nose.any_orthonormal_vector();
        // Straight up or down, so gravity does not turn the flow.
        let airflow = nose * angle.cos() + side * angle.sin();
        let position = airflow * direction * (earth.radius + altitude);
        let state = StateVector::new(position, earth.surface_velocity(position) + airflow * speed);
        v.set_free_state(system, v.parent(), state, t);
        (v, t)
    }

    /// Angle between the nose and the airflow, degrees.
    fn angle_off_airflow(v: &Vessel, system: &SolarSystem, t: SimTime) -> f64 {
        let state = v.state_at(system, t);
        let body = system.body(v.parent());
        let air = state.velocity - body.surface_velocity(state.position);
        nose(v, system, t).angle_between(air).to_degrees()
    }

    pub fn glider() -> CraftDef {
        records(include_str!("../../../assets/core/craft/glider.json5")).remove(0)
    }

    /// The glider released at `altitude` over the Cape, heading east at `speed`, with its
    /// boosters and dolly already let go.
    fn glider_in_flight(system: &SolarSystem, altitude: f64, speed: f64) -> (Vessel, SimTime) {
        let start = FlightStartDef {
            body: "Earth".into(),
            latitude: 28.6,
            longitude: -80.6,
            altitude,
            heading: 90.0,
            speed,
            climb: 0.0,
        };
        let t = SimTime::J2000;
        let mut def = glider();
        def.sequences = Some(vec![vec![
            "decoupler:booster_right".into(),
            "decoupler:booster_left".into(),
            "decoupler:dolly".into(),
        ]]);
        let mut v = Vessel::in_flight(&def, &catalog(), system, &start, t).unwrap();
        v.activate_next_stage(system, t);
        (v, t)
    }

    /// Bank (right wing down positive), heading and pitch of the glider, degrees.
    fn glider_angles(v: &Vessel, system: &SolarSystem, t: SimTime) -> (f64, f64, f64) {
        let earth = system.body(v.parent());
        let s = v.state_at(system, t);
        let up = s.position.normalize();
        let q = v.attitude_at(system, t);
        let (nose, right) = (q * DVec3::Z, q * DVec3::X);
        let north = earth.pole().reject_from(up).normalize();
        let east = north.cross(up);
        let level = nose.reject_from(up).normalize();
        let heading = level.dot(east).atan2(level.dot(north)).to_degrees();
        let bank = (-right.dot(up)).asin().to_degrees();
        let pitch = nose.dot(up).asin().to_degrees();
        (bank, heading, pitch)
    }

    pub fn comet() -> CraftDef {
        records(include_str!("../../../assets/core/craft/comet.json5")).remove(0)
    }

    /// The Comet at the start of the Shuttle Landing Facility's runway.
    fn comet_on_runway(system: &SolarSystem) -> (Vessel, SimTime) {
        let site = launch_sites()
            .into_iter()
            .find(|s| s.id == "cape_slf")
            .unwrap();
        let t = SimTime::J2000;
        let v = Vessel::on_runway(&comet(), &catalog(), system, &site, t).unwrap();
        (v, t)
    }

    /// Height of the centre of mass above the ground, ground speed and pitch (nose above the
    /// horizon), degrees.
    pub(super) fn on_the_ground(v: &Vessel, system: &SolarSystem, t: SimTime) -> (f64, f64, f64) {
        let body = system.body(v.parent());
        let s = v.state_at(system, t);
        let up = s.position.normalize();
        let fixed = body.orientation(t).inverse() * up;
        let height = s.position.length() - body.radius - body.terrain_height(fixed);
        let speed = (s.velocity - body.surface_velocity(s.position)).length();
        let pitch = nose(v, system, t).dot(up).asin().to_degrees();
        (height, speed, pitch)
    }

    /// The Comet, nearly out of propellant, gliding in at `altitude` and `speed` along a
    /// sea-level runway, descending at `glide` degrees.
    fn comet_on_approach(
        system: &SolarSystem,
        altitude: f64,
        speed: f64,
        glide: f64,
    ) -> (Vessel, SimTime) {
        let start = FlightStartDef {
            body: "Earth".into(),
            latitude: 28.6,
            longitude: -80.7,
            altitude,
            heading: 330.0,
            speed,
            climb: -glide,
        };
        let t = SimTime::J2000;
        let mut v = Vessel::in_flight(&comet(), &catalog(), system, &start, t).unwrap();
        for amount in v
            .propulsion
            .tanks
            .iter_mut()
            .filter(|t| t.section == 0)
            .flat_map(|t| &mut t.amounts)
        {
            *amount *= 0.1;
        }
        (v, t)
    }

    /// Fly `seconds`, holding the nose `pitch` degrees above the horizon.
    fn hold_pitch(v: &mut Vessel, system: &SolarSystem, t: &mut SimTime, pitch: f64, seconds: f64) {
        for _ in 0..(seconds * 50.0) as usize {
            let (_, _, now) = on_the_ground(v, system, *t);
            let rate = v.attitude_state(system, *t).angular_velocity.x;
            v.rotation_input = DVec3::new(
                (0.08 * (pitch - now) - 2.0 * rate).clamp(-1.0, 1.0),
                0.0,
                0.0,
            );
            run(v, system, t, 0.02);
        }
        v.rotation_input = DVec3::ZERO;
    }

    #[test]
    fn the_rocket_plane_takes_off_from_the_runway() {
        let system = rss();
        let (mut v, mut t) = comet_on_runway(&system);
        // Parked on its wheels, level, until something pushes it.
        run(&mut v, &system, &mut t, 2.0);
        let (height, speed, pitch) = on_the_ground(&v, &system, t);
        assert!(v.is_landed() && v.wheels_down && speed == 0.0);
        assert!(
            (height - 1.69).abs() < 0.01 && pitch.abs() < 0.1,
            "{height} {pitch}"
        );
        // Engine armed, throttle up, brakes off: it rolls straight down the runway.
        v.activate_next_stage(&system, t);
        v.set_throttle(1.0);
        v.brakes = false;
        run(&mut v, &system, &mut t, 15.0);
        let (height, speed, pitch) = on_the_ground(&v, &system, t);
        assert!(!v.is_landed() && v.wheels_down);
        assert!(
            (60.0..80.0).contains(&speed) && (height - 1.7).abs() < 0.05,
            "{speed} {height}"
        );
        assert!(pitch.abs() < 1.0 && v.attitude_state(&system, t).angular_velocity.length() < 0.01);
        // Rotate and climb out at 10°.
        hold_pitch(&mut v, &system, &mut t, 10.0, 25.0);
        let (height, _, _) = on_the_ground(&v, &system, t);
        assert!(!v.wheels_down && height > 100.0, "{height}");
        assert!(v.log.iter().any(|e| e.message == "Airborne"));
    }

    #[test]
    fn the_rocket_plane_lands_and_brakes_to_a_stop() {
        let system = rss();
        let (mut v, mut t) = comet_on_approach(&system, 10.0, 75.0, 2.0);
        hold_pitch(&mut v, &system, &mut t, 4.0, 6.0);
        assert!(v.wheels_down && !v.is_landed());
        let touchdown = v.log.iter().find(|e| e.message.starts_with("Touched down"));
        assert!(touchdown.is_some(), "{:?}", v.log);
        v.brakes = true;
        run(&mut v, &system, &mut t, 20.0);
        let (height, speed, _) = on_the_ground(&v, &system, t);
        // Parked on its wheels, struts settled, no crash.
        assert!(v.is_landed() && v.wheels_down && speed == 0.0, "{speed}");
        assert!((height - 1.7).abs() < 0.05, "{height}");
        assert!(!v.log.iter().any(|e| e.message.starts_with("Crashed")));
    }

    #[test]
    fn flying_into_the_ground_is_a_crash() {
        let system = rss();
        // Nose first at 30° down.
        let (mut v, mut t) = comet_on_approach(&system, 20.0, 60.0, 30.0);
        run(&mut v, &system, &mut t, 3.0);
        assert!(v.is_landed() && !v.wheels_down);
        assert!(
            v.log.iter().any(|e| e.message.starts_with("Crashed")),
            "{:?}",
            v.log
        );
    }

    #[test]
    fn a_glider_glides_hands_off() {
        let system = rss();
        let earth = system.body(system.find("Earth").unwrap());
        let (mut v, mut t) = glider_in_flight(&system, 1500.0, 38.0);
        let altitude = |v: &Vessel, t| v.state_at(&system, t).position.length() - earth.radius;
        // Settle, then measure two minutes of glide.
        run(&mut v, &system, &mut t, 30.0);
        let (top, mut across, mut worst): (f64, f64, f64) = (altitude(&v, t), 0.0, 0.0);
        for _ in 0..(120 * 50) {
            run(&mut v, &system, &mut t, 0.02);
            let s = v.state_at(&system, t);
            let air = s.velocity - earth.surface_velocity(s.position);
            across += air.reject_from(s.position).length() * 0.02;
            worst = worst.max(v.flight.angle_of_attack.to_degrees());
        }
        let sink = top - altitude(&v, t);
        // A sailplane: well clear of the stall, about 22 metres forward for each one down.
        assert!(worst < 8.0, "{worst}");
        assert!(
            sink > 0.0 && (18.0..30.0).contains(&(across / sink)),
            "{} {sink}",
            across / sink
        );
        let (bank, heading, _) = glider_angles(&v, &system, t);
        assert!(
            bank.abs() < 2.0 && (heading - 90.0).abs() < 5.0,
            "{bank} {heading}"
        );
    }

    #[test]
    fn ailerons_bank_the_glider_and_it_turns() {
        let system = rss();
        let (mut v, mut t) = glider_in_flight(&system, 1500.0, 38.0);
        run(&mut v, &system, &mut t, 10.0);
        v.rotation_input = DVec3::new(0.0, 0.0, 0.5);
        run(&mut v, &system, &mut t, 1.5);
        v.rotation_input = DVec3::ZERO;
        let (bank, _, _) = glider_angles(&v, &system, t);
        assert!(bank > 15.0, "{bank}");
        run(&mut v, &system, &mut t, 10.0);
        let (_, heading, _) = glider_angles(&v, &system, t);
        assert!(heading > 140.0, "{heading}");
    }

    #[test]
    fn a_finned_rocket_weathervanes_and_a_bare_one_tumbles() {
        let system = rss();
        // Diving at Mach 0.8 from 8 km, 8° off the wind: it swings about the wind, the
        // swing dying away as the air thickens.
        let (mut v, mut t) = sounder_in_the_air(&system, 0, 8_000.0, 260.0, -1.0, 0.14);
        let (mut early, mut late): (f64, f64) = (0.0, 0.0);
        for i in 0..800 {
            run(&mut v, &system, &mut t, 0.02);
            let angle = angle_off_airflow(&v, &system, t);
            if i < 200 {
                early = early.max(angle);
            } else if i >= 700 {
                late = late.max(angle);
            }
        }
        assert!(early < 9.0 && late < 1.5, "{early} {late}");
        assert!(v.flight.drag > 0.0);

        // Without its fins, the capsule's lift up front turns it over.
        let (mut v, mut t) = sounder_in_the_air(&system, 0, 8_000.0, 260.0, -1.0, 0.14);
        v.propulsion.tear_off_fins(0);
        run(&mut v, &system, &mut t, 3.0);
        assert!(angle_off_airflow(&v, &system, t) > 45.0);
    }

    #[test]
    fn a_capsule_keeps_its_heat_shield_to_the_flow() {
        let system = rss();
        // The C-1 capsule alone, falling at 2 km/s through 45 km, 30° off heat shield
        // first: it rocks (there is little damping at hypersonic speed, as in reality) but
        // never turns its side, let alone its nose, to the flow.
        let (mut v, mut t) =
            sounder_in_the_air(&system, 1, 45_000.0, 2_000.0, -1.0, 150f64.to_radians());
        let mut least: f64 = 180.0;
        for _ in 0..750 {
            run(&mut v, &system, &mut t, 0.02);
            least = least.min(angle_off_airflow(&v, &system, t));
        }
        assert!(least > 145.0, "{least}");
        // Its blunt face makes it mostly drag.
        assert!(v.flight.drag > 2.0 * v.flight.lift, "{:?}", v.flight);

        // Started nose first, it swings round.
        let (mut v, mut t) = sounder_in_the_air(&system, 1, 45_000.0, 2_000.0, -1.0, 0.35);
        let mut most: f64 = 0.0;
        for _ in 0..150 {
            run(&mut v, &system, &mut t, 0.02);
            most = most.max(angle_off_airflow(&v, &system, t));
        }
        assert!(most > 150.0, "{most}");
    }

    #[test]
    fn starts_in_the_plane_of_a_moon() {
        let system = rss();
        let moon = system.body(system.find("Moon").unwrap());
        let start = OrbitStartDef {
            body: "Earth".into(),
            periapsis_altitude: 300e3,
            apoapsis_altitude: 300e3,
            inclination: 0.0,
            longitude_of_ascending_node: 0.0,
            plane: Some("Moon".into()),
            argument_of_periapsis: 0.0,
            mean_anomaly: 0.0,
            spin: [0.0; 3],
        };
        let t = SimTime::J2000;
        let v = Vessel::in_orbit(&upper_stage(), &catalog(), &system, &start, t).unwrap();
        let normal = v.orbit(&system, t).normal();
        assert!(normal.angle_between(moon.orbit.unwrap().normal()) < 1e-9);
        // Only a body orbiting the same one has a plane to borrow.
        let mars = OrbitStartDef {
            plane: Some("Mars".into()),
            ..start
        };
        assert!(matches!(
            Vessel::in_orbit(&upper_stage(), &catalog(), &system, &mars, t),
            Err(VesselError::NotOrbiting(..))
        ));
    }

    #[test]
    fn lunar_upper_has_plenty_of_rcs() {
        let system = rss();
        let (stock, t) = leo_vessel(&system);
        let start = OrbitStartDef {
            body: "Earth".into(),
            periapsis_altitude: 300e3,
            apoapsis_altitude: 300e3,
            inclination: 0.0,
            longitude_of_ascending_node: 0.0,
            plane: None,
            argument_of_periapsis: 0.0,
            mean_anomaly: 0.0,
            spin: [0.0; 3],
        };
        let lunar = Vessel::in_orbit(&lunar_upper(), &catalog(), &system, &start, t).unwrap();
        // Pitch acceleration with wheels and RCS, forward push from the RCS, RCS delta-v.
        let agility = |v: &Vessel| {
            let (m, p) = (v.mass_properties(), &v.propulsion);
            let torque = attitude::torque_authority(p, m.center_of_mass)
                + rcs::torque_authority(p, m.center_of_mass.z, 0.0);
            let push: f64 = p.rcs.iter().map(|b| b.def.thrust).sum();
            (
                torque.x / m.inertia.diagonal().x,
                push / m.mass,
                p.rcs_delta_v(),
            )
        };
        let stock = agility(&stock);
        let (turn, push, delta_v) = agility(&lunar);
        // About 1.2 °/s², 0.26 m/s² and 110 m/s, against 0.4 °/s², 0.014 m/s² and 2.4 m/s.
        assert!(turn > 2.5 * stock.0);
        assert!(push > 15.0 * stock.1);
        assert!(delta_v > 100.0 && delta_v > 40.0 * stock.2);
        // The rocket equation at the hypergolic quads' Isp.
        let (mass, propellant) = (
            lunar.propulsion.mass(),
            lunar.propulsion.rcs_propellant_mass(),
        );
        assert!(
            (delta_v - 280.0 * crate::constants::G0 * (mass / (mass - propellant)).ln()).abs()
                < 1e-6
        );
        // SAS swings it a quarter turn in about 20 s with the RCS on, without overshoot.
        let (mut v, mut t) = (lunar, t);
        v.set_rcs(true);
        v.set_sas_mode(&system, t, SasMode::RadialOut);
        run(&mut v, &system, &mut t, 25.0);
        let target = v.nav_basis(&system, t).unwrap().radial_out;
        assert!(nose(&v, &system, t).angle_between(target) < 0.2f64.to_radians());
        assert!(v.attitude_state(&system, t).angular_velocity.length() < 0.002);
    }

    #[test]
    fn the_tumbling_satellite_flips_end_over_end() {
        let system = rss();
        let scenarios: Vec<ScenarioDef> =
            records(include_str!("../../../assets/core/scenarios/stock.json5"));
        let scenario = scenarios
            .into_iter()
            .find(|s| s.id == "dzhanibekov")
            .unwrap();
        let StartDef::Orbit(start) = &scenario.start else {
            panic!("an orbit start")
        };
        let craft: CraftDef =
            records(include_str!("../../../assets/core/craft/satellite.json5")).remove(0);
        let t = SimTime::from_seconds(scenario.start_time);
        let v = Vessel::in_orbit(&craft, &catalog(), &system, start, t).unwrap();
        // Roll, the bus's long axis, is its middle moment: spun about it, the satellite
        // turns end over end, back, and over again.
        let i = v.mass_properties().inertia.diagonal();
        assert!(i.x < i.z && i.z < i.y, "{i}");
        let first = nose(&v, &system, t);
        let along = |seconds: f64| nose(&v, &system, t.add_seconds(seconds)).dot(first);
        assert!(along(30.0) < -0.9 && along(60.0) > 0.9 && along(90.0) < -0.9);
    }

    #[test]
    fn rails_hold_a_steady_attitude_and_keep_a_turn() {
        let system = rss();
        let (mut v, mut t) = leo_vessel(&system);
        v.set_sas_mode(&system, t, SasMode::RadialOut);
        v.enter_rails(&system, t);
        let target = |v: &Vessel, t| v.nav_basis(&system, t).unwrap().radial_out;
        // It does not snap onto the target, but keeps its angle to it around the orbit.
        let off = nose(&v, &system, t).angle_between(target(&v, t));
        assert!(off > 0.1, "{off}");
        for minutes in [13.0, 47.0, 400.0] {
            let at = t.add_seconds(minutes * 60.0);
            let now = nose(&v, &system, at).angle_between(target(&v, at));
            assert!((now - off).abs() < 1e-6, "{now} vs {off}");
        }
        // Stability holds still, whatever the warp.
        let mut v = leo_vessel(&system).0;
        v.set_sas(&system, t, true);
        v.enter_rails(&system, t);
        let later = t.add_seconds(86_400.0);
        assert_eq!(v.attitude_at(&system, later), v.attitude_at(&system, t));
        // Still turning when the warp starts: it keeps turning, SAS or not.
        let mut v = leo_vessel(&system).0;
        v.set_sas_mode(&system, t, SasMode::Prograde);
        v.rotation_input = DVec3::new(0.4, -0.2, 1.0);
        run(&mut v, &system, &mut t, 1.0);
        v.rotation_input = DVec3::ZERO;
        let free = v.clone();
        v.enter_rails(&system, t);
        let later = t.add_seconds(600.0);
        let spin = v.attitude_state(&system, later);
        assert_eq!(spin, free.attitude_state(&system, later));
        assert!(spin.angular_velocity.length() > 0.01);
    }
}
