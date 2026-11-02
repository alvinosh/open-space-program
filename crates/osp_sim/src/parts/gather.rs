//! A craft's parts gathered into the release groups the physics calls stages
//! ([`crate::propulsion::Section`]).
//!
//! A stage is the parts between joints that let go. Its body is what its place and size are
//! read from: the lowest container or tank standing on the axis of the part the joint holds
//! ([`body`]). Any other container is one more part of it. Stages hang under each other along an axis, or beside the stage that holds them
//! (radial boosters). A fairing panel that lets go of its stage, stands round it and holds
//! nothing is half of the stage's fairing. Each part does what its components say, any number
//! of them, at any pose: an engine, a pod, an RCS block, a reaction wheel, a tank of any shape,
//! a wing, a gear leg. A shell standing on the body is stacked on it, and an even set of plate
//! fins round its base is the stage's fin set. Each part weighs what it is built as, once.

use std::f64::consts::{PI, TAU};

use glam::{DQuat, DVec2, DVec3};

use super::{
    Capabilities, Connector, ConnectorKind, Joint, JointWay, Part, PartId, PartKind, PartTree,
    StageBody,
};
use crate::{
    assembly::{Mount, Pose},
    component::{Component, LiftingSurface, Storage},
    craft::{Craft, CraftPart, Template},
    data::{FinDef, GearDef, PartCatalog, ShellDef, TankShape, WingDef},
    geometry::preset::{AirfoilDef, ShapeDef, WingDef as WingShape},
    propulsion::CraftError,
    shell::Shell,
    structure::{DOME_RATIO, FinSet, Tank, TankCell},
};

/// How close placements must be to what a stack of stages can hold, m and rad.
const NEAR: f64 = 1e-6;

