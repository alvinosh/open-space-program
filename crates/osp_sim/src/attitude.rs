//! Rotational dynamics: the stack's mass properties, control torque from engine gimbal and
//! reaction wheels (RCS is in [`crate::rcs`]), and the SAS autopilot.
//!
//! A vessel is a rigid body whose inertia tensor is summed from its parts ([`crate::mass`]).
//! Torque-free rotation (a spinning stage in orbit, a tumbling plane) is solved exactly for
//! any body, symmetric or not ([`coast`]), so like a coasting trajectory it is evaluated
//! analytically at any time and time warp cannot change it. Under torque, Euler's equations
//! are integrated on the physics tick ([`integrate`]).
//!
//! Vessel axes: +Z is the nose, +X the pilot's right and +Y the pilot's feet, so a positive
//! rotation about +X pitches the nose up, about +Y yaws right and about +Z rolls right.
//! Rotation commands use the same axes, −1..1 each.

use std::{array, f64::consts::PI};

use glam::{DMat3, DQuat, DVec2, DVec3};

use crate::{
    body::Body,
    data::TankShape,
    elliptic,
    mass::{self, Mass},
    orbit::StateVector,
    parts::PartKind,
    propulsion::{Engine, Propulsion, Stage},
    structure::Tank,
};

/// The inertia tensor about the centre of mass, in vessel axes, kg·m². A rocket's is
/// diagonal, with equal pitch and yaw moments; a tail fin or anything else to one side adds
/// products of inertia, and the principal axes ([`Self::principal`]) tilt away from the
/// vessel's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inertia {
    pub tensor: DMat3,
}

impl Default for Inertia {
    fn default() -> Self {
        Self {
            tensor: DMat3::ZERO,
        }
    }
}

impl Inertia {
    /// Principal axes along the vessel's, with these moments about pitch (X), yaw (Y) and
    /// roll (Z).
    pub fn about_axes(pitch: f64, yaw: f64, roll: f64) -> Self {
        Self {
            tensor: DMat3::from_diagonal(DVec3::new(pitch, yaw, roll)),
        }
    }

    /// The moments about the pitch, yaw and roll axes.
    pub fn diagonal(&self) -> DVec3 {
        DVec3::new(
            self.tensor.x_axis.x,
            self.tensor.y_axis.y,
            self.tensor.z_axis.z,
        )
    }

