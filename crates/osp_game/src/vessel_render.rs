//! Vessel visuals: every part drawn at its place on the vessel, from its geometry (see
//! [`crate::part_mesh`]) or its glTF model (engines, RCS blocks and crew pods), control
//! surfaces that move with the controls, landing gear whose struts give and whose nose
//! wheels steer, exhaust plumes from the bottom stage's engines that follow each engine's
//! thrust level, leave along its gimballed nozzle and merge with their neighbours' (see
//! [`crate::plume`]), the light they cast, and puffs from every RCS nozzle that fires. Each
//! part's visuals say which part they show, so the player can right-click it (see
//! [`crate::part_info`]).

use std::{
    collections::{HashMap, HashSet},
    f32::consts::FRAC_PI_2,
};

use bevy::{light::NotShadowCaster, prelude::*};
use osp_sim::{
    DVec3,
    aero::{self, Panel},
    component::{BuiltPart, Component, LayerDef, Look},
    data::{ControlSurfaceDef, EngineDef, TurbineExhaust},
    parts::{Part, PartId, PartKind, PartTree},
    propulsion::{Engine, Propulsion, Section},
    time::SimTime,
    vessel::VesselId,
};

use crate::{
    FrameSet,
    content::{GameContent, mod_asset_path},
    frame::{RenderFrame, rotation_to_render, to_render},
    part_info::{ShowsPart, leg_parts},
    part_mesh::{Canvas, Looks, axes, bevy_mesh, part_meshes},
    plume::{
        AXIS_LAG, Jet, JetKind, MAX_JETS, PlumeCluster, PlumeDef, PlumeMaterial, PlumeParams,
        PlumeQuality, plume_axis, plume_light,
    },
    settings::{GraphicsSettings, Quality},
    sim::Sim,
    vessel_material::{Finish, PartModel, VesselMaterial, VesselMaterialPlugin},
};

/// Exit of each nozzle of an RCS block and the way its gas leaves, in the block model's axes
/// (+X out from the hull, +Y towards the nose), in the order of `RcsBlock::firing`: pushing
/// forward blows aft, pushing back blows forward, and pushing towards positive roll blows
/// along +Z. Matches the stock Cold Gas Quad (and its stand-in box).
const RCS_NOZZLES: [(Vec3, Vec3); 4] = [
    (Vec3::new(0.072, -0.21, 0.0), Vec3::NEG_Y),
    (Vec3::new(0.072, 0.21, 0.0), Vec3::Y),
    (Vec3::new(0.072, 0.0, 0.18), Vec3::Z),
    (Vec3::new(0.072, 0.0, -0.18), Vec3::NEG_Z),
];
/// Illuminance at the edge of a plume light's range, lux.
const PLUME_LIGHT_FLOOR: f32 = 0.1;
/// Length of an RCS puff at full thrust, m, and its width per metre of length.
const RCS_PLUME_LENGTH: f32 = 1.6;
const RCS_PLUME_SPREAD: f32 = 0.35;

pub struct VesselRenderPlugin;

impl Plugin for VesselRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(VesselMaterialPlugin)
            .add_systems(Startup, spawn_vessel_visuals)
            .add_systems(
                Update,
                (
                    sync_visuals,
                    rebuild_on_change,
                    place_vessels,
                    update_engines,
                    update_control_surfaces,
                    update_gear,
                    update_rcs_plumes,
                )
                    .chain()
                    .in_set(FrameSet::Render),
            );
    }
}

#[derive(Component)]
struct VesselVisual {
    id: VesselId,
    /// The parts it was built from (`None`: not built yet). A change (a stage falls away,
    /// fins tear off, the fairing goes) rebuilds it.
    built: Option<Vec<u32>>,
}

/// Pivot of an engine (the part `engine`), at its mount.
#[derive(Component)]
struct EnginePivot {
    vessel: VesselId,
    engine: PartId,
}

/// A control surface on its hinge (at the pivot's origin), turned about `axis` (vessel render
/// axes) by its deflection.
#[derive(Component)]
struct ControlSurfaceVisual {
    vessel: VesselId,
    stage: usize,
    wing: usize,
    panel: usize,
    axis: Vec3,
}

/// A gear leg's strut or wheel, placed each frame where the strut's give puts it.
#[derive(Component)]
struct GearVisual {
    vessel: VesselId,
    leg: usize,
    wheel: bool,
}

