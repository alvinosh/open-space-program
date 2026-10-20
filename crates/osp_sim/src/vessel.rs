//! Vessels and how they move.
//!
//! A vessel is always in exactly one [`Motion`] mode:
//! - **Landed**: fixed in the body's rotating frame.
//! - **Coasting**: gravity only, so its trajectory is an exact conic. It is evaluated
//!   analytically at any time and only changes at [`TrajectoryEvent`]s. This is the same at
//!   1x and at 100 000x, so time warp cannot change where a coasting vessel ends up.
//! - **Integrated**: thrust, drag or terrain proximity, integrated with Encke's method on a
//!   fixed tick grid.
//!
//! Transitions happen at exact ticks: Coasting → Integrated at a descent event or when
//! thrust starts, Integrated → Coasting once forces stop above the coast floor.
//!
//! A vessel's position is its centre of mass, which it rotates about. Rotation follows the
//! same idea as translation ([`crate::attitude`]): without torque it is exact at any time,
//! and torque from the engine gimbal, reaction wheels or RCS is integrated on the physics
//! tick. During time warp on rails, SAS holds its attitude exactly instead of steering.
//!
//! Engines that need ullage only light while the propellant is settled, i.e. while
//! something pushes the vessel forward: thrust still running (hot staging), or RCS
//! translating forward ([`crate::rcs`]) after a coast.

use glam::{DMat3, DQuat, DVec3};
use thiserror::Error;

use crate::{
    atmosphere::AtmosphereSample,
    attitude::{self, Attitude, Inertia, MassProperties, NavBasis, NavFrame, Sas, SasMode},
    body::{Body, BodyId},
    data::{CraftDef, LaunchSiteDef, OrbitStartDef, PartCatalog},
    events::{EventKind, TrajectoryEvent, apply_event, next_event},
    integrator::Encke,
    orbit::{Elements, Orbit, StateVector},
    propulsion::{CraftError, Propulsion},
    rcs,
    structure::{self, Conditions, Part, PartLoad},
    system::SolarSystem,
    time::SimTime,
};

/// Impacts slower than this (relative to the ground) count as a landing, not a crash.
const SAFE_LANDING_SPEED: f64 = 10.0;
/// Proper acceleration along the nose needed to keep propellant settled, m/s².
const ULLAGE_ACCELERATION: f64 = 1e-3;
/// A vessel may only start coasting if it won't hit the coast floor within this many
/// seconds; prevents flip-flopping between modes on the boundary.
const COAST_HYSTERESIS: f64 = 0.1;
const MAX_LOG: usize = 64;

