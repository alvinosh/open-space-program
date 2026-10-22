//! Planet rendering: a quadtree cube-sphere per body, textured from baked surface data.
//!
//! Each of the six cube faces is subdivided around the camera. A chunk at quadtree key `k`
//! uses the baked data tile with the same key (or an ancestor's, upsampled), so detail
//! follows the data. Chunks are built on worker threads: the mesh (from the shared
//! terrain heights, so the ground you see is the ground you collide with) and three data
//! textures for the terrain shader. Until all of a chunk's replacements are ready, the
//! chunks it replaces stay visible, so the surface never has holes.
//!
//! Every chunk mesh stores its vertices relative to the chunk centre (computed in f64), and
//! the chunk's transform is recomputed each frame relative to the floating origin, so the
//! ground stays stable at millimetre scale even on an Earth-sized planet. Skirts hide
//! cracks between LOD levels.

use std::{f64::consts::FRAC_PI_2, sync::Arc};

use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    light::NotShadowCaster,
    math::{DQuat, DVec3},
    mesh::{Indices, PrimitiveTopology},
    platform::collections::{HashMap, HashSet},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};
use half::f16;
use osp_sim::body::BodyId;
use osp_terrain::{Layer, TILE_CELLS, TILE_SAMPLES, TileKey, cube::east_north};

use crate::{
    FrameSet,
    atmosphere::TransmittanceLuts,
    camera::CameraRig,
    frame::{RenderFrame, rotation_to_render, to_render},
    settings::{GraphicsSettings, Quality},
    sim::{Sim, WorldData},
    surface::Surface,
    terrain_material::{TerrainExtension, TerrainMaterial, TerrainParams},
};

/// Quads per chunk edge. Each quad spans `TILE_CELLS / CHUNK_RESOLUTION` texture cells.
const CHUNK_RESOLUTION: usize = 32;
const STRIDE: usize = TILE_CELLS / CHUNK_RESOLUTION;
/// A chunk splits when the camera is closer than this many chunk widths (at High terrain
/// detail). Each level then has about 40 chunks around the camera.
const SPLIT_DISTANCE: f64 = 2.0;
/// Smallest chunk edge length, m. Bounds the quadtree depth (~76 m chunks, 2.4 m quads on
/// Earth).
const MIN_CHUNK_SIZE: f64 = 70.0;
/// Chunks being built at once.
const MAX_BUILDS: usize = 32;
/// Chunks down to this level are always kept, so there is something to show while finer
/// ones load (otherwise nothing appears until every wanted chunk is ready).
const FALLBACK_LEVEL: u8 = 1;
/// Period of the shader's detail noise along each body-fixed axis, m.
pub const DETAIL_PERIOD: f64 = 4096.0;

pub struct PlanetPlugin;

/// Systems that spawn surface-anchored entities run before this, so new entities are
/// placed in the frame they appear.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlaceAnchored;

impl Plugin for PlanetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_bodies).add_systems(
            Update,
            (
                update_lod.before(PlaceAnchored),
                place_anchored.in_set(PlaceAnchored),
                place_stars,
            )
                .in_set(FrameSet::Render),
        );
    }
}

enum ChunkState {
    Building(Task<BuiltChunk>),
    /// Spawned `age` updates ago.
    Ready(Entity, u8),
}

/// Updates after spawning before a chunk may replace others: its textures and material
/// reach the GPU a frame or two after the entity, and replacing early leaves a hole.
const SETTLE_FRAMES: u8 = 3;

#[derive(Component)]
pub struct Planet {
    pub body: BodyId,
    max_level: u8,
    pub surface: Arc<Surface>,
    chunks: HashMap<TileKey, ChunkState>,
}

/// Marks terrain chunk entities.
#[derive(Component)]
struct PlanetChunk;

/// Something fixed to a body's surface (terrain chunks, trees, buildings). Its mesh is
/// built in body-fixed axes (converted to render axes) relative to `center`; each frame
/// the entity is moved to `center` relative to the floating origin and rotated with the
/// body.
#[derive(Component)]
pub struct SurfaceAnchor {
    pub body: BodyId,
    /// Body-fixed position, m.
    pub center: DVec3,
}

/// Self-luminous body rendered as a simple sphere (no LOD needed: never landed on).
#[derive(Component)]
struct Star(BodyId);

