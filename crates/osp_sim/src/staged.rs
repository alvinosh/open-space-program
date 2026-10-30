//! The stage converter: a craft described as a stack of stages ([`StagedCraftDef`]) written
//! out as the plain parts it is made of ([`CraftDef`]). Like symmetry in the editor, it is a
//! tool for building craft, and what it writes keeps no trace of it.
//!
//! Each stage becomes its structure, a container that weighs its `structure_mass`, with its
//! parts on nodes of it where the stage puts them: engines under its base, tanks, reaction
//! wheels and the pod inside it, RCS blocks on its forward rim, fins, wings and gear on
//! its side, and the fairing's halves round it. The structure is the skin where the tanks'
//! barrels are not (the aft skirt, intertanks, and the interstage round the engines of the
//! stage above), the plate the engines hang from, and the ring that lets go of the stage
//! above. Under a fairing with nothing else, a payload: an adapter and a satellite on it.
//! The stage's look (`appearance`) becomes paint, folded legs, stowed grid fins and a
//! raceway. Its shells stack on its top, over its pod if it has one. The top stage's
//! structure is the root; each stage below hangs from the one above by the top of its
//! stack, which has a decoupler that lets go of the stage above. A mirrored wing or leg
//! becomes two parts.

use std::{collections::BTreeMap, f64::consts::PI, sync::Arc};

use glam::{DMat3, DQuat, DVec2, DVec3};

use crate::{
    assembly::{AssemblyPart, Mount, Pose, roll_to},
    component::{
        ComponentDef, ContainerDef, DecouplerDef, Finish, GearLegDef, LiftingSurfaceDef, Look,
        PaintDef, PartDef, StorageDef, StructureDef,
    },
    craft::node_def,
    data::{CraftDef, CraftPartDef, LiveryDef, StageDef, StagedCraftDef, TankShape},
    geometry::{
        Node, ShapeDef,
        preset::{
            AirfoilDef, BeamDef, BeamSection, BoxDef, CurveDef, CylinderTankDef, LatticeDef,
            LoftedDef, NodeDef, PlacedShapeDef, RevolvedDef, RingDef, SectionDef, SegmentDef,
            ShellDef, ShellSection, SphereDef, StationDef, WallDef, WingDef as WingShape,
        },
    },
    parts::{Part, PartKind, PartTree},
    propulsion::CraftError,
    shell::Shell,
    structure::{DOME_RATIO, Tank},
};

