//! Landing gear: sprung, damped struts with wheels that roll along the ground.
//!
//! Each leg is a part of the vessel ([`PartKind::Gear`]): its strut runs out along the part's
//! −Z and its wheel rolls along the part's +Y, wherever the part is and however it is turned.
//! The wheel touches the ground below its axle. The strut is a spring, stiff enough that the load it is built for sits
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
    attitude::Attitude,
    body::Body,
    constants::G0,
    data::GearDef,
    orbit::StateVector,
    parts::{PartId, PartKind},
    propulsion::Propulsion,
    time::SimTime,
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
    /// The way the wheel rolls, straight ahead (across the strut).
    pub roll: DVec3,
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
    /// It ends in a foot, which does not roll.
    pub foot: bool,
    /// The part it belongs to, on a vessel.
    pub part: Option<PartId>,
}

impl Leg {
    /// The legs of `def` on a stage `hull_radius` wide whose base is `base` up the stack,
    /// standing out from its side and rolling along the nose.
    pub fn of(def: &GearDef, hull_radius: f64, base: f64) -> Vec<Leg> {
        let (s, c) = def.angle.to_radians().sin_cos();
        let direction = DVec3::new(c, s, 0.0);
        let mount = direction * hull_radius + DVec3::Z * (base + def.position);
        Self::placed(def, mount, direction, DVec3::Z)
    }

    /// The legs of `def` on a part at `mount` (stack coordinates) turned by `rotation`: the
    /// strut runs out along its −Z and the wheel rolls along its +Y.
    pub fn at(def: &GearDef, mount: DVec3, rotation: DQuat) -> Vec<Leg> {
        Self::placed(def, mount, rotation * DVec3::NEG_Z, rotation * DVec3::Y)
    }