/// The exhaust plume of a cluster of a stage's engines that share a look, their jets merged
/// into one volume. It hangs off the vessel, not the pivots: it runs where the
/// exhaust has been going lately and only the jets' starts follow the nozzles.
#[derive(Component)]
struct Plume {
    vessel: VesselId,
    engines: Vec<PlumeEngine>,
    look: PlumeDef,
    seed: f32,
    /// The cluster's nozzle axis smoothed over time, and when it was, while it burns.
    exhaust: Option<(Vec3, SimTime)>,
}

/// An engine of a plume's cluster (the part `engine`): its nozzle exit is `exit` along the
/// bell from the gimbal pivot at `mount` (vessel render axes).
struct PlumeEngine {
    engine: PartId,
    mount: Vec3,
    exit: f32,
    radius: f32,
    /// Nozzle exit pressure at full thrust, Pa.
    exit_pressure: f64,
    /// An open-cycle engine's turbine exhaust: out of a pipe (its outlet in the engine's
    /// axes, which swivel with the nozzle, and its radius), or lining the nozzle wall.
    pipe: Option<(Vec3, f32)>,
    film: bool,
}

impl PlumeEngine {
    fn jets(&self) -> usize {
        1 + self.pipe.is_some() as usize
    }
}

/// The light a vessel's plumes cast, at their middle.
#[derive(Component)]
struct PlumeLight {
    vessel: VesselId,
}

/// The puff from one RCS nozzle. `base` puts the nozzle exit at the origin with the gas
/// leaving along −Y.
#[derive(Component)]
struct RcsPlume {
    vessel: VesselId,
    block: PartId,
    nozzle: usize,
    base: Transform,
}

#[derive(Resource)]
struct VesselMaterials {
    /// A gear leg's strut and wheel.
    strut: Handle<VesselMaterial>,
    wheel: Handle<VesselMaterial>,
    /// Engines without a model of their own.
    engine: Handle<VesselMaterial>,
    rcs_plume: Handle<StandardMaterial>,
}

fn spawn_vessel_visuals(
    mut commands: Commands,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut materials: ResMut<Assets<VesselMaterial>>,
) {
    let mut add = |finish: Finish, base: StandardMaterial| {
        materials.add(VesselMaterial {
            base,
            extension: finish.extension(),
        })
    };
    commands.insert_resource(Looks::default());
    commands.insert_resource(VesselMaterials {
        strut: add(
            Finish::Metal,
            StandardMaterial {
                metallic: 0.85,
                perceptual_roughness: 0.48,
                ..default()
            },
        ),
        wheel: add(
            Finish::Paint,
            StandardMaterial {
                base_color: Color::srgb(0.25, 0.25, 0.28),
                perceptual_roughness: 0.6,
                ..default()
            },
        ),
        engine: add(
            Finish::HeatTint(20.0, 90.0),
            StandardMaterial {
                base_color: Color::srgb(0.15, 0.13, 0.12),
                metallic: 0.8,
                perceptual_roughness: 0.4,
                ..default()
            },
        ),
        // Faint and unlit, so a firing nozzle shows on the night side too.
        rcs_plume: standard.add(StandardMaterial {
            base_color: Color::linear_rgba(0.55, 0.6, 0.65, 0.5),
            alpha_mode: AlphaMode::Add,
            unlit: true,
            ..default()
        }),
    });
}

/// Exit radius and length of an engine's bell. Engines without real dimensions get a bell
/// that keeps a stage's engines from overlapping each other or the hull.
fn bell_size(propulsion: &Propulsion, stage: &Section, engine: &Engine) -> (f32, f32) {
    let def = &engine.def;
    if def.exit_diameter > 0.0 && def.length > 0.0 {
        return ((def.exit_diameter / 2.0) as f32, def.length as f32);
    }
    let radius = stage.diameter / 2.0;
    let mounts: Vec<_> = propulsion
        .engines_of(stage.id)
        .map(|e| (e.mount - stage.bottom).truncate())
        .collect();
    let mut room = (radius * 0.8).min(1.5);
    for (i, a) in mounts.iter().enumerate() {
        room = room.min(radius - a.length());
        for b in &mounts[i + 1..] {
            room = room.min(0.45 * a.distance(*b));
        }
    }
    let bell = room.max(0.1) as f32;
    (bell, bell * 1.6)
}