    /// The principal moments, smallest first, and the body frame: the rotation from the
    /// principal axes to vessel axes.
    pub fn principal(&self) -> (DVec3, DQuat) {
        mass::principal_axes(&self.tensor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MassProperties {
    pub mass: f64,
    /// Height of the centre of mass above the base of the stack, m.
    pub center_of_mass: f64,
    pub inertia: Inertia,
}

/// Mass distribution of the vessel: its parts' dry mass
/// ([`crate::parts::PartTree::dry_mass`]) and the propellant in its tanks. Tanks drain
/// together, so each holds its share of what is left in its stage: a liquid as a solid
/// column settled at the tank's aft end, where thrust pushes it, and a compressed gas
/// filling the tank. The centre of mass therefore moves as the tanks drain. It is kept on
/// the axis: a part off to one side (a tail fin) counts towards its height and the inertia
/// about it, not towards where it is sideways.
pub fn mass_properties(propulsion: &Propulsion) -> MassProperties {
    let parts = propulsion.parts();
    let propellant: Mass = propulsion
        .stages
        .iter()
        .flat_map(|stage| {
            parts
                .stage(stage.id)
                .filter_map(move |part| match &part.kind {
                    PartKind::Tank(tank) => {
                        Some(tank_contents(tank, stage).placed(part.position, part.rotation))
                    }
                    _ => None,
                })
        })
        .sum();
    let total = parts.dry_mass() + propellant;
    if total.mass <= 0.0 {
        return MassProperties::default();
    }
    MassProperties {
        mass: total.mass,
        center_of_mass: total.center.z - parts.base(),
        inertia: Inertia {
            tensor: total.inertia_about(DVec3::Z * total.center.z),
        },
    }
}

/// What `tank` holds now, in its own frame, its share of `stage`'s propellant.
fn tank_contents(tank: &Tank, stage: &Stage) -> Mass {
    let (mut liquid, mut full, mut gas) = (0.0, 0.0, 0.0);
    for content in &tank.contents {
        let fill = stage
            .propellants
            .iter()
            .find(|p| p.name == content.resource)
            .map_or(0.0, |p| (p.amount / p.capacity.max(1e-9)).clamp(0.0, 1.0));
        if content.gas {
            gas += content.capacity * fill;
        } else {
            liquid += content.capacity * fill;
            full += content.capacity;
        }
    }
    let (radius, length) = (tank.radius, tank.length);
    let gas = match tank.shape {
        TankShape::Cylinder => Mass::cylinder(gas, radius, length),
        TankShape::Sphere => Mass::sphere(gas, radius).placed(DVec3::Z * radius, DQuat::IDENTITY),
    };
    let column = length * liquid / full.max(1e-9);
    gas + Mass::cylinder(liquid, radius, column)
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

/// Principal moments closer than this share of the largest are taken as equal: the body
/// is symmetric, and the elliptic parameters stay well conditioned.
const SYMMETRIC: f64 = 1e-9;

/// Torque-free rotation for `dt` seconds, exact (Euler–Poinsot). The angular momentum L
/// stays fixed in space and the rotational energy E is kept, so in principal axes L runs
/// round a closed curve (the polhode) about the axis of largest or of smallest moment: the
/// rates are Jacobi elliptic functions of time, and the body turns about L by an angle
/// that is an elliptic integral of the third kind. A symmetric body's rates turn steadily
/// about its axis of symmetry instead. Turning about a principal axis, it keeps turning
/// steadily. Like a coasting orbit it is evaluated for any `dt` without stepping, so time
/// warp cannot change it.
pub fn coast(state: Attitude, inertia: Inertia, dt: f64) -> Attitude {
    let w = state.angular_velocity;
    if dt == 0.0 || w == DVec3::ZERO {
        return state;
    }
    let steady = Attitude {
        orientation: (state.orientation * DQuat::from_scaled_axis(w * dt)).normalize(),
        angular_velocity: w,
    };
    let (mut moments, axes) = inertia.principal();
    let close = SYMMETRIC * moments.z;
    if moments.y - moments.x <= close {
        (moments.x, moments.y) = ((moments.x + moments.y) / 2.0, (moments.x + moments.y) / 2.0);
    }
    if moments.z - moments.y <= close {
        (moments.y, moments.z) = ((moments.y + moments.z) / 2.0, (moments.y + moments.z) / 2.0);
    }
    let principal = DMat3::from_quat(axes);
    if moments.x <= 0.0 {
        return steady;
    }
    // A symmetric body turns steadily about L and at the same time about its axis of
    // symmetry, at a rate set by its spin about that axis: in closed form, exact however
    // slight the spin. (A rocket pitching with a whisker of roll would send the elliptic
    // parameters below off to 10¹³ and lose its attitude to rounding.)
    let symmetry = if moments.y == moments.z {
        Some((principal.x_axis, moments.x, moments.y))
    } else if moments.x == moments.y {
        Some((principal.z_axis, moments.z, moments.x))
    } else {
        None
    };
    if let Some((axis, axial, transverse)) = symmetry {
        let spin = w.dot(axis);
        let momentum = transverse * w + (axial - transverse) * spin * axis;
        let about_axis =
            DQuat::from_axis_angle(axis, (transverse - axial) / transverse * spin * dt);
        let about_momentum =
            DQuat::from_scaled_axis(state.orientation * momentum * (dt / transverse));
        return Attitude {
            orientation: (about_momentum * state.orientation * about_axis).normalize(),
            angular_velocity: about_axis.inverse() * w,
        };
    }
    let rates = principal.transpose() * w;
    let l = moments * rates;
    if l.cross(rates).length() <= 1e-12 * l.length() * rates.length() {
        return steady;
    }
    let l2 = l.length_squared();
    // 2E·I − L² = Σ Iᵢωᵢ²(I − Iᵢ), summed that way: spinning nearly about one axis, the
    // difference of the products is lost to rounding, and can even come out the wrong sign.
    let excess = |inertias: DVec3, rates: DVec3, i: f64| {
        (inertias * rates * rates).dot(DVec3::splat(i) - inertias)
    };
    // The polhode runs round axis c, of the largest moment or the smallest; a is the other
    // extreme and b the middle one, in a right-handed frame.
    let around_largest = excess(moments, rates, moments.y) < 0.0;
    let (frame, [ia, ib, ic]) = if around_largest {
        (DMat3::IDENTITY, moments.to_array())
    } else {
        let flipped = DMat3::from_cols(DVec3::Z, -DVec3::Y, DVec3::X);
        (flipped, [moments.z, moments.y, moments.x])
    };
    let body = principal * frame;
    let w0 = body.transpose() * w;
    // 2E·Ic − L² and L² − 2E·Ia.
    let inertias = DVec3::new(ia, ib, ic);
    let (above, below) = (excess(inertias, w0, ic), -excess(inertias, w0, ia));
    // ωa = Aa cn τ, ωb = sb Ab sn τ, ωc = sc Ac dn τ, with τ = τ0 + rate · t. dn stays
    // positive, so ωc keeps its sign, and Euler's equations then fix sb.
    let root = |x: f64| x.max(0.0).sqrt();
    let amplitude = DVec3::new(
        root(above / (ia * (ic - ia))),
        root(above / (ib * (ic - ib))),
        root(below / (ic * (ic - ia))),
    );
    let rate = root((ic - ib) * below / (ia * ib * ic));
    // So close to steady that the polhode is lost to rounding: steady it is.
    if !(amplitude.min_element() > 0.0 && rate > 0.0) {
        return steady;
    }
    let m = ((ib - ia) * above / ((ic - ib) * below)).clamp(0.0, 1.0 - 1e-12);
    let sc = w0.z.signum();
    let sb = sc * (ic - ib).signum();
    let am0 = (sb * w0.y / amplitude.y).atan2(w0.x / amplitude.x);
    // The turn about L: dφ/dt = L/Ia − (L/Ia)(L² − 2E·Ia) / (L² − Ia² ωa²), and the last
    // term integrates to Π(n; am τ) with n = −Ia² Aa² / (Ic² Ac²).
    let across = (ic * amplitude.z).powi(2);
    let n = -(ia * amplitude.x).powi(2) / across;
    let (tau0, pi0) = elliptic::incomplete(am0, m, n);
    let (am, sn, cn, dn) = elliptic::jacobi(tau0 + rate * dt, m);
    let (_, pi1) = elliptic::incomplete(am, m, n);
    let w1 = DVec3::new(
        amplitude.x * cn,
        sb * amplitude.y * sn,
        sc * amplitude.z * dn,
    );
    let turn = l2.sqrt() / ia * (dt - below / (rate * across) * (pi1 - pi0));
    // L's direction in the body goes to L's in space, and the body's axis a, seen across L,
    // turns by `turn` about it.
    let frame_of = |rates: DVec3| {
        let u = (DVec3::new(ia, ib, ic) * rates).normalize();
        let p = (DVec3::X - u * u.x).normalize();
        DMat3::from_cols(u, p, u.cross(p))
    };
    let space = DMat3::from_quat(state.orientation) * body * frame_of(w0);
    let about = DMat3::from_axis_angle(space.x_axis, turn);
    let turned = about * space * frame_of(w1).transpose() * body.transpose();
    let coasted = Attitude {
        orientation: DQuat::from_mat3(&turned).normalize(),
        angular_velocity: body * w1,
    };
    if coasted.orientation.is_finite() && coasted.angular_velocity.is_finite() {
        coasted
    } else {
        steady
    }
}

/// Rotation under a constant torque (vessel axes, N·m) for `dt` seconds.
pub fn integrate(state: Attitude, inertia: Inertia, torque: DVec3, dt: f64) -> Attitude {
    integrate_with(state, inertia, dt, |_| torque)
}

/// Rotation for `dt` seconds under a torque (vessel axes, N·m) that depends on the attitude,
/// like the air's: one RK4 step of Euler's equations, I ω̇ = τ − ω × Iω, and the quaternion
/// kinematics q̇ = ½ q ω.
pub fn integrate_with(
    state: Attitude,
    inertia: Inertia,
    dt: f64,
    torque: impl Fn(Attitude) -> DVec3,
) -> Attitude {
    let i = inertia.tensor + DMat3::from_diagonal(DVec3::splat(1e-9));
    let inverse = i.inverse();
    let accel = |q: DQuat, w: DVec3| {
        let attitude = Attitude {
            orientation: q.normalize(),
            angular_velocity: w,
        };
        inverse * (torque(attitude) - w.cross(i * w))
    };
    let turn = |q: DQuat, w: DVec3| q * DQuat::from_xyzw(w.x, w.y, w.z, 0.0) * 0.5;
    let (q0, w0) = (state.orientation, state.angular_velocity);
    let (k1q, k1w) = (turn(q0, w0), accel(q0, w0));
    let (q1, w1) = (q0 + k1q * (dt / 2.0), w0 + k1w * (dt / 2.0));
    let (k2q, k2w) = (turn(q1, w1), accel(q1, w1));
    let (q2, w2) = (q0 + k2q * (dt / 2.0), w0 + k2w * (dt / 2.0));
    let (k3q, k3w) = (turn(q2, w2), accel(q2, w2));
    let (q3, w3) = (q0 + k3q * dt, w0 + k3w * dt);
    let (k4q, k4w) = (turn(q3, w3), accel(q3, w3));
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

/// Turn rate, rad/s, below which stability assist locks the current attitude (and time
/// warp holds it, see [`crate::vessel::Vessel::enter_rails`]).
pub const CAPTURE_RATE: f64 = 0.005;
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
            position: None,
            diameter: None,
            design_load: None,
            propellants: [("Kerosene".into(), 1.0), ("LqdOxygen".into(), 2.36)].into(),
        });
        StageDef {
            name: "test".into(),
            engines: engines.iter().map(|s| s.to_string()).collect(),
            tanks: tanks.into_iter().collect(),
            fins: None,
            wings: Vec::new(),
            gear: Vec::new(),
            reaction_wheels: Vec::new(),
            rcs: Vec::new(),
            pod: None,
            fairing: None,
            structure_mass: structure,
            diameter: d,
            length: l,
            appearance: Default::default(),
        }
    }

    fn propulsion(stages: Vec<StageDef>) -> Propulsion {
        let craft = CraftDef {
            id: "test".into(),
            name: String::new(),
            stages,
            sequences: None,
        };
        Propulsion::from_craft(&craft, &catalog()).unwrap()
    }

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance * b.abs().max(1.0)
    }

