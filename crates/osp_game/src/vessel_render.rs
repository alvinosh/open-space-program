//! Vessel visuals: a procedural hull per stage (see [`crate::hull`]), engines, RCS blocks
//! and crew pods from their glTF models (or plain stand-ins without one), procedural fins,
//! exhaust plumes
//! from the bottom stage's engines that swivel with their gimbal and follow each engine's
//! thrust level (see [`crate::plume`]), the light they cast, and puffs from every RCS
//! nozzle that fires.

use std::f32::consts::TAU;

use bevy::{light::NotShadowCaster, prelude::*};
use osp_sim::{DVec3, propulsion::Stage, structure::FinSet};

use crate::{
    FrameSet,
    content::{GameContent, mod_asset_path},
    frame::{RenderFrame, rotation_to_render, to_render},
    hull::stage_hull,
    meshgen::MeshBuilder,
    plume::{PlumeDef, PlumeMaterial, PlumeParams, PlumeShape, plume_lumens},
    sim::Sim,
};

/// How far below a stage's forward rim its RCS blocks sit, m.
const RCS_INSET: f32 = 0.5;
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
/// Length of an RCS puff at full thrust, m, and its width per metre of length.
const RCS_PLUME_LENGTH: f32 = 1.6;
const RCS_PLUME_SPREAD: f32 = 0.35;

pub struct VesselRenderPlugin;

impl Plugin for VesselRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_vessel_visuals).add_systems(
            Update,
            (
                rebuild_on_change,
                place_vessels,
                update_engines,
                update_rcs_plumes,
            )
                .chain()
                .in_set(FrameSet::Render),
        );
    }
}

#[derive(Component)]
struct VesselVisual {
    index: usize,
    /// Which stages had fins when it was built, bottom first (`None`: not built yet).
    built: Option<Vec<bool>>,
}

/// Pivot of an engine of the bottom stage, at its mount on the stage base.
#[derive(Component)]
struct EnginePivot {
    vessel: usize,
    engine: usize,
}

/// An exhaust plume starting at its nozzle exit, `exit` below the pivot.
#[derive(Component)]
struct Plume {
    exit: f32,
    radius: f32,
    /// Nozzle exit pressure at full thrust, Pa.
    exit_pressure: f64,
    look: PlumeDef,
    seed: f32,
}

/// The light a vessel's plumes cast, at their middle.
#[derive(Component)]
struct PlumeLight {
    vessel: usize,
}

/// The puff from one RCS nozzle. `base` puts the nozzle exit at the origin with the gas
/// leaving along −Y.
#[derive(Component)]
struct RcsPlume {
    vessel: usize,
    stage: usize,
    block: usize,
    nozzle: usize,
    base: Transform,
}

#[derive(Resource)]
struct VesselMaterials {
    /// Stand-ins for missing models.
    hull: [Handle<StandardMaterial>; 2],
    /// Hull meshes' vertex colours under glossy paint, matte composite and bare metal.
    paint: Handle<StandardMaterial>,
    matte: Handle<StandardMaterial>,
    metal: Handle<StandardMaterial>,
    engine: Handle<StandardMaterial>,
    rcs_plume: Handle<StandardMaterial>,
    /// White, so the fins' vertex colours show.
    fins: Handle<StandardMaterial>,
}

fn spawn_vessel_visuals(
    mut commands: Commands,
    sim: Res<Sim>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(VesselMaterials {
        hull: [
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.92, 0.92, 0.9),
                perceptual_roughness: 0.6,
                ..default()
            }),
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.25, 0.25, 0.28),
                perceptual_roughness: 0.6,
                ..default()
            }),
        ],
        paint: materials.add(StandardMaterial {
            perceptual_roughness: 0.42,
            clearcoat: 0.6,
            clearcoat_perceptual_roughness: 0.25,
            ..default()
        }),
        matte: materials.add(StandardMaterial {
            perceptual_roughness: 0.68,
            reflectance: 0.35,
            ..default()
        }),
        metal: materials.add(StandardMaterial {
            metallic: 0.85,
            perceptual_roughness: 0.48,
            ..default()
        }),
        engine: materials.add(StandardMaterial {
            base_color: Color::srgb(0.15, 0.13, 0.12),
            metallic: 0.8,
            perceptual_roughness: 0.4,
            ..default()
        }),
        // Faint and unlit, so a firing nozzle shows on the night side too.
        rcs_plume: materials.add(StandardMaterial {
            base_color: Color::linear_rgba(0.55, 0.6, 0.65, 0.5),
            alpha_mode: AlphaMode::Add,
            unlit: true,
            ..default()
        }),
        fins: materials.add(StandardMaterial {
            perceptual_roughness: 0.5,
            metallic: 0.3,
            ..default()
        }),
    });
    for index in 0..sim.universe.vessels.len() {
        commands.spawn((
            VesselVisual { index, built: None },
            Transform::default(),
            Visibility::default(),
        ));
    }
}