impl PartTree {
    /// The tree `craft`'s parts make, gathered into stages (see the module's docs).
    pub fn from_craft(craft: &Craft, catalog: &PartCatalog) -> Result<Self, CraftError> {
        let bad = |problem: String| CraftError::BadCraft(format!("{}: {problem}", craft.id));
        let assembly = &craft.assembly;
        let parents = assembly.parents();
        let count = craft.parts.len();

        // Groups of parts between joints that let go, and the group each hangs from.
        let mut group = vec![0; count];
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut above: Vec<Option<usize>> = Vec::new();
        for p in assembly.order() {
            match parents[p] {
                Some((j, q)) if !assembly.releasable(j) => {
                    group[p] = group[q];
                    groups[group[p]].push(p);
                }
                held => {
                    group[p] = groups.len();
                    above.push(held.map(|(_, q)| group[q]));
                    groups.push(vec![p]);
                }
            }
        }
        // Each stage's body, each group's parts still nearest its joint first; a fairing panel
        // has none. What hangs from a group came after it, so its body is known by then.
        let poses = assembly.poses();
        let holds: Vec<bool> = (0..groups.len())
            .map(|g| above.contains(&Some(g)))
            .collect();
        let mut fairing = vec![false; groups.len()];
        let mut body_of = vec![usize::MAX; groups.len()];
        for g in 0..groups.len() {
            let first = groups[g][0];
            let holder = above[g].filter(|&h| !fairing[h]);
            fairing[g] = groups[g].len() == 1
                && !holds[g]
                && fairing_panel(&craft.parts[first]).is_some_and(|shell| {
                    holder.is_some_and(|h| {
                        let at = poses[first].position - poses[body_of[h]].position;
                        stands_round(shell, Pose::new(at, poses[first].rotation))
                    })
                });
            if !fairing[g] {
                body_of[g] = body(craft, &groups[g], &poses);
            }
        }
        for members in &mut groups {
            members.sort();
        }
        // The stages, numbered in the order they fire: each one after the stages hanging
        // under it on its axis and those hanging beside it.
        let side = |g: usize, h: usize| {
            let (a, b) = (poses[body_of[g]].position, poses[body_of[h]].position);
            (a - b).truncate().length() > NEAR
        };
        let mut order = Vec::new();
        let mut pending = vec![(0, false)];
        while let Some((g, done)) = pending.pop() {
            if done {
                order.push(g);
                continue;
            }
            let hung: Vec<usize> = (0..groups.len())
                .filter(|&c| !fairing[c] && above[c] == Some(g))
                .collect();
            pending.push((g, true));
            pending.extend(
                hung.iter()
                    .rev()
                    .filter(|&&c| side(c, g))
                    .map(|&c| (c, false)),
            );
            pending.extend(
                hung.iter()
                    .rev()
                    .filter(|&&c| !side(c, g))
                    .map(|&c| (c, false)),
            );
        }
        let stages = order.len();
        let mut stage_of = vec![usize::MAX; groups.len()];
        for (k, &g) in order.iter().enumerate() {
            stage_of[g] = k;
        }
        // A panel is on its holder's stage. Only a group that holds something else holds one,
        // so every stage was reached from the root's.
        for g in (0..groups.len()).filter(|&g| fairing[g]) {
            stage_of[g] = above[g].map_or(usize::MAX, |holder| stage_of[holder]);
        }

        let mut bodies = vec![0; stages];
        for &g in &order {
            bodies[stage_of[g]] = body_of[g];
        }
        let lift = DVec3::Z
            * bodies
                .iter()
                .map(|&b| poses[b].position.z)
                .fold(f64::INFINITY, f64::min);

        // The shells stacked on each stage: on the top of its body or its pod, and on
        // each other's tops.
        let pods: Vec<usize> = (0..count)
            .filter(|&p| matches!(craft.parts[p].template, Some(Template::Pod(_))))
            .collect();
        let mut stacked: Vec<usize> = Vec::new();
        loop {
            let tops: Vec<usize> = bodies
                .iter()
                .chain(&pods)
                .chain(&stacked)
                .copied()
                .collect();
            let next = assembly.joints().iter().find_map(|joint| {
                let [a, b] = &joint.ends;
                [(a, b), (b, a)].into_iter().find_map(|(held, on)| {
                    let shell = matches!(craft.parts[on.part].def.shape, ShapeDef::Shell(ref d) if d.span.is_none());
                    (tops.contains(&held.part)
                        && held.mount == Mount::Node("top".into())
                        && on.mount == Mount::Node("bottom".into())
                        && shell
                        && !stacked.contains(&on.part))
                    .then_some(on.part)
                })
            });
            match next {
                Some(shell) => stacked.push(shell),
                None => break,
            }
        }
        // Fin sets, and the mirror images of wings.
        let part_stage: Vec<usize> = (0..count).map(|p| stage_of[group[p]]).collect();
        let part_stage = &part_stage;
        let stage_parts = |s: usize| (0..count).filter(move |&p| part_stage[p] == s);
        let relative = |p: usize| {
            let body = poses[bodies[part_stage[p]]];
            Pose::new(poses[p].position - body.position, poses[p].rotation)
        };
        // How wide each stage is, and how long: up its body, its pod or its fairing,
        // whichever reaches highest.
        let mut sizes = Vec::new();
        for (s, &held) in bodies.iter().enumerate() {
            let (bottom, diameter, height) = extent(&craft.parts[held]);
            let mut length = height;
            for p in stage_parts(s) {
                let at = relative(p).position.z - bottom;
                // A pod standing on the stage's axis is its nose; one anywhere else is
                // not the stage's length.
                if let Some(Template::Pod(pod)) = &craft.parts[p].template
                    && on_axis(relative(p))
                    && upright(relative(p).rotation, yaw(relative(p).rotation))
                {
                    length = length.max(at + pod.height);
                }
                if let Some(shell) = fairing_panel(&craft.parts[p])
                    && fairing[group[p]]
                {
                    length = length.max(at + shell.sections[0].length);
                }
            }
            sizes.push((diameter, length));
        }
        let mut fin_sets = Vec::new();
        for (s, &(diameter, _)) in sizes.iter().enumerate() {
            let fins: Vec<usize> = stage_parts(s).filter(|&p| fin(&craft.parts[p])).collect();
            let radius = diameter / 2.0;
            let set = fin_set(craft, &fins, &relative, radius, catalog);
            fin_sets.push(set);
        }
        // Parts where one stage hangs from another, which engines may fire through; a part
        // that only lets go leaves what stands on it just as much there.
        let across = |q: usize| parents[q].is_some_and(|(_, h)| part_stage[h] != part_stage[q]);
        let joined = |p: usize| {
            across(p) || (0..count).any(|q| parents[q].is_some_and(|(_, h)| h == p) && across(q))
        };
        let between = |p: usize| {
            joined(p) || parents[p].is_some_and(|(_, h)| lets_go_only(&craft.parts[h]) && joined(h))
        };
        let mut parts = Vec::with_capacity(count);
        for (p, part) in craft.parts.iter().enumerate() {
            let stage = stage_of[group[p]];
            let (diameter, length) = sizes[stage];
            let local = relative(p);
            let named = |problem: String| bad(format!("`{}`: {problem}", part.id));
            // A shell that is its stage's body is not stacked on it.
            let mut kinds = Vec::new();
            if stacked.contains(&p) && p != bodies[stage] {
                kinds.extend(shell(part, local).map_err(named)?);
            } else if fairing[group[p]] {
                kinds.push(fairing_half(part, local).map_err(named)?);
            }
            kinds.extend(components(part, local, &fin_sets[stage]).map_err(named)?);
            if kinds.is_empty() {
                kinds.push(if lets_go_only(part) {
                    let (_, diameter, height) = extent(part);
                    PartKind::Decoupler {
                        mass: 0.0,
                        diameter,
                        height,
                    }
                } else {
                    PartKind::Structure { mass: 0.0 }
                });
            }
            // It weighs what it is built as, once, however many things it does.
            let mut left = part.built.mass.mass;
            for kind in &mut kinds {
                kind.weigh(left);
                left = 0.0;
            }
            parts.push(Part {
                id: PartId(p as u32),
                name: part.name.clone(),
                kinds,
                body: (p == bodies[stage]).then_some(StageBody { diameter, length }),
                stage,
                position: poses[p].position - lift,
                rotation: poses[p].rotation,
                connectors: Vec::new(),
                joint: None,
                open: between(p) && crate::aero::is_open(&part.built),
                built: Some(part.built.clone()),
                frame: Pose::IDENTITY,
                home: Pose::IDENTITY,
            });
        }
        for (child, parent) in parents.iter().enumerate() {
            let Some((j, holder)) = *parent else {
                continue;
            };
            let joint = &assembly.joints()[j];
            let frames = assembly.frames(j);
            let carries = assembly.carries(j);
            let capabilities = Capabilities {
                fluid: carries.fluid,
                power: carries.power,
                release: assembly.releasable(j),
            };
            let end = |part: usize| usize::from(joint.ends[0].part != part);
            let connector = |end: usize| {
                let mount = &joint.ends[end].mount;
                Connector {
                    name: mount.name().into(),
                    kind: match mount {
                        Mount::Node(_) => ConnectorKind::Fixed,
                        Mount::Surface { .. } => ConnectorKind::Surface,
                    },
                    position: frames[end].position,
                    capabilities,
                }
            };
            parts[holder].connectors.push(connector(end(holder)));
            parts[child].connectors.push(connector(end(child)));
            parts[child].joint = Some(Joint {
                parent: PartId(holder as u32),
                parent_connector: parts[holder].connectors.len() - 1,
                connector: parts[child].connectors.len() - 1,
                way: JointWay::AsBuilt,
            });
        }
        Ok(PartTree {
            parts,
            turned: false,
        })
    }
}

