//! The space centre's buildings (`space_centres` records, `osp_sim::space_centre`): each stands on
//! its apron as the model of the level its facility is at (`osp_game::space_centre::level_of`: a
//! career's level, a sandbox's top one), made by `tools/blender/space_centre.py`, and changes as soon
//! as the facility is upgraded. A building without a model is a plain block with its doors on its
//! front and, if people work in it, bands of windows. The one the pointer is on in the space
//! centre's view is lit.

use std::collections::HashMap;

use bevy::{math::DVec3, prelude::*};
use osp_sim::{
    data::{BuildingDef, BuildingUse, SpaceCentreDef},
    space_centre,
};

use crate::{
    content::{GameContent, mod_asset_path},
    frame::to_render,
    meshgen::MeshBuilder,
    planet::SurfaceAnchor,
    sim::Sim,
};
use osp_game::{
    FrameSet,
    career::CareerTree,
    menu::Menu,
    space_centre::{SpaceCentre, level_of},
};

pub struct SpaceCentrePlugin;

impl Plugin for SpaceCentrePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_buildings).add_systems(
            Update,
            (show_levels, light_pointed)
                .chain()
                .in_set(FrameSet::Render),
        );
    }
}

const WALL: [f32; 3] = [0.62, 0.62, 0.60];
const ROOF: [f32; 3] = [0.30, 0.31, 0.32];
const DOOR: [f32; 3] = [0.10, 0.11, 0.12];
const GLASS: [f32; 3] = [0.035, 0.05, 0.065];
/// How bright a model lights up while the pointer is on it: the editor's accent, in nits, faint
/// enough that its shape still shows.
const MODEL_LIT: LinearRgba = LinearRgba::rgb(0.22, 0.88, 0.77);
/// How bright a block lights up while the pointer is on it.
const LIT: LinearRgba = LinearRgba::rgb(0.22 * 9000.0, 0.88 * 9000.0, 0.77 * 9000.0);

fn c(color: [f32; 3]) -> LinearRgba {
    LinearRgba::rgb(color[0], color[1], color[2])
}

/// A building, in its own frame (x right, y up, -z to its front): the level shown, and what shows
/// it (its model, or a block).
#[derive(Component)]
struct Building {
    def: BuildingDef,
    level: Option<usize>,
    look: Option<Entity>,
}

/// A block standing in for a building without a model: lit by taking another material.
#[derive(Component)]
struct Block;

/// The two looks of a block: as it is, and lit.
#[derive(Resource)]
struct Looks {
    plain: Handle<StandardMaterial>,
    lit: Handle<StandardMaterial>,
}

/// A block `size` metres (width, height, length) in its own frame: x right, y up, -z to its front.
fn block(opens: BuildingUse, size: Vec3) -> MeshBuilder {
    let (w, h, l) = (size.x, size.y, size.z);
    let mut mesh = MeshBuilder::default();
    let half = Vec3::new(w, h, l) / 2.0;
    mesh.cuboid(Vec3::Y * half.y, half, Quat::IDENTITY, c(WALL));
    // A flat roof a little in from the walls.
    mesh.cuboid(
        Vec3::Y * (h + 0.5),
        Vec3::new(half.x - 1.0, 0.5, half.z - 1.0),
        Quat::IDENTITY,
        c(ROOF),
    );
    // Its doors, on its front: tall for rockets, wide for planes, a door for people elsewhere.
    let (door_w, door_h) = match opens {
        BuildingUse::Rockets => (w * 0.22, h * 0.85),
        BuildingUse::Planes => (w * 0.8, h * 0.7),
        _ => (6.0_f32.min(w * 0.3), 4.0_f32.min(h * 0.5)),
    };
    mesh.cuboid(
        Vec3::new(0.0, door_h / 2.0, -half.z - 0.1),
        Vec3::new(door_w / 2.0, door_h / 2.0, 0.15),
        Quat::IDENTITY,
        c(DOOR),
    );
    // Offices have a band of windows round them.
    if !matches!(opens, BuildingUse::Rockets | BuildingUse::Planes) {
        for k in 1..=((h / 8.0) as i32).max(1) {
            let y = k as f32 * 8.0 - 3.0;
            if y + 1.2 > h - 1.0 {
                break;
            }
            mesh.cuboid(
                Vec3::Y * y,
                Vec3::new(half.x + 0.1, 1.2, half.z + 0.1),
                Quat::IDENTITY,
                c(GLASS),
            );
        }
    }
    mesh
}