fn spawn_bodies(
    mut commands: Commands,
    sim: Res<Sim>,
    world: Res<WorldData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for body in sim.universe.system.bodies() {
        let [r, g, b] = body.color;
        if body.parent.is_none() {
            commands.spawn((
                Star(body.id),
                Mesh3d(meshes.add(Sphere::new(1.0).mesh().ico(6).unwrap())),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::linear_rgb(r * 4.0, g * 4.0, b * 4.0),
                    unlit: true,
                    ..default()
                })),
                Transform::default(),
                NotShadowCaster,
                Name::new(body.name.clone()),
            ));
            continue;
        }
        let face_size = body.radius * FRAC_PI_2;
        let max_level = (face_size / MIN_CHUNK_SIZE)
            .log2()
            .floor()
            .clamp(0.0, osp_terrain::cube::MAX_LEVEL as f64) as u8;
        commands.spawn((
            Planet {
                body: body.id,
                max_level,
                surface: Arc::new(Surface::new(body, world.datasets.get(&body.id).cloned())),
                chunks: HashMap::default(),
            },
            Name::new(body.name.clone()),
        ));
    }
}

/// The split distance (see [`SPLIT_DISTANCE`]) for each `terrain` setting.
fn split_distance(quality: Quality) -> f64 {
    match quality {
        Quality::Off | Quality::Low => 1.3,
        Quality::Medium => 1.6,
        Quality::High => SPLIT_DISTANCE,
        Quality::Ultra => 2.6,
    }
}

/// Quadtree leaves wanted around the camera, and every internal node above them.
fn wanted_chunks(
    radius: f64,
    camera_fixed: DVec3,
    max_level: u8,
    split: f64,
) -> (HashSet<TileKey>, HashSet<TileKey>) {
    fn visit(
        radius: f64,
        camera: DVec3,
        (max_level, split): (u8, f64),
        key: TileKey,
        leaves: &mut HashSet<TileKey>,
        internal: &mut HashSet<TileKey>,
    ) {
        let center = key.center_dir() * radius;
        let size = key.edge_length(radius);
        if key.level < max_level && camera.distance(center) < size * split {
            internal.insert(key);
            for child in key.children() {
                visit(radius, camera, (max_level, split), child, leaves, internal);
            }
        } else {
            leaves.insert(key);
        }
    }
    let (mut leaves, mut internal) = (HashSet::default(), HashSet::default());
    for face in 0..6 {
        visit(
            radius,
            camera_fixed,
            (max_level, split),
            TileKey::root(face),
            &mut leaves,
            &mut internal,
        );
    }
    (leaves, internal)
}

fn update_lod(
    mut commands: Commands,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    mut planets: Query<&mut Planet>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    luts: Res<TransmittanceLuts>,
    settings: Res<GraphicsSettings>,
    mut visibility: Query<&mut Visibility, With<PlanetChunk>>,
) {
    if frame.origin_body.is_none() {
        return;
    }
    let split = split_distance(settings.terrain);
    let pool = AsyncComputeTaskPool::get();
    for mut planet in &mut planets {
        let planet = &mut *planet;
        let body = sim.universe.system.body(planet.body);
        let camera_from_center = rig.position - frame.body_offset(planet.body);
        let camera_fixed = body.orientation(frame.time).inverse() * camera_from_center;
        let (leaves, internal) =
            wanted_chunks(body.radius, camera_fixed, planet.max_level, split);
        let keep = |key: &TileKey| {
            leaves.contains(key) || (key.level <= FALLBACK_LEVEL && internal.contains(key))
        };

        // Collect finished builds.
        for state in planet.chunks.values_mut() {
            if let ChunkState::Ready(_, age) = state {
                *age = age.saturating_add(1);
                continue;
            }
            let ChunkState::Building(task) = state else {
                continue;
            };
            let Some(built) = block_on(poll_once(task)) else {
                continue;
            };
            let entity = built.spawn(
                &mut commands,
                planet.body,
                (luts.texture(planet.body), &settings),
                &mut meshes,
                &mut images,
                &mut materials,
            );
            *state = ChunkState::Ready(entity, 0);
        }

        // Cancel builds that are no longer wanted.
        planet
            .chunks
            .retain(|key, state| !matches!(state, ChunkState::Building(_)) || keep(key));

        // Start builds, coarse levels first so fallbacks exist, then nearest first.
        let building = planet
            .chunks
            .values()
            .filter(|s| matches!(s, ChunkState::Building(_)))
            .count();
        let mut missing: Vec<TileKey> = leaves
            .iter()
            .chain(internal.iter().filter(|k| k.level <= FALLBACK_LEVEL))
            .filter(|k| !planet.chunks.contains_key(*k))
            .copied()
            .collect();
        missing.sort_by(|a, b| {
            let da = camera_fixed.distance(a.center_dir() * body.radius);
            let db = camera_fixed.distance(b.center_dir() * body.radius);
            a.level.cmp(&b.level).then(da.total_cmp(&db))
        });
        for key in missing
            .into_iter()
            .take(MAX_BUILDS.saturating_sub(building))
        {
            let surface = planet.surface.clone();
            let task = pool.spawn(async move { build_chunk(&surface, key) });
            planet.chunks.insert(key, ChunkState::Building(task));
        }

        // Decide what to show: wanted chunks where ready, otherwise what they replace.
        let ready: HashSet<TileKey> = planet
            .chunks
            .iter()
            .filter(|(_, s)| matches!(s, ChunkState::Ready(_, age) if *age >= SETTLE_FRAMES))
            .map(|(k, _)| *k)
            .collect();
        let mut ready_ancestors: HashSet<TileKey> = HashSet::default();
        for key in &ready {
            let mut k = *key;
            while let Some(parent) = k.parent() {
                if !ready_ancestors.insert(parent) {
                    break;
                }
                k = parent;
            }
        }
        let mut shown: HashSet<TileKey> = HashSet::default();
        for face in 0..6 {
            if let Some(keys) = cover(
                TileKey::root(face),
                &leaves,
                &internal,
                &ready,
                &ready_ancestors,
            ) {
                shown.extend(keys);
            }
        }

        planet.chunks.retain(|key, state| {
            let ChunkState::Ready(entity, _) = state else {
                return true;
            };
            if shown.contains(key) {
                if let Ok(mut v) = visibility.get_mut(*entity) {
                    v.set_if_neq(Visibility::Inherited);
                }
                true
            } else if keep(key) {
                // Ready, but waiting for its siblings before replacing its parent (or kept
                // as a fallback).
                if let Ok(mut v) = visibility.get_mut(*entity) {
                    v.set_if_neq(Visibility::Hidden);
                }
                true
            } else {
                commands.entity(*entity).despawn();
                false
            }
        });
    }
}