fn rebuild_on_change(
    mut commands: Commands,
    sim: Res<Sim>,
    content: Res<GameContent>,
    assets: Res<AssetServer>,
    materials: Res<VesselMaterials>,
    mut looks: ResMut<Looks>,
    mut vessel_materials: ResMut<Assets<VesselMaterial>>,
    mut plume_materials: ResMut<Assets<PlumeMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut visuals: Query<(Entity, &mut VesselVisual)>,
) {
    // A mod's file, such as a paint layer's picture.
    let file = |path: &str| {
        content
            .0
            .resolve_file(path)
            .and_then(|f| std::fs::read(f).ok())
    };
    let plume_looks = content
        .0
        .registry
        .all::<PlumeDef>("plumes", &mut Vec::new());
    let look = |engine: &EngineDef| {
        let burns: Vec<&str> = engine.propellants.keys().map(String::as_str).collect();
        let look = PlumeDef::for_engine(
            &plume_looks,
            engine.plume.as_deref(),
            &burns,
            engine.kind.as_deref(),
        );
        if look.is_none() {
            if let Some(id) = &engine.plume {
                warn!("plume `{id}` is not defined; the engine has no plume");
            } else if engine.kind.as_deref() != Some("electric") {
                warn!("no plume look fits engine `{}`; it has no plume", engine.id);
            }
        }
        look.cloned()
    };
    let model = |relative: &Option<String>| {
        let relative = relative.as_deref()?;
        let Some(path) = mod_asset_path(&content.0, relative) else {
            warn!("model `{relative}` is not in any mod; drawing a stand-in");
            return None;
        };
        Some(WorldAssetRoot(
            assets.load(GltfAssetLabel::Scene(0).from_asset(path)),
        ))
    };
    for (entity, mut visual) in &mut visuals {
        let Some(propulsion) = sim.universe.vessel(visual.id).map(|v| &v.propulsion) else {
            continue;
        };
        let (stages, tree) = (&propulsion.sections, propulsion.parts());
        // Parts fall away, fins tear off, fairings are jettisoned: then it is built again.
        let shape: Vec<u32> = tree.parts().iter().map(|p| p.id.0).collect();
        if visual.built.as_ref() == Some(&shape) {
            continue;
        }
        visual.built = Some(shape);
        let vessel = visual.id;

        let puff = meshes.add(Cone::new(1.0, 1.0));
        // From the nozzle exit (y = 0) down to y = −1, scaled to each plume's bounds.
        let plume_volume = meshes.add(
            Mesh::from(Cylinder::new(1.0, 1.0).mesh().resolution(24))
                .translated_by(Vec3::NEG_Y * 0.5),
        );
        let mut entity_commands = commands.entity(entity);
        entity_commands.despawn_children();
        entity_commands.with_children(|parent| {
            // The origin is the base of the stack, +Y up it (sim +Z, the nose); vessel axes
            // map to these local axes like sim to render: X → X, Y → −Z.
            let base = DVec3::Z * tree.base();
            // Each stage's engines by look, for one plume each.
            let mut clusters: Vec<(usize, PlumeDef, Vec<PlumeEngine>)> = Vec::new();
            // How many parts of each kind each stage has had so far: its stage lists them
            // in the same order.
            let mut counts: HashMap<(usize, &'static str), usize> = HashMap::new();
            let mut next = |stage: usize, kind: &'static str| {
                let k = counts.entry((stage, kind)).or_default();
                *k += 1;
                *k - 1
            };
            for part in tree.parts() {
                let Some(built) = &part.built else {
                    continue;
                };
                let Some(index) = stages.iter().position(|s| s.id == part.stage) else {
                    continue;
                };
                let stage = &stages[index];
                let pose = Transform::from_translation(to_render(part.position - base))
                    .with_rotation(rotation_to_render(part.rotation));
                let shows = ShowsPart {
                    vessel,
                    part: part.id,
                };
                let mut pen = Pen {
                    meshes: &mut meshes,
                    looks: &mut looks,
                    canvas: Canvas {
                        materials: &mut vessel_materials,
                        images: &mut images,
                        file: &file,
                    },
                };
                match &part.kind {
                    PartKind::Engine(def) => {
                        let mut pivot = parent.spawn((
                            pose,
                            Visibility::default(),
                            shows,
                            EnginePivot {
                                vessel,
                                engine: part.id,
                            },
                        ));
                        pivot.with_children(|pivot| match model(&def.model) {
                            Some(scene) => {
                                let scale = def.model_scale as f32;
                                pivot.spawn((
                                    scene,
                                    PartModel,
                                    Transform::from_scale(Vec3::splat(scale)),
                                ));
                            }
                            None => pen.draw(pivot, built, Some(&materials.engine)),
                        });
                        let Some(engine) = propulsion.engines.iter().find(|e| e.part == part.id)
                        else {
                            continue;
                        };
                        if let Some(look) = look(&engine.def) {
                            let (exit, bell) = bell_size(propulsion, stage, engine);
                            let exhaust = engine.def.turbine_exhaust.as_ref();
                            let pipe = match exhaust {
                                Some(TurbineExhaust::Pipe { exit, diameter }) => Some((
                                    Vec3::new(exit[0] as f32, exit[1] as f32, exit[2] as f32),
                                    (diameter / 2.0) as f32,
                                )),
                                _ => None,
                            };
                            let jet = PlumeEngine {
                                engine: part.id,
                                mount: pose.translation,
                                exit: bell,
                                radius: exit,
                                exit_pressure: engine.def.exit_pressure(),
                                pipe,
                                film: exhaust == Some(&TurbineExhaust::Nozzle),
                            };
                            let same = |(s, l, _): &&mut (usize, PlumeDef, _)| {
                                *s == part.stage && l.id == look.id
                            };
                            match clusters.iter_mut().find(same) {
                                Some((_, _, engines)) => engines.push(jet),
                                None => clusters.push((part.stage, look, vec![jet])),
                            }
                        }
                    }
                    PartKind::Pod(def) if def.model.is_some() => {
                        let scene = model(&def.model);
                        let mut visual = parent.spawn((pose, Visibility::default(), shows));
                        visual.with_children(|visual| match scene {
                            Some(scene) => {
                                visual.spawn((scene, PartModel));
                            }
                            None => pen.draw(visual, built, None),
                        });
                    }
                    PartKind::Rcs(def) => {
                        for (n, (exit, out)) in RCS_NOZZLES.into_iter().enumerate() {
                            parent.spawn((
                                RcsPlume {
                                    vessel,
                                    block: part.id,
                                    nozzle: n,
                                    base: pose
                                        * Transform::from_translation(exit).with_rotation(
                                            Quat::from_rotation_arc(Vec3::NEG_Y, out),
                                        ),
                                },
                                Mesh3d(puff.clone()),
                                MeshMaterial3d(materials.rcs_plume.clone()),
                                Transform::default(),
                                Visibility::Hidden,
                            ));
                        }
                        let scene = model(&def.model);
                        let mut visual = parent.spawn((pose, Visibility::default(), shows));
                        visual.with_children(|visual| match scene {
                            Some(scene) => {
                                visual.spawn((scene, PartModel));
                            }
                            None => pen.draw(visual, built, None),
                        });
                    }
                    // Drawn with the leg's give (`update_gear`).
                    PartKind::Gear(_) => {}
                    PartKind::Wing(_) | PartKind::Mirror => {
                        // The right half of a wing is its stage's wing; the left half is
                        // the other panel of the right half's.
                        let (wing, panel) = match &part.kind {
                            PartKind::Wing(_) => (Some(next(part.stage, "wing")), 0),
                            _ => (wing_of_mirror(tree, part), 1),
                        };
                        let def = wing.and_then(|w| stage.wings.get(w));
                        let control = def.and_then(|d| d.control.as_ref());
                        let mut visual = parent.spawn((pose, Visibility::default(), shows));
                        visual.with_children(|visual| match (control, wing) {
                            (Some(control), Some(w)) => {
                                let at = (index, w, panel);
                                hinged(visual, built, control, at, vessel, &mut pen);
                            }
                            _ => pen.draw(visual, built, None),
                        });
                    }
                    _ => {
                        let mut visual = parent.spawn((pose, Visibility::default(), shows));
                        visual.with_children(|visual| pen.draw(visual, built, None));
                    }
                }
            }
            for (k, (_, look, mut engines)) in clusters.drain(..).enumerate() {
                while !engines.is_empty() {
                    // As many engines as fit, with their turbine exhaust pipes.
                    let mut jets = 0;
                    let fit = engines
                        .iter()
                        .take_while(|e| {
                            jets += e.jets();
                            jets <= MAX_JETS
                        })
                        .count()
                        .max(1);
                    let rest = engines.split_off(fit);
                    parent.spawn((
                        Plume {
                            vessel,
                            engines,
                            look: look.clone(),
                            seed: 7.31 * k as f32,
                            exhaust: None,
                        },
                        Mesh3d(plume_volume.clone()),
                        MeshMaterial3d(plume_materials.add(PlumeMaterial::default())),
                        Transform::default(),
                        Visibility::Hidden,
                        NotShadowCaster,
                    ));
                    engines = rest;
                }
            }
            let strut = meshes.add(Cylinder::new(1.0, 1.0));
            let leg_parts = leg_parts(tree, stages);
            for (k, leg) in osp_sim::gear::legs(stages).iter().enumerate() {
                let r = leg.wheel_radius as f32;
                let shows = leg_parts
                    .get(k)
                    .copied()
                    .flatten()
                    .map(|part| ShowsPart { vessel, part });
                let mut visual = parent.spawn((
                    GearVisual {
                        vessel,
                        leg: k,
                        wheel: false,
                    },
                    Mesh3d(strut.clone()),
                    MeshMaterial3d(materials.strut.clone()),
                    Transform::default(),
                ));
                if let Some(shows) = shows {
                    visual.insert(shows);
                }
                let mut visual = parent.spawn((
                    GearVisual {
                        vessel,
                        leg: k,
                        wheel: true,
                    },
                    Mesh3d(meshes.add(Cylinder::new(r, 0.7 * r).mesh().resolution(24))),
                    MeshMaterial3d(materials.wheel.clone()),
                    Transform::default(),
                ));
                if let Some(shows) = shows {
                    visual.insert(shows);
                }
            }
            parent.spawn((
                PlumeLight { vessel },
                PointLight {
                    intensity: 0.0,
                    range: 400.0,
                    radius: 1.0,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::default(),
            ));
        });
    }
}

/// What parts are drawn with: their meshes and the materials of their looks.
struct Pen<'a> {
    meshes: &'a mut Assets<Mesh>,
    looks: &'a mut Looks,
    canvas: Canvas<'a>,
}

