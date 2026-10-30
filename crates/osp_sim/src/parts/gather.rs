//! A craft's parts gathered into the stages today's physics runs on, until it runs on parts
//! (step 8 of `docs/vessel-overhaul.md`).
//!
//! A stage is the parts between joints that let go, and its body is the one container with
//! a `bottom` and a `top` node, standing upright: the stage is as wide as its `bottom` node
//! and as long as from there to its `top`, and its parts are placed from its base. Stages
//! hang under each other along an axis, or beside the stage that holds them (radial
//! boosters); the root's stage stands on the vessel's axis. A fairing panel that lets go of
//! its stage is half of the stage's fairing. Each part becomes the kind of part a stage
//! knows: a template's engine (upright, anywhere on its stage), pod, RCS block or reaction
//! wheel; a cylinder or sphere tank; a shell stacked on the body; an even set of plate fins
//! around its base; a wing on its side, with its mirror image if it has one; a gear leg
//! standing out from it. Each part weighs what it is built as. A craft the stages cannot
//! hold is refused with what is missing.

use std::f64::consts::TAU;

use glam::{DMat3, DQuat, DVec2, DVec3};

use super::{Capabilities, Connector, ConnectorKind, Joint, Part, PartId, PartKind, PartTree};
use crate::{
    assembly::{Mount, Pose},
    component::Component,
    craft::{Craft, CraftPart, Template},
    data::{FinDef, GearDef, PartCatalog, ShellDef, TankShape, WingDef},
    geometry::preset::{AirfoilDef, ShapeDef, WingDef as WingShape},
    propulsion::CraftError,
    shell::Shell,
    structure::{DOME_RATIO, FinSet, Tank},
};

/// How close placements must be to what a stack of stages can hold, m and rad.
const NEAR: f64 = 1e-6;

impl PartTree {
    /// The tree `craft`'s parts make, gathered into stages (see the module's docs).
    pub fn from_craft(craft: &Craft, catalog: &PartCatalog) -> Result<Self, CraftError> {
        let bad = |problem: String| {
            CraftError::BadCraft(format!(
                "{}: {problem} (until the physics runs on parts)",
                craft.id
            ))
        };
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
        for members in &mut groups {
            members.sort();
        }
        let fairing: Vec<bool> = groups
            .iter()
            .map(|g| g.len() == 1 && fairing_panel(&craft.parts[g[0]]).is_some())
            .collect();
        // Each stage's body, standing upright.
        let poses = assembly.poses();
        let mut body_of = vec![usize::MAX; groups.len()];
        for g in (0..groups.len()).filter(|&g| !fairing[g]) {
            let found: Vec<usize> = groups[g]
                .iter()
                .copied()
                .filter(|&p| is_body(&craft.parts[p]))
                .collect();
            let [body] = found.as_slice() else {
                return Err(bad(format!(
                    "the stage with `{}` needs one body, a container with a `bottom` and a \
                     `top` node, not {}",
                    craft.parts[groups[g][0]].id,
                    found.len()
                )));
            };
            let pose = poses[*body];
            if !upright(pose.rotation, yaw(pose.rotation))
                || (g == 0 && pose.position.truncate().length() > NEAR)
            {
                return Err(bad(format!(
                    "`{}` must stand upright, and the root's stage on the axis",
                    craft.parts[*body].id
                )));
            }
            body_of[g] = *body;
        }
        // The stages, numbered in the order they fire: each one after the stage hanging
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
            let under: Vec<usize> = hung.iter().copied().filter(|&c| !side(c, g)).collect();
            if under.len() > 1 {
                return Err(bad(format!(
                    "{} stages hang one under another from `{}`",
                    under.len(),
                    craft.parts[groups[g][0]].id
                )));
            }
            pending.push((g, true));
            pending.extend(
                hung.iter()
                    .rev()
                    .filter(|&&c| side(c, g))
                    .map(|&c| (c, false)),
            );
            pending.extend(under.iter().map(|&c| (c, false)));
        }
        let stages = order.len();
        let mut stage_of = vec![usize::MAX; groups.len()];
        for (k, &g) in order.iter().enumerate() {
            stage_of[g] = k;
        }
        for g in 0..groups.len() {
            if fairing[g]
                && let Some(holder) = above[g].filter(|&h| !fairing[h])
            {
                stage_of[g] = stage_of[holder];
            }
            if stage_of[g] == usize::MAX {
                return Err(bad(format!(
                    "`{}` lets go of what is no stage",
                    craft.parts[groups[g][0]].id
                )));
            }
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
            let nodes = &craft.parts[held].built.geometry;
            let (Some(bottom), Some(top)) = (
                body_node(&craft.parts[held], "bottom"),
                body_node(&craft.parts[held], "top"),
            ) else {
                unreachable!("bodies have both nodes: {}", nodes.nodes.len());
            };
            let mut length = top.position.z - bottom.position.z;
            for p in stage_parts(s) {
                let at = relative(p).position.z - bottom.position.z;
                if let Some(Template::Pod(pod)) = &craft.parts[p].template {
                    length = length.max(at + pod.height);
                }
                if let Some(shell) = fairing_panel(&craft.parts[p]) {
                    length = length.max(at + shell.sections[0].length);
                }
            }
            sizes.push((bottom.size, length));
        }
        let mut fin_sets = Vec::new();
        for (s, &(diameter, _)) in sizes.iter().enumerate() {
            let fins: Vec<usize> = stage_parts(s).filter(|&p| fin(&craft.parts[p])).collect();
            let radius = diameter / 2.0;
            let set = fin_set(craft, &fins, &relative, radius, catalog).map_err(&bad)?;
            fin_sets.push(set);
        }
        let mut partner = vec![None; count];
        for p in 0..count {
            let ShapeDef::Mirrored(inner) = &craft.parts[p].def.shape else {
                continue;
            };
            let flipped = mirror(relative(p));
            let found = stage_parts(part_stage[p]).find(|&q| {
                let (other, b) = (&craft.parts[q], relative(q));
                other.def.shape == **inner
                    && other.def.structure == craft.parts[p].def.structure
                    && other.def.components == craft.parts[p].def.components
                    && (b.position - flipped.position).length() < NEAR
                    && b.rotation.dot(flipped.rotation).abs() > 1.0 - 1e-12
            });
            let Some(q) = found else {
                return Err(bad(format!(
                    "the mirrored `{}` has no partner mirroring it across the stage",
                    craft.parts[p].id
                )));
            };
            partner[p] = Some(q);
            partner[q] = Some(p);
        }

