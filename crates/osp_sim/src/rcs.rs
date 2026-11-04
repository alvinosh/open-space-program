//! Reaction control thrusters: small fixed nozzles that turn the vessel and push it along,
//! for attitude control without the main engines and to settle propellant (ullage) before
//! an engine start.
//!
//! Each block ([`RcsBlock`]) has four nozzles along its own axes: forward, back and both ways
//! across. A block on a stage's forward rim faces outwards, so its forward and back nozzles
//! pitch and yaw the vessel or push it along its axis, and its sideways ones, which go round
//! the axis, roll it or push it sideways. A block anywhere else, turned any way, has the same
//! four nozzles pushing along its own axes from where it is. Each nozzle fires at the sum of
//! what the rotation and translation commands ask of it, clipped to 0..1, so torque and force
//! follow the command until nozzles saturate. A push away from the centre of mass also turns
//! the vessel, as it would for real.

use glam::DVec3;

use crate::propulsion::{Propulsion, RcsBlock};

/// A block this close to the centre of mass's level along its own X, m, has no lever to turn
/// the vessel with.
const LEVEL: f64 = 1e-9;

/// Opposite nozzles cancel only to rounding error. A net force this small, relative to the
/// thrust firing, is no force, so that turning does not end exact coasting.
const CANCELLED: f64 = 1e-9;

/// What the blocks with propellant give pushing along the nose, N: one nozzle each.
pub fn forward_thrust(propulsion: &Propulsion, pressure: f64) -> f64 {
    (propulsion.rcs.iter())
        .filter(|block| !block.shut && propulsion.flown(block.section))
        .filter(|block| {
            propulsion
                .rcs_fraction(block.section)
                .is_some_and(|f| f > 0.0)
        })
        .map(|block| {
            let along = (block.rotation * DVec3::Z).z.max(0.0);
            block.nozzle_thrust(pressure) * along
        })
        .sum()
}

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
    let base = propulsion.parts().base();
    for k in 0..propulsion.rcs.len() {
        let block = &propulsion.rcs[k];
        // A vessel that has joined this one keeps its thrusters to itself.
        if block.shut || !propulsion.flown(block.section) {
            propulsion.rcs[k].firing = [0.0; 4];
            continue;
        }
        let (part, lever) = (block.part, lever(block, base, center_of_mass));
        let throttles = throttles(block, lever, rotation, translation);
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
        let (f, t) = push(block, lever, throttles);
        let fired = throttle > 0.0 && propulsion.draw(part, &need);
        let block = &mut propulsion.rcs[k];
        block.firing = if fired { throttles } else { [0.0; 4] };
        if !fired {
            continue;
        }
        force += f * thrust;
        torque += t * thrust;
        firing += throttle * thrust;
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
    let base = propulsion.parts().base();
    let mut authority = DVec3::ZERO;
    for block in propulsion
        .rcs
        .iter()
        .filter(|b| !b.shut && propulsion.flown(b.section) && has_propellant(propulsion, b))
    {
        let lever = lever(block, base, center_of_mass);
        let about = |axis: DVec3| {
            let (_, torque) = push(block, lever, throttles(block, lever, axis, DVec3::ZERO));
            torque.dot(axis)
        };
        authority += block.nozzle_thrust(pressure)
            * DVec3::new(about(DVec3::X), about(DVec3::Y), about(DVec3::Z));
    }
    authority
}

/// Where `block` is from the centre of mass, `center_of_mass` above the base of the stack
/// (`base` up the assembly frame), in vessel axes, m.
fn lever(block: &RcsBlock, base: f64, center_of_mass: f64) -> DVec3 {
    block.mount - DVec3::Z * (base + center_of_mass)
}

/// Where `axis` (a unit axis of the block's own frame) points in vessel axes. A block turned
/// about the vessel's axis leaves its own +Z a few 1e-17 off it, which would make a forward
/// push fire the sideways nozzles a hair: components that close to 0 or ±1 are exactly that.
fn pointing(block: &RcsBlock, axis: DVec3) -> DVec3 {
    let snap = |c: f64| match c {
        c if c.abs() < 1e-12 => 0.0,
        c if (c.abs() - 1.0).abs() < 1e-12 => c.signum(),
        c => c,
    };
    let v = block.rotation * axis;
    DVec3::new(snap(v.x), snap(v.y), snap(v.z))
}

/// Direction each nozzle of `block` pushes the vessel: along its +Z, its −Z, its +Y and its −Y.
fn nozzles(block: &RcsBlock) -> [DVec3; 4] {
    let (ahead, across) = (pointing(block, DVec3::Z), pointing(block, DVec3::Y));
    [ahead, -ahead, across, -across]
}

/// How hard each nozzle of `block` fires for the commands, 0..1, in the order of [`nozzles`].
///
/// The pairing is the one for a block on a stage's rim, where it faces out (its +X away from
/// the axis): a push of its forward nozzle then turns the vessel about the block's −Y, and a
/// sideways push about its +Z; the nozzles that push the way the commands ask fire, and the
/// opposite ones do not. That is pitch and yaw from the forward and back nozzles and roll from
/// the sideways ones. A block whose centre of mass lies the other way along its X has the
/// pairing reversed (`lever` is where it is from the centre of mass), and one level with it
/// turns nothing. This is only a rule for which nozzles serve which command: the force and
/// the torque of a block anywhere are exact, but a block placed so that its nozzles serve other
/// axes than these gets less out of a command than it could.
fn throttles(block: &RcsBlock, lever: DVec3, rotation: DVec3, translation: DVec3) -> [f64; 4] {
    let (ahead, across) = (pointing(block, DVec3::Z), pointing(block, DVec3::Y));
    let out = lever.dot(pointing(block, DVec3::X));
    let sense = if out.abs() < LEVEL { 0.0 } else { out.signum() };
    let axial = translation.dot(ahead) - sense * rotation.dot(across);
    let sideways = translation.dot(across) + sense * rotation.dot(ahead);
    [axial, -axial, sideways, -sideways].map(|t| t.clamp(0.0, 1.0))
}

