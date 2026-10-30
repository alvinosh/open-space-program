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

use crate::propulsion::{Propulsion, RcsBlock};

/// Opposite nozzles cancel only to rounding error. A net force this small, relative to the
/// thrust firing, is no force, so that turning does not end exact coasting.
const CANCELLED: f64 = 1e-9;

/// Fire the RCS of every stage for `dt` seconds, each block on the tanks that feed it.
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
    let floor = crate::propulsion::floor(&propulsion.sections);
    for k in 0..propulsion.rcs.len() {
        let block = &propulsion.rcs[k];
        let Some(section) = propulsion.sections.iter().find(|s| s.id == block.section) else {
            continue;
        };
        // The blocks sit around the top of the hull.
        let height = section.bottom.z - floor + section.hull_length() - center_of_mass;
        {
            let (part, mount) = (block.part, block.mount);
            let throttles = throttles(mount, rotation, translation);
            let throttle: f64 = throttles.iter().sum();
            let mass = block.nozzle_mass_flow() * throttle * dt;
            let thrust = block.nozzle_thrust(pressure);
            let mixture: Vec<(String, f64)> = block
                .def
                .propellants
                .iter()
                .map(|(name, ratio)| (name.clone(), mass * ratio))
                .collect();
            let need: Vec<(&str, f64)> = mixture.iter().map(|(n, kg)| (n.as_str(), *kg)).collect();
            let fired = throttle > 0.0 && propulsion.draw(part, &need);
            let block = &mut propulsion.rcs[k];
            block.firing = if fired { throttles } else { [0.0; 4] };
            if !fired {
                continue;
            }
            let (f, t) = push(mount, height, throttles);
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

/// Stop every nozzle: the RCS is off, or the vessel is on rails.
pub fn idle(propulsion: &mut Propulsion) {
    for block in &mut propulsion.rcs {
        block.firing = [0.0; 4];
    }
}

/// The largest torque the RCS can produce about each vessel axis right now, N·m, from
/// blocks that have propellant left.
pub fn torque_authority(propulsion: &Propulsion, center_of_mass: f64, pressure: f64) -> DVec3 {
    let mut authority = DVec3::ZERO;
    for (base, stage) in crate::propulsion::placed(&propulsion.sections) {
        // The blocks sit around the top of the hull.
        let height = base + stage.hull_length() - center_of_mass;
        let blocks = propulsion.rcs_of(stage.id);
        for block in blocks.filter(|b| has_propellant(propulsion, b)) {
            let about = |axis: DVec3| {
                let (_, torque) = push(
                    block.mount,
                    height,
                    throttles(block.mount, axis, DVec3::ZERO),
                );
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

/// Whether the tanks feeding `block` still hold some of each of its propellants.
fn has_propellant(propulsion: &Propulsion, block: &RcsBlock) -> bool {
    block
        .def
        .propellants
        .keys()
        .all(|name| propulsion.reachable(block.part, name) > 0.0)
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
        let com = mass_properties(&p).center_of_mass.z;
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
        let id = p.sections[0].id;
        let before = p.propellant_mass(id);
        let (force, torque) = fire(&mut p, com, DVec3::ZERO, DVec3::Z, 0.0, 1.0);
        // Every block's forward nozzle, at full thrust.
        let blocks: Vec<_> = p.rcs_of(id).collect();
        let thrust = blocks.len() as f64 * blocks[0].def.thrust;
        assert!(
            (force - DVec3::Z * thrust).length() < 1e-9 * thrust,
            "{force}"
        );
        assert!(torque.length() < 1e-9 * thrust, "{torque}");
        assert!(blocks.iter().all(|b| b.firing == [1.0, 0.0, 0.0, 0.0]));
        let used = before - p.propellant_mass(id);
        let expected = blocks.len() as f64 * blocks[0].nozzle_mass_flow();
        assert!(
            (used - expected).abs() < 1e-9 * expected,
            "{used} {expected}"
        );
        idle(&mut p);
        assert!(p.rcs_of(id).all(|b| b.firing == [0.0; 4]));
    }

    #[test]
    fn nothing_without_propellant() {
        let (mut p, com) = upper();
        let id = p.sections[0].id;
        let rcs_propellants: Vec<_> = p.rcs[0].def.propellants.keys().cloned().collect();
        for tank in p.tanks.iter_mut().filter(|t| t.section == id) {
            for (amount, content) in tank.amounts.iter_mut().zip(&tank.contents) {
                if rcs_propellants.contains(&content.resource) {
                    *amount = 0.0;
                }
            }
        }
        assert_eq!(p.rcs_fraction(id), Some(0.0));
        assert_eq!(p.fuel_fraction(id), Some(1.0));
        assert_eq!(torque_authority(&p, com, 0.0), DVec3::ZERO);
        let fired = fire(&mut p, com, DVec3::X, DVec3::Z, 0.0, 0.02);
        assert_eq!(fired, (DVec3::ZERO, DVec3::ZERO));
    }
}
