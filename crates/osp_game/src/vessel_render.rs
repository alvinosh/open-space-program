//! Placeholder vessel visuals: one cylinder per stage, a nose cone, and an engine bell per
//! engine that swivels with its gimbal and carries an exhaust plume.

use bevy::prelude::*;
use osp_sim::{DVec3, propulsion::Stage};

use crate::{
    FrameSet,
    frame::{RenderFrame, rotation_to_render, to_render},
    sim::Sim,
};

pub struct VesselRenderPlugin;

impl Plugin for VesselRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_vessel_visuals).add_systems(
            Update,
            (rebuild_on_staging, place_vessels, update_engines)
                .chain()
                .in_set(FrameSet::Render),
        );
    }
}

#[derive(Component)]
struct VesselVisual {
    index: usize,
    stage_count: usize,
}

/// Pivot of an engine of the bottom stage, at its mount on the stage base.
#[derive(Component)]
struct EnginePivot {
    vessel: usize,
    engine: usize,
}

#[derive(Component)]
struct Plume;

#[derive(Resource)]
struct VesselMaterials {
    hull: [Handle<StandardMaterial>; 2],
    engine: Handle<StandardMaterial>,
    plume: Handle<StandardMaterial>,
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
        engine: materials.add(StandardMaterial {
            base_color: Color::srgb(0.15, 0.13, 0.12),
            metallic: 0.8,
            perceptual_roughness: 0.4,
            ..default()
        }),
        plume: materials.add(StandardMaterial {
            base_color: Color::linear_rgba(4.0, 2.2, 0.8, 0.6),
            alpha_mode: AlphaMode::Add,
            unlit: true,
            ..default()
        }),
    });
    for index in 0..sim.universe.vessels.len() {
        commands.spawn((
            VesselVisual {
                index,
                // Forces a build on the first frame.
                stage_count: usize::MAX,
            },
            Transform::default(),
            Visibility::default(),
        ));
    }
}

/// Bell radius that keeps a stage's engines from overlapping each other or the hull.
fn bell_radius(stage: &Stage) -> f32 {
    let radius = stage.diameter / 2.0;
    let mounts: Vec<_> = stage.engines.iter().map(|e| e.mount).collect();
    let mut room = (radius * 0.8).min(1.5);
    for (i, a) in mounts.iter().enumerate() {
        room = room.min(radius - a.length());
        for b in &mounts[i + 1..] {
            room = room.min(0.45 * a.distance(*b));
        }
    }
    room.max(0.1) as f32
}

fn rebuild_on_staging(
    mut commands: Commands,
    sim: Res<Sim>,
    materials: Res<VesselMaterials>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut visuals: Query<(Entity, &mut VesselVisual)>,
) {
    for (entity, mut visual) in &mut visuals {
        let stages = &sim.universe.vessels[visual.index].propulsion.stages;
        if visual.stage_count == stages.len() {
            continue;
        }
        visual.stage_count = stages.len();
        let vessel = visual.index;

        let mut entity_commands = commands.entity(entity);
        entity_commands.despawn_children();
        entity_commands.with_children(|parent| {
            // The origin is the base of the stack; build upwards along +Y (sim +Z, the nose).
            // Vessel axes map to these local axes like sim to render: X → X, Y → −Z.
            let mut y = 0.0;
            let mut radius = 1.0;
            for (i, stage) in stages.iter().enumerate() {
                radius = (stage.diameter / 2.0) as f32;
                let length = stage.length as f32;
                let bell = bell_radius(stage);
                let cone = meshes.add(Cone::new(bell, bell * 1.6));
                let plume = meshes.add(Cone::new(bell * 0.9, 1.0));
                for (j, engine) in stage.engines.iter().enumerate() {
                    let mount = to_render(DVec3::new(engine.mount.x, engine.mount.y, 0.0));
                    let mut pivot = parent.spawn((
                        Transform::from_translation(mount + Vec3::Y * y),
                        Visibility::default(),
                    ));
                    if i == 0 {
                        pivot.insert(EnginePivot { vessel, engine: j });
                    }
                    pivot.with_children(|pivot| {
                        pivot.spawn((
                            Mesh3d(cone.clone()),
                            MeshMaterial3d(materials.engine.clone()),
                            Transform::from_xyz(0.0, bell * 0.3, 0.0),
                        ));
                        if i == 0 {
                            pivot.spawn((
                                Plume,
                                Mesh3d(plume.clone()),
                                MeshMaterial3d(materials.plume.clone()),
                                Transform::default(),
                                Visibility::Hidden,
                            ));
                        }
                    });
                }
                parent.spawn((
                    Mesh3d(meshes.add(Cylinder::new(radius, length))),
                    MeshMaterial3d(materials.hull[i % 2].clone()),
                    Transform::from_xyz(0.0, y + length / 2.0, 0.0),
                ));
                y += length;
            }
            let nose = radius * 2.5;
            parent.spawn((
                Mesh3d(meshes.add(Cone::new(radius, nose))),
                MeshMaterial3d(materials.hull[0].clone()),
                Transform::from_xyz(0.0, y + nose / 2.0, 0.0),
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

/// Swivel the bottom stage's bells to their gimbal angles and size their plumes.
fn update_engines(
    sim: Res<Sim>,
    mut pivots: Query<(&EnginePivot, &mut Transform, &Children), Without<Plume>>,
    mut plumes: Query<(&mut Transform, &mut Visibility), With<Plume>>,
) {
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
            let Ok((mut plume, mut visibility)) = plumes.get_mut(child) else {
                continue;
            };
            if engine.thrust <= 0.0 {
                *visibility = Visibility::Hidden;
                continue;
            }
            *visibility = Visibility::Inherited;
            // Plumes balloon as ambient pressure drops.
            let expansion =
                1.0 + 2.0 * (1.0 - (vessel.flight.static_pressure / 101_325.0).min(1.0));
            let length = (12.0 + 30.0 * vessel.propulsion.throttle()) as f32;
            plume.translation = Vec3::new(0.0, -length / 2.0, 0.0);
            plume.rotation = Quat::from_rotation_x(std::f32::consts::PI);
            plume.scale = Vec3::new(expansion as f32, length, expansion as f32);
        }
    }
}