/// The parts `def` describes.
pub fn convert(
    def: &StagedCraftDef,
    catalog: &crate::data::PartCatalog,
) -> Result<CraftDef, CraftError> {
    let tree = PartTree::from_staged(def, catalog)?;
    let bad = |problem: String| CraftError::BadCraft(format!("{}: {problem}", def.id));
    let mut taken = Vec::new();
    let ids: Vec<String> = def
        .stages
        .iter()
        .map(|s| {
            let id = unique(slug(&s.name), &taken);
            taken.push(id.clone());
            id
        })
        .collect();
    let mut parts = Vec::new();
    // The parts each stage with a livery paints: its body, shells, tanks and fairing.
    let mut liveried: Vec<(&LiveryDef, Vec<String>)> = Vec::new();
    for s in (0..def.stages.len()).rev() {
        let stage = &def.stages[s];
        let livery = match &stage.appearance.livery {
            Some(id) => Some(
                catalog
                    .liveries
                    .iter()
                    .find(|l| &l.id == id)
                    .ok_or_else(|| bad(format!("no livery `{id}`")))?,
            ),
            None => None,
        };
        let on: Vec<&Part> = tree.stage(s).collect();
        let hull = on
            .iter()
            .find(|p| matches!(p.kind, PartKind::Hull { .. }))
            .expect("every stage has a hull");
        let base = hull.position;
        let relative = |p: &Part| Pose::new(p.position - base, p.rotation);
        let radius = stage.diameter / 2.0;
        let stage_tanks: Vec<&Tank> = on
            .iter()
            .filter_map(|p| match &p.kind {
                PartKind::Tank(t) => Some(t),
                _ => None,
            })
            .collect();
        let above = def.stages.get(s + 1);
        let pod_height = on
            .iter()
            .find_map(|p| match &p.kind {
                PartKind::Pod(pod) => Some(pod.height),
                _ => None,
            })
            .unwrap_or(0.0);
        let build = structure(stage, &stage_tanks, pod_height, above).map_err(&bad)?;
        let paint = paint_of(stage);
        let mut body = Body::new(
            &ids[s],
            build.shape.clone(),
            build.paint.clone(),
            build.mass,
        )
        .map_err(&bad)?;
        body.nodes = build.nodes.clone();
        let mut mounted = Vec::new();
        let mut count: BTreeMap<&str, usize> = BTreeMap::new();
        let mut name = |kind: &'static str| {
            let k = count.entry(kind).or_default();
            *k += 1;
            format!("{kind}_{k}")
        };
        let mut tanks = stage.tanks.iter();
        for part in &on {
            let at = relative(part);
            match &part.kind {
                PartKind::Hull { .. } | PartKind::Decoupler | PartKind::Shell(_) => {}
                PartKind::Mirror | PartKind::Structure { .. } => {
                    unreachable!("no mirrors or bare structure on a staged tree")
                }
                PartKind::Engine(engine) => {
                    let node = name("engine");
                    let child = template(&ids[s], &node, &engine.id);
                    let shape = PartDef::engine(engine)?.shape;
                    mounted.push(body.mount(child, &shape, "top", at, &node).map_err(&bad)?);
                }
                PartKind::Rcs(block) => {
                    let node = name("rcs");
                    let child = template(&ids[s], &node, &block.id);
                    let shape = PartDef::rcs(block).shape;
                    mounted.push(
                        body.mount(child, &shape, "mount", at, &node)
                            .map_err(&bad)?,
                    );
                }
                PartKind::ReactionWheel(wheel) => {
                    let node = name("wheel");
                    let child = template(&ids[s], &node, &wheel.id);
                    let shape = PartDef::reaction_wheel(wheel).shape;
                    mounted.push(
                        body.mount(child, &shape, "bottom", at, &node)
                            .map_err(&bad)?,
                    );
                }
                PartKind::Pod(pod) => {
                    let node = "pod".to_string();
                    let child = template(&ids[s], &node, &pod.id);
                    let shape = PartDef::pod(pod).shape;
                    mounted.push(
                        body.mount(child, &shape, "bottom", at, &node)
                            .map_err(&bad)?,
                    );
                }
                PartKind::Tank(tank) => {
                    let tank_def = tanks.next().expect("one tank part per tank");
                    let node = name("tank");
                    let shape = match tank.shape {
                        TankShape::Cylinder => ShapeDef::CylinderTank(CylinderTankDef {
                            diameter: 2.0 * tank.radius,
                            length: tank.length,
                            dome: DOME_RATIO,
                            common_bulkhead: None,
                            aft_skirt: 0.0,
                            forward_skirt: 0.0,
                        }),
                        TankShape::Sphere => ShapeDef::Sphere(SphereDef {
                            diameter: 2.0 * tank.radius,
                        }),
                    };
                    let mut child = shaped(&ids[s], &node, shape.clone());
                    child.name = Some(tank.tank_type.clone());
                    child.paint = paint.clone();
                    child.components = vec![ComponentDef::Storage(StorageDef {
                        cell: None,
                        tank_type: tank_def.tank_type.clone(),
                        contents: tank_def.propellants.clone(),
                        design_load: tank_def.design_load,
                    })];
                    mounted.push(
                        body.mount(child, &shape, "bottom", at, &node)
                            .map_err(&bad)?,
                    );
                }
                PartKind::Fins(set) => {
                    let fins = &set.def;
                    let shape = ShapeDef::Wing(WingShape {
                        root_chord: fins.root_chord,
                        tip_chord: fins.tip_chord,
                        span: fins.span,
                        sweep: fins.sweep,
                        dihedral: 0.0,
                        twist: 0.0,
                        airfoil: AirfoilDef::Plate {
                            thickness: fins.thickness,
                            bevel: 0.15,
                        },
                    });
                    for k in 0..fins.count {
                        let angle = std::f64::consts::TAU * k as f64 / fins.count as f64;
                        let root = (DVec2::from_angle(angle) * radius).extend(fins.root_chord);
                        let pose = Pose::new(root, DQuat::from_rotation_z(angle));
                        let node = name("fin");
                        let mut child = shaped(&ids[s], &node, shape.clone());
                        child.name = Some("Fin".into());
                        child.structure = StructureDef {
                            material: Some(fins.material.clone()),
                            ..StructureDef::default()
                        };
                        child.paint = look(BARE_FIN, Finish::Metal);
                        child.components = vec![lifting(None)];
                        mounted.push(
                            body.mount(child, &shape, "root", pose, &node)
                                .map_err(&bad)?,
                        );
                    }
                }
                PartKind::Wing(wing) => {
                    let dihedral = wing.dihedral.to_radians();
                    let shape = ShapeDef::Wing(WingShape {
                        root_chord: wing.root_chord,
                        tip_chord: wing.tip_chord,
                        span: wing.span * dihedral.cos(),
                        sweep: wing.sweep,
                        dihedral: wing.dihedral,
                        twist: 0.0,
                        airfoil: AirfoilDef::Naca {
                            thickness: wing.thickness,
                            camber: 0.0,
                            camber_position: 0.4,
                        },
                    });
                    let angle = wing.angle.to_radians();
                    let along = DVec3::new(dihedral.cos(), -dihedral.sin(), 0.0);
                    let root = (DVec2::from_angle(angle) * radius).extend(wing.position);
                    let turn = DQuat::from_rotation_z(angle)
                        * DQuat::from_axis_angle(along, wing.incidence.to_radians());
                    let right = Pose::new(root, turn);
                    let halves = if wing.mirror { 2.0 } else { 1.0 };
                    let node = name("wing");
                    let mut child = shaped(&ids[s], &node, shape.clone());
                    child.name = Some(wing.name.clone());
                    child.structure = StructureDef {
                        mass: Some(wing.mass / halves),
                        ..StructureDef::default()
                    };
                    child.components = vec![lifting(wing.control.clone())];
                    child.paint = look(WING_SKIN, Finish::Paint);
                    let left = wing.mirror.then(|| {
                        let left_node = format!("{node}_left");
                        let left_shape = ShapeDef::Mirrored(Box::new(shape.clone()));
                        let mut left = child.clone();
                        left.id = format!("{}_{left_node}", ids[s]);
                        left.shape = Some(left_shape.clone());
                        (left, left_shape, left_node)
                    });
                    mounted.push(
                        body.mount(child, &shape, "root", right, &node)
                            .map_err(&bad)?,
                    );
                    if let Some((left, left_shape, left_node)) = left {
                        let pose = mirror(right);
                        let left = body.mount(left, &left_shape, "root", pose, &left_node);
                        mounted.push(left.map_err(&bad)?);
                    }
                }
                PartKind::Gear(leg) => {
                    let legs = if leg.mirror { 2 } else { 1 };
                    let strut = ShapeDef::Compound(vec![PlacedShapeDef {
                        name: String::new(),
                        shape: ShapeDef::Beam(BeamDef {
                            length: leg.length,
                            section: BeamSection::Circle {
                                diameter: leg.wheel_radius / 2.0,
                            },
                        }),
                        position: [0.0; 3],
                        rotation: [180.0, 0.0, 0.0],
                        unseen: false,
                    }]);
                    let node = name("gear");
                    for k in 0..legs {
                        let angle = if k == 0 { leg.angle } else { 180.0 - leg.angle }.to_radians();
                        let out = DVec2::from_angle(angle).extend(0.0);
                        let turn = DMat3::from_cols(DVec3::Z.cross(-out), DVec3::Z, -out);
                        let pose = Pose::new(
                            out * radius + DVec3::Z * leg.position,
                            DQuat::from_mat3(&turn),
                        );
                        let node = if k == 0 {
                            node.clone()
                        } else {
                            format!("{node}_left")
                        };
                        let mut child = shaped(&ids[s], &node, strut.clone());
                        child.name = Some(leg.name.clone());
                        child.components = vec![ComponentDef::Gear(GearLegDef {
                            length: leg.length,
                            wheel_radius: leg.wheel_radius,
                            stroke: leg.stroke,
                            load: leg.load,
                            mass: leg.mass / legs as f64,
                            brakes: leg.brakes,
                            steering: leg.steering,
                        })];
                        mounted.push(
                            body.mount(child, &strut, "bottom", pose, &node)
                                .map_err(&bad)?,
                        );
                    }
                }
                PartKind::FairingHalf {
                    mass,
                    radius,
                    length,
                    shells,
                    ..
                } => {
                    let mut sections = vec![ShellSection {
                        length: *length,
                        top_diameter: None,
                        profile: Default::default(),
                    }];
                    sections.extend(shells.iter().map(section));
                    let shape = ShapeDef::Shell(ShellDef {
                        bottom_diameter: 2.0 * radius,
                        sections,
                        span: Some([-90.0, 90.0]),
                        cap_bottom: false,
                        cap_top: false,
                    });
                    let node = name("fairing");
                    let mut child = shaped(&ids[s], &node, shape.clone());
                    child.name = Some(part.name.clone());
                    // A sandwich of composite a few centimetres thick, which weighs alike
                    // all over.
                    child.structure = StructureDef {
                        thickness: Some(FAIRING_WALL),
                        ..mass_only(*mass)
                    };
                    // Bare composite inside and along the cut.
                    child.paint = PaintDef {
                        inside: Some(Look {
                            color: Some(shade(COMPOSITE, 2.0)),
                            finish: Finish::Composite,
                        }),
                        ..paint.clone()
                    };
                    child.components = vec![ComponentDef::Decoupler(DecouplerDef {
                        node: "bottom".into(),
                        impulse: 0.0,
                        crossfeed: false,
                    })];
                    mounted.push(
                        body.mount(child, &shape, "bottom", at, &node)
                            .map_err(&bad)?,
                    );
                }
            }
        }

        for extra in build.extras {
            let child = CraftPartDef {
                name: Some(extra.name.into()),
                ..extra.part
            };
            let node = name(extra.node);
            let child = CraftPartDef {
                id: format!("{}_{node}", ids[s]),
                shape: Some(extra.shape.clone()),
                ..child
            };
            mounted.push(
                body.mount(child, &extra.shape, extra.by, extra.pose, &node)
                    .map_err(&bad)?,
            );
        }

        // The stack: the body and the shells on it, bottom first.
        let pod = mounted.iter().any(|p| p.id == format!("{}_pod", ids[s]));
        let mut stack = vec![body.part(stage)];
        for part in &on {
            let PartKind::Shell(shell) = &part.kind else {
                continue;
            };
            let node = format!("shell_{}", stack.len());
            let mut child = shaped(&ids[s], &node, shell_shape(shell));
            child.name = Some(shell.name.clone());
            child.structure = mass_only(shell.mass);
            child.paint = paint.clone();
            stack.push(child);
        }
        let top = s + 1 == def.stages.len();
        if top {
            for k in 1..stack.len() {
                stack[k].on = Some(format!("{}.top", stack[k - 1].id));
            }
        } else if pod {
            return Err(bad(format!(
                "only the top stage can have a pod, not `{}`",
                stage.name
            )));
        } else {
            let last = stack.len() - 1;
            stack[last].on = Some(format!("{}.bottom", ids[s + 1]));
            stack[last]
                .components
                .push(ComponentDef::Decoupler(DecouplerDef {
                    node: "top".into(),
                    impulse: 0.0,
                    crossfeed: false,
                }));
            for k in 0..last {
                stack[k].on = Some(format!("{}.bottom", stack[k + 1].id));
            }
            for part in &mut stack {
                part.by = "top".into();
            }
        }
        if let Some(livery) = livery {
            let painted = stack.iter().chain(&mounted).filter(|p| {
                p.template.is_none()
                    && p.components.iter().all(|c| {
                        !matches!(c, ComponentDef::LiftingSurface(_) | ComponentDef::Gear(_))
                    })
                    && (p.id == ids[s]
                        || p.components
                            .iter()
                            .any(|c| matches!(c, ComponentDef::Storage(_)))
                        || matches!(p.shape, Some(ShapeDef::Shell(_))))
            });
            liveried.push((livery, painted.map(|p| p.id.clone()).collect()));
        }
        parts.extend(stack);
        parts.extend(mounted);
    }
    let mut craft = CraftDef {
        id: def.id.clone(),
        name: def.name.clone(),
        parts,
        sequences: None,
    };
    paint_liveries(&mut craft, &liveried, catalog).map_err(&bad)?;
    let parts = craft.parts;

    let sequences = def
        .sequences
        .as_ref()
        .map(|list| {
            list.iter()
                .map(|seq| {
                    seq.iter()
                        .map(|a| {
                            let (what, stage) = a.split_once(':').ok_or_else(|| {
                                CraftError::BadSequence(format!("`{a}`: write `<kind>:<stage>`"))
                            })?;
                            let s = def
                                .stages
                                .iter()
                                .position(|s| s.name == stage.trim())
                                .ok_or_else(|| {
                                    CraftError::BadSequence(format!("`{a}`: no such stage"))
                                })?;
                            Ok(format!("{}:{}", what.trim(), ids[s]))
                        })
                        .collect::<Result<Vec<_>, CraftError>>()
                })
                .collect::<Result<Vec<_>, CraftError>>()
        })
        .transpose()?;
    Ok(CraftDef {
        id: def.id.clone(),
        name: def.name.clone(),
        parts,
        sequences,
    })
}

