//! Rotational dynamics: the stack's mass properties, control torque from engine gimbal and
//! reaction wheels (RCS is in [`crate::rcs`]), and the SAS autopilot.
//!
//! A vessel is a rigid, axisymmetric body: its inertia about the centre of mass is diagonal
//! in vessel axes, with equal pitch and yaw moments. That makes torque-free rotation (a
//! spinning stage in orbit) exactly solvable ([`coast`]), so like a coasting trajectory it
//! is evaluated analytically at any time and time warp cannot change it. Under torque,
//! Euler's equations are integrated on the physics tick ([`integrate`]).
//!
//! Vessel axes: +Z is the nose, +X the pilot's right and +Y the pilot's feet, so a positive
//! rotation about +X pitches the nose up, about +Y yaws right and about +Z rolls right.
//! Rotation commands use the same axes, −1..1 each.

use std::{array, f64::consts::PI};

use glam::{DMat3, DQuat, DVec2, DVec3};

use crate::{
    body::Body,
    orbit::StateVector,
    propulsion::{Engine, Propulsion, Stage},
};

/// Principal moments of inertia about the centre of mass.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Inertia {
    /// About the pitch and yaw axes (vessel X and Y), kg·m².
    pub transverse: f64,
    /// About the roll axis (vessel Z), kg·m².
    pub axial: f64,
}