impl Pen<'_> {
    /// `part`'s pieces as children of `parent`, each in its look, or all in `material`.
    fn draw(
        &mut self,
        parent: &mut ChildSpawnerCommands,
        part: &BuiltPart,
        material: Option<&Handle<VesselMaterial>>,
    ) {
        for (mesh, look, layers) in part_meshes(part) {
            self.mesh(parent, &mesh, Vec3::ZERO, (look, layers), material);
        }
    }

    fn mesh(
        &mut self,
        parent: &mut ChildSpawnerCommands,
        mesh: &osp_sim::geometry::draw::DrawMesh,
        offset: Vec3,
        (look, layers): (Look, &[LayerDef]),
        material: Option<&Handle<VesselMaterial>>,
    ) {
        let material = material
            .cloned()
            .unwrap_or_else(|| self.looks.material(look, layers, &mut self.canvas));
        parent.spawn((
            Mesh3d(self.meshes.add(bevy_mesh(mesh, offset))),
            MeshMaterial3d(material),
        ));
    }
}

/// Which of its stage's wings the mirror image `part` is the left half of: the one whose
/// right half sits across the vessel's plane of symmetry from it.
fn wing_of_mirror(tree: &PartTree, part: &Part) -> Option<usize> {
    let across = part.position * DVec3::new(-1.0, 1.0, 1.0);
    tree.stage(part.stage)
        .filter(|p| matches!(p.kind, PartKind::Wing(_)))
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let (a, b) = (a.position.distance(across), b.position.distance(across));
            a.total_cmp(&b)
        })
        .map(|(w, _)| w)
}

