//! Placeholder vessel visuals: one cylinder per stage, a nose cone and an exhaust plume.

use bevy::prelude::*;

use crate::{
    FrameSet,
    frame::{RenderFrame, rotation_to_render},
    sim::Sim,
};

pub struct VesselRenderPlugin;

impl Plugin for VesselRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_vessel_visuals).add_systems(
            Update,
            (rebuild_on_staging, place_vessels, update_plumes)
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

        let mut entity_commands = commands.entity(entity);
        entity_commands.despawn_children();
        entity_commands.with_children(|parent| {
            // Vessel origin is the bottom of the stack; build upwards along +Y (sim +Z nose).
            let mut y = 0.0;
            let mut radius = 1.0;
            for (i, stage) in stages.iter().enumerate() {
                radius = (stage.diameter / 2.0) as f32;
                let length = stage.length as f32;
                if !stage.engines.is_empty() {
                    let bell = (radius * 0.8).min(1.5);
                    parent.spawn((
                        Mesh3d(meshes.add(Cone::new(bell, bell * 1.6))),
                        MeshMaterial3d(materials.engine.clone()),
                        Transform::from_xyz(0.0, y + bell * 0.3, 0.0),
                    ));
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
            parent.spawn((
                Plume,
                Mesh3d(meshes.add(Cone::new(radius * 0.9, 1.0))),
                MeshMaterial3d(materials.plume.clone()),
                Transform::default(),
                Visibility::Hidden,
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
        let local = vessel.state_at(system, frame.time).position;
        transform.translation = frame.to_render(vessel.parent(), local);
        transform.rotation = rotation_to_render(vessel.attitude_at(system, frame.time));
    }
}

fn update_plumes(
    sim: Res<Sim>,
    visuals: Query<(&VesselVisual, &Children)>,
    mut plumes: Query<(&mut Transform, &mut Visibility), With<Plume>>,
) {
    for (visual, children) in &visuals {
        let vessel = &sim.universe.vessels[visual.index];
        for child in children.iter() {
            let Ok((mut transform, mut visibility)) = plumes.get_mut(child) else {
                continue;
            };
            if !vessel.propulsion.any_engine_running() {
                *visibility = Visibility::Hidden;
                continue;
            }
            *visibility = Visibility::Inherited;
            // Plumes balloon as ambient pressure drops.
            let expansion =
                1.0 + 2.0 * (1.0 - (vessel.flight.static_pressure / 101_325.0).min(1.0));
            let length = (12.0 + 30.0 * vessel.propulsion.throttle()) as f32;
            transform.translation = Vec3::new(0.0, -length / 2.0, 0.0);
            transform.rotation = Quat::from_rotation_x(std::f32::consts::PI);
            transform.scale = Vec3::new(expansion as f32, length, expansion as f32);
        }
    }
}