/// Paint each stage's parts with its livery, its layers' heights from the stage's base: the
/// base of its body, the first part in each list.
fn paint_liveries(
    craft: &mut CraftDef,
    liveried: &[(&LiveryDef, Vec<String>)],
    catalog: &crate::data::PartCatalog,
) -> Result<(), String> {
    if liveried.is_empty() {
        return Ok(());
    }
    let built = crate::craft::Craft::build(craft, catalog).map_err(|e| e.to_string())?;
    let poses = built.assembly.poses();
    let height = |id: &str| built.find(id).map(|i| poses[i].position.z);
    for (livery, ids) in liveried {
        let Some(base) = ids.first().and_then(|id| height(id)) else {
            continue;
        };
        for id in ids {
            let rise = height(id).unwrap_or(base) - base;
            let part = craft
                .parts
                .iter_mut()
                .find(|p| &p.id == id)
                .expect("a part of the craft");
            let scheme = livery.paint.lowered(rise);
            let mut pieces = std::mem::take(&mut part.paint.pieces);
            pieces.extend(scheme.pieces.clone());
            part.paint = PaintDef {
                pieces,
                inside: part.paint.inside.or(scheme.inside),
                ..scheme
            };
        }
    }
    Ok(())
}

/// A stage's body while its parts are mounted on it.
struct Body {
    id: String,
    shape: ShapeDef,
    paint: PaintDef,
    /// Its share of the stage's structure mass, kg.
    mass: f64,
    joined: AssemblyPart,
    nodes: Vec<crate::geometry::preset::NodeDef>,
}