/// What `part`'s template and components make it do on its stage, at `local` from the stage's
/// base: one entry for each, none for what the stages need nothing of but its mass.
fn components(
    part: &CraftPart,
    local: Pose,
    fins: &Option<FinSet>,
) -> Result<Vec<PartKind>, String> {
    let mut kinds = Vec::new();
    if let Some(Template::Pod(pod)) = &part.template {
        kinds.push(PartKind::Pod(pod.clone()));
    }
    for component in &part.built.components {
        match component {
            Component::Decoupler(_)
            | Component::DockingPort(_)
            | Component::Container(_)
            | Component::Crew { .. }
            | Component::Battery(_)
            | Component::SurfaceLayer(_)
            | Component::FuelCell(_)
            | Component::SolarArray(_)
            | Component::Rtg(_)
            | Component::ElectricLoad(_)
            | Component::Avionics(_)
            | Component::Antenna(_) => {}
            Component::Engine(engine) => kinds.push(PartKind::Engine(engine.clone())),
            Component::Rcs(block) => kinds.push(PartKind::Rcs(block.clone())),
            Component::ReactionWheel(wheel) => kinds.push(PartKind::ReactionWheel(wheel.clone())),
            Component::Storage(storage) => kinds.push(tank(part, storage, local)?),
            Component::LiftingSurface(_) if fin(part) && fins.is_some() => {
                kinds.push(PartKind::Fins(fins.clone().expect("a set")));
            }
            Component::LiftingSurface(surface) => {
                kinds.push(PartKind::Wing(wing(part, surface, local)));
            }
            Component::Gear(leg) => {
                // Its strut runs out along the part's −Z and its wheel rolls along its +Y, so
                // the pose is all the physics needs; `position` and `angle` say where the
                // strut stands on the stage in the terms of a stage's gear list.
                let strut = local.rotation * DVec3::NEG_Z;
                kinds.push(PartKind::Gear(GearDef {
                    name: part.name.clone(),
                    position: local.position.z,
                    angle: strut.y.atan2(strut.x).to_degrees(),
                    length: leg.length,
                    wheel_radius: leg.wheel_radius,
                    stroke: leg.stroke,
                    load: leg.load,
                    mass: 0.0,
                    mirror: false,
                    brakes: leg.brakes,
                    steering: leg.steering,
                    foot: leg.foot,
                }));
            }
        }
    }
    Ok(kinds)
}