impl Inertia {
    pub fn diagonal(&self) -> DVec3 {
        DVec3::new(self.transverse, self.transverse, self.axial)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MassProperties {
    pub mass: f64,
    /// Height of the centre of mass above the base of the stack, m.
    pub center_of_mass: f64,
    pub inertia: Inertia,
}

/// Mass distribution of a stack of stages, bottom first. Each stage is a cylinder of its
/// diameter and length: tanks are thin shells along it, structure (and payload) a uniform
/// solid, engines point masses at their mounts on the stage's base, fins rings of mass at
/// their centroid, and propellant a solid column settled at the aft end, where thrust pushes
/// it. The centre of mass therefore moves as the tanks drain.
pub fn mass_properties(stages: &[Stage]) -> MassProperties {
    // Sums of m, m·z, m·z² and each element's own moments about its centre.
    let (mut mass, mut first, mut second) = (0.0, 0.0, 0.0);
    let (mut own_transverse, mut axial) = (0.0, 0.0);
    let mut add = |m: f64, z: f64, transverse: f64, roll: f64| {
        mass += m;
        first += m * z;
        second += m * z * z;
        own_transverse += transverse;
        axial += roll;
    };
    let mut base = 0.0;
    for stage in stages {
        let (r2, length) = ((stage.diameter / 2.0).powi(2), stage.length);
        let middle = base + length / 2.0;
        let m = stage.tank_mass;
        add(m, middle, m * (r2 / 2.0 + length * length / 12.0), m * r2);
        let m = stage.structure_mass;
        add(
            m,
            middle,
            m * (r2 / 4.0 + length * length / 12.0),
            m * r2 / 2.0,
        );
        for engine in &stage.engines {
            let (m, d2) = (engine.def.mass, engine.mount.length_squared());
            add(m, base, m * d2 / 2.0, m * d2);
        }
        if let Some(fins) = &stage.fins {
            let (out, height) = fins.centroid();
            let (m, d2) = (fins.mass, (stage.diameter / 2.0 + out).powi(2));
            add(m, base + height, m * d2 / 2.0, m * d2);
        }
        let m = stage.propellant_mass();
        let fill = (m / stage.propellant_capacity().max(1e-9)).clamp(0.0, 1.0);
        let h = length * fill;
        add(
            m,
            base + h / 2.0,
            m * (r2 / 4.0 + h * h / 12.0),
            m * r2 / 2.0,
        );
        base += length;
    }
    if mass <= 0.0 {
        return MassProperties::default();
    }
    let center_of_mass = first / mass;
    MassProperties {
        mass,
        center_of_mass,
        inertia: Inertia {
            // Parallel-axis theorem, summed: Σ m (z − c)² = Σ m z² − M c².
            transverse: own_transverse + (second - mass * center_of_mass * center_of_mass).max(0.0),
            axial,
        },
    }
}

fn bottom_engines(propulsion: &Propulsion) -> impl Iterator<Item = &Engine> {
    propulsion
        .stages
        .first()
        .into_iter()
        .flat_map(|s| &s.engines)
}

fn gimbal_range(engine: &Engine) -> f64 {
    engine.def.gimbal_range.clamp(0.0, 90.0).to_radians()
}

fn wheel_torque(propulsion: &Propulsion) -> f64 {
    propulsion.stages.iter().map(|s| s.wheel_torque).sum()
}

/// The largest torque the actuators can produce about each vessel axis right now, N·m:
/// reaction wheels anywhere in the stack, plus the gimbal of the bottom stage's burning
/// engines (side force times the lever arm to the centre of mass; for roll, the engine's
/// distance from the axis).
pub fn torque_authority(propulsion: &Propulsion, center_of_mass: f64) -> DVec3 {
    let arm = center_of_mass.abs();
    let gimbal: DVec3 = bottom_engines(propulsion)
        .map(|e| e.thrust * gimbal_range(e).sin() * DVec3::new(arm, arm, e.mount.length()))
        .sum();
    gimbal + DVec3::splat(wheel_torque(propulsion))
}

/// Swivel the bottom stage's engines and drive the reaction wheels for `command` (−1..1
/// about each vessel axis). Returns the thrust force and the torque about the centre of
/// mass, both in vessel axes.
///
/// Each engine tilts for pitch and yaw together, plus sideways around the axis for roll if
/// it is off-centre; a command beyond what one engine can do is scaled back to its range.
pub fn actuate(propulsion: &mut Propulsion, center_of_mass: f64, command: DVec3) -> (DVec3, DVec3) {
    let command = command.clamp(DVec3::NEG_ONE, DVec3::ONE);
    let mut force = DVec3::ZERO;
    let mut torque = command * wheel_torque(propulsion);
    let Some(stage) = propulsion.stages.first_mut() else {
        return (force, torque);
    };
    // Thrust tilted towards +Y at the tail (below the centre of mass) pitches the nose up.
    let lever = if center_of_mass < 0.0 { -1.0 } else { 1.0 };
    for engine in &mut stage.engines {
        let mut tilt = DVec2::new(-command.y, command.x) * lever;
        if let Some(radial) = engine.mount.try_normalize() {
            tilt += radial.perp() * command.z;
        }
        if tilt.length_squared() > 1.0 {
            tilt = tilt.normalize();
        }
        engine.deflection = if engine.thrust > 0.0 {
            tilt * gimbal_range(engine)
        } else {
            DVec2::ZERO
        };
        let angle = engine.deflection.length();
        let side = engine.deflection.normalize_or_zero() * angle.sin();
        let f = engine.thrust * DVec3::new(side.x, side.y, angle.cos());
        let r = DVec3::new(engine.mount.x, engine.mount.y, -center_of_mass);
        force += f;
        torque += r.cross(f);
    }
    (force, torque)
}

/// Orientation (vessel to inertial axes) and angular velocity (vessel axes, rad/s).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Attitude {
    pub orientation: DQuat,
    pub angular_velocity: DVec3,
}

impl Attitude {
    pub fn nose(&self) -> DVec3 {
        self.orientation * DVec3::Z
    }