/// A lifting surface with a control surface, in its part's axes: the wing cut along its
/// hinge, and the control surface on a pivot at the hinge that turns it with the controls.
/// `at` is (stage index, wing, panel) for [`ControlSurfaceVisual`].
fn hinged(
    parent: &mut ChildSpawnerCommands,
    built: &BuiltPart,
    control: &ControlSurfaceDef,
    (stage, wing, panel): (usize, usize, usize),
    vessel: VesselId,
    pen: &mut Pen,
) {
    let Some(surface) = built.components.iter().find_map(|c| match c {
        Component::LiftingSurface(s) => Some(s),
        _ => None,
    }) else {
        return;
    };
    let geometry = &built.geometry;
    let name = surface.loft.as_str();
    let (skin, _) = built.paint.of(name);
    let flap_look = Look {
        color: Some(skin.color().map(|c| c * 0.82)),
        ..skin
    };
    // Shares of the span from the loft's first station, which is the tip on a mirrored
    // wing.
    let share = |s: f64| if surface.reversed { 1.0 - s } else { s };
    let span = |a: f64, b: f64| {
        let (a, b) = (share(a), share(b));
        [a.min(b), a.max(b)]
    };
    let [inner, outer] = control.span;
    let hinge = 1.0 - control.chord;
    let fixed = [
        (span(0.0, inner), [0.0, 1.0]),
        (span(inner, outer), [0.0, hinge]),
        (span(outer, 1.0), [0.0, 1.0]),
    ];
    for (along, chord) in fixed {
        if along[1] - along[0] > 1e-6
            && let Some(mesh) = geometry.draw_loft(name, along, chord)
            && !mesh.is_empty()
        {
            pen.mesh(parent, &mesh, Vec3::ZERO, (skin, &[]), None);
        }
    }
    let (Some(a), Some(b), Some(flap)) = (
        geometry.loft_point(name, share(inner), hinge),
        geometry.loft_point(name, share(outer), hinge),
        geometry.draw_loft(name, span(inner, outer), [hinge, 1.0]),
    ) else {
        return;
    };
    let Some(planform) = surface.panels.first().filter(|_| !flap.is_empty()) else {
        return;
    };
    // Turning the trailing edge (aft of the hinge) away from the lifting side.
    let line = (b - a).normalize();
    let sign = -line.cross(planform.chord).dot(planform.normal).signum();
    let start = axes(a.as_vec3().to_array());
    parent
        .spawn((
            ControlSurfaceVisual {
                vessel,
                stage,
                wing,
                panel,
                axis: axes((line * sign).as_vec3().to_array()),
            },
            Transform::from_translation(start),
            Visibility::default(),
        ))
        .with_children(|pivot| pen.mesh(pivot, &flap, start, (flap_look, &[]), None));
}