    /// The angle between two orientations, rad, good down to rounding (unlike
    /// `DQuat::angle_between`, which cannot see below about 3e-8).
    fn angle(a: DQuat, b: DQuat) -> f64 {
        2.0 * (a * b.inverse()).xyz().length()
    }

    #[test]
    fn solid_cylinder_inertia() {
        let p = propulsion(vec![stage(&[], 0.0, 1000.0, 2.0, 10.0)]);
        let m = mass_properties(&p);
        assert!(close(m.center_of_mass, 5.0, 1e-12));
        assert!(close(m.inertia.diagonal().z, 1000.0 * 1.0 / 2.0, 1e-12));
        assert!(close(
            m.inertia.diagonal().x,
            1000.0 * (0.25 + 100.0 / 12.0),
            1e-12
        ));
        assert_eq!(m.inertia.diagonal().x, m.inertia.diagonal().y);
    }

    #[test]
    fn stacking_uses_the_parallel_axis_theorem() {
        let one = stage(&[], 0.0, 1000.0, 2.0, 10.0);
        let p = propulsion(vec![one.clone(), one]);
        let m = mass_properties(&p);
        assert!(close(m.center_of_mass, 10.0, 1e-12));
        // Two cylinders, each 5 m from the common centre.
        let each = 1000.0 * (0.25 + 100.0 / 12.0) + 1000.0 * 25.0;
        assert!(close(m.inertia.diagonal().x, 2.0 * each, 1e-12));
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
        let full = mass_properties(&alone);
        drain(&mut alone);
        assert!(mass_properties(&alone).center_of_mass < full.center_of_mass);
        // Under the upper stage, the stack's centre of mass climbs towards it.
        let mut stack =
            Propulsion::from_craft(&crate::vessel::tests::pathfinder(), &catalog()).unwrap();
        let full = mass_properties(&stack);
        drain(&mut stack);
        let low = mass_properties(&stack);
        assert!(
            low.center_of_mass > full.center_of_mass + 5.0,
            "{low:?} {full:?}"
        );
        assert!(low.inertia.diagonal().x < full.inertia.diagonal().x);
    }

