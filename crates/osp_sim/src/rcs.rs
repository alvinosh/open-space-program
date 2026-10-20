//! Reaction control thrusters: small fixed nozzles that turn the vessel and push it along,
//! for attitude control without the main engines and to settle propellant (ullage) before
//! an engine start.
//!
//! Each block ([`RcsBlock`]) has four nozzles, pushing forward, back and both ways around
//! the vessel axis, and a stage's blocks sit evenly around its forward rim. Forward and back
//! nozzles pitch and yaw the vessel or push it along its axis; the sideways ones roll it or
//! push it sideways. Each nozzle fires at the sum of what the rotation and translation
//! commands ask of it, clipped to 0..1, so torque and force follow the command until
//! nozzles saturate. A sideways push away from the centre of mass also turns the vessel, as
//! it would for real.

use glam::{DVec2, DVec3};

use crate::propulsion::{Propellant, Propulsion, RcsBlock, Stage};

/// Opposite nozzles cancel only to rounding error. A net force this small, relative to the
/// thrust firing, is no force, so that turning does not end exact coasting.
const CANCELLED: f64 = 1e-9;

/// Fire the RCS of every stage for `dt` seconds, each on its own stage's propellant.
/// `rotation` and `translation` are commands per vessel axis, −1..1 (translation: right,
/// down, forward). Returns the force and the torque about the centre of mass
/// (`center_of_mass` above the base of the stack), both in vessel axes. Blocks that have
/// run out of propellant do nothing.
pub fn fire(
    propulsion: &mut Propulsion,
    center_of_mass: f64,
    rotation: DVec3,
    translation: DVec3,
    pressure: f64,
    dt: f64,
) -> (DVec3, DVec3) {
    let (mut force, mut torque, mut firing) = (DVec3::ZERO, DVec3::ZERO, 0.0);
    let mut base = 0.0;
    for stage in &mut propulsion.stages {
        let height = base + stage.length - center_of_mass;
        base += stage.length;
        for block in &stage.rcs {
            let throttles = throttles(block.mount, rotation, translation);
            let throttle: f64 = throttles.iter().sum();
            let mass = block.nozzle_mass_flow() * throttle * dt;
            if throttle <= 0.0 || !draw(&mut stage.propellants, block, mass) {
                continue;
            }
            let thrust = block.nozzle_thrust(pressure);
            let (f, t) = push(block.mount, height, throttles);
            force += f * thrust;
            torque += t * thrust;
            firing += throttle * thrust;
        }
    }
    if force.length() <= CANCELLED * firing {
        force = DVec3::ZERO;
    }
    (force, torque)
}

/// The largest torque the RCS can produce about each vessel axis right now, N·m, from
/// blocks that have propellant left.
pub fn torque_authority(propulsion: &Propulsion, center_of_mass: f64, pressure: f64) -> DVec3 {
    let mut authority = DVec3::ZERO;
    let mut base = 0.0;
    for stage in &propulsion.stages {
        let height = base + stage.length - center_of_mass;
        base += stage.length;
        for block in stage.rcs.iter().filter(|b| has_propellant(stage, b)) {
            let about = |axis: DVec3| {
                let (_, torque) = push(block.mount, height, throttles(block.mount, axis, DVec3::ZERO));
                torque.dot(axis)
            };
            authority += block.nozzle_thrust(pressure)
                * DVec3::new(about(DVec3::X), about(DVec3::Y), about(DVec3::Z));
        }
    }
    authority
}

/// Direction each nozzle of a block at `mount` pushes the vessel: forward, back, around the
/// axis towards positive roll, and the other way.
fn nozzles(mount: DVec2) -> [DVec3; 4] {
    let around = mount.normalize_or_zero().perp().extend(0.0);
    [DVec3::Z, DVec3::NEG_Z, around, -around]
}

/// How hard each nozzle of a block at `mount` fires for the commands, 0..1, in the order of
/// [`nozzles`].
fn throttles(mount: DVec2, rotation: DVec3, translation: DVec3) -> [f64; 4] {
    let radial = mount.normalize_or_zero();
    // A forward push at `mount` turns the vessel about r × ẑ = (radial.y, −radial.x).
    let axial = translation.z + radial.y * rotation.x - radial.x * rotation.y;
    let around = translation.truncate().dot(radial.perp()) + rotation.z;
    [axial, -axial, around, -around].map(|t| t.clamp(0.0, 1.0))
}