        // Parts where one stage hangs from another, which engines may fire through.
        let between = |p: usize| {
            let across = |q: usize| parents[q].is_some_and(|(_, h)| part_stage[h] != part_stage[q]);
            across(p) || (0..count).any(|q| parents[q].is_some_and(|(_, h)| h == p) && across(q))
        };
        let mut parts = Vec::with_capacity(count);
        for (p, part) in craft.parts.iter().enumerate() {
            let stage = stage_of[group[p]];
            let (diameter, length) = sizes[stage];
            let radius = diameter / 2.0;
            let local = relative(p);
            let mirrored = matches!(part.def.shape, ShapeDef::Mirrored(_));
            let kind = if p == bodies[stage] {
                Ok(PartKind::Hull {
                    structure_mass: part.built.mass.mass,
                    diameter,
                    length,
                })
            } else if stacked.contains(&p) {
                shell(part, local)
            } else if fairing[group[p]] {
                fairing_half(part, local)
            } else if mirrored && fin_or_wing(part) {
                Ok(PartKind::Mirror)
            } else {
                let pair = partner[p].map(|q| craft.parts[q].built.mass.mass);
                kind(part, local, radius, pair, &fin_sets[stage])
            }
            .map_err(|problem| bad(format!("`{}`: {problem}", part.id)))?;
            parts.push(Part {
                id: PartId(p as u32),
                name: part.name.clone(),
                kind,
                stage,
                position: poses[p].position - lift,
                rotation: poses[p].rotation,
                connectors: Vec::new(),
                joint: None,
                open: between(p) && crate::aero::is_open(&part.built),
                built: Some(part.built.clone()),
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
            });
        }
        Ok(PartTree { parts })
    }
}