    /// Angular velocity in inertial axes.
    pub fn angular_velocity_inertial(&self) -> DVec3 {
        self.orientation * self.angular_velocity
    }
}

/// Torque-free rotation for `dt` seconds, exact (a free symmetric top): the body turns
/// about its fixed angular momentum at |L| / I⊥ while spinning about its own axis at
/// ωz (I⊥ − Iz) / I⊥, so the transverse rate precesses in vessel axes.
pub fn coast(state: Attitude, inertia: Inertia, dt: f64) -> Attitude {
    if dt == 0.0 || inertia.transverse <= 0.0 {
        return state;
    }
    let w = state.angular_velocity;
    let spin = w.z * (inertia.transverse - inertia.axial) / inertia.transverse;
    let momentum = state.orientation * (inertia.diagonal() * w);
    let precession = match momentum.try_normalize() {
        Some(axis) => DQuat::from_axis_angle(axis, momentum.length() / inertia.transverse * dt),
        None => DQuat::IDENTITY,
    };
    Attitude {
        orientation: (precession * state.orientation * DQuat::from_rotation_z(spin * dt))
            .normalize(),
        angular_velocity: DQuat::from_rotation_z(-spin * dt) * w,
    }
}

/// Rotation under a constant torque (vessel axes, N·m) for `dt` seconds: one RK4 step of
/// Euler's equations, I ω̇ = τ − ω × Iω, and the quaternion kinematics q̇ = ½ q ω.
pub fn integrate(state: Attitude, inertia: Inertia, torque: DVec3, dt: f64) -> Attitude {
    let i = inertia.diagonal().max(DVec3::splat(1e-9));
    let accel = |w: DVec3| (torque - w.cross(i * w)) / i;
    let turn = |q: DQuat, w: DVec3| q * DQuat::from_xyzw(w.x, w.y, w.z, 0.0) * 0.5;
    let (q0, w0) = (state.orientation, state.angular_velocity);
    let (k1q, k1w) = (turn(q0, w0), accel(w0));
    let (q1, w1) = (q0 + k1q * (dt / 2.0), w0 + k1w * (dt / 2.0));
    let (k2q, k2w) = (turn(q1, w1), accel(w1));
    let (q2, w2) = (q0 + k2q * (dt / 2.0), w0 + k2w * (dt / 2.0));
    let (k3q, k3w) = (turn(q2, w2), accel(w2));
    let (q3, w3) = (q0 + k3q * dt, w0 + k3w * dt);
    let (k4q, k4w) = (turn(q3, w3), accel(w3));
    Attitude {
        orientation: (q0 + (k1q + k2q * 2.0 + k3q * 2.0 + k4q) * (dt / 6.0)).normalize(),
        angular_velocity: w0 + (k1w + 2.0 * k2w + 2.0 * k3w + k4w) * (dt / 6.0),
    }
}

/// Reference for prograde and the other navigation directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavFrame {
    /// Velocity relative to the ground (and the air, which turns with it).
    #[default]
    Surface,
    /// Inertial velocity around the body.
    Orbit,
    /// Velocity relative to the target (prograde is closing on it when it points at it).
    Target,
}

impl NavFrame {
    pub fn label(self) -> &'static str {
        match self {
            NavFrame::Surface => "Surface",
            NavFrame::Orbit => "Orbit",
            NavFrame::Target => "Target",
        }
    }
}

/// Prograde, orbit normal and radial-out directions, in inertial axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavBasis {
    pub prograde: DVec3,
    pub normal: DVec3,
    pub radial_out: DVec3,
}

impl NavBasis {
    /// Below this speed, m/s, prograde is undefined (on the pad, for instance).
    pub const MIN_SPEED: f64 = 0.1;

    /// From a state relative to `body`. The target frame needs the target's state, so
    /// here (without it) it is the orbit frame; see `Vessel::nav_basis`.
    pub fn new(frame: NavFrame, body: &Body, state: &StateVector) -> Option<Self> {
        let velocity = match frame {
            NavFrame::Surface => state.velocity - body.surface_velocity(state.position),
            NavFrame::Orbit | NavFrame::Target => state.velocity,
        };
        Self::from_velocity(state.position, velocity)
    }

    /// The target frame: prograde along the velocity relative to `target` (both relative
    /// to the same body); normal and radial stay those of the orbit.
    pub fn relative_to(state: &StateVector, target: &StateVector) -> Option<Self> {
        let orbital = Self::orbital(state)?;
        let relative = state.velocity - target.velocity;
        (relative.length() >= Self::MIN_SPEED).then(|| Self {
            prograde: relative.normalize(),
            ..orbital
        })
    }