/// Force and torque of a block at `mount`, `height` above the centre of mass, with its
/// nozzles at `throttles`, per newton of nozzle thrust.
fn push(mount: DVec2, height: f64, throttles: [f64; 4]) -> (DVec3, DVec3) {
    let r = mount.extend(height);
    nozzles(mount).into_iter().zip(throttles).fold(
        (DVec3::ZERO, DVec3::ZERO),
        |(force, torque), (direction, throttle)| {
            let f = direction * throttle;
            (force + f, torque + r.cross(f))
        },
    )
}

fn has_propellant(stage: &Stage, block: &RcsBlock) -> bool {
    block
        .def
        .propellants
        .keys()
        .all(|name| stage.propellants.iter().any(|p| &p.name == name && p.amount > 0.0))
}

/// Take `mass` kg of a block's propellants from its stage, if there is enough of each.
fn draw(propellants: &mut [Propellant], block: &RcsBlock, mass: f64) -> bool {
    let enough = block.def.propellants.iter().all(|(name, ratio)| {
        propellants
            .iter()
            .any(|p| &p.name == name && p.amount >= mass * ratio)
    });
    if enough {
        for (name, ratio) in &block.def.propellants {
            if let Some(p) = propellants.iter_mut().find(|p| &p.name == name) {
                p.amount -= mass * ratio;
            }
        }
    }
    enough
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        attitude::mass_properties,
        vessel::tests::{catalog, upper_stage},
    };

    fn upper() -> (Propulsion, f64) {
        let p = Propulsion::from_craft(&upper_stage(), &catalog()).unwrap();
        let com = mass_properties(&p.stages).center_of_mass;
        (p, com)
    }

    #[test]
    fn turning_does_not_push() {
        let (mut p, com) = upper();
        let authority = torque_authority(&p, com, 0.0);
        assert!(authority.min_element() > 0.0, "{authority}");
        for command in [DVec3::X, DVec3::NEG_Y, DVec3::Z, DVec3::new(0.3, -0.5, 0.2)] {
            let (force, torque) = fire(&mut p, com, command, DVec3::ZERO, 0.0, 0.02);
            assert_eq!(force, DVec3::ZERO);
            let expected = command * authority;
            assert!(
                (torque - expected).length() < 1e-9 * expected.length(),
                "{torque} {expected}"
            );
        }
    }

    #[test]
    fn pushing_forward_does_not_turn() {
        let (mut p, com) = upper();
        let before = p.stages[0].propellant_mass();
        let (force, torque) = fire(&mut p, com, DVec3::ZERO, DVec3::Z, 0.0, 1.0);
        // Every block's forward nozzle, at full thrust.
        let blocks = &p.stages[0].rcs;
        let thrust = blocks.len() as f64 * blocks[0].def.thrust;
        assert!((force - DVec3::Z * thrust).length() < 1e-9 * thrust, "{force}");
        assert!(torque.length() < 1e-9 * thrust, "{torque}");
        let used = before - p.stages[0].propellant_mass();
        let expected = blocks.len() as f64 * blocks[0].nozzle_mass_flow();
        assert!((used - expected).abs() < 1e-9 * expected, "{used} {expected}");
    }

    #[test]
    fn nothing_without_propellant() {
        let (mut p, com) = upper();
        let stage = &mut p.stages[0];
        let rcs_propellants: Vec<_> = stage.rcs[0].def.propellants.keys().cloned().collect();
        for prop in &mut stage.propellants {
            if rcs_propellants.contains(&prop.name) {
                prop.amount = 0.0;
            }
        }
        assert_eq!(stage.rcs_fraction(), Some(0.0));
        assert_eq!(stage.fuel_fraction(), Some(1.0));
        assert_eq!(torque_authority(&p, com, 0.0), DVec3::ZERO);
        let fired = fire(&mut p, com, DVec3::X, DVec3::Z, 0.0, 0.02);
        assert_eq!(fired, (DVec3::ZERO, DVec3::ZERO));
    }
}