/// Force and torque of `block` at `lever` from the centre of mass, with its nozzles at
/// `throttles`, per newton of nozzle thrust.
fn push(block: &RcsBlock, lever: DVec3, throttles: [f64; 4]) -> (DVec3, DVec3) {
    nozzles(block).into_iter().zip(throttles).fold(
        (DVec3::ZERO, DVec3::ZERO),
        |(force, torque), (direction, throttle)| {
            let f = direction * throttle;
            (force + f, torque + lever.cross(f))
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

    /// The stage converter turns a block on the rim to face outwards (its +X away from the
    /// axis), so its sideways nozzles go round the axis: the convention the rest relies on.
    #[test]
    fn rim_blocks_face_outwards() {
        let (p, _) = upper();
        assert!(p.rcs.len() >= 4);
        for block in &p.rcs {
            let out = block.rotation * DVec3::X;
            let radial = block.mount.truncate().extend(0.0).normalize();
            assert!((out - radial).length() < 1e-9, "{out} {radial}");
            let around = block.rotation * DVec3::Y;
            assert!(
                (around - DVec3::Z.cross(radial)).length() < 1e-9,
                "{around}"
            );
        }
    }

    /// A copy of the upper stage's first block, moved and turned.
    fn moved(p: &Propulsion, mount: DVec3, rotation: glam::DQuat) -> Propulsion {
        let mut p = p.clone();
        p.rcs.truncate(1);
        p.rcs[0].mount = mount;
        p.rcs[0].rotation = rotation;
        p
    }

    /// A block turned so that its forward nozzle pushes sideways (along +X) pushes the vessel
    /// that way, and only that way: its sideways nozzles go along Z and Y, so with the lever
    /// it makes the torque the lever and the force give.
    #[test]
    fn a_block_turned_on_its_side_pushes_along_its_own_axes() {
        let (p, com) = upper();
        let base = p.parts().base();
        let rotation = glam::DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2);
        // 1 m out along X, 3 m above the centre of mass.
        let at = DVec3::new(1.0, 0.0, base + com + 3.0);
        let mut p = moved(&p, at, rotation);
        let thrust = p.rcs[0].def.thrust;
        // The block's +Z is the vessel's +X: a push along X.
        let (force, torque) = fire(&mut p, com, DVec3::ZERO, DVec3::X, 0.0, 0.02);
        assert!(
            (force - DVec3::X * thrust).length() < 1e-9 * thrust,
            "{force}"
        );
        // About the centre of mass: r × F = (1, 0, 3) × (T, 0, 0) = (0, 3T, 0).
        assert!(
            (torque - DVec3::Y * 3.0 * thrust).length() < 1e-9 * thrust,
            "{torque}"
        );
        assert_eq!(p.rcs[0].firing, [1.0, 0.0, 0.0, 0.0]);
        // Its sideways nozzles are the block's ±Y, the vessel's ±Y here: a push along Y, from
        // the same lever: r × F = (1, 0, 3) × (0, T, 0) = (−3T, 0, T), turning it about X and,
        // as the block is 1 m off the axis, about Z.
        let (force, torque) = fire(&mut p, com, DVec3::ZERO, DVec3::Y, 0.0, 0.02);
        assert!(
            (force - DVec3::Y * thrust).length() < 1e-9 * thrust,
            "{force}"
        );
        let expected = DVec3::new(-3.0, 0.0, 1.0) * thrust;
        assert!((torque - expected).length() < 1e-9 * thrust, "{torque}");
        // The block's X points down, towards the centre of mass's level: the pairing is the
        // reverse of a rim block's, so a command about X still fires the nozzle that turns it
        // that way, and its authority is positive about X and Y. Its nozzles are not paired
        // with roll (the rim rule gives its forward-facing +X axis, not Z, to its sideways
        // nozzles), so it offers none about Z.
        let authority = torque_authority(&p, com, 0.0);
        assert!(authority.x > 0.0 && authority.y > 0.0, "{authority}");
        assert_eq!(authority.z, 0.0);
    }

    /// Where a block sits changes its lever: the same push from 3 m below the centre of mass
    /// turns the vessel the other way about X, and the same way about Z.
    #[test]
    fn a_block_below_the_centre_of_mass_turns_the_vessel_the_other_way() {
        let (p, com) = upper();
        let base = p.parts().base();
        let up = DVec3::new(1.0, 0.0, base + com + 3.0);
        let down = DVec3::new(1.0, 0.0, base + com - 3.0);
        let turn = |at| {
            let mut p = moved(&p, at, glam::DQuat::IDENTITY);
            fire(&mut p, com, DVec3::ZERO, DVec3::Y, 0.0, 0.02).1
        };
        let (above, below) = (turn(up), turn(down));
        assert!(
            (above.x + below.x).abs() < 1e-9 * above.x.abs(),
            "{above} {below}"
        );
        assert!(above.x < 0.0 && below.x > 0.0);
        assert!((above.z - below.z).abs() < 1e-9 * above.z.abs() && above.z > 0.0);
        assert!(above.y.abs() < 1e-9 && below.y.abs() < 1e-9);
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