/// The kind of part `part` is on its stage, at `local` from the stage's base, on a stage
/// `radius` wide.
fn kind(
    part: &CraftPart,
    local: Pose,
    radius: f64,
    partner: Option<f64>,
    fins: &Option<FinSet>,
) -> Result<PartKind, String> {
    if let Some(Template::Pod(pod)) = &part.template {
        if !(on_axis(local) && upright(local.rotation, 0.0)) {
            return Err("a pod sits upright on the axis of its stage".into());
        }
        return Ok(PartKind::Pod(pod.clone()));
    }
    let what: Vec<&Component> = part
        .built
        .components
        .iter()
        // What the stages need nothing of but its mass.
        .filter(|c| {
            !matches!(
                c,
                Component::Decoupler(_)
                    | Component::Container(_)
                    | Component::Crew { .. }
                    | Component::Battery(_)
            )
        })
        .collect();
    match what.as_slice() {
        [Component::Engine(engine)] => {
            if !upright(local.rotation, yaw(local.rotation)) {
                return Err("an engine stands upright, firing down its stage".into());
            }
            Ok(PartKind::Engine(engine.clone()))
        }
        [Component::Rcs(block)] => {
            if !local
                .rotation
                .mul_vec3(DVec3::Z)
                .abs_diff_eq(DVec3::Z, NEAR)
            {
                return Err("an RCS block stands upright".into());
            }
            Ok(PartKind::Rcs(block.clone()))
        }
        [Component::ReactionWheel(wheel)] => Ok(PartKind::ReactionWheel(wheel.clone())),
        [Component::Storage(_)] => tank(part, local),
        [Component::LiftingSurface(surface)] if fin(part) => fins
            .clone()
            .map(PartKind::Fins)
            .ok_or_else(|| format!("{surface:?} is no fin")),
        [Component::LiftingSurface(surface)] => {
            let ShapeDef::Wing(shape) = &part.def.shape else {
                return Err("a lifting surface is a `wing`".into());
            };
            let mut def = wing(shape, local, radius)?;
            def.name = part.name.clone();
            def.mass = part.built.mass.mass + partner.unwrap_or(0.0);
            def.mirror = partner.is_some();
            def.control = surface.control.clone();
            Ok(PartKind::Wing(def))
        }
        [Component::Gear(leg)] => {
            let strut = local.rotation * DVec3::NEG_Z;
            let rolls = local.rotation * DVec3::Y;
            let mount = local.position;
            let outward = strut.truncate();
            if strut.z.abs() > NEAR
                || !rolls.abs_diff_eq(DVec3::Z, NEAR)
                || (mount.truncate() - outward * radius).length() > NEAR
            {
                return Err(
                    "a gear leg stands straight out from its stage's side, rolling along it".into(),
                );
            }
            Ok(PartKind::Gear(GearDef {
                name: part.name.clone(),
                position: mount.z,
                angle: outward.y.atan2(outward.x).to_degrees(),
                length: leg.length,
                wheel_radius: leg.wheel_radius,
                stroke: leg.stroke,
                load: leg.load,
                mass: leg.mass,
                mirror: false,
                brakes: leg.brakes,
                steering: leg.steering,
            }))
        }
        [] => Ok(PartKind::Structure {
            mass: part.built.mass.mass,
        }),
        _ => Err("a stage has no part like it".into()),
    }
}

/// Whether `part` is a stage's body: a container with a `bottom` and a `top` node.
fn is_body(part: &CraftPart) -> bool {
    part.built
        .components
        .iter()
        .any(|c| matches!(c, Component::Container(_)))
        && body_node(part, "bottom").is_some()
        && body_node(part, "top").is_some()
}

fn body_node<'a>(part: &'a CraftPart, name: &str) -> Option<&'a crate::geometry::Node> {
    part.built
        .nodes
        .iter()
        .chain(&part.built.geometry.nodes)
        .find(|n| n.name == name)
}

/// A shell stacked on its stage, `local` from the stage's base.
fn shell(part: &CraftPart, local: Pose) -> Result<PartKind, String> {
    let ShapeDef::Shell(shell) = &part.def.shape else {
        unreachable!("stacked parts are shells");
    };
    let [section] = shell.sections.as_slice() else {
        return Err("a shell on a stage has one section".into());
    };
    if !(on_axis(local) && upright(local.rotation, 0.0)) {
        return Err("a shell stands upright on the axis".into());
    }
    let def = ShellDef {
        name: Some(part.name.clone()),
        length: section.length,
        bottom_diameter: Some(shell.bottom_diameter),
        top_diameter: section.top_diameter,
        profile: section.profile,
        mass: part.built.mass.mass,
    };
    Ok(PartKind::Shell(Shell::new(
        &def,
        local.position.z,
        shell.bottom_diameter / 2.0,
    )?))
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

fn fairing_half(part: &CraftPart, local: Pose) -> Result<PartKind, String> {
    let shell = fairing_panel(part).expect("a fairing panel");
    let [a, b] = shell.span.expect("a span");
    let barrel = &shell.sections[0];
    if !(on_axis(local)
        && local.position.z.abs() <= NEAR
        && upright(local.rotation, yaw(local.rotation)))
        || barrel
            .top_diameter
            .is_some_and(|d| (d - shell.bottom_diameter).abs() > NEAR)
    {
        return Err("a fairing half stands round its stage from its base, straight up".into());
    }
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
        mass: part.built.mass.mass,
        angle: facing.rem_euclid(TAU),
        radius,
        length: barrel.length,
        shells,
    })
}