/// Whether all `part` does is let go.
fn lets_go_only(part: &CraftPart) -> bool {
    let components = &part.built.components;
    !components.is_empty()
        && components
            .iter()
            .all(|c| matches!(c, Component::Decoupler(_)))
}

/// The body of a stage of `members`, nearest the joint that holds it first: the lowest
/// container or tank upright on the axis of the part that joint holds (a container before a
/// tank at the same height); or else the container nearest the joint; or else the part the
/// joint holds.
fn body(craft: &Craft, members: &[usize], poses: &[Pose]) -> usize {
    let held = members[0];
    let axis = poses[held].position.truncate();
    let nodes = |p: usize| {
        let part = &craft.parts[p];
        Some(body_node(part, "bottom")?).filter(|_| body_node(part, "top").is_some())
    };
    let has =
        |p: usize, what: fn(&Component) -> bool| craft.parts[p].built.components.iter().any(what);
    let container = |p: usize| has(p, |c| matches!(c, Component::Container(_)));
    let tank = |p: usize| has(p, |c| matches!(c, Component::Storage(_)));
    let mut lowest: Option<(usize, f64)> = None;
    for &p in members {
        let Some(bottom) = nodes(p) else {
            continue;
        };
        let pose = poses[p];
        let standing = upright(pose.rotation, yaw(pose.rotation))
            && (pose.position.truncate() - axis).length() <= NEAR;
        if !(standing && (container(p) || tank(p))) {
            continue;
        }
        let base = pose.position.z + bottom.position.z;
        let better = lowest.is_none_or(|(q, low)| {
            base < low - NEAR || (base <= low + NEAR && container(p) && !container(q))
        });
        if better {
            lowest = Some((p, base));
        }
    }
    lowest
        .map(|(p, _)| p)
        .or_else(|| {
            let bodies = members.iter().copied();
            bodies
                .filter(|&p| container(p))
                .find(|&p| nodes(p).is_some())
        })
        .unwrap_or(held)
}

/// How `part` stands as a stage's body, in its own frame: the height of its base, how wide it
/// is and how long. Its `bottom` and `top` nodes say, or else the surfaces it has.
fn extent(part: &CraftPart) -> (f64, f64, f64) {
    if let (Some(bottom), Some(top)) = (body_node(part, "bottom"), body_node(part, "top")) {
        let length = (top.position.z - bottom.position.z).max(0.0);
        return (bottom.position.z, bottom.size, length);
    }
    let mut triangles = Vec::new();
    crate::aero::outer_triangles(&part.built, |p| p, &mut triangles);
    let (mut low, mut high, mut reach) = (f64::INFINITY, f64::NEG_INFINITY, 0.0_f64);
    for corner in triangles.iter().flatten() {
        (low, high) = (low.min(corner.z), high.max(corner.z));
        reach = reach.max(corner.truncate().length());
    }
    if low > high {
        return (0.0, 0.0, 0.0);
    }
    (low, 2.0 * reach, high - low)
}

fn body_node<'a>(part: &'a CraftPart, name: &str) -> Option<&'a crate::geometry::Node> {
    part.built
        .nodes
        .iter()
        .chain(&part.built.geometry.nodes)
        .find(|n| n.name == name)
}

