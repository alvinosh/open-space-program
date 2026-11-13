//! Pictures of parts for the parts list (`osp_game::part_pictures`). Each part asked for is built
//! by itself and drawn as the hangar draws it, from its geometry in its paint or from its model,
//! in a cell of a grid of its own: turned a little towards the viewer and scaled to fill the cell.
//! One orthographic camera looks straight at the grid and draws it into one image, which the
//! list shows a cell at a time. The grid is far below everything, on a render layer no other
//! camera sees, lit only by its own lights, so the pictures look the same wherever the game is.
//!
//! The camera draws only while a picture is on screen and one is unfinished. A cell is finished
//! once it has been drawn for a few frames after the last change to it (put in, its model
//! spawned, its framing moved) with no shader still compiling; the image keeps it when the
//! camera stops. Cells are kept after their part stops being wanted until one is needed for
//! another, the longest unwanted first.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use bevy::{
    app::{HierarchyPropagatePlugin, Propagate},
    asset::UntypedAssetId,
    camera::{Exposure, RenderTarget, ScalingMode, primitives::Aabb, visibility::RenderLayers},
    light::{AmbientLight, NotShadowCaster},
    math::DVec3,
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        render_resource::{PipelineCache, TextureFormat},
        view::Tonemapping,
    },
    world_serialization::WorldInstance,
};
use osp_game::part_pictures::{LooseParts, PartInHand, PartPictures, ShowsLoose};
use osp_sim::{
    craft::Craft,
    data::{CraftDef, CraftPartDef, PartCatalog},
    parts::{PartKind, PartTree},
};

use crate::{
    FrameSet,
    content::{GameContent, catalog, mod_asset_path},
    frame::{rotation_to_render, to_render},
    part_mesh::{Canvas, Looks, bevy_mesh, part_meshes},
    vessel_material::{Finish, PartModel, VesselMaterial},
};

/// Render layer of the grid, which no other camera sees.
const LAYER: usize = 9;
/// The grid: columns and rows of cells, each this many pixels square in the image.
const COLUMNS: u32 = 12;
const ROWS: u32 = 10;
const CELL_PX: u32 = 160;
/// A cell's size in the world, m, and where the grid's middle is: far below everything.
const CELL: f32 = 10.0;
const GRID: Vec3 = Vec3::new(0.0, -60_000.0, 0.0);
/// How much of its cell a part fills, across its longer side.
const FILL: f32 = 0.9;
/// How a part is turned to be seen: about its axis, then its top tipped towards the viewer,
/// radians.
const YAW: f32 = -0.55;
const TIP: f32 = 0.32;
/// Frames a cell is drawn after its last change, with no shader compiling, before it is finished.
const SETTLE_FRAMES: u32 = 6;
/// The most parts built and put in cells a frame.
const MOST_A_FRAME: usize = 6;

pub struct PartPicturesPlugin;

impl Plugin for PartPicturesPlugin {
    fn build(&self, app: &mut App) {
        let compiling = Compiling::default();
        app.add_plugins(HierarchyPropagatePlugin::<RenderLayers>::new(PostUpdate))
            .insert_resource(compiling.clone())
            .add_systems(Startup, spawn_grid)
            .add_systems(
                Update,
                (
                    take_wanted,
                    frame_cells,
                    run_camera,
                    draw_part_in_hand,
                    draw_loose_parts,
                    see_through_loose,
                )
                    .chain()
                    .in_set(FrameSet::Render),
            );
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(compiling)
                .add_systems(Render, note_compiling.in_set(RenderSystems::Cleanup));
        }
    }
}

/// Whether the renderer was compiling a shader last frame: pictures drawn then may be missing
/// what uses it.
#[derive(Resource, Clone, Default)]
struct Compiling(Arc<AtomicBool>);

fn note_compiling(cache: Res<PipelineCache>, compiling: Res<Compiling>) {
    let waiting = cache.waiting_pipelines().next().is_some();
    compiling.0.store(waiting, Ordering::Relaxed);
}

#[derive(Component)]
struct GridCamera;