impl Body {
    fn new(id: &str, shape: ShapeDef, paint: PaintDef, mass: f64) -> Result<Self, String> {
        let geometry = Arc::new(shape.build()?);
        Ok(Self {
            id: id.into(),
            shape,
            paint,
            mass,
            joined: AssemblyPart::new(id, geometry),
            nodes: Vec::new(),
        })
    }

    /// `child`, of `shape`, held by its node `by` at `pose` from the body's base, on a new
    /// node of the body named `name`.
    fn mount(
        &mut self,
        mut child: CraftPartDef,
        shape: &ShapeDef,
        by: &str,
        pose: Pose,
        name: &str,
    ) -> Result<CraftPartDef, String> {
        let geometry = Arc::new(shape.build()?);
        let mut own = AssemblyPart::new(&child.id, geometry);
        for node in &child.nodes {
            own.extra_nodes.push(node.node()?);
        }
        let mount = Mount::Node(by.into());
        let far = own
            .frame(&mount)
            .ok_or_else(|| format!("`{}` has no node `{by}`", child.id))?;
        let node = own.node(by).expect("framed");
        let held = Node {
            name: name.into(),
            position: pose.point(node.position),
            direction: -(pose.rotation * node.direction),
            size: node.size,
        };
        self.joined.extra_nodes.push(held.clone());
        let near = self
            .joined
            .frame(&Mount::Node(name.into()))
            .expect("just added");
        let roll = roll_to(near, far, pose)
            .ok_or_else(|| format!("`{}` cannot be held where its stage puts it", child.id))?;
        self.nodes.push(node_def(name, &held));
        child.on = Some(format!("{}.{name}", self.id));
        child.by = by.into();
        child.roll = degrees(roll);
        Ok(child)
    }

    /// The body as a part of the craft.
    fn part(&self, stage: &crate::data::StageDef) -> CraftPartDef {
        let mut part = blank(self.id.clone());
        part.shape = Some(self.shape.clone());
        part.name = Some(stage.name.clone());
        // Its walls weigh nothing: the mass is the equipment's.
        part.structure = StructureDef {
            thickness: (self.mass > 0.0).then_some(0.0),
            ..mass_only(self.mass)
        };
        part.components = vec![ComponentDef::Container(ContainerDef::default())];
        part.nodes = self.nodes.clone();
        part.paint = self.paint.clone();
        part
    }
}

/// White paint, black composite, bare titanium, gold foil and solar cells, linear RGB.
const WHITE: [f32; 3] = [0.85, 0.85, 0.83];
const COMPOSITE: [f32; 3] = [0.028, 0.028, 0.032];
const TITANIUM: [f32; 3] = [0.55, 0.55, 0.57];
const GOLD_FOIL: [f32; 3] = [0.85, 0.62, 0.25];
const SOLAR_CELLS: [f32; 3] = [0.03, 0.05, 0.14];
/// Bare aluminium fins, and a wing's painted skin.
const BARE_FIN: [f32; 3] = [0.55, 0.57, 0.6];
const WING_SKIN: [f32; 3] = [0.86, 0.86, 0.84];
/// How tall the band of composite round an engine stage's base is, m, and the ring that
/// lets go of the stage above.
const ENGINE_SECTION: f64 = 0.55;
/// How thick a fairing's wall is, m.
const FAIRING_WALL: f64 = 0.04;
const SEPARATION_RING: f64 = 0.06;

