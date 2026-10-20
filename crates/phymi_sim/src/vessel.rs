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

use glam::{DMat3, DQuat, DVec3};
use thiserror::Error;

use crate::{
    atmosphere::AtmosphereSample,
    body::{Body, BodyId},
    data::{CraftDef, LaunchSiteDef, OrbitStartDef, PartCatalog},
    events::{EventKind, TrajectoryEvent, apply_event, next_event},
    integrator::Encke,
    orbit::{Elements, Orbit, StateVector},
    propulsion::{CraftError, Propulsion},
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
    /// Position and attitude in the body-fixed (rotating) frame.
    Landed {
        position: DVec3,
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
}

#[derive(Debug, Clone)]
pub struct Vessel {
    pub name: String,
    parent: BodyId,
    motion: Motion,
    /// Vessel-local to inertial. Local +Z is the nose / thrust axis.
    attitude: DQuat,
    /// Commanded (pitch, yaw, roll) in -1..1.
    pub rotation_input: DVec3,
    /// Rotation rate at full input, rad/s. Stand-in until torque/MoI is modelled.
    pub max_rotation_rate: f64,
    pub propulsion: Propulsion,
    propellant_settled: bool,
    pub flight: FlightData,
    pub log: Vec<LogEntry>,
}

impl Vessel {
    fn new(
        craft: &CraftDef,
        catalog: &PartCatalog,
        parent: BodyId,
        motion: Motion,
    ) -> Result<Self, VesselError> {
        let propulsion = Propulsion::from_craft(craft, catalog)?;
        Ok(Self {
            name: if craft.name.is_empty() {
                craft.id.clone()
            } else {
                craft.name.clone()
            },
            parent,
            motion,
            attitude: DQuat::IDENTITY,
            rotation_input: DVec3::ZERO,
            max_rotation_rate: 0.15,
            flight: FlightData {
                mass: propulsion.mass(),
                ..Default::default()
            },
            propulsion,
            propellant_settled: false,
            log: Vec::new(),
        })
    }

    /// Standing on the surface at a launch site, nose up, pitch axis pointing east.
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
        let position = up * (body.radius + body.terrain_height(up));

        let mut vessel = Self::new(
            craft,
            catalog,
            parent,
            Motion::Landed { position, attitude },
        )?;
        vessel.attitude = body.orientation(time) * attitude;
        vessel.propellant_settled = true;
        vessel.record(
            time,
            format!("{} on the pad at {}", vessel.name, site.display_name()),
        );
        Ok(vessel)
    }

    /// In orbit, from elements relative to the body's equator. Nose prograde.
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

        let mut vessel = Self::new(
            craft,
            catalog,
            parent,
            Motion::Coasting(Orbit::new(body.gm, time, state)),
        )?;
        vessel.set_free_state(system, parent, state, time);
        vessel.attitude = DQuat::from_rotation_arc(DVec3::Z, state.velocity.normalize());
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

    /// Whether the next step needs numerical integration (rails warp is impossible).
    pub fn needs_physics(&self) -> bool {
        matches!(self.motion, Motion::Integrated(_)) || self.propulsion.wants_thrust()
    }

    /// State relative to [`Self::parent`] at `time`. Exact for landed and coasting vessels;
    /// integrated vessels are extrapolated linearly from their last step, which is only
    /// meant for rendering between physics ticks.
    pub fn state_at(&self, system: &SolarSystem, time: SimTime) -> StateVector {
        let body = system.body(self.parent);
        match &self.motion {
            Motion::Landed { position, .. } => {
                let position = body.orientation(time) * *position;
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

    pub fn attitude_at(&self, system: &SolarSystem, time: SimTime) -> DQuat {
        match &self.motion {
            Motion::Landed { attitude, .. } => {
                system.body(self.parent).orientation(time) * *attitude
            }
            _ => self.attitude,
        }
    }

    pub fn nose(&self) -> DVec3 {
        self.attitude * DVec3::Z
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
        self.propulsion.set_throttle(throttle);
    }

    pub fn activate_next_stage(&mut self, time: SimTime) {
        if let Some(message) = self.propulsion.activate_next_stage() {
            self.record(time, message);
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
        if !self.is_landed() {
            let omega = DVec3::new(
                -self.rotation_input.x,
                self.rotation_input.y,
                self.rotation_input.z,
            ) * self.max_rotation_rate;
            self.attitude = (self.attitude * DQuat::from_scaled_axis(omega * dt)).normalize();
        } else {
            self.attitude = self.attitude_at(system, t0);
        }

        let body = system.body(self.parent);
        let state = self.state_at(system, t0);
        let air = air_at(body, &state);
        let mut messages = Vec::new();
        let thrust = self
            .propulsion
            .burn(air.pressure, dt, self.propellant_settled, &mut messages);
        for message in messages {
            self.record(t0, message);
        }
        let mass = self.propulsion.mass();
        let thrust_accel = self.nose() * (thrust / mass);
        let drag_accel = drag(body, &air, self.propulsion.reference_area(), mass);
        let proper = thrust_accel + drag_accel(state.position, state.velocity);
        let air_speed = (state.velocity - body.surface_velocity(state.position)).length();
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
        };
        self.propellant_settled = self.is_landed() || proper.dot(self.nose()) > ULLAGE_ACCELERATION;

        match self.motion {
            Motion::Landed { .. } => {
                let up = state.position.normalize();
                let weight = body.gm / state.position.length_squared();
                if thrust_accel.dot(up) <= weight {
                    return;
                }
                self.motion = Motion::Integrated(Encke::new(body.gm, t0, state));
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
        let drag_accel = drag(
            body,
            &air,
            self.propulsion.reference_area(),
            self.propulsion.mass(),
        );
        encke.step(end, |r, v| thrust_accel + drag_accel(r, v));
        let state = encke.state();

        // Ground contact.
        let orientation = body.orientation(end);
        let fixed_dir = orientation.inverse() * state.position.normalize();
        let surface = body.radius + body.terrain_height(fixed_dir);
        if state.position.length() < surface {
            let speed = (state.velocity - body.surface_velocity(state.position)).length();
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
                position: fixed_dir * surface,
                attitude: orientation.inverse() * self.attitude,
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

/// Drag acceleration with air co-rotating with the body; density held for the step.
fn drag(
    body: &Body,
    air: &AtmosphereSample,
    area: f64,
    mass: f64,
) -> impl Fn(DVec3, DVec3) -> DVec3 {
    let omega = body.angular_velocity();
    let (density, speed_of_sound) = (air.density, air.speed_of_sound);
    move |r, v| {
        if density == 0.0 {
            return DVec3::ZERO;
        }
        let v_air = v - omega.cross(r);
        let speed = v_air.length();
        -0.5 * density * speed * v_air * drag_coefficient(speed / speed_of_sound) * area / mass
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
            tanks: records(include_str!("../../../assets/core/tanks/stock.json5")),
        }
    }

    pub fn pathfinder() -> CraftDef {
        records(include_str!("../../../assets/core/craft/pathfinder.json5")).remove(0)
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

    const DT: Ticks = Ticks::from_millis(20);

    #[test]
    fn pad_vessel_sits_on_rotating_surface() {
        let system = rss();
        let (mut v, start) = pad_vessel(&system);
        let earth = system.body(v.parent());
        let s = v.state_at(&system, start);
        assert!((s.position.length() - earth.radius).abs() < 1e-6);
        // Kennedy at 28.6°N moves east at ~408 m/s.
        assert!(
            (s.velocity.length() - 408.0).abs() < 2.0,
            "{}",
            s.velocity.length()
        );
        assert!(v.nose().dot(s.position.normalize()) > 0.9999);
        let mut t = start;
        let mut events = Vec::new();
        for _ in 0..500 {
            v.step(&system, t, t + DT, &mut events);
            t = t + DT;
        }
        assert!(v.is_landed() && events.is_empty());
        // A year later it is still exactly on the ground.
        let later = t.add_seconds(365.25 * 86_400.0);
        assert!((v.state_at(&system, later).position.length() - earth.radius).abs() < 1e-6);
    }

    #[test]
    fn pad_vessel_stands_on_the_pad_deck() {
        let mut system = rss();
        let sites = launch_sites();
        crate::terrain::install(&mut system, Vec::new(), &sites);
        let (mut v, start) = pad_vessel(&system);
        let earth = system.body(v.parent());
        let deck = sites[0].pad.as_ref().unwrap().elevation.unwrap();
        let altitude = |v: &Vessel, t| v.state_at(&system, t).position.length() - earth.radius;
        assert!((altitude(&v, start) - deck).abs() < 1e-6);
        // It stays put on the raised deck rather than sinking or bouncing.
        let mut t = start;
        let mut events = Vec::new();
        for _ in 0..200 {
            v.step(&system, t, t + DT, &mut events);
            t = t + DT;
        }
        assert!(v.is_landed() && events.is_empty());
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

    #[test]
    fn liftoff_and_ullage() {
        let system = rss();
        let (mut v, mut t) = pad_vessel(&system);
        let mut events = Vec::new();
        v.set_throttle(1.0);
        v.activate_next_stage(t);
        for _ in 0..(20 * 50) {
            v.step(&system, t, t + DT, &mut events);
            t = t + DT;
        }
        assert!(events.iter().any(|e| e.kind == VesselEventKind::Liftoff));
        let altitude = v.state_at(&system, t).position.length() - system.body(v.parent()).radius;
        assert!(altitude > 500.0, "{altitude}");

        // Cut throttle (drag now decelerates us: propellant floats forward), then try to
        // light the upper stage: it needs ullage and fails.
        v.set_throttle(0.0);
        v.step(&system, t, t + DT, &mut events);
        t = t + DT;
        v.activate_next_stage(t);
        v.set_throttle(1.0);
        v.step(&system, t, t + DT, &mut events);
        assert!(!v.propulsion.any_engine_running());
        assert!(v.log.iter().any(|e| e.message.contains("ullage")));
    }
}
