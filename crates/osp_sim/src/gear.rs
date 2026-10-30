//! Landing gear: sprung, damped struts with wheels that roll along the ground.
//!
//! Each leg's strut stands out from the hull ([`GearDef`]) and its wheel touches the ground
//! below its axle. The strut is a spring, stiff enough that the load it is built for sits
//! it a third of the way into its travel, damped a little under critical, and much stiffer
//! once it bottoms out. The ground pushes up along its normal; along the ground the wheel
//! rolls the way it points (rolling resistance, plus brakes) and grips sideways like a
//! tyre, both up to the friction the load allows, smoothed over a slip speed so that
//! rolling to a stop does not chatter.
//!
//! Anything else that touches the ground (the hull, a wing tip: see [`skid`]) skids on
//! it: the ground stops it going in, stiffly, and drags on it as it slides.
//!
//! The contact is stiff, so a vessel near the ground on its gear is integrated in short
//! substeps (see `Vessel::step`).

use glam::{DQuat, DVec3};

use crate::{
    attitude::Attitude, body::Body, constants::G0, data::GearDef, orbit::StateVector,
    propulsion::Section, time::SimTime,
};

/// Rolling resistance of a tyre on paving.
const ROLLING: f64 = 0.02;
/// Braking friction of a tyre on dry paving.
const BRAKING: f64 = 0.6;
/// Sideways grip of a tyre.
const GRIP: f64 = 0.8;
/// Speeds over which rolling and sideways friction build up to their full value, m/s.
const ROLL_SLIP: f64 = 0.1;
const SIDE_SLIP: f64 = 0.3;
/// Strut damping over critical.
const DAMPING_RATIO: f64 = 0.6;
/// How much stiffer the strut is once it has bottomed out.
const BOTTOMED: f64 = 20.0;
/// A strut pointing less steeply down than this (cosine) cannot touch the ground.
const MIN_DOWNWARDS: f64 = 0.2;
/// Under a skidding point, the whole vessel's weight would sink the ground this far, m.
const SKID_SINK: f64 = 0.05;
/// Friction of the hull sliding on the ground, and the speed over which it builds up, m/s.
const SKID_FRICTION: f64 = 0.5;
const SKID_SLIP: f64 = 0.5;

/// One gear leg, sized from its definition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Leg {
    /// Where the strut meets the hull, in stack coordinates (X to the pilot's right, Y to
    /// their feet, Z up the stack from its base), m.
    pub mount: DVec3,
    /// Along the strut, out from the hull.
    pub direction: DVec3,
    /// Hull to axle, fully extended, m.
    pub length: f64,
    pub wheel_radius: f64,
    /// Strut travel, m.
    pub stroke: f64,
    /// N/m and N·s/m.
    pub stiffness: f64,
    pub damping: f64,
    pub brakes: bool,
    /// Steering range each way, rad.
    pub steering: f64,
    /// kg.
    pub mass: f64,
}

impl Leg {
    /// The legs of `def` on a stage `hull_radius` wide whose base is `base` up the stack.
    pub fn of(def: &GearDef, hull_radius: f64, base: f64) -> Vec<Leg> {
        let (s, c) = def.angle.to_radians().sin_cos();
        let direction = DVec3::new(c, s, 0.0);
        let stroke = def.stroke.max(1e-3);
        let stiffness = def.load * G0 / (stroke / 3.0);
        let count = if def.mirror { 2 } else { 1 };
        let leg = Leg {
            mount: direction * hull_radius + DVec3::Z * (base + def.position),
            direction,
            length: def.length,
            wheel_radius: def.wheel_radius,
            stroke,
            stiffness,
            damping: 2.0 * DAMPING_RATIO * (stiffness * def.load).sqrt(),
            brakes: def.brakes,
            steering: def.steering.to_radians(),
            mass: def.mass / count as f64,
        };
        let mut legs = vec![leg];
        if def.mirror {
            let flip = |v: DVec3| DVec3::new(-v.x, v.y, v.z);
            legs.push(Leg {
                mount: flip(leg.mount),
                direction: flip(leg.direction),
                ..leg
            });
        }
        legs
    }