    #[test]
    fn propellant_settles_in_its_own_tank() {
        // A 10 m tank at the base of a bare 40 m stage: full, the propellant fills the tank
        // and the centre of mass is the tank's, not halfway up the stage.
        let mut p = propulsion(vec![stage(&[], 10.0, 0.0, 3.7, 40.0)]);
        assert!(close(mass_properties(&p).center_of_mass, 5.0, 1e-9));
        // Half drained, what is left is a 5 m column at the bottom of the tank.
        for prop in &mut p.stages[0].propellants {
            prop.amount *= 0.5;
        }
        let (wall, left) = (p.stages[0].tank_mass, p.stages[0].propellant_mass());
        let expected = (wall * 5.0 + left * 2.5) / (wall + left);
        assert!(close(mass_properties(&p).center_of_mass, expected, 1e-9));
    }

    #[test]
    fn a_compressed_gas_fills_its_bottle() {
        let mut def = stage(&[], 0.0, 0.0, 3.7, 5.0);
        def.tanks.push(TankDef {
            tank_type: "COPV".into(),
            shape: TankShape::Sphere,
            length: 0.0,
            position: Some(1.0),
            diameter: Some(1.35),
            design_load: None,
            propellants: [("Nitrogen".into(), 1.0)].into(),
        });
        let mut p = propulsion(vec![def]);
        // However much is left, the gas is a solid sphere inside the bottle's shell.
        for share in [1.0, 0.3] {
            for prop in &mut p.stages[0].propellants {
                prop.amount = prop.capacity * share;
            }
            let m = mass_properties(&p);
            let (wall, gas) = (p.stages[0].tank_mass, p.stages[0].propellant_mass());
            let r2 = 0.675f64.powi(2);
            assert!(close(m.center_of_mass, 1.675, 1e-9));
            assert!(close(
                m.inertia.diagonal().z,
                (0.4 * gas + 2.0 / 3.0 * wall) * r2,
                1e-9
            ));
        }
    }