/// Chunks that cover `key`'s area with ready meshes: the wanted ones where possible, else
/// what they replace. `None` if the area cannot be covered yet.
fn cover(
    key: TileKey,
    leaves: &HashSet<TileKey>,
    internal: &HashSet<TileKey>,
    ready: &HashSet<TileKey>,
    ready_ancestors: &HashSet<TileKey>,
) -> Option<Vec<TileKey>> {
    if leaves.contains(&key) {
        if ready.contains(&key) {
            return Some(vec![key]);
        }
        // Merging: show the finer chunks being replaced until this one is ready.
        return old_descendants(key, ready, ready_ancestors);
    }
    if internal.contains(&key) {
        let mut all = Vec::new();
        let mut complete = true;
        for child in key.children() {
            match cover(child, leaves, internal, ready, ready_ancestors) {
                Some(keys) => all.extend(keys),
                None => {
                    complete = false;
                    break;
                }
            }
        }
        if complete {
            return Some(all);
        }
        // Splitting: keep showing this chunk until all of its replacements are ready.
        if ready.contains(&key) {
            return Some(vec![key]);
        }
    }
    None
}

fn old_descendants(
    key: TileKey,
    ready: &HashSet<TileKey>,
    ready_ancestors: &HashSet<TileKey>,
) -> Option<Vec<TileKey>> {
    if ready.contains(&key) {
        return Some(vec![key]);
    }
    if !ready_ancestors.contains(&key) {
        return None;
    }
    let mut all = Vec::new();
    for child in key.children() {
        all.extend(old_descendants(child, ready, ready_ancestors)?);
    }
    Some(all)
}

/// A chunk built on a worker thread, ready to become assets.
struct BuiltChunk {
    mesh: Mesh,
    color: Image,
    surface: Image,
    cover: Image,
    center: DVec3,
    params: TerrainParams,
}

impl BuiltChunk {
    fn spawn(
        mut self,
        commands: &mut Commands,
        body: BodyId,
        (transmittance, settings): (Handle<Image>, &GraphicsSettings),
        meshes: &mut Assets<Mesh>,
        images: &mut Assets<Image>,
        materials: &mut Assets<TerrainMaterial>,
    ) -> Entity {
        self.params.quality.x = crate::atmosphere::view_steps(settings.effects) as f32;
        let material = materials.add(TerrainMaterial {
            base: StandardMaterial {
                base_color: Color::WHITE,
                perceptual_roughness: 0.9,
                // Skirts face sideways; render both sides rather than fuss over winding.
                cull_mode: None,
                ..default()
            },
            extension: TerrainExtension {
                params: self.params,
                color: images.add(self.color),
                surface: images.add(self.surface),
                cover: images.add(self.cover),
                transmittance,
            },
        });
        commands
            .spawn((
                PlanetChunk,
                SurfaceAnchor {
                    body,
                    center: self.center,
                },
                Mesh3d(meshes.add(self.mesh)),
                MeshMaterial3d(material),
                Transform::default(),
                Visibility::Hidden,
                NotShadowCaster,
            ))
            .id()
    }
}