    /// The axle with the strut `compression` into its travel, stack coordinates.
    pub fn axle(&self, compression: f64) -> DVec3 {
        self.mount + self.direction * (self.length - compression)
    }

    /// Drag area (C_D · A) of the wheel and strut hanging in the airflow, m².
    pub fn drag_area(&self) -> f64 {
        let r = self.wheel_radius;
        0.6 * 2.0 * r * 0.7 * r + 0.08 * self.length
    }
}

/// Every gear leg of a stack of stages, bottom first.
pub fn legs(stages: &[Section]) -> Vec<Leg> {
    let mut legs = Vec::new();
    for (base, stage) in crate::propulsion::placed(stages) {
        for def in &stage.gear {
            legs.extend(Leg::of(def, stage.diameter / 2.0, base));
        }
    }
    legs
}

/// How far below the axis the vessel's lowest wheel reaches when it stands level on its
/// gear, each strut a third of the way into its travel: the axis's height above level
/// ground, m. None without a leg that reaches down.
pub fn rest_height(legs: &[Leg]) -> Option<f64> {
    legs.iter()
        .filter(|l| l.direction.y > MIN_DOWNWARDS)
        .map(|l| l.axle(l.stroke / 3.0).y + l.wheel_radius)
        .reduce(f64::max)
}

/// What the ground does to the vessel through its wheels.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Contact {
    /// Inertial axes, N.
    pub force: DVec3,
    /// About the centre of mass, vessel axes, N·m.
    pub torque: DVec3,
    /// How far into its travel each strut is, m (0 off the ground).
    pub compression: Vec<f64>,
}

impl Contact {
    pub fn touching(&self) -> bool {
        self.compression.iter().any(|&c| c > 0.0)
    }
}

/// The ground's push on each wheel of a vessel at `state` (relative to `body`, at `time`),
/// turned to `attitude`, its centre of mass at `center_of_mass` from the middle of its base
/// (see [`crate::attitude::MassProperties`]). `brakes` brakes
/// the wheels that have them; `steer` (−1..1, right positive) turns the ones that steer.
#[allow(clippy::too_many_arguments)]
pub fn contact(
    legs: &[Leg],
    body: &Body,
    time: SimTime,
    state: &StateVector,
    attitude: &Attitude,
    center_of_mass: DVec3,
    brakes: bool,
    steer: f64,
) -> Contact {
    let mut contact = Contact {
        compression: vec![0.0; legs.len()],
        ..Default::default()
    };
    let orientation = attitude.orientation;
    let spin = orientation * attitude.angular_velocity;
    let to_fixed = body.orientation(time).inverse();
    let mut torque = DVec3::ZERO;
    for (i, leg) in legs.iter().enumerate() {
        let to_world = |p: DVec3| state.position + orientation * (p - center_of_mass);
        let extended = to_world(leg.axle(0.0));
        let up = extended.normalize();
        let strut = orientation * leg.direction;
        let downwards = -strut.dot(up);
        if downwards < MIN_DOWNWARDS {
            continue;
        }
        let ground = body.radius + body.terrain_height(to_fixed * up);
        let compression = (ground - (extended.length() - leg.wheel_radius)) / downwards;
        if compression <= 0.0 {
            continue;
        }
        contact.compression[i] = compression;
        let touch = to_world(leg.axle(compression)) - up * leg.wheel_radius;
        let velocity =
            state.velocity + spin.cross(touch - state.position) - body.surface_velocity(touch);
        let rate = -velocity.dot(up) / downwards;
        let bottomed = (compression - leg.stroke).max(0.0);
        let spring = leg.stiffness * (compression + BOTTOMED * bottomed);
        let normal = (spring + leg.damping * rate).max(0.0);

        // The way the wheel rolls: the nose, turned about the strut by the steering.
        let turn = DQuat::from_axis_angle(leg.direction, leg.steering * steer.clamp(-1.0, 1.0));
        let forward = (orientation * (turn * DVec3::Z))
            .reject_from(up)
            .normalize_or_zero();
        let side = up.cross(forward);
        let braking = if brakes && leg.brakes { BRAKING } else { 0.0 };
        let roll = -(ROLLING + braking) * normal * (velocity.dot(forward) / ROLL_SLIP).tanh();
        let grip = -GRIP * normal * (velocity.dot(side) / SIDE_SLIP).tanh();
        let force = up * normal + forward * roll + side * grip;
        contact.force += force;
        torque += (touch - state.position).cross(force);
    }
    contact.torque = orientation.inverse() * torque;
    contact
}