fn place_vessels(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    mut visuals: Query<(&VesselVisual, &mut Transform)>,
) {
    if frame.origin_body.is_none() {
        return;
    }
    let system = &sim.universe.system;
    for (visual, mut transform) in &mut visuals {
        let Some(vessel) = sim.universe.vessel(visual.id) else {
            continue;
        };
        let base = vessel.base_at(system, frame.time);
        transform.translation = frame.to_render(vessel.parent(), base);
        transform.rotation = rotation_to_render(vessel.attitude_at(system, frame.time));
    }
}

/// Swivel the bottom stage's engines to their gimbal angles, shape their plumes to each
/// engine's thrust level, the ambient pressure and the airflow, and light the vessel with
/// them.
fn update_engines(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    mut pivots: Query<(&EnginePivot, &mut Transform), Without<Plume>>,
    mut plumes: Query<
        (
            &mut Plume,
            &MeshMaterial3d<PlumeMaterial>,
            &mut Transform,
            &mut Visibility,
        ),
        Without<PlumeLight>,
    >,
    mut lights: Query<(&PlumeLight, &mut PointLight, &mut Transform), Without<EnginePivot>>,
    mut materials: ResMut<Assets<PlumeMaterial>>,
    settings: Res<GraphicsSettings>,
) {
    let system = &sim.universe.system;
    let quality = PlumeQuality::new(settings.effects);
    let engine = |vessel: VesselId, part: PartId| {
        sim.universe
            .vessel(vessel)?
            .propulsion
            .engines
            .iter()
            .find(|e| e.part == part)
    };
    for (pivot, mut transform) in &mut pivots {
        if let Some(engine) = engine(pivot.vessel, pivot.engine) {
            transform.rotation = Quat::from_rotation_arc(Vec3::NEG_Y, nozzle_axis(engine));
        }
    }
    // Per vessel: height of the base above the ground, m, the local vertical in vessel
    // render axes and in the world's, and the air's velocity past the vessel (vessel render
    // axes), m/s.
    let around: HashMap<VesselId, (f64, Vec3, Vec3, Vec3)> = sim
        .universe
        .vessels
        .iter()
        .map(|v| {
            let body = system.body(v.parent());
            let base = v.base_at(system, frame.time);
            let fixed = body.orientation(frame.time).inverse() * base.normalize();
            let height = base.length() - body.radius - body.terrain_height(fixed);
            let into = v.attitude_at(system, frame.time).inverse();
            let state = v.state_at(system, frame.time);
            let airflow = body.surface_velocity(state.position) - state.velocity;
            let around = (
                height,
                to_render(into * base.normalize()).normalize(),
                to_render(base.normalize()).normalize(),
                to_render(into * airflow),
            );
            (v.id(), around)
        })
        .collect();
    // Per vessel: lumens, and lumen-weighted colour and position.
    let mut glow: HashMap<VesselId, (f32, Vec3, Vec3)> = HashMap::new();
    for (mut plume, material, mut transform, mut visibility) in &mut plumes {
        let (Some(vessel), Some(&(height, up, world_up, airflow))) =
            (sim.universe.vessel(plume.vessel), around.get(&plume.vessel))
        else {
            continue;
        };
        // The jets of the engines that burn, and their thrust and exit area.
        let mut jets = Vec::new();
        let (mut thrust, mut area) = (0.0, 0.0);
        for e in &plume.engines {
            let Some(engine) = engine(plume.vessel, e.engine).filter(|e| e.level > 0.0) else {
                continue;
            };
            let nozzle = nozzle_axis(engine);
            jets.push(Jet {
                exit: e.mount + nozzle * e.exit,
                nozzle,
                radius: e.radius,
                exit_pressure: e.exit_pressure,
                level: engine.level,
                kind: JetKind::Main { film: e.film },
            });
            if let Some((outlet, radius)) = e.pipe {
                let swivel = Quat::from_rotation_arc(Vec3::NEG_Y, nozzle);
                jets.push(Jet {
                    exit: e.mount + swivel * outlet,
                    nozzle,
                    radius,
                    // A low-pressure duct.
                    exit_pressure: 80_000.0,
                    level: engine.level,
                    kind: JetKind::TurbineExhaust,
                });
            }
            thrust += engine.thrust as f32;
            area += std::f32::consts::PI * e.radius * e.radius;
        }
        if jets.is_empty() {
            *visibility = Visibility::Hidden;
            plume.exhaust = None;
            continue;
        }
        *visibility = Visibility::Inherited;
        let nozzle = jets
            .iter()
            .map(|j| j.nozzle * j.level as f32)
            .sum::<Vec3>()
            .normalize_or(Vec3::NEG_Y);
        let exhaust = match plume.exhaust {
            Some((last, at)) => {
                let dt = frame.time.seconds_since(at) as f32;
                last.lerp(nozzle, 1.0 - (-dt.max(0.0) / AXIS_LAG).exp())
                    .normalize()
            }
            None => nozzle,
        };
        plume.exhaust = Some((exhaust, frame.time));
        let jet = thrust / area.max(1e-6);
        let axis = plume_axis(exhaust, airflow, vessel.flight.dynamic_pressure as f32, jet);
        let cluster = PlumeCluster::new(
            &jets,
            axis,
            vessel.flight.static_pressure,
            height as f32,
            up,
        );
        let end = cluster.end_radius();
        *transform = Transform::from_translation(cluster.origin)
            .with_rotation(cluster.rotation)
            .with_scale(Vec3::new(end, cluster.length, end));
        if let Some(mut material) = materials.get_mut(&material.0) {
            material.params =
                PlumeParams::new(&cluster, &plume.look, plume.seed, quality, world_up);
        }
        let (lumens, color, middle) = plume_light(&cluster, &plume.look);
        let (sum, colors, at) = glow.entry(plume.vessel).or_default();
        *sum += lumens;
        *colors += lumens * color;
        *at += lumens * (cluster.origin + axis * middle);
    }
    for (light, mut point, mut transform) in &mut lights {
        let (lumens, color, at) = glow.get(&light.vessel).copied().unwrap_or_default();
        point.intensity = lumens;
        if lumens > 0.0 {
            let c = color / lumens;
            point.color = Color::linear_rgb(c.x, c.y, c.z);
            transform.translation = at / lumens;
            // Out to where it lights the ground about as much as a full moon.
            point.range = (lumens / (4.0 * std::f32::consts::PI * PLUME_LIGHT_FLOOR))
                .sqrt()
                .clamp(50.0, 50_000.0);
        }
        // The tower and the vessel cast the flame's shadows (six more shadow views).
        point.shadow_maps_enabled = false && lumens > 0.0 && settings.shadows >= Quality::High;
    }
}