fn shade(color: [f32; 3], by: f32) -> [f32; 3] {
    color.map(|c| c * by)
}

fn look(color: [f32; 3], finish: Finish) -> PaintDef {
    PaintDef {
        color: Some(color),
        finish,
        ..PaintDef::default()
    }
}

/// A stage's paint, from its look.
fn paint_of(stage: &StageDef) -> PaintDef {
    look(stage.appearance.paint.unwrap_or(WHITE), Finish::Paint)
}

/// A stage's structure and what it carries for show.
struct Build {
    shape: ShapeDef,
    paint: PaintDef,
    /// The stage's structure mass, kg.
    mass: f64,
    /// Its `bottom` and `top` nodes, at the base of the stage and where what stands on it
    /// does.
    nodes: Vec<NodeDef>,
    /// How wide it is at its top, m.
    top_radius: f64,
    extras: Vec<Extra>,
}

/// A part on the structure that is there to be seen: a folded leg, a stowed grid fin, a
/// payload's bus.
struct Extra {
    name: &'static str,
    /// Names its node on the structure.
    node: &'static str,
    part: CraftPartDef,
    shape: ShapeDef,
    /// Its node the structure holds.
    by: &'static str,
    /// Where it is, from the stage's base.
    pose: Pose,
}

/// The structure of `stage`, whose tanks are `tanks`, with a pod `pod_height` tall on top
/// (0 without one), and whose next stage up is `above`.
fn structure(
    stage: &StageDef,
    tanks: &[&Tank],
    pod_height: f64,
    above: Option<&StageDef>,
) -> Result<Build, String> {
    let radius = stage.diameter / 2.0;
    let look_of = &stage.appearance;
    let paint = look_of.paint.unwrap_or(WHITE);
    let pod = stage.pod.is_some();
    let height = stage.length - pod_height;
    let mut build = Build {
        shape: ShapeDef::Sphere(SphereDef { diameter: 1.0 }),
        paint: paint_of(stage),
        mass: stage.structure_mass,
        nodes: Vec::new(),
        top_radius: radius,
        extras: Vec::new(),
    };
    let bare = stage.engines.is_empty() && stage.tanks.is_empty() && !pod;
    let (outside, walls, top) = if bare && above.is_none() && stage.fairing.is_some() {
        build.paint = look(COMPOSITE, Finish::Composite);
        let walls = payload(&mut build, radius, height);
        ("adapter", walls, 0.9)
    } else {
        let walls = skin(stage, tanks, above, radius, height, &mut build.paint);
        if above.is_some_and(|a| !a.engines.is_empty()) && stage.shells.is_empty() && !pod {
            build.top_radius = above.map_or(radius, |a| a.diameter / 2.0);
        }
        ("skin", walls, height)
    };
    build.paint.pieces = std::mem::take(&mut build.paint.pieces)
        .into_iter()
        .map(|(piece, look)| (format!("{outside}/{piece}"), look))
        .collect();
    // The structure is drawn as its walls, and weighs as what it holds: a solid cylinder
    // of the whole stage, unseen inside it.
    build.shape = ShapeDef::Compound(vec![
        PlacedShapeDef {
            name: outside.into(),
            shape: ShapeDef::Revolved(RevolvedDef {
                span: None,
                walls,
                regions: Vec::new(),
            }),
            position: [0.0; 3],
            rotation: [0.0; 3],
            unseen: false,
        },
        PlacedShapeDef {
            name: "equipment".into(),
            shape: ShapeDef::Ring(RingDef {
                diameter: stage.diameter,
                height: stage.length,
                thickness: radius,
            }),
            position: [0.0; 3],
            rotation: [0.0; 3],
            unseen: true,
        },
    ]);
    build.nodes = vec![
        NodeDef {
            name: "bottom".into(),
            position: [0.0; 3],
            direction: [0.0, 0.0, -1.0],
            size: stage.diameter,
        },
        // Over a pod, the top of the stage is the top of the pod.
        NodeDef {
            name: "top".into(),
            position: [0.0, 0.0, round(if pod { stage.length } else { top })],
            direction: [0.0, 0.0, 1.0],
            size: 2.0 * round(build.top_radius),
        },
    ];
    // What the stage shows of its look.
    let length = stage.length;
    for k in 0..look_of.legs {
        let angle = std::f64::consts::FRAC_PI_4
            + std::f64::consts::TAU * k as f64 / f64::from(look_of.legs);
        build
            .extras
            .push(leg(radius, (0.23 * length).min(10.0), angle));
    }
    for k in 0..look_of.grid_fins {
        let angle = std::f64::consts::TAU * k as f64 / f64::from(look_of.grid_fins);
        build.extras.push(grid_fin(radius, height - 1.4, angle));
    }
    let tanks_top = tanks
        .iter()
        .filter(|t| t.shape == TankShape::Cylinder)
        .map(|t| t.base + t.length)
        .fold(0.0, f64::max)
        .min(height);
    if look_of.raceway && tanks_top > 1.0 {
        build
            .extras
            .push(raceway(radius, 0.7, tanks_top - 0.2, shade(paint, 0.94)));
    }
    Ok(build)
}