    /// The orbit frame (inertial velocity), which burns are planned in.
    pub fn orbital(state: &StateVector) -> Option<Self> {
        Self::from_velocity(state.position, state.velocity)
    }

    fn from_velocity(position: DVec3, velocity: DVec3) -> Option<Self> {
        if velocity.length() < Self::MIN_SPEED {
            return None;
        }
        let prograde = velocity.normalize();
        let normal = position.cross(velocity).try_normalize()?;
        Some(Self {
            prograde,
            normal,
            radial_out: prograde.cross(normal),
        })
    }

    /// The basis as a rotation matrix (columns prograde, normal, radial out).
    pub fn matrix(&self) -> DMat3 {
        DMat3::from_cols(self.prograde, self.normal, self.radial_out)
    }
}

/// What SAS steers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SasMode {
    /// Stop rotating and hold the attitude.
    #[default]
    Stability,
    Prograde,
    Retrograde,
    Normal,
    AntiNormal,
    RadialOut,
    RadialIn,
    /// Along the next planned burn (see [`crate::plan`]).
    Burn,
    /// Towards the target, and away from it.
    Target,
    AntiTarget,
}

impl SasMode {
    pub const ALL: [SasMode; 10] = [
        SasMode::Stability,
        SasMode::Prograde,
        SasMode::Retrograde,
        SasMode::Normal,
        SasMode::AntiNormal,
        SasMode::RadialOut,
        SasMode::RadialIn,
        SasMode::Burn,
        SasMode::Target,
        SasMode::AntiTarget,
    ];

    /// Name used in actions and UI data.
    pub fn id(self) -> &'static str {
        match self {
            SasMode::Stability => "stability",
            SasMode::Prograde => "prograde",
            SasMode::Retrograde => "retrograde",
            SasMode::Normal => "normal",
            SasMode::AntiNormal => "anti_normal",
            SasMode::RadialOut => "radial_out",
            SasMode::RadialIn => "radial_in",
            SasMode::Burn => "burn",
            SasMode::Target => "target",
            SasMode::AntiTarget => "anti_target",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            SasMode::Stability => "Stability",
            SasMode::Prograde => "Prograde",
            SasMode::Retrograde => "Retrograde",
            SasMode::Normal => "Normal",
            SasMode::AntiNormal => "Anti-normal",
            SasMode::RadialOut => "Radial out",
            SasMode::RadialIn => "Radial in",
            SasMode::Burn => "Burn",
            SasMode::Target => "Target",
            SasMode::AntiTarget => "Anti-target",
        }
    }

    /// Where the nose should point; `None` for stability, which holds an attitude instead,
    /// and for burn and target modes, whose direction comes from the flight plan or the
    /// target rather than the frame.
    pub fn direction(self, basis: &NavBasis) -> Option<DVec3> {
        match self {
            SasMode::Stability | SasMode::Burn | SasMode::Target | SasMode::AntiTarget => None,
            SasMode::Prograde => Some(basis.prograde),
            SasMode::Retrograde => Some(-basis.prograde),
            SasMode::Normal => Some(basis.normal),
            SasMode::AntiNormal => Some(-basis.normal),
            SasMode::RadialOut => Some(basis.radial_out),
            SasMode::RadialIn => Some(-basis.radial_out),
        }
    }
}

/// Turn rate, rad/s, below which stability assist locks the current attitude.
const CAPTURE_RATE: f64 = 0.005;
/// Time constant of the rate loop, s.
const RESPONSE: f64 = 0.25;
/// Commanded rate per radian of error close to the target, 1/s.
const GAIN: f64 = 1.5;
/// Share of the authority planned for braking, leaving margin for changing thrust.
const BRAKING: f64 = 0.5;
/// Fastest turn SAS commands, rad/s.
const MAX_RATE: f64 = 0.5;

/// Stability augmentation: holds an attitude or points the nose along a navigation
/// direction, with any torque the vessel has.
///
/// For each axis it commands the fastest turn rate that can still be stopped at the
/// target with half the available angular acceleration (a time-optimal braking curve,
/// linear close in), then the torque that reaches that rate in [`RESPONSE`] seconds.
#[derive(Debug, Clone, Default)]
pub struct Sas {
    enabled: bool,
    mode: SasMode,
    /// The attitude stability assist holds, captured once rotation has been damped out.
    hold: Option<DQuat>,
}