/// A shell stacked on its stage, `local` from the stage's base: one barrel or cone, upright on
/// the axis. Anything else is one more part of the stage.
fn shell(part: &CraftPart, local: Pose) -> Result<Option<PartKind>, String> {
    let ShapeDef::Shell(shell) = &part.def.shape else {
        unreachable!("stacked parts are shells");
    };
    let [section] = shell.sections.as_slice() else {
        return Ok(None);
    };
    if !(on_axis(local) && upright(local.rotation, 0.0)) {
        return Ok(None);
    }
    let def = ShellDef {
        name: Some(part.name.clone()),
        length: section.length,
        bottom_diameter: Some(shell.bottom_diameter),
        top_diameter: section.top_diameter,
        profile: section.profile,
        mass: part.built.mass.mass,
    };
    Ok(Some(PartKind::Shell(Shell::new(
        &def,
        local.position.z,
        shell.bottom_diameter / 2.0,
    )?)))
}

/// A fairing panel's shape: a shell over part of a turn that lets go of its base.
fn fairing_panel(part: &CraftPart) -> Option<&crate::geometry::preset::ShellDef> {
    let ShapeDef::Shell(shell) = &part.def.shape else {
        return None;
    };
    let partial = shell
        .span
        .is_some_and(|[a, b]| (b - a).abs() < 360.0 - 1e-9);
    let lets_go = part
        .built
        .components
        .iter()
        .any(|c| matches!(c, Component::Decoupler(d) if d.node == "bottom"));
    (partial && lets_go).then_some(shell)
}

/// Whether a panel `shell` `local` from its stage's base is a half of its fairing: round the
/// stage from its base, straight up, its barrel straight. Anything else is a part of its own
/// that lets go.
fn stands_round(shell: &crate::geometry::preset::ShellDef, local: Pose) -> bool {
    on_axis(local)
        && local.position.z.abs() <= NEAR
        && upright(local.rotation, yaw(local.rotation))
        && shell.sections[0]
            .top_diameter
            .is_none_or(|d| (d - shell.bottom_diameter).abs() <= NEAR)
}

fn fairing_half(part: &CraftPart, local: Pose) -> Result<PartKind, String> {
    let shell = fairing_panel(part).expect("a fairing panel");
    let [a, b] = shell.span.expect("a span");
    let barrel = &shell.sections[0];
    let radius = shell.bottom_diameter / 2.0;
    let (mut base, mut below) = (barrel.length, radius);
    let mut shells = Vec::new();
    for section in &shell.sections[1..] {
        let def = ShellDef {
            name: None,
            length: section.length,
            bottom_diameter: Some(2.0 * below),
            top_diameter: section.top_diameter,
            profile: section.profile,
            mass: 0.0,
        };
        let nose = Shell::new(&def, base, below)?;
        (base, below) = (nose.top(), nose.top_radius);
        shells.push(nose);
    }
    let facing = (a + b).to_radians() / 2.0 + yaw(local.rotation);
    Ok(PartKind::FairingHalf {
        mass: 0.0,
        angle: facing.rem_euclid(TAU),
        radius,
        length: barrel.length,
        shells,
    })
}

/// `storage` as a cylinder tank with √2:1 domes, or a sphere, at any pose. A cylinder that does
/// not lie along the stage's axis is hung on the side of the stack, not carried by it: it holds
/// its pressure but takes no share of the stack's weight.
fn tank(part: &CraftPart, storage: &Storage, local: Pose) -> Result<PartKind, String> {
    // A tank is its own mirror image, so a mirrored one is the same tank.
    let shape = match &part.def.shape {
        ShapeDef::Mirrored(inner) => &**inner,
        shape => shape,
    };
    let plain = match shape {
        ShapeDef::CylinderTank(t)
            if (t.dome - DOME_RATIO).abs() < 1e-12
                && t.common_bulkhead.is_none()
                && t.aft_skirt == 0.0
                && t.forward_skirt == 0.0 =>
        {
            Some((TankShape::Cylinder, t.diameter / 2.0, t.length))
        }
        ShapeDef::Sphere(s) => Some((TankShape::Sphere, s.diameter / 2.0, s.diameter)),
        _ => None,
    };
    let Some((shape, radius, length)) = plain else {
        return Ok(cell_tank(part, storage, local));
    };
    // Its lowest point: the tank's axis runs up from its base along `axis`.
    let axis = local.rotation * DVec3::Z;
    let upright = axis.z > 1.0 - 1e-12;
    let along = axis.truncate().length() < NEAR;
    let base = if upright {
        local.position.z
    } else if shape == TankShape::Sphere {
        local.position.z + axis.z * radius - radius
    } else {
        lowest_of_cylinder(local.position.z, axis, radius, length)
    };
    let mut tank = Tank::of(&part.built, storage, shape, radius, length, base);
    if !along {
        tank.max_axial_load = None;
    }
    Ok(PartKind::Tank(tank))
}