/// The skin of a stage `height` tall where its tanks' barrels are not, the plate its engines
/// hang from or the floor under its tanks, the ring that lets go of the stage above, and a
/// lid where nothing stands on it.
fn skin(
    stage: &StageDef,
    tanks: &[&Tank],
    above: Option<&StageDef>,
    radius: f64,
    height: f64,
    paint: &mut PaintDef,
) -> Vec<WallDef> {
    let near = 1e-6 * radius.max(1.0);
    // Where the tanks' barrels make the outside.
    let mut barrels: Vec<(f64, f64)> = tanks
        .iter()
        .filter(|t| t.shape == TankShape::Cylinder && t.radius >= radius - near)
        .map(|t| {
            let dome = DOME_RATIO * t.radius;
            (t.base + dome, t.base + t.length - dome)
        })
        .filter(|(lo, hi)| hi > lo)
        .collect();
    barrels.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut gaps = Vec::new();
    let mut at = 0.0;
    for (lo, hi) in barrels {
        if lo > at + near {
            gaps.push((at, lo.min(height)));
        }
        at = at.max(hi);
    }
    if at < height - near {
        gaps.push((at, height));
    }
    gaps.retain(|(lo, hi)| hi - lo > near);
    let engines = !stage.engines.is_empty();
    let shells = !stage.shells.is_empty();
    // The interstage round the engines of the stage above narrows to it.
    let houses = above.is_some_and(|a| !a.engines.is_empty()) && !shells && stage.pod.is_none();
    let top_radius = match above {
        Some(a) if houses => a.diameter / 2.0,
        _ => radius,
    };
    let interstage = Look {
        color: Some(stage.appearance.interstage.unwrap_or(COMPOSITE)),
        finish: Finish::Composite,
    };
    let composite = Look {
        color: None,
        finish: Finish::Composite,
    };
    let mut walls: Vec<(String, Vec<[f64; 2]>, Option<Look>)> = Vec::new();
    let mut intertanks = 0;
    for &(lo, hi) in &gaps {
        let top = hi >= height - near;
        let r = |z: f64| {
            if top {
                radius + (top_radius - radius) * ((z - lo) / (hi - lo))
            } else {
                radius
            }
        };
        let mut lo = lo;
        if lo <= near && engines {
            let band = ENGINE_SECTION.min(hi);
            walls.push((
                "engine_section".into(),
                vec![[radius, 0.0], [r(band), band]],
                Some(composite),
            ));
            lo = band;
        }
        let mut hi_main = hi;
        if top && above.is_some() && hi - lo > SEPARATION_RING {
            hi_main = hi - SEPARATION_RING;
        }
        if hi_main > lo + near {
            let (name, look) = if lo <= near || (engines && lo <= ENGINE_SECTION + near) {
                ("aft_skirt".to_string(), None)
            } else if top && houses {
                ("interstage".to_string(), Some(interstage))
            } else if top {
                ("forward_skirt".to_string(), None)
            } else {
                intertanks += 1;
                (format!("intertank_{intertanks}"), None)
            };
            walls.push((name, vec![[r(lo), lo], [r(hi_main), hi_main]], look));
        }
        if hi_main < hi {
            let ring = Look {
                color: Some(TITANIUM),
                finish: Finish::Metal,
            };
            walls.push((
                "separation_ring".into(),
                vec![[r(hi_main), hi_main], [r(hi), hi]],
                Some(ring),
            ));
        }
    }
    // The base: the plate the engines hang from, or the floor under the tanks.
    if engines {
        let plate = Look {
            color: Some(shade(COMPOSITE, 1.4)),
            finish: Finish::Composite,
        };
        walls.push((
            "thrust_plate".into(),
            vec![[0.0, 0.0], [radius, 0.0]],
            Some(plate),
        ));
    } else {
        walls.push(("floor".into(), vec![[0.0, 0.0], [radius, 0.0]], None));
    }
    // A lid where nothing stands on it. A stage that is all pod is only its floor.
    if above.is_none() && !shells && height > near {
        walls.push((
            "lid".into(),
            vec![[top_radius, height], [0.0, height]],
            None,
        ));
    }
    for (name, _, look) in &walls {
        if let Some(look) = look {
            paint.pieces.insert(name.clone(), *look);
        }
    }
    walls
        .into_iter()
        .map(|(name, points, _)| wall(&name, &points))
        .collect()
}

/// A wall through `points` ((radius, height)), rounded to a nanometre.
fn wall(name: &str, points: &[[f64; 2]]) -> WallDef {
    let points: Vec<[f64; 2]> = points.iter().map(|p| p.map(round)).collect();
    WallDef {
        name: name.into(),
        curve: CurveDef {
            start: points[0],
            segments: points[1..].iter().map(|p| SegmentDef::Line(*p)).collect(),
        },
    }
}

/// `x` to the nearest nanometre, the double nearest that decimal.
fn round(x: f64) -> f64 {
    (x * 1e9).round() / 1e9 + 0.0
}