#[derive(Debug, Error)]
pub enum VesselError {
    #[error(transparent)]
    Craft(#[from] CraftError),
    #[error("unknown body `{0}`")]
    UnknownBody(String),
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
    parent: BodyId,
    motion: Motion,
    attitude: AttitudeTrack,
    /// The pilot's rotation command (pitch up, yaw right, roll right), −1..1. Overrides
    /// SAS while non-zero.
    pub rotation_input: DVec3,
    /// The pilot's translation command (right, down, forward), −1..1, for the RCS.
    pub translation_input: DVec3,
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
    pub flight: FlightData,
    /// Load on every tank and fin set as of the last physics step.
    pub structure: Vec<PartLoad>,
    pub log: Vec<LogEntry>,
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
        let propulsion = Propulsion::from_craft(craft, catalog)?;
        let mass = attitude::mass_properties(&propulsion.stages);
        Ok(Self {
            name: if craft.name.is_empty() {
                craft.id.clone()
            } else {
                craft.name.clone()
            },
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
            structure: Vec::new(),
            log: Vec::new(),
        })
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
        let base = up * (body.radius + body.terrain_height(up));

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

    /// In orbit, from elements relative to the body's equator. Nose prograde, feet towards
    /// the body.
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
        let rp = body.radius + start.periapsis_altitude;
        let ra = body.radius + start.apoapsis_altitude.max(start.periapsis_altitude);
        let elements = Elements {
            semi_major_axis: (rp + ra) / 2.0,
            eccentricity: (ra - rp) / (ra + rp),
            inclination: start.inclination.to_radians(),
            longitude_of_ascending_node: start.longitude_of_ascending_node.to_radians(),
            argument_of_periapsis: start.argument_of_periapsis.to_radians(),
            true_anomaly: 0.0,
        };
        let equatorial =
            Orbit::from_elements(body.gm, &elements, start.mean_anomaly.to_radians(), time);
        let frame = body.rotation.equatorial_frame();
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

    /// Whether the next step needs numerical integration (rails warp is impossible).
    pub fn needs_physics(&self) -> bool {
        matches!(self.motion, Motion::Integrated(_))
            || self.propulsion.wants_thrust()
            || (self.rcs && self.translation_input != DVec3::ZERO)
    }

    /// Mass, centre of mass and inertia as of the last physics step.
    pub fn mass_properties(&self) -> MassProperties {
        self.mass
    }

    /// State of the centre of mass relative to [`Self::parent`] at `time`. Exact for
    /// landed and coasting vessels; integrated vessels are extrapolated linearly from their
    /// last step, which is only meant for rendering between physics ticks.
    pub fn state_at(&self, system: &SolarSystem, time: SimTime) -> StateVector {
        let body = system.body(self.parent);
        match &self.motion {
            Motion::Landed { base, attitude } => {
                let fixed = *base + *attitude * DVec3::Z * self.mass.center_of_mass;
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

    /// Position of the base of the stack (where the engines are) relative to the parent.
    pub fn base_at(&self, system: &SolarSystem, time: SimTime) -> DVec3 {
        let nose = self.attitude_at(system, time) * DVec3::Z;
        self.state_at(system, time).position - nose * self.mass.center_of_mass
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

    pub fn set_nav_frame(&mut self, system: &SolarSystem, time: SimTime, frame: NavFrame) {
        self.rebase_attitude(system, time);
        self.nav_frame = frame;
    }

    /// Prograde, normal and radial directions in the current navigation frame.
    pub fn nav_basis(&self, system: &SolarSystem, time: SimTime) -> Option<NavBasis> {
        let body = system.body(self.parent);
        NavBasis::new(self.nav_frame, body, &self.state_at(system, time))
    }

    /// Time warp is about to skip physics from `time`. With SAS on, the vessel holds its
    /// attitude exactly (relative to the navigation frame for direction modes, snapping
    /// onto the target) instead of steering; with SAS off it keeps rotating freely.
    pub fn enter_rails(&mut self, system: &SolarSystem, time: SimTime) {
        // Nothing steers on rails.
        self.flight.control = DVec3::ZERO;
        rcs::idle(&mut self.propulsion);
        if self.is_landed() || !self.sas.enabled() || self.attitude.hold != Hold::Free {
            return;
        }
        let state = self.attitude_state(system, time);
        let basis = self.nav_basis(system, time);
        let target = basis.and_then(|b| self.sas.mode().direction(&b));
        let (orientation, hold) = match (target, basis) {
            (Some(direction), Some(basis)) => (
                (DQuat::from_rotation_arc(state.nose(), direction) * state.orientation).normalize(),
                Hold::Tracking {
                    frame: self.nav_frame,
                    basis: basis.matrix(),
                },
            ),
            _ => (state.orientation, Hold::Inertial),
        };
        self.attitude = AttitudeTrack {
            epoch: time,
            state: Attitude {
                orientation,
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
        if self.is_landed() {
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

    /// Arm the bottom stage, or jettison it and arm the next. The rest of the stack keeps
    /// moving as it was, so its centre of mass (the vessel's position) moves up the stack.
    pub fn activate_next_stage(&mut self, system: &SolarSystem, time: SimTime) {
        if self.destroyed {
            return;
        }
        let stages = self.propulsion.stages.len();
        let dropped = self.propulsion.stages.first().map_or(0.0, |s| s.length);
        let Some(message) = self.propulsion.activate_next_stage() else {
            return;
        };
        self.record(time, message);
        if self.propulsion.stages.len() == stages {
            return;
        }
        let attitude = self.attitude_state(system, time);
        let before = self.mass.center_of_mass;
        self.mass = attitude::mass_properties(&self.propulsion.stages);
        let offset = attitude.nose() * (dropped + self.mass.center_of_mass - before);
        let shift = StateVector::new(offset, attitude.angular_velocity_inertial().cross(offset));
        match &mut self.motion {
            // TODO: with debris, the spent stage stays on the ground instead.
            Motion::Landed { .. } => {}
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
        let mut messages = Vec::new();
        self.propulsion
            .burn(air.pressure, dt, self.propellant_settled, &mut messages);
        for message in messages {
            self.record(t0, message);
        }
        self.mass = attitude::mass_properties(&self.propulsion.stages);
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
        let rcs = self.rcs && !self.is_landed();
        let mut authority = attitude::torque_authority(&self.propulsion, com);
        if rcs {
            authority += rcs::torque_authority(&self.propulsion, com, air.pressure);
        }
        let command = if self.is_landed() || self.destroyed {
            DVec3::ZERO
        } else {
            self.rotation_command(body, &state, start, authority)
        };
        let (mut force, mut torque) = attitude::actuate(&mut self.propulsion, com, command);
        if rcs {
            let translation = self.translation_input.clamp(DVec3::NEG_ONE, DVec3::ONE);
            let (f, t) = rcs::fire(
                &mut self.propulsion,
                com,
                command,
                translation,
                air.pressure,
                dt,
            );
            force += f;
            torque += t;
        } else {
            rcs::idle(&mut self.propulsion);
        }
        let track = &self.attitude;
        if !self.is_landed()
            && (torque != DVec3::ZERO
                || track.inertia != self.mass.inertia
                || track.hold != Hold::Free)
        {
            let inertia = self.mass.inertia;
            let end = if torque == DVec3::ZERO {
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
        let drag_accel = drag(body, &air, &self.propulsion, mass);
        let drag_now = drag_accel(state.position, state.velocity);
        let proper = thrust_accel + drag_now;
        let air_velocity = state.velocity - body.surface_velocity(state.position);
        let air_speed = air_velocity.length();
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
            static_pressure: air.pressure,
            proper_acceleration: proper.length(),
            control: command,
            torque_authority: authority,
            structure_load,
        };
        let nose = start.nose();
        self.propellant_settled = self.is_landed() || proper.dot(nose) > ULLAGE_ACCELERATION;

        let conditions = Conditions {
            // On the ground, the ground holds the stack up.
            acceleration: if self.is_landed() {
                body.gm / state.position.length_squared() * state.position.normalize().dot(nose)
            } else {
                proper.dot(nose)
            },
            drag: (-drag_now.dot(nose) * mass).max(0.0),
            dynamic_pressure: self.flight.dynamic_pressure,
            mach: self.flight.mach,
            angle_of_attack: if air_speed > 1.0 {
                nose.angle_between(air_velocity)
            } else {
                0.0
            },
        };
        self.check_structure(&conditions, t0, out);

        match self.motion {
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
                self.integrate(system, t1, thrust_accel, out);
            }
            Motion::Coasting(orbit) if thrust > 0.0 => {
                self.motion = Motion::Integrated(Encke::new(orbit.mu(), t0, orbit.state_at(t0)));
                self.integrate(system, t1, thrust_accel, out);
            }
            Motion::Coasting(_) => {
                let mut t = t0;
                while let Some(event) = self.next_trajectory_event(system, t, t1) {
                    self.apply_trajectory_event(system, event, out);
                    t = event.time;
                    if event.kind == EventKind::Descent {
                        // Finish this tick under physics from the exact event time.
                        self.integrate(system, t1, DVec3::ZERO, out);
                        break;
                    }
                }
            }
            Motion::Integrated(_) => self.integrate(system, t1, thrust_accel, out),
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
        self.structure = structure::loads(&self.propulsion.stages, conditions);
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
            let stage = &mut self.propulsion.stages[load.stage];
            let name = stage.name.clone();
            let percent = load.fraction * 100.0;
            match load.part {
                Part::Fins => {
                    stage.fins = None;
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

    /// The pilot's input if any, otherwise what SAS wants.
    fn rotation_command(
        &mut self,
        body: &Body,
        state: &StateVector,
        attitude: Attitude,
        authority: DVec3,
    ) -> DVec3 {
        if self.rotation_input != DVec3::ZERO {
            self.sas.release();
            return self.rotation_input.clamp(DVec3::NEG_ONE, DVec3::ONE);
        }
        if !self.sas.enabled() {
            return DVec3::ZERO;
        }
        let alpha = authority / self.mass.inertia.diagonal().max(DVec3::splat(1e-9));
        let target = NavBasis::new(self.nav_frame, body, state)
            .and_then(|basis| self.sas.mode().direction(&basis));
        self.sas.steer(attitude, alpha, target)
    }

    /// Integrate to `end`, then resolve ground contact, SOI changes and a return to coasting.
    fn integrate(
        &mut self,
        system: &SolarSystem,
        end: SimTime,
        thrust_accel: DVec3,
        out: &mut Vec<VesselEvent>,
    ) {
        let Motion::Integrated(encke) = &mut self.motion else {
            return;
        };
        let body = system.body(self.parent);
        let air = air_at(body, &encke.state());
        let drag_accel = drag(body, &air, &self.propulsion, self.propulsion.mass());
        encke.step(end, |r, v| thrust_accel + drag_accel(r, v));
        let state = encke.state();

        // Ground contact at the base of the stack, while moving towards the ground.
        let attitude = self.attitude_state(system, end);
        let base = state.position - attitude.nose() * self.mass.center_of_mass;
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

/// Drag acceleration with air co-rotating with the body; density held for the step. The
/// body's drag scales with its frontal area, the fins' with their own drag area.
fn drag(
    body: &Body,
    air: &AtmosphereSample,
    propulsion: &Propulsion,
    mass: f64,
) -> impl Fn(DVec3, DVec3) -> DVec3 + use<> {
    let omega = body.angular_velocity();
    let (density, speed_of_sound) = (air.density, air.speed_of_sound);
    let (area, fins) = (propulsion.reference_area(), propulsion.fin_drag_area());
    move |r, v| {
        if density == 0.0 {
            return DVec3::ZERO;
        }
        let v_air = v - omega.cross(r);
        let speed = v_air.length();
        let drag_area = drag_coefficient(speed / speed_of_sound) * area + fins;
        -0.5 * density * speed * v_air * drag_area / mass
    }
}

/// Crude transonic drag rise for a slender rocket.
fn drag_coefficient(mach: f64) -> f64 {
    let base = 0.3;
    let wave = 0.5 * (-((mach - 1.1) / 0.35).powi(2)).exp();
    let supersonic = if mach > 1.0 { 0.1 / mach } else { 0.0 };
    base + wave + supersonic
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{data::ScenarioDef, data::StartDef, system::tests::rss, time::Ticks};

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
        }
    }

    pub fn pathfinder() -> CraftDef {
        records(include_str!("../../../assets/core/craft/pathfinder.json5")).remove(0)
    }

    pub fn upper_stage() -> CraftDef {
        records(include_str!("../../../assets/core/craft/upper_stage.json5")).remove(0)
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
            argument_of_periapsis: 0.0,
            mean_anomaly: 0.0,
        };
        let t = SimTime::J2000;
        (
            Vessel::in_orbit(&upper_stage(), &catalog(), system, &start, t).unwrap(),
            t,
        )
    }

    const DT: Ticks = Ticks::from_millis(20);

    fn run(v: &mut Vessel, system: &SolarSystem, t: &mut SimTime, seconds: f64) {
        let mut events = Vec::new();
        for _ in 0..(seconds * 50.0).round() as usize {
            v.step(system, *t, *t + DT, &mut events);
            *t = *t + DT;
        }
    }

    fn nose(v: &Vessel, system: &SolarSystem, t: SimTime) -> DVec3 {
        v.attitude_at(system, t) * DVec3::Z
    }

    #[test]
    fn pad_vessel_sits_on_rotating_surface() {
        let system = rss();
        let (mut v, start) = pad_vessel(&system);
        let earth = system.body(v.parent());
        assert!((v.base_at(&system, start).length() - earth.radius).abs() < 1e-6);
        let s = v.state_at(&system, start);
        // The centre of mass is up the stack; Kennedy at 28.6°N moves east at ~408 m/s.
        let com = v.mass_properties().center_of_mass;
        assert!(com > 20.0 && com < 40.0, "{com}");
        assert!(((s.position.length() - earth.radius) - com).abs() < 1e-6);
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
        assert!((v.base_at(&system, later).length() - earth.radius).abs() < 1e-6);
    }

    #[test]
    fn pad_vessel_stands_on_the_pad_deck() {
        let mut system = rss();
        let sites = launch_sites();
        crate::terrain::install(&mut system, Vec::new(), &sites);
        let (mut v, start) = pad_vessel(&system);
        let earth = system.body(v.parent());
        let deck = sites[0].pad.as_ref().unwrap().elevation.unwrap();
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

        // Cut throttle (drag now decelerates us: propellant floats forward), then try to
        // light the upper stage: it needs ullage and fails.
        v.set_throttle(0.0);
        v.step(&system, t, t + DT, &mut events);
        t = t + DT;
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
        let booster = v.propulsion.stages[0].length;
        let upper_base =
            |v: &Vessel, extra: f64| v.base_at(&system, t) + nose(v, &system, t) * extra;
        let before = upper_base(&v, booster);
        let com_before = v.mass_properties().center_of_mass;
        v.activate_next_stage(&system, t);
        assert_eq!(v.propulsion.stages.len(), 2);
        let after = upper_base(&v, 0.0);
        assert!(
            (before - after).length() < 1e-6,
            "{}",
            (before - after).length()
        );
        assert!(v.mass_properties().center_of_mass < com_before);
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
        assert!(v.propulsion.stages[0].rcs_fraction().unwrap() < 1.0);
        v.set_throttle(1.0);
        run(&mut v, &system, &mut t, 1.0);
        assert!(v.propulsion.any_engine_running());
    }

    #[test]
    fn rcs_turns_the_stage_without_leaving_its_orbit() {
        let system = rss();
        let (mut v, mut t) = leo_vessel(&system);
        for stage in &mut v.propulsion.stages {
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
        assert_eq!(v.propulsion.stages.len(), 3);
        v.activate_next_stage(&system, t);
        fly(&mut v, &mut t, 60.0);
        let log: Vec<_> = v.log.iter().map(|e| &e.message).collect();
        assert!(v.propulsion.any_engine_running(), "{log:?}");
        assert!(!v.is_destroyed());
        // The tanks work hard (full tanks at liftoff, high g near burnout) but hold.
        assert!(peak > 0.3 && peak < 1.0, "{peak}");
    }

    #[test]
    fn an_overloaded_stack_collapses_on_the_pad() {
        let system = rss();
        let mut craft = pathfinder();
        craft.stages[2].structure_mass *= 100.0;
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
        assert!(!v.propulsion.stages[0].active);
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
            argument_of_periapsis: 0.0,
            mean_anomaly: 0.0,
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
        assert!(v.propulsion.stages[0].fins.is_some());
        assert!(v.flight.structure_load < 1.0);
        // Thirty degrees off: the fins go, the rest of the rocket holds.
        fly_at(&mut v, 600.0, 30f64.to_radians());
        assert!(v.propulsion.stages[0].fins.is_none());
        assert!(v.log.iter().any(|e| e.message.contains("fins torn off")));
        assert!(!v.is_destroyed());
    }

    #[test]
    fn rails_hold_the_sas_direction() {
        let system = rss();
        let (mut v, t) = leo_vessel(&system);
        v.set_sas_mode(&system, t, SasMode::RadialOut);
        v.enter_rails(&system, t);
        let target = |v: &Vessel, t| v.nav_basis(&system, t).unwrap().radial_out;
        // It snaps onto the target, then follows it around the orbit.
        for minutes in [0.0, 13.0, 47.0, 400.0] {
            let at = t.add_seconds(minutes * 60.0);
            assert!(nose(&v, &system, at).angle_between(target(&v, at)) < 1e-6);
        }
        // Stability holds still, whatever the warp.
        let mut v = leo_vessel(&system).0;
        v.set_sas(&system, t, true);
        v.enter_rails(&system, t);
        let later = t.add_seconds(86_400.0);
        assert_eq!(v.attitude_at(&system, later), v.attitude_at(&system, t));
    }
}