/// Add the ground's push on `points` (inertial positions of hull points and wing tips) of a
/// vessel of `mass` at `state`, turned to `attitude`, to `contact`: stiff, damped, with
/// sliding friction. Returns the fastest any of them is moving into the ground, m/s.
pub fn skid(
    points: &[DVec3],
    body: &Body,
    time: SimTime,
    state: &StateVector,
    attitude: &Attitude,
    mass: f64,
    contact: &mut Contact,
) -> f64 {
    let stiffness = mass * G0 / SKID_SINK;
    let damping = 2.0 * DAMPING_RATIO * (stiffness * mass).sqrt();
    let spin = attitude.orientation * attitude.angular_velocity;
    let to_fixed = body.orientation(time).inverse();
    let (mut torque, mut fastest) = (DVec3::ZERO, 0.0f64);
    for &point in points {
        let up = point.normalize();
        let depth = body.radius + body.terrain_height(to_fixed * up) - point.length();
        if depth <= 0.0 {
            continue;
        }
        let velocity =
            state.velocity + spin.cross(point - state.position) - body.surface_velocity(point);
        let into = -velocity.dot(up);
        fastest = fastest.max(into);
        let normal = (stiffness * depth + damping * into).max(0.0);
        let sliding = velocity.reject_from(up);
        let friction = -sliding.normalize_or_zero()
            * SKID_FRICTION
            * normal
            * (sliding.length() / SKID_SLIP).tanh();
        let force = up * normal + friction;
        contact.force += force;
        torque += (point - state.position).cross(force);
    }
    contact.torque += attitude.orientation.inverse() * torque;
    fastest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leg(angle: f64, mirror: bool) -> GearDef {
        GearDef {
            name: "Main".into(),
            position: 2.0,
            angle,
            length: 1.0,
            wheel_radius: 0.3,
            stroke: 0.3,
            load: 1000.0,
            mass: 40.0,
            mirror,
            brakes: true,
            steering: 0.0,
        }
    }

    #[test]
    fn legs_stand_out_from_the_hull_and_mirror() {
        let legs = Leg::of(&leg(60.0, true), 0.5, 10.0);
        assert_eq!(legs.len(), 2);
        let (right, left) = (legs[0], legs[1]);
        assert!(right.mount.x > 0.0 && left.mount.x < 0.0);
        assert!((right.mount.y - left.mount.y).abs() < 1e-12);
        assert!((right.mount.z - 12.0).abs() < 1e-12);
        assert!((right.direction.length() - 1.0).abs() < 1e-12 && right.direction.y > 0.8);
        // Sprung so that its load sits it a third into its travel.
        assert!((right.stiffness * 0.1 - 1000.0 * G0).abs() < 1e-6);
        assert_eq!(right.mass, 20.0);
    }

    #[test]
    fn standing_height_is_the_lowest_wheel() {
        let legs = Leg::of(&leg(90.0, false), 0.5, 0.0);
        // Hull 0.5, strut 1.0 less 0.1 settled, wheel 0.3.
        assert!((rest_height(&legs).unwrap() - 1.7).abs() < 1e-12);
        assert_eq!(rest_height(&Leg::of(&leg(0.0, false), 0.5, 0.0)), None);
    }
}