/// A cylinder tank with √2:1 domes, or a sphere, upright.
fn tank(part: &CraftPart, local: Pose) -> Result<PartKind, String> {
    let (shape, radius, length) = match &part.def.shape {
        ShapeDef::CylinderTank(t)
            if (t.dome - DOME_RATIO).abs() < 1e-12
                && t.common_bulkhead.is_none()
                && t.aft_skirt == 0.0
                && t.forward_skirt == 0.0 =>
        {
            (TankShape::Cylinder, t.diameter / 2.0, t.length)
        }
        ShapeDef::Sphere(s) => (TankShape::Sphere, s.diameter / 2.0, s.diameter),
        _ => return Err("a stage's tank is a cylinder with √2:1 domes, or a sphere".into()),
    };
    if !upright(local.rotation, yaw(local.rotation)) {
        return Err("a tank stands upright".into());
    }
    Ok(PartKind::Tank(Tank::of(
        &part.built,
        shape,
        radius,
        length,
        local.position.z,
    )))
}

/// Whether `part` is a fin: a lifting surface whose airfoil is a plate.
fn fin(part: &CraftPart) -> bool {
    fin_or_wing(part)
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

/// A stage's fins as a set: identical plates of solid metal, evenly round its base with
/// their trailing edges on it.
fn fin_set(
    craft: &Craft,
    fins: &[usize],
    relative: &impl Fn(usize) -> Pose,
    radius: f64,
    catalog: &PartCatalog,
) -> Result<Option<FinSet>, String> {
    let Some(&first) = fins.first() else {
        return Ok(None);
    };
    let one = &craft.parts[first];
    let (ShapeDef::Wing(shape), Some(material)) = (&one.def.shape, &one.def.structure.material)
    else {
        return Err(format!(
            "the fin `{}` is a plate `wing` of a material",
            one.id
        ));
    };
    let AirfoilDef::Plate { thickness, bevel } = shape.airfoil else {
        unreachable!("fins are plates");
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
            return Err(format!(
                "the fins on a stage are alike, standing out from its side with their \
                 trailing edges on its base, as `{}` is not",
                part.id
            ));
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
        return Err(
            "a stage's fins are two or more plain plates of solid metal, evenly round it from \
             its +X"
                .into(),
        );
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
    FinSet::new(&def, catalog)
        .map(Some)
        .map_err(|e| e.to_string())
}

/// A wing record for a `wing` shape whose root is at `local` on a stage `radius` wide: its
/// angle round the stage from where its root is, and its incidence from how it is turned
/// about its span.
fn wing(shape: &WingShape, local: Pose, radius: f64) -> Result<WingDef, String> {
    let AirfoilDef::Naca {
        thickness, camber, ..
    } = shape.airfoil
    else {
        return Err("a wing has a NACA airfoil".into());
    };
    let root = local.position;
    let angle = root.y.atan2(root.x);
    if camber != 0.0 || shape.twist != 0.0 || (root.truncate().length() - radius).abs() > NEAR {
        return Err("a wing is untwisted and uncambered, its root on its stage's side".into());
    }
    let dihedral = shape.dihedral.to_radians();
    let along = DVec3::new(dihedral.cos(), -dihedral.sin(), 0.0);
    let (axis, turned) = (DQuat::from_rotation_z(angle).inverse() * local.rotation).to_axis_angle();
    let incidence = if turned.abs() < 1e-12 {
        0.0
    } else if axis.cross(along).length() < NEAR {
        turned * axis.dot(along).signum()
    } else {
        return Err("a wing is turned only about its span".into());
    };
    Ok(WingDef {
        name: String::new(),
        position: root.z,
        angle: angle.to_degrees(),
        root_chord: shape.root_chord,
        tip_chord: shape.tip_chord,
        span: shape.span / dihedral.cos(),
        sweep: shape.sweep,
        dihedral: shape.dihedral,
        incidence: incidence.to_degrees(),
        thickness,
        mass: 0.0,
        mirror: false,
        control: None,
    })
}

/// `pose` reflected across the YZ plane.
fn mirror(pose: Pose) -> Pose {
    let flip = DMat3::from_diagonal(DVec3::new(-1.0, 1.0, 1.0));
    let turn = flip * DMat3::from_quat(pose.rotation) * flip;
    Pose::new(flip * pose.position, DQuat::from_mat3(&turn).normalize())
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