    /// `def` as a leg at `mount`, and its mirror image across the vessel's plane of symmetry
    /// if it has one.
    fn placed(def: &GearDef, mount: DVec3, direction: DVec3, roll: DVec3) -> Vec<Leg> {
        let stroke = def.stroke.max(1e-3);
        let stiffness = def.load * G0 / (stroke / 3.0);
        let count = if def.mirror { 2 } else { 1 };
        let leg = Leg {
            mount,
            direction,
            roll,
            length: def.length,
            wheel_radius: def.wheel_radius,
            stroke,
            stiffness,
            damping: 2.0 * DAMPING_RATIO * (stiffness * def.load).sqrt(),
            brakes: def.brakes,
            steering: def.steering.to_radians(),
            foot: def.foot,
            mass: def.mass / count as f64,
            part: None,
        };
        let mut legs = vec![leg];
        if def.mirror {
            let flip = |v: DVec3| DVec3::new(-v.x, v.y, v.z);
            legs.push(Leg {
                mount: flip(leg.mount),
                direction: flip(leg.direction),
                roll: flip(leg.roll),
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

    /// The way the wheel rolls in the world when the vessel is turned to `orientation` and
    /// steered by `steer` (−1..1), along the ground whose normal is `up`; zero if the wheel
    /// would roll straight into or out of the ground.
    fn heading(&self, orientation: DQuat, steer: f64, up: DVec3) -> DVec3 {
        let turn = DQuat::from_axis_angle(self.direction, self.steering * steer.clamp(-1.0, 1.0));
        (orientation * (turn * self.roll))
            .reject_from(up)
            .normalize_or_zero()
    }
}

/// Every gear leg of a vessel, in the order of its parts. A stage hanging beside the
/// vessel's axis (a dolly under the belly) carries its legs out with it, and a leg that is
/// let go with its part is gone.
pub fn legs(propulsion: &Propulsion) -> Vec<Leg> {
    let tree = propulsion.parts();
    let base = DVec3::Z * tree.base();
    let mut legs = Vec::new();
    for part in tree.parts() {
        for kind in &part.kinds {
            if let PartKind::Gear(def) = kind {
                legs.extend(
                    Leg::at(def, part.position - base, part.rotation)
                        .into_iter()
                        .map(|leg| Leg {
                            part: Some(part.id),
                            ..leg
                        }),
                );
            }
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

        // The way the wheel rolls: where it points, turned about the strut by the steering.
        let forward = leg.heading(orientation, steer, up);
        let side = up.cross(forward);
        let braking = if brakes && leg.brakes { BRAKING } else { 0.0 };
        // A foot does not roll: it grips the same whichever way it is pushed.
        let along = if leg.foot { GRIP } else { ROLLING + braking };
        let roll = -along * normal * (velocity.dot(forward) / ROLL_SLIP).tanh();
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
    use crate::{data::CraftDef, test_data::records, vessel::tests::catalog};

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
            foot: false,
        }
    }

    #[test]
    fn a_foot_grips_the_way_a_wheel_would_roll() {
        let system = crate::system::tests::rss();
        let moon = system.body(system.find("Moon").unwrap());
        let time = SimTime::J2000;
        let up = moon.orientation(time) * DVec3::X;
        // A leg straight down, on the ground, sliding the way its wheel would roll.
        let leg = |foot: bool| {
            let mut def = leg(0.0, false);
            def.foot = foot;
            let turn = DQuat::from_rotation_arc(DVec3::NEG_Z, DVec3::NEG_Z);
            Leg::at(&def, DVec3::ZERO, turn)[0]
        };
        let orientation = DQuat::from_rotation_arc(DVec3::Z, up);
        let attitude = Attitude {
            orientation,
            angular_velocity: DVec3::ZERO,
        };
        let sliding = orientation * DVec3::Y;
        let state = StateVector {
            position: up * (moon.radius + 1.2),
            velocity: moon.surface_velocity(up * moon.radius) + sliding,
        };
        let drag = |foot: bool| {
            let contact = contact(
                &[leg(foot)],
                moon,
                time,
                &state,
                &attitude,
                DVec3::ZERO,
                false,
                0.0,
            );
            assert!(contact.touching());
            -contact.force.dot(sliding) / contact.force.dot(up)
        };
        assert!(drag(false) < 0.05, "a wheel rolls: {}", drag(false));
        assert!(drag(true) > 0.5, "a foot grips: {}", drag(true));
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

    /// The pose the stage converter gives a leg standing out at `angle` from the hull: its
    /// −Z along the strut, its +Y along the nose.
    fn stage_pose(angle: f64) -> DQuat {
        let out = glam::DVec2::from_angle(angle.to_radians()).extend(0.0);
        DQuat::from_mat3(&glam::DMat3::from_cols(
            DVec3::Z.cross(-out),
            DVec3::Z,
            -out,
        ))
    }

    #[test]
    fn a_leg_at_a_pose_runs_out_along_minus_z_and_rolls_along_y() {
        let def = leg(90.0, false);
        let mount = DVec3::new(1.0, 2.0, 3.0);
        let [leg] = Leg::at(&def, mount, DQuat::IDENTITY).try_into().unwrap();
        assert_eq!(leg.mount, mount);
        assert_eq!(leg.direction, DVec3::NEG_Z);
        assert_eq!(leg.roll, DVec3::Y);
        // Turned a quarter about the strut, the wheel rolls the other way.
        let turned = DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2);
        let [leg] = Leg::at(&def, mount, turned).try_into().unwrap();
        assert!((leg.roll - DVec3::NEG_X).length() < 1e-12);
        assert!((leg.direction - DVec3::NEG_Z).length() < 1e-12);
    }

    #[test]
    fn a_leg_placed_as_a_stage_places_it_is_the_stage_s_leg() {
        for (angle, mirror) in [(60.0, true), (90.0, false), (35.0, true)] {
            let def = leg(angle, mirror);
            let old = Leg::of(&def, 0.5, 10.0);
            let (s, c) = angle.to_radians().sin_cos();
            let mount = DVec3::new(c, s, 0.0) * 0.5 + DVec3::Z * 12.0;
            let new = Leg::at(&def, mount, stage_pose(angle));
            assert_eq!(old.len(), new.len());
            for (old, new) in old.iter().zip(&new) {
                assert!((old.mount - new.mount).length() < 1e-12);
                assert!((old.direction - new.direction).length() < 1e-12);
                assert!((old.roll - new.roll).length() < 1e-12);
                assert_eq!(
                    (old.stiffness, old.damping, old.mass),
                    (new.stiffness, new.damping, new.mass)
                );
            }
        }
    }

    #[test]
    fn a_wheel_rolls_the_way_it_points_turned_by_its_steering() {
        let mut def = leg(90.0, false);
        def.steering = 30.0;
        // On level ground, the vessel upright, a wheel built to roll along X goes along X.
        let sideways = Leg {
            roll: DVec3::X,
            ..Leg::of(&def, 0.5, 0.0)[0]
        };
        let up = DVec3::Y;
        let orientation = DQuat::IDENTITY;
        let straight = sideways.heading(orientation, 0.0, up);
        assert!((straight - DVec3::X).length() < 1e-12);
        // Full steering swings it 30° about the strut (here along +Y, the ground's normal).
        let steered = sideways.heading(orientation, 1.0, up);
        assert!((steered.angle_between(DVec3::X) - 30f64.to_radians()).abs() < 1e-9);
        assert!(steered.dot(up).abs() < 1e-12);
        // Rolling straight into the ground, it has no heading.
        let into = Leg {
            roll: DVec3::Y,
            ..sideways
        };
        assert_eq!(into.heading(orientation, 0.0, up), DVec3::ZERO);
    }

    /// A shell hanging a wheel under its tail as `placement` says.
    fn wheel_under_a_tail(placement: &str) -> Propulsion {
        let text = format!(
            r#"{{ id: "wheel_test", name: "", parts: [
                {{ id: "host", shape: {{ shell: {{ bottom_diameter: 1.0,
                    sections: [{{ length: 3.0 }}], cap_bottom: true, cap_top: true }} }},
                  structure: {{ mass: 120 }}, components: [{{ container: {{}} }}] }},
                {{ id: "wheel", on: "host.bottom", by: "mount", {placement},
                  shape: {{ ring: {{ diameter: 0.12, height: 0.2, thickness: 0.06 }} }},
                  structure: {{ mass: 1 }},
                  nodes: [{{ name: "mount", position: [0, 0, 0], direction: [0, 0, 1], size: 0.1 }}],
                  components: [{{ gear: {{ length: 0.8, load: 300, mass: 10 }} }}] }}
            ] }}"#
        );
        let def: CraftDef = records(&text).remove(0);
        Propulsion::from_craft(&def, &catalog()).unwrap()
    }

    #[test]
    fn a_vessels_legs_are_its_gear_parts_however_they_are_turned() {
        let ahead = wheel_under_a_tail("roll: 0");
        let turned = wheel_under_a_tail("roll: 90");
        for p in [&ahead, &turned] {
            let legs = legs(p);
            let [leg] = legs.as_slice() else {
                panic!("{legs:?}")
            };
            // Out of the tail, down the stack, from the base.
            assert!((leg.mount - DVec3::ZERO).length() < 1e-9, "{}", leg.mount);
            assert!((leg.direction - DVec3::NEG_Z).length() < 1e-9);
            assert!(leg.roll.dot(leg.direction).abs() < 1e-9);
            let part = p.parts().get(leg.part.unwrap()).unwrap();
            assert!(part.is(|k| matches!(k, PartKind::Gear(_))));
        }
        // A quarter turn about the strut turns the way it rolls a quarter.
        let (a, b) = (legs(&ahead)[0].roll, legs(&turned)[0].roll);
        assert!(a.dot(b).abs() < 1e-9 && (a.cross(b).length() - 1.0).abs() < 1e-9);
        // The whole part weighs, not only the leg.
        assert!((turned.mass() - (120.0 + 11.0)).abs() < 1e-9);
    }
}