impl Sas {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn mode(&self) -> SasMode {
        self.mode
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.hold = None;
    }

    /// Select a mode; this also switches SAS on.
    pub fn set_mode(&mut self, mode: SasMode) {
        self.enabled = true;
        self.mode = mode;
        self.hold = None;
    }

    /// The pilot is steering: let go of the held attitude, and capture a new one once
    /// they stop.
    pub fn release(&mut self) {
        self.hold = None;
    }

    /// Rotation command (−1..1 per vessel axis). `alpha` is the largest angular
    /// acceleration about each axis, rad/s²; `target` is where the mode wants the nose,
    /// if that is defined right now (otherwise SAS holds attitude).
    pub fn steer(&mut self, state: Attitude, alpha: DVec3, target: Option<DVec3>) -> DVec3 {
        let error = match target.filter(|_| self.mode != SasMode::Stability) {
            Some(direction) => pointing_error(state.orientation, direction),
            None => match self.hold {
                Some(hold) => attitude_error(state.orientation, hold),
                None => {
                    if state.angular_velocity.length() < CAPTURE_RATE {
                        self.hold = Some(state.orientation);
                    }
                    DVec3::ZERO
                }
            },
        };
        let w = state.angular_velocity;
        DVec3::from_array(array::from_fn(|i| axis_command(error[i], w[i], alpha[i])))
    }
}

fn axis_command(error: f64, rate: f64, alpha: f64) -> f64 {
    if alpha <= 0.0 {
        return 0.0;
    }
    let speed = (GAIN * error.abs())
        .min((2.0 * BRAKING * alpha * error.abs()).sqrt())
        .min(MAX_RATE);
    ((speed.copysign(error) - rate) / (RESPONSE * alpha)).clamp(-1.0, 1.0)
}

/// Rotation (vessel axes, radians) that turns the nose onto `direction` without rolling.
fn pointing_error(orientation: DQuat, direction: DVec3) -> DVec3 {
    let target = orientation.inverse() * direction;
    let axis = DVec3::Z.cross(target);
    let angle = axis.length().atan2(target.z);
    match axis.try_normalize() {
        Some(axis) => axis * angle,
        // Exactly backwards: any turn will do.
        None if target.z < 0.0 => DVec3::new(PI, 0.0, 0.0),
        None => DVec3::ZERO,
    }
}