/// One visual per vessel: vessels that are new (debris, a scenario started afresh) get
/// theirs, and the visuals of vessels that are gone go with them.
fn sync_visuals(mut commands: Commands, sim: Res<Sim>, visuals: Query<(Entity, &VesselVisual)>) {
    let mut shown = HashSet::new();
    for (entity, visual) in &visuals {
        if sim.universe.vessel(visual.id).is_some() {
            shown.insert(visual.id);
        } else {
            commands.entity(entity).despawn();
        }
    }
    for vessel in &sim.universe.vessels {
        if !shown.contains(&vessel.id()) {
            commands.spawn((
                VesselVisual {
                    id: vessel.id(),
                    built: None,
                },
                Transform::default(),
                Visibility::default(),
            ));
        }
    }
}

/// Put each gear leg's strut between its mount and its axle as far as the strut has given,
/// and its wheel on the axle, turned by the steering.
fn update_gear(sim: Res<Sim>, mut parts: Query<(&GearVisual, &mut Transform)>) {
    for (visual, mut transform) in &mut parts {
        let Some(vessel) = sim.universe.vessel(visual.vessel) else {
            continue;
        };
        let legs = osp_sim::gear::legs(&vessel.propulsion.sections);
        let Some(leg) = legs.get(visual.leg) else {
            continue;
        };
        let give = vessel
            .gear_compression()
            .get(visual.leg)
            .copied()
            .unwrap_or(0.0)
            .min(leg.stroke);
        let (mount, axle) = (to_render(leg.mount), to_render(leg.axle(give)));
        let along = to_render(leg.direction);
        *transform = if visual.wheel {
            let steer = (leg.steering * vessel.flight.control.y.clamp(-1.0, 1.0)) as f32;
            Transform::from_translation(axle).with_rotation(
                Quat::from_axis_angle(along, steer) * Quat::from_rotation_z(FRAC_PI_2),
            )
        } else {
            let width = 0.25 * leg.wheel_radius as f32;
            Transform::from_translation((mount + axle) / 2.0)
                .with_rotation(Quat::from_rotation_arc(Vec3::Y, along))
                .with_scale(Vec3::new(width, mount.distance(axle), width))
        };
    }
}

