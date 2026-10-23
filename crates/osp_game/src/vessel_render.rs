//! Vessel visuals: a procedural hull per stage (see [`crate::hull`]), engines, RCS blocks
//! and crew pods from their glTF models (or plain stand-ins without one), procedural fins
//! and wings, control surfaces that move with the controls, landing gear whose struts
//! give and whose nose wheels steer,
//! exhaust plumes from the bottom stage's engines that follow each engine's thrust level,
//! leave along its gimballed nozzle and merge with their neighbours' (see [`crate::plume`]),
//! the light they cast, and puffs from every RCS nozzle that fires.

use std::{
    collections::{HashMap, HashSet},
    f32::consts::{FRAC_PI_2, TAU},
};

use bevy::{light::NotShadowCaster, prelude::*};
use osp_sim::{
    DVec3,
    aero::{self, Panel},
    data::{EngineDef, TurbineExhaust, WingDef},
    propulsion::{Engine, Stage},
    structure::FinSet,
    time::SimTime,
    vessel::VesselId,
};

use crate::{
    FrameSet,
    content::{GameContent, mod_asset_path},
    frame::{RenderFrame, rotation_to_render, to_render},
    hull::stage_hull,
    meshgen::MeshBuilder,
    plume::{
        AXIS_LAG, Jet, JetKind, MAX_JETS, PlumeCluster, PlumeDef, PlumeMaterial, PlumeParams,
        PlumeQuality, plume_axis, plume_light,
    },
    settings::{GraphicsSettings, Quality},
    sim::Sim,
    vessel_material::{Finish, PartModel, VesselMaterial, VesselMaterialPlugin},
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
    /// The stages it was built from, bottom first: id, fins, fairing on (`None`: not built
    /// yet). A change (a stage falls away, fins tear off, the fairing goes) rebuilds it.
    built: Option<Vec<(usize, bool, bool)>>,
}