/// Each building's place: an anchor on its apron, and its frame turned the way its front faces. What
/// it looks like comes with its level (`show_levels`).
fn spawn_buildings(
    mut commands: Commands,
    sim: Res<Sim>,
    content: Res<GameContent>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let plain = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.8,
        ..default()
    });
    let lit = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.8,
        emissive: LIT,
        ..default()
    });
    commands.insert_resource(Looks { plain, lit });
    let system = &sim.universe.system;
    let registry = &content.0.registry;
    for def in registry.all::<SpaceCentreDef>("space_centres", &mut Vec::new()) {
        let Some(body_id) = system.find(&def.body) else {
            continue;
        };
        let body = system.body(body_id);
        let standing = space_centre::standing(&def, body, &[]);
        for (building, standing) in def.buildings.iter().zip(standing) {
            // Its frame (x right, y up, -z to its front) to body-fixed render axes.
            let axis = |v: DVec3| to_render(v);
            let rotation = Quat::from_mat3(&Mat3::from_cols(
                axis(standing.right),
                axis(standing.up),
                axis(-standing.forward),
            ));
            commands
                .spawn((
                    SurfaceAnchor {
                        body: body_id,
                        center: standing.base,
                    },
                    Transform::default(),
                    Visibility::default(),
                    Name::new(building.display_name().to_owned()),
                ))
                .with_children(|parent| {
                    parent.spawn((
                        Building {
                            def: building.clone(),
                            level: None,
                            look: None,
                        },
                        Transform::from_rotation(rotation),
                        Visibility::default(),
                    ));
                });
        }
    }
}

/// Show each building at the level it is at: its model, or a block of its size.
#[allow(clippy::too_many_arguments)]
fn show_levels(
    mut commands: Commands,
    mut buildings: Query<(Entity, &mut Building)>,
    (career, menu, sim): (Res<CareerTree>, Res<Menu>, Res<Sim>),
    content: Res<GameContent>,
    assets: Res<AssetServer>,
    looks: Option<Res<Looks>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(looks) = looks else {
        return;
    };
    for (entity, mut building) in &mut buildings {
        let level = level_of(&building.def, &career, &menu, &sim);
        if building.level == Some(level) {
            continue;
        }
        if let Some(old) = building.look.take() {
            commands.entity(old).despawn();
        }
        let model = (building.def.levels.get(level))
            .and_then(|l| l.model.as_deref())
            .and_then(|relative| {
                let path = mod_asset_path(&content.0, relative);
                if path.is_none() {
                    warn!("model `{relative}` is not in any mod; drawing a block");
                }
                path
            });
        let look = match model {
            Some(path) => commands
                .spawn((
                    WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path))),
                    Transform::default(),
                ))
                .id(),
            None => {
                let [along, across, height] = building.def.size_at(level);
                let size = Vec3::new(across as f32, height as f32, along as f32);
                let mesh = block(building.def.opens, size).build();
                let mesh = if crate::dev_graphics::on() {
                    crate::dev_graphics::faceted(mesh)
                } else {
                    mesh
                };
                commands
                    .spawn((
                        Block,
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(looks.plain.clone()),
                        Transform::default(),
                    ))
                    .id()
            }
        };
        commands.entity(entity).add_child(look);
        building.look = Some(look);
        building.level = Some(level);
    }
}

/// Light the building the pointer is on in the space centre's view: a block takes the lit material,
/// a model's own materials glow (each building's model has materials of its own).
fn light_pointed(
    centre: Res<SpaceCentre>,
    looks: Option<Res<Looks>>,
    buildings: Query<(Entity, &Building)>,
    children: Query<&Children>,
    mut shown: Query<(&mut MeshMaterial3d<StandardMaterial>, Has<Block>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut glow: Local<HashMap<AssetId<StandardMaterial>, LinearRgba>>,
) {
    let Some(looks) = looks else {
        return;
    };
    let pointed = centre.pointed();
    for (entity, building) in &buildings {
        let lit = pointed == Some(building.def.id.as_str());
        for part in children.iter_descendants(entity) {
            let Ok((mut material, is_block)) = shown.get_mut(part) else {
                continue;
            };
            if is_block {
                let wanted = if lit { &looks.lit } else { &looks.plain };
                if material.0 != *wanted {
                    material.0 = wanted.clone();
                }
                continue;
            }
            let id = material.0.id();
            let Some(now) = materials.get(id).map(|m| m.emissive) else {
                continue;
            };
            let own = *glow.entry(id).or_insert(now);
            let wanted = if lit { MODEL_LIT } else { own };
            if now != wanted
                && let Some(mut m) = materials.get_mut(id)
            {
                m.emissive = wanted;
            }
        }
    }
}