/// A payload under a fairing: an adapter cone on the stage's base, and a satellite on it, in
/// gold foil with its solar panels folded against it and a small antenna on top. Returns
/// the adapter's walls.
fn payload(build: &mut Build, radius: f64, height: f64) -> Vec<WallDef> {
    let adapter = 0.9;
    let top = 0.5 * radius;
    build.top_radius = top;
    let half = 0.45 * radius;
    let bus = (height - adapter).clamp(1.0, 4.5);
    let mut satellite = blank(String::new());
    satellite.paint = look(GOLD_FOIL, Finish::Metal);
    let bus_shape = ShapeDef::Cuboid(BoxDef {
        size: [2.0 * half, 2.0 * half, bus],
        fill: Default::default(),
    });
    build.extras.push(Extra {
        name: "Payload",
        node: "payload",
        part: satellite,
        shape: bus_shape,
        by: "bottom",
        pose: Pose::new(DVec3::Z * adapter, DQuat::IDENTITY),
    });
    for side in [1.0, -1.0] {
        let mut panel = blank(String::new());
        panel.paint = look(SOLAR_CELLS, Finish::Paint);
        let shape = ShapeDef::Cuboid(BoxDef {
            size: [0.05, 1.8 * half, 0.92 * bus],
            fill: Default::default(),
        });
        let at = DVec3::new(side * (half + 0.05), 0.0, adapter + 0.04 * bus);
        build.extras.push(Extra {
            name: "Solar panel",
            node: "panel",
            part: panel,
            shape,
            by: "bottom",
            pose: Pose::new(at, DQuat::IDENTITY),
        });
    }
    let mut antenna = blank(String::new());
    antenna.paint = look(WHITE, Finish::Paint);
    let shape = ShapeDef::Shell(ShellDef {
        bottom_diameter: 1.2 * half,
        sections: vec![ShellSection {
            length: 0.25,
            top_diameter: Some(0.0),
            profile: Default::default(),
        }],
        span: None,
        cap_bottom: true,
        cap_top: false,
    });
    build.extras.push(Extra {
        name: "Antenna",
        node: "antenna",
        part: antenna,
        shape,
        by: "bottom",
        pose: Pose::new(DVec3::Z * (adapter + bus + 0.05), DQuat::IDENTITY),
    });
    vec![
        wall("floor", &[[0.0, 0.0], [radius, 0.0]]),
        wall("adapter", &[[radius, 0.0], [radius, 0.12], [top, adapter]]),
        wall("deck", &[[top, adapter], [0.0, adapter]]),
    ]
}

/// Something mounted on the stage's side at `angle`, `out` from its axis and `up` from its
/// base, by a `mount` node at its origin, its +X out and +Z up the stage.
fn on_side(mut part: CraftPartDef, out: f64, up: f64, angle: f64) -> (CraftPartDef, Pose) {
    part.nodes.push(NodeDef {
        name: "mount".into(),
        position: [0.0; 3],
        direction: [-1.0, 0.0, 0.0],
        size: 0.0,
    });
    let at = (DVec2::from_angle(angle) * out).extend(up);
    (part, Pose::new(at, DQuat::from_rotation_z(angle)))
}

/// A landing leg folded up against the stage at `angle`: a tapered strut from just below
/// its base up to `length`, and a foot.
fn leg(radius: f64, length: f64, angle: f64) -> Extra {
    let (w0, t0, w1, t1) = (0.75, 0.32, 0.34, 0.18);
    let strut = ShapeDef::Lofted(LoftedDef {
        name: "strut".into(),
        section: SectionDef::Polygon(vec![[0.0, -1.0], [1.0, -1.0], [1.0, 1.0], [0.0, 1.0]]),
        stations: vec![
            StationDef {
                z: 0.0,
                offset: [0.0, 0.0],
                scale: [t0, w0],
                twist: 0.0,
            },
            StationDef {
                z: length + 0.5,
                offset: [0.0, 0.0],
                scale: [t1, w1],
                twist: 0.0,
            },
        ],
        fill: Default::default(),
    });
    let foot = ShapeDef::Cuboid(BoxDef {
        size: [0.48, 1.7 * w0, 0.24],
        fill: Default::default(),
    });
    let shape = ShapeDef::Compound(vec![
        PlacedShapeDef {
            name: String::new(),
            shape: strut,
            position: [0.0; 3],
            rotation: [0.0; 3],
            unseen: false,
        },
        PlacedShapeDef {
            name: "foot".into(),
            shape: foot,
            position: [0.6 * t0, 0.0, 0.0],
            rotation: [0.0; 3],
            unseen: false,
        },
    ]);
    let mut part = blank(String::new());
    part.paint = look(COMPOSITE, Finish::Composite);
    part.paint.pieces.insert(
        "foot/box".into(),
        Look {
            color: Some(TITANIUM),
            finish: Finish::Metal,
        },
    );
    let (part, pose) = on_side(part, radius + 0.02, -0.5, angle);
    Extra {
        name: "Landing leg",
        node: "leg",
        part,
        shape,
        by: "mount",
        pose,
    }
}