/// Pivot of an engine of the bottom stage, at its mount on the stage base.
#[derive(Component)]
struct EnginePivot {
    vessel: VesselId,
    engine: usize,
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

/// The exhaust plume of a cluster of the bottom stage's engines that share a look, their
/// jets merged into one volume. It hangs off the vessel, not the pivots: it runs where the
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

/// An engine of a plume's cluster: its nozzle exit is `exit` along the bell from the gimbal
/// pivot at `mount` (vessel render axes).
struct PlumeEngine {
    engine: usize,
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
    stage: usize,
    block: usize,
    nozzle: usize,
    base: Transform,
}

#[derive(Resource)]
struct VesselMaterials {
    /// Stand-ins for missing models.
    hull: [Handle<VesselMaterial>; 2],
    /// Hull meshes' vertex colours under glossy paint, matte composite and bare metal.
    paint: Handle<VesselMaterial>,
    matte: Handle<VesselMaterial>,
    metal: Handle<VesselMaterial>,
    engine: Handle<VesselMaterial>,
    rcs_plume: Handle<StandardMaterial>,
    /// White, so the fins' vertex colours show.
    fins: Handle<VesselMaterial>,
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
    commands.insert_resource(VesselMaterials {
        hull: [
            add(
                Finish::Paint,
                StandardMaterial {
                    base_color: Color::srgb(0.92, 0.92, 0.9),
                    perceptual_roughness: 0.6,
                    ..default()
                },
            ),
            add(
                Finish::Paint,
                StandardMaterial {
                    base_color: Color::srgb(0.25, 0.25, 0.28),
                    perceptual_roughness: 0.6,
                    ..default()
                },
            ),
        ],
        paint: add(
            Finish::Paint,
            StandardMaterial {
                perceptual_roughness: 0.42,
                clearcoat: 0.6,
                clearcoat_perceptual_roughness: 0.25,
                ..default()
            },
        ),
        matte: add(
            Finish::Composite,
            StandardMaterial {
                perceptual_roughness: 0.68,
                reflectance: 0.35,
                ..default()
            },
        ),
        metal: add(
            Finish::Metal,
            StandardMaterial {
                metallic: 0.85,
                perceptual_roughness: 0.48,
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
        fins: add(
            Finish::Paint,
            StandardMaterial {
                perceptual_roughness: 0.5,
                metallic: 0.3,
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

/// A stand-in engine bell from the gimbal pivot down to an exit `radius` wide, `length`
/// below: open, and seen from inside as well as out.
fn bell_mesh(radius: f32, length: f32) -> Mesh {
    const SEGMENTS: usize = 24;
    const STEPS: usize = 8;
    let throat = 0.35 * radius;
    let profile: Vec<(f32, f32)> = (0..=STEPS)
        .map(|i| {
            let t = i as f32 / STEPS as f32;
            (throat + (radius - throat) * t.powf(1.6), -length * t)
        })
        .collect();
    let mut mesh = MeshBuilder::default();
    let white = LinearRgba::WHITE;
    for w in profile.windows(2) {
        let [(r0, y0), (r1, y1)] = [w[0], w[1]];
        for k in 0..SEGMENTS {
            let (a0, a1) = (
                TAU * k as f32 / SEGMENTS as f32,
                TAU * (k + 1) as f32 / SEGMENTS as f32,
            );
            let p = |r: f32, y: f32, a: f32| Vec3::new(r * a.cos(), y, -r * a.sin());
            let (a, b, c, d) = (p(r0, y0, a0), p(r0, y0, a1), p(r1, y1, a1), p(r1, y1, a0));
            mesh.quad(a, b, c, d, white);
            mesh.quad(d, c, b, a, white);
        }
    }
    mesh.build()
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

/// A wing panel's fixed part (stack render axes) and, if it has one, its control surface:
/// the hinge's start (stack render axes), the axis it turns about so that a positive
/// deflection moves its trailing edge away from the lifting side, and its mesh about the
/// hinge. Slabs as thick as the airfoil, the control surface half that.
fn wing_meshes(wing: &WingDef, panel: &Panel) -> (MeshBuilder, Option<(Vec3, Vec3, MeshBuilder)>) {
    const SKIN: [f32; 3] = [0.86, 0.86, 0.84];
    const FLAP: [f32; 3] = [0.7, 0.71, 0.72];
    let n = panel.normal;
    let half = |chord: f64| wing.thickness * chord / 2.0;
    // A slab through the outline `corners` (root leading edge, tip leading edge, tip
    // trailing edge, root trailing edge), `thick` apart at each, relative to `origin`.
    let slab = |mesh: &mut MeshBuilder,
                corners: [DVec3; 4],
                thick: [f64; 4],
                origin: DVec3,
                color: [f32; 3]| {
        let color = LinearRgba::rgb(color[0], color[1], color[2]);
        let point = |i: usize, side: f64| to_render(corners[i] + n * thick[i] * side - origin);
        let center = corners.iter().map(|&c| to_render(c - origin)).sum::<Vec3>() / 4.0;
        let mut face = |q: [Vec3; 4]| {
            let [a, b, c, d] = q;
            if (b - a).cross(c - a).dot(a - center) >= 0.0 {
                mesh.quad(a, b, c, d, color);
            } else {
                mesh.quad(d, c, b, a, color);
            }
        };
        face([0, 1, 2, 3].map(|i| point(i, 1.0)));
        face([0, 1, 2, 3].map(|i| point(i, -1.0)));
        for i in 0..4 {
            let j = (i + 1) % 4;
            face([point(i, 1.0), point(i, -1.0), point(j, -1.0), point(j, 1.0)]);
        }
    };
    let section = |eta: f64, from: f64, to: f64| {
        let (leading_edge, chord) = panel.station(eta);
        (
            leading_edge - DVec3::Z * chord * from,
            leading_edge - DVec3::Z * chord * to,
            half(chord),
        )
    };
    let piece = |mesh: &mut MeshBuilder,
                 span: [f64; 2],
                 chord: [f64; 2],
                 scale: f64,
                 origin: DVec3,
                 color| {
        if span[1] <= span[0] {
            return;
        }
        let (a0, a1, ha) = section(span[0], chord[0], chord[1]);
        let (b0, b1, hb) = section(span[1], chord[0], chord[1]);
        slab(
            mesh,
            [a0, b0, b1, a1],
            [ha, hb, hb, ha].map(|h| h * scale),
            origin,
            color,
        );
    };
    let mut fixed = MeshBuilder::default();
    let Some(control) = &wing.control else {
        piece(&mut fixed, [0.0, 1.0], [0.0, 1.0], 1.0, DVec3::ZERO, SKIN);
        return (fixed, None);
    };
    let [inner, outer] = control.span;
    let hinge = 1.0 - control.chord;
    piece(&mut fixed, [0.0, inner], [0.0, 1.0], 1.0, DVec3::ZERO, SKIN);
    piece(
        &mut fixed,
        [inner, outer],
        [0.0, hinge],
        1.0,
        DVec3::ZERO,
        SKIN,
    );
    piece(&mut fixed, [outer, 1.0], [0.0, 1.0], 1.0, DVec3::ZERO, SKIN);
    let (start, _, _) = section(inner, hinge, 1.0);
    let (end, _, _) = section(outer, hinge, 1.0);
    let line = (end - start).normalize();
    // Turning the trailing edge (aft of the hinge) away from the lifting side.
    let sign = line.cross(DVec3::Z).dot(n).signum();
    let mut flap = MeshBuilder::default();
    piece(&mut flap, [inner, outer], [hinge, 1.0], 0.5, start, FLAP);
    (
        fixed,
        Some((to_render(start), to_render(line * sign), flap)),
    )
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
    let look = |engine: &EngineDef| {
        let burns: Vec<&str> = engine.propellants.keys().map(String::as_str).collect();
        let look = PlumeDef::for_engine(&looks, engine.plume.as_deref(), &burns);
        if look.is_none()
            && let Some(id) = &engine.plume
        {
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
        let Some(stages) = sim.universe.vessel(visual.id).map(|v| &v.propulsion.stages) else {
            continue;
        };
        let shape: Vec<_> = stages
            .iter()
            .map(|s| (s.id, s.fins.is_some(), s.fairing_mass() > 0.0))
            .collect();
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
            // The origin is the base of the stack; build upwards along +Y (sim +Z, the nose).
            // Vessel axes map to these local axes like sim to render: X → X, Y → −Z.
            let mut y = 0.0;
            // The bottom stage's engines by look, for one plume each.
            let mut clusters: Vec<(PlumeDef, Vec<PlumeEngine>)> = Vec::new();
            for (i, stage) in stages.iter().enumerate() {
                let radius = (stage.diameter / 2.0) as f32;
                let length = stage.length as f32;
                for (j, engine) in stage.engines.iter().enumerate() {
                    let mount = to_render(DVec3::new(engine.mount.x, engine.mount.y, 0.0));
                    let (exit, bell) = bell_size(stage, j);
                    let mount = mount + Vec3::Y * y;
                    let mut pivot =
                        parent.spawn((Transform::from_translation(mount), Visibility::default()));
                    if i == 0 {
                        pivot.insert(EnginePivot { vessel, engine: j });
                    }
                    pivot.with_children(|pivot| match model(&engine.def.model) {
                        Some(scene) => {
                            let scale = engine.def.model_scale as f32;
                            pivot.spawn((
                                scene,
                                PartModel,
                                Transform::from_scale(Vec3::splat(scale)),
                            ));
                        }
                        None => {
                            pivot.spawn((
                                Mesh3d(meshes.add(bell_mesh(exit, bell))),
                                MeshMaterial3d(materials.engine.clone()),
                                Transform::default(),
                            ));
                        }
                    });
                    if i == 0
                        && let Some(look) = look(&engine.def)
                    {
                        let exhaust = engine.def.turbine_exhaust.as_ref();
                        let pipe = match exhaust {
                            Some(TurbineExhaust::Pipe { exit, diameter }) => Some((
                                Vec3::new(exit[0] as f32, exit[1] as f32, exit[2] as f32),
                                (diameter / 2.0) as f32,
                            )),
                            _ => None,
                        };
                        let jet = PlumeEngine {
                            engine: j,
                            mount,
                            exit: bell,
                            radius: exit,
                            exit_pressure: engine.def.exit_pressure(),
                            pipe,
                            film: exhaust == Some(&TurbineExhaust::Nozzle),
                        };
                        match clusters.iter_mut().find(|(l, _)| l.id == look.id) {
                            Some((_, engines)) => engines.push(jet),
                            None => clusters.push((look, vec![jet])),
                        }
                    }
                }
                for (k, (look, mut engines)) in clusters.drain(..).enumerate() {
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
                            parent.spawn((scene, PartModel, at));
                        }
                        None => {
                            let r = (pod.diameter / 2.0) as f32;
                            parent.spawn((
                                Mesh3d(meshes.add(ConicalFrustum {
                                    radius_top: (pod.top_diameter() / 2.0) as f32,
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
                for (w, wing) in stage.wings.iter().enumerate() {
                    let panels = Panel::of(wing, stage.diameter / 2.0, y as f64);
                    for (k, panel) in panels.iter().enumerate() {
                        let (fixed, flap) = wing_meshes(wing, panel);
                        parent.spawn((
                            Mesh3d(meshes.add(fixed.build())),
                            MeshMaterial3d(materials.fins.clone()),
                            Transform::default(),
                        ));
                        if let Some((hinge, axis, flap)) = flap {
                            parent
                                .spawn((
                                    ControlSurfaceVisual {
                                        vessel,
                                        stage: i,
                                        wing: w,
                                        panel: k,
                                        axis,
                                    },
                                    Transform::from_translation(hinge),
                                    Visibility::default(),
                                ))
                                .with_children(|pivot| {
                                    pivot.spawn((
                                        Mesh3d(meshes.add(flap.build())),
                                        MeshMaterial3d(materials.fins.clone()),
                                        Transform::default(),
                                    ));
                                });
                        }
                    }
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
                            parent.spawn((scene, PartModel, at));
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
            let strut = meshes.add(Cylinder::new(1.0, 1.0));
            for (k, leg) in osp_sim::gear::legs(stages).iter().enumerate() {
                let r = leg.wheel_radius as f32;
                parent.spawn((
                    GearVisual {
                        vessel,
                        leg: k,
                        wheel: false,
                    },
                    Mesh3d(strut.clone()),
                    MeshMaterial3d(materials.metal.clone()),
                    Transform::default(),
                ));
                parent.spawn((
                    GearVisual {
                        vessel,
                        leg: k,
                        wheel: true,
                    },
                    Mesh3d(meshes.add(Cylinder::new(r, 0.7 * r).mesh().resolution(24))),
                    MeshMaterial3d(materials.hull[1].clone()),
                    Transform::default(),
                ));
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
    let engine = |vessel: VesselId, engine: usize| {
        sim.universe
            .vessel(vessel)?
            .propulsion
            .stages
            .first()
            .and_then(|s| s.engines.get(engine))
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
        let legs = osp_sim::gear::legs(&vessel.propulsion.stages);
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
        let stages = &vessel.propulsion.stages;
        let Some(stage) = stages.get(visual.stage) else {
            continue;
        };
        let Some(wing) = stage.wings.get(visual.wing) else {
            continue;
        };
        let Some(control) = &wing.control else {
            continue;
        };
        let base = stages[..visual.stage].iter().map(|s| s.length).sum();
        let Some(panel) = Panel::of(wing, stage.diameter / 2.0, base)
            .get(visual.panel)
            .copied()
        else {
            continue;
        };
        let com = vessel.mass_properties().center_of_mass;
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
            .and_then(|v| v.propulsion.stages.get(plume.stage))
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