/// The grid and what is in its cells.
#[derive(Resource)]
struct Grid {
    cells: Vec<Option<Cell>>,
    /// The asking last taken (`PartPictures::wanted`), and what of it waits for a cell.
    asked: Option<u64>,
    waiting: Vec<(String, CraftPartDef)>,
    /// Frames drawn since the last change, with no shader compiling.
    quiet: u32,
    /// Something changed this frame: what is drawn is not settled.
    changed: bool,
    /// What an engine with no model of its own is drawn in, as in the hangar.
    engine: Handle<VesselMaterial>,
}

/// A part in a cell.
struct Cell {
    name: String,
    part: CraftPartDef,
    root: Entity,
    /// The asking it was last wanted in.
    wanted: u64,
    /// Where its drawn meshes are, seen as it is turned to be seen, in its own scale: the middle
    /// and the half size across the longer side. None until its meshes are in.
    framed: Option<(Vec3, f32)>,
    finished: bool,
}

/// The middle of cell `index` in the world, and where it is in the image, px.
fn cell_place(index: usize) -> (Vec3, Rect) {
    let (column, row) = (index as u32 % COLUMNS, index as u32 / COLUMNS);
    let x = (column as f32 + 0.5 - COLUMNS as f32 / 2.0) * CELL;
    let y = (ROWS as f32 / 2.0 - row as f32 - 0.5) * CELL;
    let min = Vec2::new((column * CELL_PX) as f32, (row * CELL_PX) as f32);
    (
        GRID + Vec3::new(x, y, 0.0),
        Rect::from_corners(min, min + Vec2::splat(CELL_PX as f32)),
    )
}

/// How every part is turned to be seen.
fn turn() -> Quat {
    Quat::from_rotation_x(TIP) * Quat::from_rotation_y(YAW)
}