/// A grid fin stowed flat against the stage at `angle`, its middle `up` from the base: a
/// frame with a diagonal lattice, on an actuator housing.
fn grid_fin(radius: f64, up: f64, angle: f64) -> Extra {
    let (half_w, half_h, standoff, pitch) = (0.7, 0.8, 0.18, 0.22);
    let mut nodes: Vec<[f64; 3]> = Vec::new();
    let mut members = Vec::new();
    let mut node = |s: f64, u: f64| {
        let p = [standoff, s, u].map(round);
        let found = nodes
            .iter()
            .position(|q| (q[1] - s).abs() < 1e-6 && (q[2] - u).abs() < 1e-6);
        found.unwrap_or_else(|| {
            nodes.push(p);
            nodes.len() - 1
        })
    };
    let corners = [
        (-half_w, -half_h),
        (half_w, -half_h),
        (half_w, half_h),
        (-half_w, half_h),
    ];
    for i in 0..4 {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        members.push([node(a.0, a.1), node(b.0, b.1)]);
    }
    let n = ((half_w + half_h) * 2.0 / pitch) as i32;
    for k in -n..=n {
        let c = f64::from(k) * pitch;
        for dir in [1.0, -1.0] {
            // Where the line u = dir·s + c crosses the frame.
            let mut ends = Vec::new();
            for s in [-half_w, half_w] {
                let u = dir * s + c;
                if u.abs() <= half_h {
                    ends.push((s, u));
                }
            }
            for u in [-half_h, half_h] {
                let s = (u - c) * dir;
                if s.abs() <= half_w {
                    ends.push((s, u));
                }
            }
            ends.dedup_by(|p, q| (p.0 - q.0).abs() < 1e-6 && (p.1 - q.1).abs() < 1e-6);
            if ends.len() >= 2 {
                let (a, b) = (ends[0], ends[ends.len() - 1]);
                if (a.0 - b.0).hypot(a.1 - b.1) > 1e-3 {
                    members.push([node(a.0, a.1), node(b.0, b.1)]);
                }
            }
        }
    }
    let housing = ShapeDef::Cuboid(BoxDef {
        size: [0.16, 0.64, 0.44],
        fill: Default::default(),
    });
    let shape = ShapeDef::Compound(vec![
        PlacedShapeDef {
            name: String::new(),
            shape: ShapeDef::Lattice(LatticeDef { nodes, members }),
            position: [0.0; 3],
            rotation: [0.0; 3],
            unseen: false,
        },
        PlacedShapeDef {
            name: "housing".into(),
            shape: housing,
            position: [0.08, 0.0, half_h + 0.25 - 0.22],
            rotation: [0.0; 3],
            unseen: false,
        },
    ]);
    let mut part = blank(String::new());
    part.structure.member_area = Some(PI * 0.025 * 0.025);
    part.paint = look(TITANIUM, Finish::Metal);
    part.paint.pieces.insert(
        "housing/box".into(),
        Look {
            color: Some(shade(COMPOSITE, 1.5)),
            finish: Finish::Composite,
        },
    );
    let (part, pose) = on_side(part, radius, up, angle);
    Extra {
        name: "Grid fin",
        node: "grid_fin",
        part,
        shape,
        by: "mount",
        pose,
    }
}

/// A cable raceway: a low box up the side of the stage from `from` to `to`.
fn raceway(radius: f64, from: f64, to: f64, color: [f32; 3]) -> Extra {
    let shape = ShapeDef::Compound(vec![PlacedShapeDef {
        name: String::new(),
        shape: ShapeDef::Cuboid(BoxDef {
            size: [0.11, 0.3, to - from],
            fill: Default::default(),
        }),
        position: [0.055, 0.0, 0.0],
        rotation: [0.0; 3],
        unseen: false,
    }]);
    let mut part = blank(String::new());
    part.paint = look(color, Finish::Paint);
    let (part, pose) = on_side(part, radius, from, std::f64::consts::FRAC_PI_8);
    Extra {
        name: "Raceway",
        node: "raceway",
        part,
        shape,
        by: "mount",
        pose,
    }
}

fn template(stage: &str, node: &str, id: &str) -> CraftPartDef {
    CraftPartDef {
        template: Some(id.into()),
        ..blank(format!("{stage}_{node}"))
    }
}

fn shaped(stage: &str, node: &str, shape: ShapeDef) -> CraftPartDef {
    CraftPartDef {
        shape: Some(shape),
        ..blank(format!("{stage}_{node}"))
    }
}

fn blank(id: String) -> CraftPartDef {
    CraftPartDef {
        id,
        name: None,
        on: None,
        by: "bottom".into(),
        at: None,
        roll: 0.0,
        template: None,
        shape: None,
        structure: StructureDef::default(),
        components: Vec::new(),
        nodes: Vec::new(),
        paint: PaintDef::default(),
    }
}

fn lifting(control: Option<crate::data::ControlSurfaceDef>) -> ComponentDef {
    ComponentDef::LiftingSurface(LiftingSurfaceDef {
        loft: None,
        control,
    })
}

fn mass_only(mass: f64) -> StructureDef {
    StructureDef {
        mass: (mass > 0.0).then_some(mass),
        ..StructureDef::default()
    }
}

fn shell_shape(shell: &Shell) -> ShapeDef {
    ShapeDef::Shell(ShellDef {
        bottom_diameter: 2.0 * shell.bottom_radius,
        sections: vec![section(shell)],
        span: None,
        cap_bottom: false,
        cap_top: false,
    })
}

fn section(shell: &Shell) -> ShellSection {
    ShellSection {
        length: shell.length,
        top_diameter: (shell.top_radius != shell.bottom_radius).then_some(2.0 * shell.top_radius),
        profile: shell.profile,
    }
}

/// `pose` reflected across the YZ plane.
fn mirror(pose: Pose) -> Pose {
    let flip = DMat3::from_diagonal(DVec3::new(-1.0, 1.0, 1.0));
    let turn = flip * DMat3::from_quat(pose.rotation) * flip;
    Pose::new(flip * pose.position, DQuat::from_mat3(&turn).normalize())
}

/// Radians as degrees from −180 to 180, rounded to a billionth.
fn degrees(radians: f64) -> f64 {
    let wrapped = (radians + PI).rem_euclid(2.0 * PI) - PI;
    (wrapped.to_degrees() * 1e9).round() / 1e9 + 0.0
}

/// `name` as an id: lower case, words joined by underscores.
fn slug(name: &str) -> String {
    let mut id = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() {
            id.extend(c.to_lowercase());
        } else if !id.ends_with('_') && !id.is_empty() {
            id.push('_');
        }
    }
    let id = id.trim_end_matches('_').to_string();
    if id.is_empty() { "stage".into() } else { id }
}

fn unique(id: String, taken: &[String]) -> String {
    let mut candidate = id.clone();
    let mut k = 2;
    while taken.contains(&candidate) {
        candidate = format!("{id}_{k}");
        k += 1;
    }
    candidate
}

#[cfg(test)]
mod tests;
