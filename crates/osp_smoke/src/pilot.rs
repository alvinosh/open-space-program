//! The hands on the controls: points the nose with the game's own SAS control law (so it
//! steers as the player's stability assist does), and flies a rocket up to orbit the way a
//! player would (a short pitch kick, a gravity turn, then levelling off).

use osp_sim::{
    DVec3,
    attitude::{Sas, SasMode},
};

use crate::world::World;

/// Vertical deceleration the levelling off plans for, m/s².
const VACUUM_BRAKING: f64 = 2.5;
/// Time constant of the roll rate damping, s (SAS's own).
const ROLL_RESPONSE: f64 = 0.25;

/// Steers by the stick: sets the vessel's rotation command each tick.
pub struct Pilot {
    sas: Sas,
}

impl Pilot {
    pub fn new() -> Self {
        let mut sas = Sas::default();
        // Any mode but stability: the direction comes from the caller.
        sas.set_mode(SasMode::Prograde);
        Self { sas }
    }

    /// Turns the nose towards `direction` (inertial), and keeps the roll from running away.
    pub fn point(&mut self, w: &mut World, direction: DVec3) {
        let now = w.now();
        let system = &w.universe.system;
        let v = w.vessel();
        let attitude = v.attitude_state(system, now);
        let inertia = v
            .mass_properties()
            .inertia
            .diagonal()
            .max(DVec3::splat(1e-9));
        let alpha = v.flight.torque_authority / inertia;
        let mut command = self.sas.steer(attitude, alpha, Some(direction));
        command.z = if alpha.z > 0.0 {
            (-attitude.angular_velocity.z / (ROLL_RESPONSE * alpha.z)).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        w.vessel_mut().rotation_input = command;
    }
}

/// Up, east and north at the vessel (inertial axes).
pub fn local_frame(w: &World) -> (DVec3, DVec3, DVec3) {
    let system = &w.universe.system;
    let v = w.vessel();
    let body = system.body(v.parent());
    let up = v.state_at(system, w.now()).position.normalize();
    let pole = body.orientation(w.now()) * DVec3::Z;
    let east = pole.cross(up).normalize_or_zero();
    (up, east, up.cross(east))
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Leg {
    /// Straight up until the tower is clear.
    Vertical,
    /// The pitch program, kept close to the airflow so the air asks little of the structure.
    Turn,
    /// Above the air: thrust steered to level off at the target altitude.
    Vacuum,
}

/// How a launch is flown.
#[derive(Debug, Clone, Copy)]
pub struct Profile {
    /// Orbit inclination wanted, degrees.
    pub inclination: f64,
    /// Circular orbit altitude wanted, m.
    pub altitude: f64,
    /// Tower height to clear before pitching over, m.
    pub clear: f64,
    /// The pitch program: the nose is `90° · (altitude / turn_end)^turn_shape` below the
    /// vertical, so the climb bends over smoothly. Altitude (m) and shape.
    pub turn_end: f64,
    pub turn_shape: f64,
    /// Most the nose strays from the airflow while the air is thick, degrees.
    pub max_aoa: f64,
    /// Dynamic pressure below which the gravity turn gives way to vacuum guidance, Pa.
    pub vacuum_q: f64,
}

/// A launch to orbit: flies in a plane fixed at liftoff.
pub struct Ascent {
    pilot: Pilot,
    profile: Profile,
    leg: Leg,
    /// The orbit's normal (inertial), fixed when the launch begins.
    pub plane: DVec3,
}

impl Ascent {
    /// Plans the launch from where the stack stands now: due-east-ish azimuth for the
    /// inclination, the Earth's turning allowed for by steering on the plane.
    pub fn new(w: &World, profile: Profile) -> Self {
        let (up, east, north) = local_frame(w);
        let body = w.universe.system.body(w.vessel().parent());
        let pole = body.orientation(w.now()) * DVec3::Z;
        let lat = up.dot(pole).asin();
        let cos_az = (profile.inclination.to_radians().cos() / lat.cos()).clamp(-1.0, 1.0);
        let azimuth = cos_az.asin(); // from north, towards the east
        let heading = north * azimuth.cos() + east * azimuth.sin();
        Self {
            pilot: Pilot::new(),
            profile,
            leg: Leg::Vertical,
            plane: up.cross(heading).normalize(),
        }
    }

    /// The horizontal direction along the plane at the vessel, leaning against any drift
    /// out of it.
    fn level(&self, up: DVec3, velocity: DVec3) -> DVec3 {
        let along = self.plane.cross(up).normalize_or_zero();
        let across = velocity.dot(self.plane);
        let speed = velocity.reject_from(up).length().max(300.0);
        (along - self.plane * (across / speed).clamp(-0.4, 0.4)).normalize_or_zero()
    }

    /// The pilot's turn, before each tick of the climb.
    pub fn steer(&mut self, w: &mut World) {
        let now = w.now();
        let system = &w.universe.system;
        let v = w.vessel();
        let body = system.body(v.parent());
        let state = v.state_at(system, now);
        let up = state.position.normalize();
        let altitude = state.position.length() - body.radius;
        let ground = state.velocity - body.surface_velocity(state.position);
        let level = self.level(up, state.velocity);
        let profile = self.profile;

        if self.leg == Leg::Vertical && altitude > profile.clear {
            self.leg = Leg::Turn;
        }
        let levelling = self.leg == Leg::Turn
            && v.flight.dynamic_pressure < profile.vacuum_q
            && altitude > 30_000.0;
        if levelling {
            self.leg = Leg::Vacuum;
        }

        let direction = match self.leg {
            Leg::Vertical => up,
            Leg::Turn => {
                let share = (altitude / profile.turn_end)
                    .clamp(0.0, 1.0)
                    .powf(profile.turn_shape);
                let pitch = (90.0 * (1.0 - share)).to_radians();
                let program = up * pitch.sin() + level * pitch.cos();
                match ground.try_normalize() {
                    // Thick air and moving: the nose stays within a few degrees of the airflow.
                    Some(flow) if ground.length() > 60.0 && v.flight.dynamic_pressure > 100.0 => {
                        let off = flow.angle_between(program);
                        let max = profile.max_aoa.to_radians();
                        if off > max {
                            flow.lerp(program, max / off).normalize()
                        } else {
                            program
                        }
                    }
                    _ => program,
                }
            }
            Leg::Vacuum => {
                // Climb to the target altitude on a braking curve that arrives with no
                // vertical speed, and thrust just enough upwards to follow it.
                let r = state.position.length();
                let radial = state.velocity.dot(up);
                let tangential = state.velocity.reject_from(up).length();
                let to_go = body.radius + profile.altitude - r;
                let braking = (2.0 * VACUUM_BRAKING * to_go.abs()).sqrt().copysign(to_go);
                let wanted = braking.min(0.02 * to_go).clamp(-300.0, 1200.0);
                let up_accel =
                    (wanted - radial) / 15.0 + body.gm / (r * r) - tangential * tangential / r;
                let thrust = (v.flight.thrust / v.flight.mass).max(1.0);
                let sin = (up_accel / thrust).clamp(-0.15, 0.98);
                up * sin + level * (1.0 - sin * sin).sqrt()
            }
        };
        if levelling {
            w.event("Guidance: levelling off for the target orbit");
        }
        self.pilot.point(w, direction);
    }
}