/// Turn each control surface to the deflection the controls ask of it.
fn update_control_surfaces(
    sim: Res<Sim>,
    mut surfaces: Query<(&ControlSurfaceVisual, &mut Transform)>,
) {
    for (visual, mut transform) in &mut surfaces {
        let Some(vessel) = sim.universe.vessel(visual.vessel) else {
            continue;
        };
        let stages = &vessel.propulsion.sections;
        let Some(stage) = stages.get(visual.stage) else {
            continue;
        };
        let Some(wing) = stage.wings.get(visual.wing) else {
            continue;
        };
        let Some(control) = &wing.control else {
            continue;
        };
        let base = stage.bottom.z - osp_sim::propulsion::floor(stages);
        let Some(panel) = Panel::of(wing, stage.diameter / 2.0, base)
            .get(visual.panel)
            .copied()
        else {
            continue;
        };
        let com = vessel.mass_properties().center_of_mass.z;
        let angle = aero::deflection(control, &panel, vessel.flight.control, com);
        transform.rotation = Quat::from_axis_angle(visual.axis, angle as f32);
    }
}

/// Where an engine's bell points, along its exhaust (opposite the tilted thrust), in vessel
/// render axes.
fn nozzle_axis(engine: &Engine) -> Vec3 {
    let angle = engine.deflection.length();
    let side = engine.deflection.normalize_or_zero() * angle.sin();
    -to_render(DVec3::new(side.x, side.y, angle.cos())).normalize()
}

/// Size each RCS puff by how hard its nozzle fired on the last physics tick.
fn update_rcs_plumes(
    sim: Res<Sim>,
    mut plumes: Query<(&RcsPlume, &mut Transform, &mut Visibility)>,
) {
    for (plume, mut transform, mut visibility) in &mut plumes {
        let firing = sim
            .universe
            .vessel(plume.vessel)
            .and_then(|v| v.propulsion.rcs.iter().find(|b| b.part == plume.block))
            .map_or(0.0, |b| b.firing[plume.nozzle]) as f32;
        if firing <= 0.0 {
            *visibility = Visibility::Hidden;
            continue;
        }
        *visibility = Visibility::Inherited;
        // The cone's apex (+Y) sits on the nozzle and it widens along the gas (−Y).
        let length = RCS_PLUME_LENGTH * firing;
        let width = length * RCS_PLUME_SPREAD;
        *transform = plume.base
            * Transform::from_translation(Vec3::NEG_Y * length / 2.0)
                .with_scale(Vec3::new(width, length, width));
    }
}