fn spawn_grid(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<VesselMaterial>>,
    mut pictures: ResMut<PartPictures>,
) {
    let layer = RenderLayers::layer(LAYER);
    let atlas = images.add(Image::new_target_texture(
        COLUMNS * CELL_PX,
        ROWS * CELL_PX,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    pictures.set_atlas(atlas.clone());
    let engine = materials.add(VesselMaterial {
        base: StandardMaterial {
            base_color: Color::srgb(0.15, 0.13, 0.12),
            metallic: 0.8,
            perceptual_roughness: 0.4,
            ..default()
        },
        extension: Finish::HeatTint(20.0, 90.0).extension(),
    });
    commands.insert_resource(Grid {
        cells: (0..COLUMNS * ROWS).map(|_| None).collect(),
        asked: None,
        waiting: Vec::new(),
        quiet: 0,
        changed: false,
        engine,
    });
    commands.spawn((
        GridCamera,
        Camera3d::default(),
        Camera {
            // Before the views, so the list shows this frame's pictures.
            order: -2,
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::from(atlas),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: COLUMNS as f32 * CELL,
                height: ROWS as f32 * CELL,
            },
            // Deep enough for a part that reaches towards the camera further than it is wide.
            near: 0.0,
            far: 100.0 * CELL,
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_translation(GRID + Vec3::Z * 50.0 * CELL).looking_at(GRID, Vec3::Y),
        Msaa::Sample4,
        // As the view is drawn (`osp_game::camera`).
        Tonemapping::KhronosPbrNeutral,
        Exposure::default(),
        // Enough that the side away from the lights still shows its shape.
        AmbientLight {
            color: Color::srgb(0.85, 0.9, 1.0),
            brightness: 600.0,
            ..default()
        },
        layer.clone(),
    ));
    // A key light from the upper left in front, and a fainter fill from the right.
    for (illuminance, from) in [
        (6500.0, Vec3::new(-1.0, 1.4, 1.2)),
        (2000.0, Vec3::new(1.5, 0.3, 0.8)),
    ] {
        commands.spawn((
            DirectionalLight {
                illuminance,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_translation(from).looking_at(Vec3::ZERO, Vec3::Y),
            layer.clone(),
        ));
    }
}

/// Take what is wanted: what already has a cell keeps it, the rest waits for one, and a few a frame
/// are built and put in cells.
#[allow(clippy::too_many_arguments)]
fn take_wanted(
    mut commands: Commands,
    mut grid: ResMut<Grid>,
    mut pictures: ResMut<PartPictures>,
    content: Res<GameContent>,
    assets: Res<AssetServer>,
    mut looks: ResMut<Looks>,
    (mut meshes, mut materials, mut images): (
        ResMut<Assets<Mesh>>,
        ResMut<Assets<VesselMaterial>>,
        ResMut<Assets<Image>>,
    ),
    mut parts: Local<Option<Arc<PartCatalog>>>,
) {
    let grid = &mut *grid;
    grid.changed = false;
    if parts.is_none() || content.is_changed() {
        *parts = Some(Arc::new(catalog(&content.0.registry, &mut Vec::new())));
    }
    let Some(catalog) = parts.as_deref() else {
        return;
    };
    let (wanted, asked) = pictures.wanted();
    if grid.asked != Some(asked) || content.is_changed() {
        grid.asked = Some(asked);
        grid.waiting.clear();
        for (name, part) in wanted.iter().take(grid.cells.len()) {
            let has = (grid.cells.iter_mut().flatten())
                .find(|c| &c.name == name && &c.part == part && !content.is_changed());
            match has {
                Some(cell) => cell.wanted = asked,
                None => grid.waiting.push((name.clone(), part.clone())),
            }
        }
        grid.waiting.reverse();
    }
    let file = |path: &str| {
        content
            .0
            .resolve_file(path)
            .and_then(|f| std::fs::read(f).ok())
    };
    let model = |relative: &str| {
        let path = mod_asset_path(&content.0, relative)?;
        Some(WorldAssetRoot(
            assets.load(GltfAssetLabel::Scene(0).from_asset(path)),
        ))
    };
    for _ in 0..MOST_A_FRAME {
        let Some((name, part)) = grid.waiting.pop() else {
            break;
        };
        // The cell it had (its part changed), an empty one, or the one unwanted longest.
        let index = (grid
            .cells
            .iter()
            .position(|c| c.as_ref().is_some_and(|c| c.name == name)))
        .or_else(|| grid.cells.iter().position(Option::is_none))
        .or_else(|| {
            (grid.cells.iter().enumerate())
                .filter_map(|(i, c)| Some((i, c.as_ref()?.wanted)))
                .filter(|(_, wanted)| *wanted != asked)
                .min_by_key(|(_, wanted)| *wanted)
                .map(|(i, _)| i)
        });
        let Some(index) = index else {
            break;
        };
        if let Some(old) = grid.cells[index].take() {
            commands.entity(old.root).despawn();
            pictures.set_finished(&old.name, None);
        }
        let (middle, _) = cell_place(index);
        let root = commands
            .spawn((
                Transform::from_translation(middle).with_rotation(turn()),
                // Shown once it is framed, so it never spills into another cell.
                Visibility::Hidden,
                RenderLayers::layer(LAYER),
                Propagate(RenderLayers::layer(LAYER)),
            ))
            .id();
        match built(std::slice::from_ref(&part), catalog) {
            Ok(tree) => {
                let mut pen = Pen {
                    looks: &mut looks,
                    meshes: &mut meshes,
                    canvas: Canvas {
                        materials: &mut materials,
                        images: &mut images,
                        file: &file,
                    },
                    engine: &grid.engine,
                    model: &model,
                };
                draw(&mut commands, root, &tree, &mut pen);
            }
            Err(e) => warn!("no picture of `{name}`: {e}"),
        }
        grid.cells[index] = Some(Cell {
            name,
            part,
            root,
            wanted: asked,
            framed: None,
            finished: false,
        });
        grid.changed = true;
    }
}

/// `parts` built by themselves, the first holding the rest.
fn built(parts: &[CraftPartDef], catalog: &PartCatalog) -> Result<PartTree, String> {
    let mut parts = parts.to_vec();
    if let Some(first) = parts.first_mut() {
        if first.id.is_empty() {
            first.id = "picture".into();
        }
        (first.on, first.at) = (None, None);
    }
    let alone = CraftDef {
        id: "picture".into(),
        name: String::new(),
        parts,
        sequences: None,
        triggers: Vec::new(),
        rcs_shut: Vec::new(),
        abort: Vec::new(),
    };
    let craft = Craft::build(&alone, catalog).map_err(|e| e.to_string())?;
    PartTree::from_craft(&craft, catalog).map_err(|e| e.to_string())
}

/// What parts are drawn with: their meshes and the materials of their looks, the engine metal, and
/// a mod's model by its path, if it has it.
struct Pen<'a> {
    looks: &'a mut Looks,
    meshes: &'a mut Assets<Mesh>,
    canvas: Canvas<'a>,
    engine: &'a Handle<VesselMaterial>,
    model: &'a dyn Fn(&str) -> Option<WorldAssetRoot>,
}

/// Draw `tree`'s parts under `root` as the hangar does: a template's own model for a part that is
/// only that, its geometry otherwise (an engine's in the engine metal), with the models its shape
/// is made of.
fn draw(commands: &mut Commands, root: Entity, tree: &PartTree, pen: &mut Pen) {
    let base = DVec3::Z * tree.base();
    for part in tree.parts() {
        let Some(built) = &part.built else {
            continue;
        };
        let pose = Transform::from_translation(to_render(part.position - base))
            .with_rotation(rotation_to_render(part.rotation));
        let (own, metal) = match part.kinds.as_slice() {
            [PartKind::Engine(def)] => (
                def.model.as_ref().map(|m| (m, def.model_scale as f32)),
                Some(pen.engine),
            ),
            [PartKind::Pod(def)] => (def.model.as_ref().map(|m| (m, 1.0)), None),
            [PartKind::Rcs(def)] => (def.model.as_ref().map(|m| (m, 1.0)), None),
            [PartKind::ReactionWheel(def)] => (def.model.as_ref().map(|m| (m, 1.0)), None),
            _ => (None, None),
        };
        let visual = commands
            .spawn((pose, Visibility::default(), ChildOf(root)))
            .id();
        if let Some((scene, scale)) =
            own.and_then(|(path, scale)| Some(((pen.model)(path)?, scale)))
        {
            commands.spawn((
                scene,
                PartModel,
                Transform::from_scale(Vec3::splat(scale)),
                ChildOf(visual),
            ));
            continue;
        }
        for (mesh, look, layers) in part_meshes(built) {
            let material = match metal {
                Some(metal) => metal.clone(),
                None => pen.looks.material(look, layers, &mut pen.canvas),
            };
            commands.spawn((
                Mesh3d(pen.meshes.add(bevy_mesh(&mesh, Vec3::ZERO))),
                MeshMaterial3d(material),
                ChildOf(visual),
            ));
        }
        for (shape, at, turned) in built.geometry.models() {
            let Some(scene) = (pen.model)(&shape.path) else {
                continue;
            };
            commands.spawn((
                scene,
                PartModel,
                Transform::from_translation(to_render(at))
                    .with_rotation(rotation_to_render(turned))
                    .with_scale(Vec3::splat(shape.scale as f32)),
                ChildOf(visual),
            ));
        }
    }
}

/// Frame each unfinished cell's part once its meshes are in, and again if they move: turned to be
/// seen, its drawn bounds in the middle of its cell, filling it.
fn frame_cells(
    mut grid: ResMut<Grid>,
    children: Query<&Children>,
    mut roots: Query<(&mut Transform, &mut Visibility, &GlobalTransform)>,
    drawn: Query<(&Aabb, &GlobalTransform)>,
    scenes: Query<(&WorldAssetRoot, Has<WorldInstance>)>,
    assets: Res<AssetServer>,
) {
    let grid = &mut *grid;
    let view = turn();
    for (index, cell) in grid.cells.iter_mut().enumerate() {
        let Some(cell) = cell.as_mut().filter(|c| !c.finished) else {
            continue;
        };
        let Ok((mut transform, mut visibility, root)) = roots.get_mut(cell.root) else {
            continue;
        };
        // Not before its models are in, or have failed to come.
        let loading = (children.iter_descendants(cell.root))
            .filter_map(|e| scenes.get(e).ok())
            .any(|(scene, spawned)| !spawned && !assets.load_state(&scene.0).is_failed());
        if loading {
            continue;
        }
        let into_root = root.affine().inverse();
        let mut low = Vec3::splat(f32::INFINITY);
        let mut high = Vec3::splat(f32::NEG_INFINITY);
        for (aabb, at) in children
            .iter_descendants(cell.root)
            .filter_map(|e| drawn.get(e).ok())
        {
            let local = into_root * at.affine();
            let (centre, half) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
            for corner in 0..8 {
                let sign = Vec3::new(
                    if corner & 1 == 0 { -1.0 } else { 1.0 },
                    if corner & 2 == 0 { -1.0 } else { 1.0 },
                    if corner & 4 == 0 { -1.0 } else { 1.0 },
                );
                let seen = view * local.transform_point3(centre + sign * half);
                low = low.min(seen);
                high = high.max(seen);
            }
        }
        if !low.is_finite() {
            // Nothing to draw: the picture is the empty cell.
            if cell.framed.is_none() {
                cell.framed = Some((Vec3::ZERO, 1.0));
                grid.changed = true;
            }
            continue;
        }
        let middle = (low + high) / 2.0;
        let half = ((high - low) / 2.0).truncate().max_element().max(1e-3);
        let moved = cell.framed.is_none_or(|(m, h)| {
            m.distance(middle) > 1e-3 * half || (h - half).abs() > 1e-3 * half
        });
        if !moved {
            continue;
        }
        cell.framed = Some((middle, half));
        let scale = FILL * CELL / 2.0 / half;
        let (place, _) = cell_place(index);
        // Turned and scaled about the root: its middle, as seen, goes to the cell's middle.
        *transform = Transform {
            translation: place - middle * scale,
            rotation: view,
            scale: Vec3::splat(scale),
        };
        *visibility = Visibility::Inherited;
        grid.changed = true;
    }
}

/// What is drawn of the part in hand.
#[derive(Default)]
struct InHand {
    parts: Vec<CraftPartDef>,
    root: Option<Entity>,
}

/// Draw what is in hand at the pointer, as the hangar draws its parts, with its middle where it was
/// asked to be: its drawn bounds are worked out afresh, as a model comes in after the meshes.
#[allow(clippy::too_many_arguments)]
fn draw_part_in_hand(
    mut commands: Commands,
    mut hand: ResMut<PartInHand>,
    grid: Res<Grid>,
    content: Res<GameContent>,
    assets: Res<AssetServer>,
    mut looks: ResMut<Looks>,
    (mut meshes, mut materials, mut images): (
        ResMut<Assets<Mesh>>,
        ResMut<Assets<VesselMaterial>>,
        ResMut<Assets<Image>>,
    ),
    children: Query<&Children>,
    mut roots: Query<(&mut Transform, &mut Visibility, &GlobalTransform)>,
    drawn: Query<(&Aabb, &GlobalTransform)>,
    mut in_hand: Local<InHand>,
) {
    if in_hand.parts != hand.parts() || content.is_changed() {
        if let Some(old) = in_hand.root.take() {
            commands.entity(old).despawn();
        }
        in_hand.parts = hand.parts().to_vec();
        if in_hand.parts.is_empty() {
            return;
        }
        let parts = catalog(&content.0.registry, &mut Vec::new());
        match built(&in_hand.parts, &parts) {
            Ok(tree) => {
                let root = commands
                    .spawn((Transform::default(), Visibility::Hidden))
                    .id();
                let file =
                    |path: &str| (content.0.resolve_file(path)).and_then(|f| std::fs::read(f).ok());
                let model = |relative: &str| {
                    let path = mod_asset_path(&content.0, relative)?;
                    Some(WorldAssetRoot(
                        assets.load(GltfAssetLabel::Scene(0).from_asset(path)),
                    ))
                };
                let mut pen = Pen {
                    looks: &mut looks,
                    meshes: &mut meshes,
                    canvas: Canvas {
                        materials: &mut materials,
                        images: &mut images,
                        file: &file,
                    },
                    engine: &grid.engine,
                    model: &model,
                };
                draw(&mut commands, root, &tree, &mut pen);
                in_hand.root = Some(root);
            }
            Err(e) => warn!("cannot draw what is in hand: {e}"),
        }
        return;
    }
    let Some(root) = in_hand.root else {
        return;
    };
    let Ok((mut transform, mut visibility, at)) = roots.get_mut(root) else {
        return;
    };
    // Found whether it is shown or not: the editor works out where its nodes are from it.
    let middle = drawn_middle(root, at, &children, &drawn);
    hand.set_middle(middle);
    let Some((point, rotation)) = hand.place() else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    let Some(middle) = middle else {
        return;
    };
    transform.translation = point - rotation * middle;
    transform.rotation = rotation;
    visibility.set_if_neq(Visibility::Inherited);
}

/// The middle of what is drawn under `root` (placed at `at`), in `root`'s frame: of its drawn
/// bounds, worked out afresh, as a model comes in after the meshes. None while nothing is drawn.
fn drawn_middle(
    root: Entity,
    at: &GlobalTransform,
    children: &Query<&Children>,
    drawn: &Query<(&Aabb, &GlobalTransform)>,
) -> Option<Vec3> {
    let into_root = at.affine().inverse();
    let (mut low, mut high) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for (aabb, place) in (children.iter_descendants(root)).filter_map(|e| drawn.get(e).ok()) {
        let local = into_root * place.affine();
        let (centre, half) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
        for corner in 0..8 {
            let sign = Vec3::new(
                if corner & 1 == 0 { -1.0 } else { 1.0 },
                if corner & 2 == 0 { -1.0 } else { 1.0 },
                if corner & 4 == 0 { -1.0 } else { 1.0 },
            );
            let seen = local.transform_point3(centre + sign * half);
            low = low.min(seen);
            high = high.max(seen);
        }
    }
    low.is_finite().then(|| (low + high) / 2.0)
}

/// How opaque a loose part is drawn: see-through, as KSP draws parts taken off the craft.
const LOOSE_ALPHA: f32 = 0.35;

/// What is drawn of the loose groups: the parts drawn, and each group's root.
#[derive(Default)]
struct LooseDrawn {
    parts: Vec<Vec<CraftPartDef>>,
    roots: Vec<Entity>,
}

/// A mesh of a loose group, in its see-through material.
#[derive(Component)]
struct SeeThrough;

/// Draw the parts left about the editor's hangar ([`LooseParts`]) as the hangar draws its parts,
/// each group with its middle where it was asked to be; drawn again when any of them changes.
#[allow(clippy::too_many_arguments)]
fn draw_loose_parts(
    mut commands: Commands,
    loose: Res<LooseParts>,
    grid: Res<Grid>,
    content: Res<GameContent>,
    assets: Res<AssetServer>,
    mut looks: ResMut<Looks>,
    (mut meshes, mut materials, mut images): (
        ResMut<Assets<Mesh>>,
        ResMut<Assets<VesselMaterial>>,
        ResMut<Assets<Image>>,
    ),
    children: Query<&Children>,
    mut roots: Query<(&mut Transform, &mut Visibility, &GlobalTransform)>,
    drawn: Query<(&Aabb, &GlobalTransform)>,
    mut shown: Local<LooseDrawn>,
) {
    let wanted: Vec<Vec<CraftPartDef>> = (loose.groups().iter())
        .map(|(parts, _)| parts.clone())
        .collect();
    if wanted != shown.parts || content.is_changed() {
        for root in shown.roots.drain(..) {
            commands.entity(root).try_despawn();
        }
        shown.parts = wanted;
        let parts = catalog(&content.0.registry, &mut Vec::new());
        let file = |path: &str| (content.0.resolve_file(path)).and_then(|f| std::fs::read(f).ok());
        let model = |relative: &str| {
            let path = mod_asset_path(&content.0, relative)?;
            Some(WorldAssetRoot(
                assets.load(GltfAssetLabel::Scene(0).from_asset(path)),
            ))
        };
        let mut pen = Pen {
            looks: &mut looks,
            meshes: &mut meshes,
            canvas: Canvas {
                materials: &mut materials,
                images: &mut images,
                file: &file,
            },
            engine: &grid.engine,
            model: &model,
        };
        let mut roots = Vec::new();
        for (index, group) in shown.parts.iter().enumerate() {
            let root = commands
                .spawn((Transform::default(), Visibility::Hidden, ShowsLoose(index)))
                .id();
            match built(group, &parts) {
                Ok(tree) => draw(&mut commands, root, &tree, &mut pen),
                Err(e) => warn!("cannot draw a loose part: {e}"),
            }
            roots.push(root);
        }
        shown.roots = roots;
        return;
    }
    for (index, &root) in shown.roots.iter().enumerate() {
        let Ok((mut transform, mut visibility, at)) = roots.get_mut(root) else {
            continue;
        };
        let place = loose.groups().get(index).and_then(|(_, place)| *place);
        let Some((point, rotation)) = place else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        let Some(middle) = drawn_middle(root, at, &children, &drawn) else {
            continue;
        };
        transform.translation = point - rotation * middle;
        transform.rotation = rotation;
        visibility.set_if_neq(Visibility::Inherited);
    }
}

/// Draw the meshes of loose groups see-through: each in a copy of its own material, faded, that
/// casts no shadow. A model's materials come in after it, so this looks every frame.
#[allow(clippy::type_complexity)]
fn see_through_loose(
    mut commands: Commands,
    roots: Query<Entity, With<ShowsLoose>>,
    children: Query<&Children>,
    vessel: Query<&MeshMaterial3d<VesselMaterial>>,
    standard: Query<&MeshMaterial3d<StandardMaterial>, Without<SeeThrough>>,
    vessel_materials: Res<Assets<VesselMaterial>>,
    mut standard_materials: ResMut<Assets<StandardMaterial>>,
    mut made: Local<HashMap<UntypedAssetId, Handle<StandardMaterial>>>,
) {
    for root in &roots {
        for mesh in children.iter_descendants(root) {
            let source = if let Ok(material) = vessel.get(mesh) {
                let base = vessel_materials.get(&material.0).map(|m| m.base.clone());
                base.map(|base| (material.0.id().untyped(), base))
            } else if let Ok(material) = standard.get(mesh) {
                let base = standard_materials.get(&material.0).cloned();
                base.map(|base| (material.0.id().untyped(), base))
            } else {
                None
            };
            let Some((id, base)) = source else {
                continue;
            };
            let faded = made
                .entry(id)
                .or_insert_with(|| {
                    let mut faded = base;
                    faded.base_color.set_alpha(LOOSE_ALPHA);
                    faded.alpha_mode = AlphaMode::Blend;
                    standard_materials.add(faded)
                })
                .clone();
            commands
                .entity(mesh)
                .remove::<MeshMaterial3d<VesselMaterial>>()
                .insert((MeshMaterial3d(faded), SeeThrough, NotShadowCaster));
        }
    }
}

/// Draw while a picture is on screen and one is unfinished; finish what has been drawn, settled,
/// for long enough.
fn run_camera(
    mut grid: ResMut<Grid>,
    mut pictures: ResMut<PartPictures>,
    compiling: Res<Compiling>,
    mut camera: Single<&mut Camera, With<GridCamera>>,
) {
    let grid = &mut *grid;
    let drawing = camera.is_active;
    if drawing {
        let busy = grid.changed || compiling.0.load(Ordering::Relaxed);
        grid.quiet = if busy { 0 } else { grid.quiet + 1 };
        if grid.quiet >= SETTLE_FRAMES {
            for (index, cell) in grid.cells.iter_mut().enumerate() {
                if let Some(cell) = cell.as_mut().filter(|c| c.framed.is_some() && !c.finished) {
                    cell.finished = true;
                    pictures.set_finished(&cell.name, Some(cell_place(index).1));
                }
            }
        }
    } else {
        grid.quiet = 0;
    }
    let unfinished = grid.cells.iter().flatten().any(|c| !c.finished);
    let draw = pictures.shown() && unfinished;
    if camera.is_active != draw {
        camera.is_active = draw;
    }
}