/// Exit radius and length of an engine's bell. Engines without real dimensions get a bell
/// that keeps a stage's engines from overlapping each other or the hull.
fn bell_size(stage: &Stage, engine: usize) -> (f32, f32) {
    let def = &stage.engines[engine].def;
    if def.exit_diameter > 0.0 && def.length > 0.0 {
        return ((def.exit_diameter / 2.0) as f32, def.length as f32);
    }
    let radius = stage.diameter / 2.0;
    let mounts: Vec<_> = stage.engines.iter().map(|e| e.mount).collect();
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

/// Trapezoidal plates around the stage base, `radius` out from the axis.
fn fin_mesh(fins: &FinSet, radius: f32) -> Mesh {
    let d = &fins.def;
    let (root, tip, span) = (d.root_chord as f32, d.tip_chord as f32, d.span as f32);
    let (sweep, half) = (d.sweep as f32, d.thickness as f32 / 2.0);
    // Outline in (out from the hull, up from the stage base).
    let outline = [
        (0.0, 0.0),
        (0.0, root),
        (span, root - sweep),
        (span, root - sweep - tip),
    ];
    let color = LinearRgba::rgb(0.55, 0.57, 0.6);
    let mut mesh = MeshBuilder::default();
    for k in 0..d.count {
        // Same axes as the RCS ring and the engine layout: sim X/Y map to render X/−Z.
        let angle = TAU * k as f32 / d.count as f32;
        let out = Vec3::new(angle.cos(), 0.0, -angle.sin());
        let side = Vec3::new(-angle.sin(), 0.0, -angle.cos());
        let point = |(o, up): (f32, f32), s: f32| out * (radius + o) + Vec3::Y * up + side * s;
        let center = point((span / 3.0, root / 2.0), 0.0);
        let mut face = |corners: [Vec3; 4]| {
            let [a, b, c, e] = corners;
            if (b - a).cross(c - a).dot(a - center) >= 0.0 {
                mesh.quad(a, b, c, e, color);
            } else {
                mesh.quad(e, c, b, a, color);
            }
        };
        face(outline.map(|p| point(p, half)));
        face(outline.map(|p| point(p, -half)));
        for i in 0..4 {
            let (a, b) = (outline[i], outline[(i + 1) % 4]);
            face([
                point(a, half),
                point(a, -half),
                point(b, -half),
                point(b, half),
            ]);
        }
    }
    mesh.build()
}

fn rebuild_on_change(
    mut commands: Commands,
    sim: Res<Sim>,
    content: Res<GameContent>,
    assets: Res<AssetServer>,
    materials: Res<VesselMaterials>,
    mut plume_materials: ResMut<Assets<PlumeMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut visuals: Query<(Entity, &mut VesselVisual)>,
) {
    let looks = content
        .0
        .registry
        .all::<PlumeDef>("plumes", &mut Vec::new());
    let look = |id: &Option<String>| {
        let id = id.as_deref()?;
        let look = looks.iter().find(|l| l.id == id);
        if look.is_none() {
            warn!("plume `{id}` is not defined; the engine has no plume");
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
        let stages = &sim.universe.vessels[visual.index].propulsion.stages;
        let shape: Vec<bool> = stages.iter().map(|s| s.fins.is_some()).collect();
        if visual.built.as_ref() == Some(&shape) {
            continue;
        }
        visual.built = Some(shape);
        let vessel = visual.index;

        let puff = meshes.add(Cone::new(1.0, 1.0));
        // From the nozzle exit (y = 0) down to y = −1, scaled to each plume's bounds.
        let plume_volume = meshes.add(
            Mesh::from(Cylinder::new(1.0, 1.0).mesh().resolution(24))
                .translated_by(Vec3::NEG_Y * 0.5),
        );
        let mut entity_commands = commands.entity(entity);
        entity_commands.despawn_children();
        entity_commands.with_children(|parent| {
            // The origin is the base of the stack; build upwards along +Y (sim +Z, the nose).
            // Vessel axes map to these local axes like sim to render: X → X, Y → −Z.
            let mut y = 0.0;
            for (i, stage) in stages.iter().enumerate() {
                let radius = (stage.diameter / 2.0) as f32;
                let length = stage.length as f32;
                for (j, engine) in stage.engines.iter().enumerate() {
                    let mount = to_render(DVec3::new(engine.mount.x, engine.mount.y, 0.0));
                    let (exit, bell) = bell_size(stage, j);
                    let mut pivot = parent.spawn((
                        Transform::from_translation(mount + Vec3::Y * y),
                        Visibility::default(),
                    ));
                    if i == 0 {
                        pivot.insert(EnginePivot { vessel, engine: j });
                    }
                    pivot.with_children(|pivot| {
                        match model(&engine.def.model) {
                            Some(scene) => {
                                pivot.spawn(scene);
                            }
                            None => {
                                pivot.spawn((
                                    Mesh3d(meshes.add(Cone::new(exit, bell))),
                                    MeshMaterial3d(materials.engine.clone()),
                                    Transform::from_xyz(0.0, -bell / 2.0, 0.0),
                                ));
                            }
                        }
                        if i == 0
                            && let Some(look) = look(&engine.def.plume)
                        {
                            pivot.spawn((
                                Plume {
                                    exit: bell,
                                    radius: exit,
                                    exit_pressure: engine.def.exit_pressure(),
                                    look,
                                    seed: 7.31 * j as f32,
                                },
                                Mesh3d(plume_volume.clone()),
                                MeshMaterial3d(plume_materials.add(PlumeMaterial::default())),
                                Transform::default(),
                                Visibility::Hidden,
                                NotShadowCaster,
                            ));
                        }
                    });
                }

                let pod_height = stage.pod.as_ref().map_or(0.0, |p| p.height as f32);
                let hull = length - pod_height;
                if hull > 0.01 {
                    let parts = stage_hull(stage, stages.get(i + 1), hull);
                    for (mesh, material) in [
                        (parts.paint, &materials.paint),
                        (parts.matte, &materials.matte),
                        (parts.metal, &materials.metal),
                    ] {
                        if !mesh.is_empty() {
                            parent.spawn((
                                Mesh3d(meshes.add(mesh.build())),
                                MeshMaterial3d(material.clone()),
                                Transform::from_xyz(0.0, y, 0.0),
                            ));
                        }
                    }
                }
                if let Some(pod) = &stage.pod {
                    let at = Transform::from_xyz(0.0, y + hull, 0.0);
                    match model(&pod.model) {
                        Some(scene) => {
                            parent.spawn((scene, at));
                        }
                        None => {
                            let r = (pod.diameter / 2.0) as f32;
                            parent.spawn((
                                Mesh3d(meshes.add(ConicalFrustum {
                                    radius_top: r / 3.0,
                                    radius_bottom: r,
                                    height: pod_height,
                                })),
                                MeshMaterial3d(materials.hull[0].clone()),
                                at * Transform::from_xyz(0.0, pod_height / 2.0, 0.0),
                            ));
                        }
                    }
                }
                if let Some(fins) = &stage.fins {
                    parent.spawn((
                        Mesh3d(meshes.add(fin_mesh(fins, radius))),
                        MeshMaterial3d(materials.fins.clone()),
                        Transform::from_xyz(0.0, y, 0.0),
                    ));
                }
                for (k, block) in stage.rcs.iter().enumerate() {
                    let angle = block.mount.y.atan2(block.mount.x) as f32;
                    let at = Transform::from_translation(
                        to_render(DVec3::new(block.mount.x, block.mount.y, 0.0))
                            + Vec3::Y * (y + length - RCS_INSET),
                    )
                    .with_rotation(Quat::from_rotation_y(angle));
                    for (n, (exit, out)) in RCS_NOZZLES.into_iter().enumerate() {
                        parent.spawn((
                            RcsPlume {
                                vessel,
                                stage: i,
                                block: k,
                                nozzle: n,
                                base: at
                                    * Transform::from_translation(exit)
                                        .with_rotation(Quat::from_rotation_arc(Vec3::NEG_Y, out)),
                            },
                            Mesh3d(puff.clone()),
                            MeshMaterial3d(materials.rcs_plume.clone()),
                            Transform::default(),
                            Visibility::Hidden,
                        ));
                    }
                    match model(&block.def.model) {
                        Some(scene) => {
                            parent.spawn((scene, at));
                        }
                        None => {
                            parent.spawn((
                                Mesh3d(meshes.add(Cuboid::new(0.14, 0.3, 0.24))),
                                MeshMaterial3d(materials.hull[0].clone()),
                                at * Transform::from_xyz(0.07, 0.0, 0.0),
                            ));
                        }
                    }
                }
                y += length;
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
        let vessel = &sim.universe.vessels[visual.index];
        let base = vessel.base_at(system, frame.time);
        transform.translation = frame.to_render(vessel.parent(), base);
        transform.rotation = rotation_to_render(vessel.attitude_at(system, frame.time));
    }
}

/// Swivel the bottom stage's engines to their gimbal angles, shape their plumes to each
/// engine's thrust level and the ambient pressure, and light the vessel with them.
fn update_engines(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    mut pivots: Query<(&EnginePivot, &mut Transform, &Children), Without<Plume>>,
    mut plumes: Query<
        (
            &Plume,
            &MeshMaterial3d<PlumeMaterial>,
            &mut Transform,
            &mut Visibility,
        ),
        Without<PlumeLight>,
    >,
    mut lights: Query<(&PlumeLight, &mut PointLight, &mut Transform), Without<EnginePivot>>,
    mut materials: ResMut<Assets<PlumeMaterial>>,
) {
    let system = &sim.universe.system;
    // Per vessel: height of the base above the ground, m, and the local vertical in vessel
    // render axes.
    let ground: Vec<(f64, Vec3)> = sim
        .universe
        .vessels
        .iter()
        .map(|v| {
            let body = system.body(v.parent());
            let base = v.base_at(system, frame.time);
            let fixed = body.orientation(frame.time).inverse() * base.normalize();
            let height = base.length() - body.radius - body.terrain_height(fixed);
            let up = v.attitude_at(system, frame.time).inverse() * base.normalize();
            (height, to_render(up).normalize())
        })
        .collect();
    // Per vessel: lumens, and lumen-weighted colour and position.
    let mut glow: Vec<(f32, Vec3, Vec3)> =
        vec![(0.0, Vec3::ZERO, Vec3::ZERO); sim.universe.vessels.len()];
    for (pivot, mut transform, children) in &mut pivots {
        let vessel = &sim.universe.vessels[pivot.vessel];
        let Some(engine) = vessel
            .propulsion
            .stages
            .first()
            .and_then(|s| s.engines.get(pivot.engine))
        else {
            continue;
        };
        // The bell points along the exhaust, opposite the (tilted) thrust.
        let angle = engine.deflection.length();
        let side = engine.deflection.normalize_or_zero() * angle.sin();
        let exhaust = -to_render(DVec3::new(side.x, side.y, angle.cos()));
        transform.rotation = Quat::from_rotation_arc(Vec3::NEG_Y, exhaust.normalize());

        for child in children.iter() {
            let Ok((plume, material, mut plume_transform, mut visibility)) = plumes.get_mut(child)
            else {
                continue;
            };
            let level = engine.level as f32;
            if level <= 0.0 {
                *visibility = Visibility::Hidden;
                continue;
            }
            *visibility = Visibility::Inherited;
            // How far the exhaust runs along its axis before it hits the ground.
            let (height, up) = ground[pivot.vessel];
            let axis = transform.rotation * Vec3::NEG_Y;
            let exit = transform.translation + axis * plume.exit;
            let falling = -axis.dot(up);
            let to_ground = if falling > 0.2 {
                (height as f32 + exit.dot(up)) / falling
            } else {
                f32::INFINITY
            };
            let shape = PlumeShape::new(
                plume.radius,
                plume.exit_pressure,
                engine.level,
                vessel.flight.static_pressure,
                to_ground,
            );
            let end = shape.end_radius();
            *plume_transform = Transform::from_xyz(0.0, -plume.exit, 0.0).with_scale(Vec3::new(
                end,
                shape.length,
                end,
            ));
            if let Some(mut material) = materials.get_mut(&material.0) {
                material.params = PlumeParams::new(&shape, &plume.look, level, plume.seed);
            }
            let lumens = plume_lumens(&shape, &plume.look, level);
            let middle = exit + axis * (0.5 * shape.fade).min(shape.ground - 1.0).max(0.0);
            let (sum, color, at) = &mut glow[pivot.vessel];
            *sum += lumens;
            *color += lumens * Vec3::from(plume.look.flame);
            *at += lumens * middle;
        }
    }
    for (light, mut point, mut transform) in &mut lights {
        let (lumens, color, at) = glow[light.vessel];
        point.intensity = lumens;
        if lumens > 0.0 {
            let c = color / lumens;
            point.color = Color::linear_rgb(c.x, c.y, c.z);
            transform.translation = at / lumens;
        }
    }
}

/// Size each RCS puff by how hard its nozzle fired on the last physics tick.
fn update_rcs_plumes(
    sim: Res<Sim>,
    mut plumes: Query<(&RcsPlume, &mut Transform, &mut Visibility)>,
) {
    for (plume, mut transform, mut visibility) in &mut plumes {
        let firing = sim.universe.vessels[plume.vessel]
            .propulsion
            .stages
            .get(plume.stage)
            .and_then(|s| s.rcs.get(plume.block))
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