    #[test]
    fn a_tail_fin_tilts_the_principal_axes() {
        // A rocket is symmetric about its axis: the principal axes are the vessel's, with
        // the smallest moment about the roll axis.
        let p = Propulsion::from_craft(&crate::vessel::tests::pathfinder(), &catalog()).unwrap();
        let rocket = mass_properties(&p);
        let (moments, axes) = rocket.inertia.principal();
        assert!(close(moments.x, rocket.inertia.diagonal().z, 1e-9));
        assert!((axes * DVec3::X).z.abs() > 1.0 - 1e-12);
        // The Comet's fin stands on top: it couples pitch and roll, so its roll axis tips a
        // little in the plane of symmetry, and nothing couples left and right.
        let craft = crate::vessel::tests::comet();
        let plane = mass_properties(&Propulsion::from_craft(&craft, &catalog()).unwrap());
        let tensor = plane.inertia.tensor;
        assert!(tensor.y_axis.z.abs() > 1.0, "{tensor}");
        assert!(tensor.x_axis.y.abs() < 1e-6 && tensor.x_axis.z.abs() < 1e-6);
        let (_, axes) = plane.inertia.principal();
        let roll = axes * DVec3::X;
        let tilt = roll.angle_between(DVec3::Z * roll.z.signum());
        assert!(tilt > 1e-3 && tilt < 0.1 && roll.x.abs() < 1e-9, "{roll}");
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
        (state, Inertia::about_axes(5.0e6, 5.0e6, 4.0e5))
    }