/// Rotation (vessel axes, radians) from `orientation` to `target`.
fn attitude_error(orientation: DQuat, target: DQuat) -> DVec3 {
    let delta = orientation.inverse() * target;
    let delta = if delta.w < 0.0 { -delta } else { delta };
    delta.to_scaled_axis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        data::{CraftDef, StageDef, TankDef, TankShape},
        vessel::tests::catalog,
    };

    /// A stage with a kerolox tank `tank` metres long, if non-zero.
    fn stage(engines: &[&str], tank: f64, structure: f64, d: f64, l: f64) -> StageDef {
        let tanks = (tank > 0.0).then(|| TankDef {
            tank_type: "Isogrid Al-Li".into(),
            shape: TankShape::Cylinder,
            length: tank,
            diameter: None,
            design_load: None,
            propellants: [("Kerosene".into(), 1.0), ("LqdOxygen".into(), 2.36)].into(),
        });
        StageDef {
            name: "test".into(),
            engines: engines.iter().map(|s| s.to_string()).collect(),
            tanks: tanks.into_iter().collect(),
            fins: None,
            reaction_wheels: Vec::new(),
            rcs: Vec::new(),
            pod: None,
            structure_mass: structure,
            diameter: d,
            length: l,
        }
    }

    fn propulsion(stages: Vec<StageDef>) -> Propulsion {
        let craft = CraftDef {
            id: "test".into(),
            name: String::new(),
            stages,
        };
        Propulsion::from_craft(&craft, &catalog()).unwrap()
    }

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance * b.abs().max(1.0)
    }

    #[test]
    fn solid_cylinder_inertia() {
        let p = propulsion(vec![stage(&[], 0.0, 1000.0, 2.0, 10.0)]);
        let m = mass_properties(&p.stages);
        assert!(close(m.center_of_mass, 5.0, 1e-12));
        assert!(close(m.inertia.axial, 1000.0 * 1.0 / 2.0, 1e-12));
        assert!(close(
            m.inertia.transverse,
            1000.0 * (0.25 + 100.0 / 12.0),
            1e-12
        ));
    }

    #[test]
    fn stacking_uses_the_parallel_axis_theorem() {
        let one = stage(&[], 0.0, 1000.0, 2.0, 10.0);
        let p = propulsion(vec![one.clone(), one]);
        let m = mass_properties(&p.stages);
        assert!(close(m.center_of_mass, 10.0, 1e-12));
        // Two cylinders, each 5 m from the common centre.
        let each = 1000.0 * (0.25 + 100.0 / 12.0) + 1000.0 * 25.0;
        assert!(close(m.inertia.transverse, 2.0 * each, 1e-12));
    }

    #[test]
    fn centre_of_mass_moves_as_the_booster_drains() {
        let drain = |p: &mut Propulsion| {
            for prop in &mut p.stages[0].propellants {
                prop.amount *= 0.1;
            }
        };
        // Alone, the propellant left sits at the bottom of the tank.
        let mut alone = propulsion(vec![stage(&["K-1 Booster"], 38.0, 0.0, 3.7, 42.0)]);
        let full = mass_properties(&alone.stages);
        drain(&mut alone);
        assert!(mass_properties(&alone.stages).center_of_mass < full.center_of_mass);
        // Under the upper stage, the stack's centre of mass climbs towards it.
        let mut stack =
            Propulsion::from_craft(&crate::vessel::tests::pathfinder(), &catalog()).unwrap();
        let full = mass_properties(&stack.stages);
        drain(&mut stack);
        let low = mass_properties(&stack.stages);
        assert!(
            low.center_of_mass > full.center_of_mass + 5.0,
            "{low:?} {full:?}"
        );
        assert!(low.inertia.transverse < full.inertia.transverse);
    }

    #[test]
    fn gimbal_torque_has_the_right_sign_and_size() {
        let mut p = propulsion(vec![stage(&["K-1 Vacuum"], 0.0, 5_000.0, 3.7, 14.0)]);
        let engine = &mut p.stages[0].engines[0];
        engine.thrust = 1e6;
        let range = engine.def.gimbal_range.to_radians();
        let com = 7.0;
        let (force, torque) = actuate(&mut p, com, DVec3::X);
        // Full pitch up: side force towards the feet at the tail, nose up.
        assert!(close(torque.x, 1e6 * range.sin() * com, 1e-12), "{torque}");
        assert!(torque.y.abs() < 1e-6 && torque.z.abs() < 1e-6);
        assert!(force.y > 0.0 && close(force.length(), 1e6, 1e-12));
        let authority = torque_authority(&p, com);
        assert!(close(authority.x, torque.x, 1e-12));
        // A single engine on the axis cannot roll.
        assert_eq!(authority.z, 0.0);
        let (_, torque) = actuate(&mut p, com, DVec3::Y);
        assert!(close(torque.y, 1e6 * range.sin() * com, 1e-12));
    }

    #[test]
    fn clusters_roll_by_gimballing_sideways() {
        let mut p = propulsion(vec![stage(&["K-1 Booster"; 9], 0.0, 5_000.0, 3.7, 40.0)]);
        for e in &mut p.stages[0].engines {
            e.thrust = 8e5;
        }
        let (force, torque) = actuate(&mut p, 20.0, DVec3::Z);
        assert!(torque.z > 0.0);
        assert!(torque.x.abs() < 1e-6 && torque.y.abs() < 1e-6, "{torque}");
        // Sideways tilts cancel: no net side force.
        assert!(force.x.abs() < 1e-6 && force.y.abs() < 1e-6);
        assert!(close(torque_authority(&p, 20.0).z, torque.z, 1e-9));
    }

    fn spinning() -> (Attitude, Inertia) {
        let state = Attitude {
            orientation: DQuat::from_euler(glam::EulerRot::XYZ, 0.3, -0.7, 1.1),
            angular_velocity: DVec3::new(0.02, -0.035, 0.4),
        };
        (
            state,
            Inertia {
                transverse: 5.0e6,
                axial: 4.0e5,
            },
        )
    }

    #[test]
    fn exact_coasting_matches_integration() {
        let (state, inertia) = spinning();
        let mut numeric = state;
        for _ in 0..(120 * 500) {
            numeric = integrate(numeric, inertia, DVec3::ZERO, 0.002);
        }
        let exact = coast(state, inertia, 120.0);
        assert!(exact.orientation.angle_between(numeric.orientation) < 1e-8);
        assert!((exact.angular_velocity - numeric.angular_velocity).length() < 1e-10);
    }

    #[test]
    fn coasting_conserves_angular_momentum() {
        let (state, inertia) = spinning();
        let momentum = |s: Attitude| s.orientation * (inertia.diagonal() * s.angular_velocity);
        let later = coast(state, inertia, 3.0e7);
        let (before, after) = (momentum(state), momentum(later));
        assert!((before - after).length() < 1e-9 * before.length());
        // Rotational energy too.
        let energy = |s: Attitude| {
            s.angular_velocity
                .dot(inertia.diagonal() * s.angular_velocity)
        };
        assert!(close(energy(later), energy(state), 1e-12));
    }

    /// Fly `sas` for `seconds` with angular acceleration `alpha` on every axis.
    fn fly(
        sas: &mut Sas,
        mut state: Attitude,
        target: Option<DVec3>,
        alpha: f64,
        seconds: f64,
    ) -> Attitude {
        let inertia = Inertia {
            transverse: 1.0,
            axial: 1.0,
        };
        for _ in 0..(seconds * 50.0) as usize {
            let command = sas.steer(state, DVec3::splat(alpha), target);
            assert!(command.abs().max_element() <= 1.0);
            state = integrate(state, inertia, command * alpha, 0.02);
        }
        state
    }

    #[test]
    fn sas_turns_onto_a_direction_without_overshoot() {
        let mut sas = Sas::default();
        sas.set_mode(SasMode::Prograde);
        let start = Attitude {
            orientation: DQuat::IDENTITY,
            angular_velocity: DVec3::new(0.0, 0.0, 0.2),
        };
        let target = DVec3::new(1.0, 1.0, -0.2).normalize();
        // A heavy stage: about 0.6 °/s².
        let alpha = 0.01;
        let mid = fly(&mut sas, start, Some(target), alpha, 30.0);
        let end = fly(&mut sas, mid, Some(target), alpha, 30.0);
        assert!(
            end.nose().angle_between(target) < 1e-3,
            "{}",
            end.nose().angle_between(target)
        );
        assert!(end.angular_velocity.length() < 1e-3);
        // Roll was damped, not steered.
        assert!(end.angular_velocity.z.abs() < 1e-4);
    }

    #[test]
    fn stability_damps_rotation_then_holds() {
        let mut sas = Sas::default();
        sas.set_enabled(true);
        let start = Attitude {
            orientation: DQuat::IDENTITY,
            angular_velocity: DVec3::new(0.05, -0.03, 0.1),
        };
        let settled = fly(&mut sas, start, None, 0.05, 20.0);
        assert!(settled.angular_velocity.length() < 1e-4);
        // Knocked off: it comes back to the held attitude.
        let held = sas.hold.unwrap();
        let knocked = Attitude {
            angular_velocity: DVec3::new(0.02, 0.0, 0.0),
            ..settled
        };
        let back = fly(&mut sas, knocked, None, 0.05, 20.0);
        assert!(back.orientation.angle_between(held) < 1e-3);
    }
}