/// How high the lowest point of a cylinder is that runs along `axis` from `start` high.
fn lowest_of_cylinder(start: f64, axis: DVec3, radius: f64, length: f64) -> f64 {
    start + (axis.z * length).min(0.0) - radius * axis.truncate().length()
}

/// `storage` in a cell of no plain shape: as long as the cell, and for the figures that want a
/// cylinder as wide as one of that length and volume, its contents settling in the cell itself.
fn cell_tank(part: &CraftPart, storage: &Storage, local: Pose) -> PartKind {
    let geometry = &part.built.geometry;
    let level = |volume: f64| {
        (geometry.settle(&storage.cell, DVec3::Z, volume)).map(|settled| settled.level)
    };
    let found = level(0.0).zip(level(storage.volume));
    // A cell that has no settling of its own (a box) is as long as the part.
    let (low, high) = found.unwrap_or_else(|| {
        let (bottom, _, height) = extent(part);
        (bottom, bottom + height)
    });
    let length = (high - low).max(1e-6);
    let radius = (storage.volume / (PI * length)).sqrt();
    let axis = local.rotation * DVec3::Z;
    let start = local.position.z + axis.z * low;
    let base = lowest_of_cylinder(start, axis, radius, length);
    let mut tank = Tank::of(
        &part.built,
        storage,
        TankShape::Cylinder,
        radius,
        length,
        base,
    );
    if axis.truncate().length() >= NEAR {
        tank.max_axial_load = None;
    }
    if found.is_some() {
        tank.cell = Some(TankCell {
            geometry: geometry.clone(),
            name: storage.cell.clone(),
        });
    }
    PartKind::Tank(tank)
}

/// Whether `part` is a fin: a lifting surface whose airfoil is a plate, and nothing else but
/// what the stages need only the mass of (a fin carries its whole set's mass).
fn fin(part: &CraftPart) -> bool {
    fin_or_wing(part)
        && part.built.components.iter().all(|c| {
            matches!(
                c,
                Component::LiftingSurface(_)
                    | Component::Decoupler(_)
                    | Component::DockingPort(_)
                    | Component::Container(_)
                    | Component::Crew { .. }
                    | Component::Battery(_)
                    | Component::SurfaceLayer(_)
                    | Component::FuelCell(_)
                    | Component::SolarArray(_)
                    | Component::Rtg(_)
                    | Component::ElectricLoad(_)
                    | Component::Avionics(_)
                    | Component::Antenna(_)
            )
        })
        && matches!(
            wing_shape(part),
            Some(WingShape {
                airfoil: AirfoilDef::Plate { .. },
                ..
            })
        )
}

fn fin_or_wing(part: &CraftPart) -> bool {
    part.built
        .components
        .iter()
        .any(|c| matches!(c, Component::LiftingSurface(_)))
}

fn wing_shape(part: &CraftPart) -> Option<&WingShape> {
    match &part.def.shape {
        ShapeDef::Wing(w) => Some(w),
        ShapeDef::Mirrored(inner) => match &**inner {
            ShapeDef::Wing(w) => Some(w),
            _ => None,
        },
        _ => None,
    }
}