fn data_image(format: TextureFormat, data: Vec<u8>, filtered: bool) -> Image {
    let size = TILE_SAMPLES as u32;
    let mut image = Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    let filter = if filtered {
        ImageFilterMode::Linear
    } else {
        ImageFilterMode::Nearest
    };
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: filter,
        min_filter: filter,
        ..default()
    });
    image
}

/// Build a chunk's mesh and data textures (runs on a worker thread).
fn build_chunk(surface: &Surface, key: TileKey) -> BuiltChunk {
    let n = TILE_SAMPLES;
    let radius = surface.radius;
    let level = key.level;

    // Heights on the sample grid plus a one-sample border (for normals at the edges). Inside
    // the tile, read the elevation tile directly; the border lies in neighbouring tiles.
    let data = surface.data.as_deref();
    let height_view = data.and_then(|d| d.view(Layer::Height, key));
    let bordered = n + 2;
    let mut dirs = Vec::with_capacity(bordered * bordered);
    let mut heights = Vec::with_capacity(bordered * bordered);
    for j in 0..bordered {
        for i in 0..bordered {
            let (fi, fj) = (i as f64 - 1.0, j as f64 - 1.0);
            let dir = key.sample_dir(fi, fj);
            let inside = (1..=n).contains(&i) && (1..=n).contains(&j);
            let height = match &height_view {
                Some(view) if inside => {
                    surface
                        .terrain
                        .height_over(dir, view.height(fi, fj) as f64, level)
                }
                _ => surface.terrain.height_lod(dir, level),
            };
            heights.push(height);
            dirs.push(dir);
        }
    }
    let at = |i: usize, j: usize| (j + 1) * bordered + (i + 1);
    let position = |k: usize| dirs[k] * (radius + heights[k]);

    let center = position(at(n / 2, n / 2));

    // Textures.
    let color_view = data.and_then(|d| d.view(Layer::Color, key));
    let lights_view = data.and_then(|d| d.view(Layer::Lights, key));
    let cover_view = surface.cover_view(key);

    let mut color = Vec::with_capacity(n * n * 4);
    let mut surface_texels = Vec::with_capacity(n * n * 8);
    let mut cover = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            let (fi, fj) = (i as f64, j as f64);
            let k = at(i, j);
            let dir = dirs[k];

            let rgb = color_view.as_ref().map_or(surface.fallback_color, |v| {
                v.rgb(fi, fj).map(|c| c.round() as u8)
            });
            let lights = lights_view
                .as_ref()
                .map_or(0, |v| v.byte_linear(0, fi, fj).round() as u8);
            color.extend([rgb[0], rgb[1], rgb[2], lights]);

            // Normal from neighbouring positions, as east/north components.
            let dx = position(k + 1) - position(k - 1);
            let dy = position(k + bordered) - position(k - bordered);
            let mut normal = dx.cross(dy).normalize_or(dir);
            if normal.dot(dir) < 0.0 {
                normal = -normal;
            }
            let (east, north) = east_north(dir);
            let raw_height = height_view
                .as_ref()
                .map_or(heights[k], |v| v.height(fi, fj) as f64);

            let natural = cover_view
                .as_ref()
                .map(|v| surface.cover_from_view(v, fi, fj, dir))
                .unwrap_or((surface.fallback_cover, 0.0));
            let (class, water) = surface.paved(dir, natural);
            for value in [
                normal.dot(east) as f32,
                normal.dot(north) as f32,
                raw_height as f32,
                water,
            ] {
                surface_texels.extend(f16::from_f32(value).to_le_bytes());
            }
            cover.push(class as u8);
        }
    }

    // Mesh: every STRIDE-th sample, relative to the chunk centre, plus skirts.
    let res = CHUNK_RESOLUTION;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let grid_index = |i: usize, j: usize| (j * (res + 1) + i) as u32;
    let mut edge = Vec::new();
    for j in 0..=res {
        for i in 0..=res {
            let k = at(i * STRIDE, j * STRIDE);
            let uv = [i as f32 / res as f32, j as f32 / res as f32];
            positions.push(to_render(position(k) - center).to_array());
            normals.push(to_render(dirs[k]).to_array());
            uvs.push(uv);
            if i == 0 || j == 0 || i == res || j == res {
                edge.push((i, j, k, uv));
            }
        }
    }
    let mut indices: Vec<u32> = Vec::new();
    for j in 0..res {
        for i in 0..res {
            let a = grid_index(i, j);
            let b = grid_index(i + 1, j);
            let c = grid_index(i, j + 1);
            let d = grid_index(i + 1, j + 1);
            indices.extend([a, b, d, a, d, c]);
        }
    }
    let cell = key.edge_length(radius) / res as f64;
    let skirt_depth = (cell * 2.0).max(1.0);
    let first_skirt = ((res + 1) * (res + 1)) as u32;
    let mut skirt_of: HashMap<(usize, usize), u32> = HashMap::default();
    for (s, &(i, j, k, uv)) in edge.iter().enumerate() {
        positions.push(to_render(position(k) - dirs[k] * skirt_depth - center).to_array());
        normals.push(to_render(dirs[k]).to_array());
        uvs.push(uv);
        skirt_of.insert((i, j), first_skirt + s as u32);
    }
    let edge_point = |side: usize, k: usize| match side {
        0 => (k, 0),
        1 => (k, res),
        2 => (0, k),
        _ => (res, k),
    };
    for side in 0..4 {
        for k in 0..res {
            let (i0, j0) = edge_point(side, k);
            let (i1, j1) = edge_point(side, k + 1);
            let (top0, top1) = (grid_index(i0, j0), grid_index(i1, j1));
            let (low0, low1) = (skirt_of[&(i0, j0)], skirt_of[&(i1, j1)]);
            indices.extend([top0, low0, low1, top0, low1, top1]);
        }
    }
    // Make every triangle counter-clockwise seen from outside, whatever each cube face's
    // axes are.
    for tri in indices.chunks_exact_mut(3) {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| Vec3::from(positions[i as usize]));
        let outward = Vec3::from(normals[tri[0] as usize]);
        if (b - a).cross(c - a).dot(outward) < 0.0 {
            tri.swap(1, 2);
        }
    }
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices));

    // Render axes in f64 (see `frame::to_render`), wrapped before narrowing to f32.
    let detail = DVec3::new(center.x, center.z, -center.y);
    let wrap = |v: f64| v.rem_euclid(DETAIL_PERIOD) as f32;
    let data_level =
        |view: &Option<osp_terrain::TileView>| view.as_ref().map_or(0.0, |v| v.data_level() as f32);
    let params = TerrainParams {
        planet: to_render(-center).extend(radius as f32),
        detail_origin: Vec4::new(wrap(detail.x), wrap(detail.y), wrap(detail.z), 0.0),
        levels: Vec4::new(
            level as f32,
            data_level(&color_view),
            data_level(&cover_view),
            if surface.earthlike { 1.0 } else { 0.0 },
        ),
        // Set when the chunk spawns.
        quality: Vec4::ZERO,
    };
    BuiltChunk {
        mesh,
        color: data_image(TextureFormat::Rgba8UnormSrgb, color, true),
        surface: data_image(TextureFormat::Rgba16Float, surface_texels, true),
        cover: data_image(TextureFormat::R8Uint, cover, false),
        center,
        params,
    }
}

fn place_anchored(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    mut anchored: Query<(&SurfaceAnchor, &mut Transform)>,
) {
    if frame.origin_body.is_none() {
        return;
    }
    let mut orientations: HashMap<BodyId, DQuat> = HashMap::default();
    for (anchor, mut transform) in &mut anchored {
        let rotation = *orientations.entry(anchor.body).or_insert_with(|| {
            sim.universe
                .system
                .body(anchor.body)
                .orientation(frame.time)
        });
        // Body offset and anchor are both f64; only the small sum becomes f32.
        transform.translation = frame.to_render(anchor.body, rotation * anchor.center);
        transform.rotation = rotation_to_render(rotation);
    }
}

fn place_stars(sim: Res<Sim>, frame: Res<RenderFrame>, mut stars: Query<(&Star, &mut Transform)>) {
    if frame.origin_body.is_none() {
        return;
    }
    for (star, mut transform) in &mut stars {
        let body = sim.universe.system.body(star.0);
        transform.translation = frame.to_render(star.0, DVec3::ZERO);
        transform.scale = Vec3::splat(body.radius as f32);
    }
}