    #[test]
    fn exact_coasting_matches_integration() {
        let (state, inertia) = spinning();
        let mut numeric = state;
        for _ in 0..(120 * 500) {
            numeric = integrate(numeric, inertia, DVec3::ZERO, 0.002);
        }
        let exact = coast(state, inertia, 120.0);
        assert!(angle(exact.orientation, numeric.orientation) < 1e-11);
        assert!((exact.angular_velocity - numeric.angular_velocity).length() < 1e-12);
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

    /// A lopsided body, its principal axes tilted off the vessel's, and two tumbles: one
    /// mostly about the axis of largest moment, one mostly about the smallest.
    fn tumbling() -> (Inertia, [Attitude; 2]) {
        let axes = DMat3::from_quat(DQuat::from_euler(glam::EulerRot::XYZ, 0.5, -0.2, 0.9));
        let moments = DVec3::new(2.0e6, 5.0e6, 7.0e6);
        let inertia = Inertia {
            tensor: axes * DMat3::from_diagonal(moments) * axes.transpose(),
        };
        let orientation = DQuat::from_euler(glam::EulerRot::XYZ, 0.3, -0.7, 1.1);
        let state = |principal: DVec3| Attitude {
            orientation,
            angular_velocity: axes * principal,
        };
        let states = [
            state(DVec3::new(0.05, -0.08, 0.3)),
            state(DVec3::new(0.4, 0.06, -0.05)),
        ];
        (inertia, states)
    }

    fn momentum(inertia: Inertia, s: Attitude) -> DVec3 {
        s.orientation * (inertia.tensor * s.angular_velocity)
    }

    #[test]
    fn a_lopsided_body_coasts_as_integration_says() {
        let (inertia, states) = tumbling();
        for state in states {
            let mut numeric = state;
            for _ in 0..(120 * 500) {
                numeric = integrate(numeric, inertia, DVec3::ZERO, 0.002);
            }
            let exact = coast(state, inertia, 120.0);
            let off = angle(exact.orientation, numeric.orientation);
            assert!(off < 1e-11, "{off}");
            let rates = (exact.angular_velocity - numeric.angular_velocity).length();
            assert!(rates < 1e-12, "{rates}");
        }
    }

    #[test]
    fn coasting_in_pieces_is_coasting_at_once() {
        let (inertia, states) = tumbling();
        for state in states {
            let pieces = coast(coast(state, inertia, 1_000.3), inertia, 52_345.7);
            let once = coast(state, inertia, 53_346.0);
            assert!(angle(pieces.orientation, once.orientation) < 1e-10);
            assert!((pieces.angular_velocity - once.angular_velocity).length() < 1e-12);
            // A year on, momentum and energy are as they were.
            let later = coast(state, inertia, 3.0e7);
            let (before, after) = (momentum(inertia, state), momentum(inertia, later));
            assert!((before - after).length() < 1e-9 * before.length());
            let energy = |s: Attitude| s.angular_velocity.dot(inertia.tensor * s.angular_velocity);
            assert!(close(energy(later), energy(state), 1e-12));
        }
    }

    #[test]
    fn a_rocket_pitching_with_a_whisker_of_roll_coasts_smoothly() {
        // The Pathfinder near max-Q, pitched over by the pilot: pitch and yaw moments equal,
        // roll 200 times smaller, and a roll rate down at rounding level. Drawn between
        // physics ticks, it once jittered by a tenth of a degree.
        let inertia = Inertia::about_axes(1.288e8, 1.288e8, 6.53e5);
        let state = Attitude {
            orientation: DQuat::from_euler(glam::EulerRot::XYZ, 0.3, -0.7, 1.1),
            angular_velocity: DVec3::new(-0.0446, -0.0002, 3e-8),
        };
        for dt in [0.004f64, 0.012, 0.02, 600.0] {
            let mut numeric = state;
            let steps = (dt / 0.002).ceil();
            for _ in 0..steps as usize {
                numeric = integrate(numeric, inertia, DVec3::ZERO, dt / steps);
            }
            let exact = coast(state, inertia, dt);
            let off = angle(exact.orientation, numeric.orientation);
            assert!(off < 1e-10, "{dt}: {off}");
            let rates = (exact.angular_velocity - numeric.angular_velocity).length();
            assert!(rates < 1e-12, "{dt}: {rates}");
        }
    }

    #[test]
    fn a_nearly_symmetric_body_spinning_about_one_axis_keeps_spinning() {
        // A slender rocket pitching: pitch and yaw moments a hair apart, a whisker of yaw.
        // 2E·Ic − L² is then far below the rounding of either product.
        for apart in [1e-3, 1e-6, 1e-10] {
            let inertia = Inertia::about_axes(2.0e8, 2.0e8 * (1.0 + apart), 8.6e5);
            let state = Attitude {
                orientation: DQuat::IDENTITY,
                angular_velocity: DVec3::new(1e-9, 0.03, 0.0),
            };
            for dt in [0.004, 1.0, 600.0] {
                let later = coast(state, inertia, dt);
                assert!(later.orientation.is_finite() && later.angular_velocity.is_finite());
                let (before, after) = (momentum(inertia, state), momentum(inertia, later));
                assert!(
                    (before - after).length() < 1e-9 * before.length(),
                    "{apart} {dt}"
                );
            }
        }
    }

    #[test]
    fn a_spin_about_the_middle_axis_flips_over() {
        // The Dzhanibekov effect: spun about the axis of middle moment, a body turns itself
        // over again and again; spun about either other axis, it keeps spinning.
        let inertia = Inertia::about_axes(1.0e4, 2.0e4, 3.0e4);
        let spin = |about: DVec3| Attitude {
            orientation: DQuat::IDENTITY,
            angular_velocity: about + DVec3::splat(1e-3),
        };
        let along = |state: Attitude, axis: DVec3, t: f64| {
            coast(state, inertia, t).angular_velocity.dot(axis)
        };
        let middle = spin(DVec3::Y);
        let flipped = (0..400).map(|k| along(middle, DVec3::Y, k as f64 * 0.5));
        assert!(flipped.fold(f64::INFINITY, f64::min) < -0.99);
        for axis in [DVec3::X, DVec3::Z] {
            let steady = (0..400).map(|k| along(spin(axis), axis, k as f64 * 0.5));
            assert!(steady.fold(f64::INFINITY, f64::min) > 0.99);
        }
        // Right through a flip it agrees with step-by-step integration.
        let mut numeric = middle;
        for _ in 0..(60 * 1000) {
            numeric = integrate(numeric, inertia, DVec3::ZERO, 0.001);
        }
        let exact = coast(middle, inertia, 60.0);
        assert!(angle(exact.orientation, numeric.orientation) < 1e-10);
        assert!((exact.angular_velocity - numeric.angular_velocity).length() < 1e-10);
    }

    /// Fly `sas` for `seconds` with angular acceleration `alpha` on every axis.
    fn fly(
        sas: &mut Sas,
        mut state: Attitude,
        target: Option<DVec3>,
        alpha: f64,
        seconds: f64,
    ) -> Attitude {
        let inertia = Inertia::about_axes(1.0, 1.0, 1.0);
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