/// A stage's fins as a set, if they make one: identical plain plates of solid metal, two or
/// more, evenly round its base with their trailing edges on it. Fins that make none are wings.
fn fin_set(
    craft: &Craft,
    fins: &[usize],
    relative: &impl Fn(usize) -> Pose,
    radius: f64,
    catalog: &PartCatalog,
) -> Option<FinSet> {
    let &first = fins.first()?;
    let one = &craft.parts[first];
    let (ShapeDef::Wing(shape), Some(material)) = (&one.def.shape, &one.def.structure.material)
    else {
        return None;
    };
    let AirfoilDef::Plate { thickness, bevel } = shape.airfoil else {
        return None;
    };
    let plain = shape.dihedral == 0.0 && shape.twist == 0.0 && (bevel - 0.15).abs() < 1e-12;
    let solid = one.def.structure.solidity == 1.0 && one.def.structure.mass.is_none();
    let mut angles = Vec::new();
    for &f in fins {
        let part = &craft.parts[f];
        let at = relative(f);
        let angle = at.position.y.atan2(at.position.x);
        let placed = (at.position.truncate() - DVec2::from_angle(angle) * radius).length() <= NEAR
            && (at.position.z - shape.root_chord).abs() <= NEAR
            && upright(at.rotation, angle);
        let alike = part.def.shape == one.def.shape
            && part.def.structure == one.def.structure
            && part.def.components == one.def.components;
        if !alike || !placed {
            return None;
        }
        angles.push(angle.rem_euclid(TAU));
    }
    angles.sort_by(f64::total_cmp);
    let n = fins.len();
    let even = angles
        .iter()
        .enumerate()
        .all(|(k, a)| (a - TAU * k as f64 / n as f64).abs() <= NEAR);
    if n < 2 || !even || !plain || !solid {
        return None;
    }
    let def = FinDef {
        count: n as u32,
        root_chord: shape.root_chord,
        tip_chord: shape.tip_chord,
        span: shape.span,
        sweep: shape.sweep,
        thickness,
        material: material.clone(),
    };
    FinSet::new(&def, catalog).ok()
}

/// How far a camber of 1 (over the chord) moves the angle where a section lifts nothing, rad:
/// measured NACA four-digit sections have about −2.1° at 2% and −4.1° at 4%.
const CAMBER_ZERO_LIFT: f64 = 1.8;

/// A wing record for lifting surface `surface` of `part`, whose root is at `local` on its
/// stage. The physics needs of it what its panels do not say: how thick it is, the flap, and
/// what its camber and twist are worth in angle of attack (half its twist is the mean along its
/// span). `position` and `angle` say where its root stands on the stage, in the terms of a
/// stage's wing list.
fn wing(part: &CraftPart, surface: &LiftingSurface, local: Pose) -> WingDef {
    let root = local.position;
    let (first, last) = (surface.panels.first(), surface.panels.last());
    let span: f64 = (surface.panels.iter())
        .map(|p| (p.tip - p.root).reject_from(p.chord).length())
        .sum();
    let sweep = first
        .zip(last)
        .map_or(0.0, |(a, b)| (b.tip - a.root).dot(a.chord));
    // As its airfoil says, or as its panels measure for a plate or a loft of another shape.
    let measured = first.map_or(0.0, |p| (p.thickness[0] + p.thickness[1]) / 2.0);
    let (thickness, camber, twist, dihedral) =
        wing_shape(part).map_or((measured, 0.0, 0.0, 0.0), |shape| match shape.airfoil {
            AirfoilDef::Naca {
                thickness, camber, ..
            } => (thickness, camber, shape.twist, shape.dihedral),
            _ => (measured, 0.0, shape.twist, shape.dihedral),
        });
    let turned = first.map_or(0.0, |plan| {
        crate::aero::Panel::at(plan, local.position, local.rotation).incidence
    });
    WingDef {
        name: part.name.clone(),
        position: root.z,
        angle: root.y.atan2(root.x).to_degrees(),
        root_chord: first.map_or(0.0, |p| p.root_chord),
        tip_chord: last.map_or(0.0, |p| p.tip_chord),
        span,
        sweep,
        dihedral,
        incidence: turned.to_degrees(),
        camber_incidence: (CAMBER_ZERO_LIFT * camber).to_degrees() + twist / 2.0,
        thickness,
        mass: 0.0,
        mirror: false,
        control: surface.control.clone(),
    }
}

fn on_axis(pose: Pose) -> bool {
    pose.position.truncate().length() <= NEAR
}

/// Whether `rotation` is a turn of `yaw` about +Z.
fn upright(rotation: DQuat, yaw: f64) -> bool {
    rotation.dot(DQuat::from_rotation_z(yaw)).abs() > 1.0 - 1e-12
}

/// How far `rotation` turns +X about +Z.
fn yaw(rotation: DQuat) -> f64 {
    let x = rotation * DVec3::X;
    x.y.atan2(x.x)
}
