//! Trees and bushes near the camera, placed from the land-cover data.
//!
//! The ground is split into fixed cells (cube-sphere quadtree nodes at [`CELL_LEVEL`]),
//! independent of the terrain LOD so trees do not jump around as chunks split. Each cell
//! within [`RADIUS`] gets one merged mesh built on a worker thread: candidates on a jittered
//! grid are kept according to the land-cover class there, and the class and latitude pick
//! the species (pines in needleleaf forest, palms and broadleaf trees in the subtropics,
//! mangroves along tropical coasts, ...). Cells near the camera get detailed trees that
//! cast shadows; farther ones get cheaper shapes.

use std::sync::Arc;

use bevy::{
    light::NotShadowCaster,
    math::DVec3,
    platform::collections::HashMap,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
};
use osp_sim::body::BodyId;
use osp_terrain::{
    Cover, TILE_CELLS, TileKey,
    cube::{east_north, lat_lon},
    noise::{fbm, random},
};

use crate::{
    FrameSet,
    camera::CameraRig,
    frame::{RenderFrame, to_render},
    meshgen::MeshBuilder,
    planet::{PlaceAnchored, Planet, SurfaceAnchor},
    settings::{GraphicsSettings, Quality},
    sim::Sim,
    surface::Surface,
};

/// Quadtree level of tree cells (~305 m on Earth).
const CELL_LEVEL: u8 = 15;
/// Trees are shown within this distance of the camera, m (at High).
const RADIUS: f64 = 2_500.0;
/// Within this distance cells get detailed trees, m (at High).
const DETAIL_RADIUS: f64 = 900.0;
/// Cells keep their trees, and their detail, this far past [`RADIUS`] and
/// [`DETAIL_RADIUS`], m. More than the camera moves orbiting the vessel at the default
/// zoom (twice the 120 m distance), so turning the view does not reload trees.
const HYSTERESIS: f64 = 300.0;
/// No trees above this camera altitude, m.
const MAX_ALTITUDE: f64 = 6_000.0;
/// Candidate spacing, m.
const SPACING: f64 = 6.0;
/// Cells being built at once.
const MAX_BUILDS: usize = 16;

pub struct TreesPlugin;

impl Plugin for TreesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(
            Update,
            update_trees.before(PlaceAnchored).in_set(FrameSet::Render),
        );
    }
}

enum CellState {
    Building {
        task: Task<Option<TreeMesh>>,
        detailed: bool,
        /// The cell's previous mesh, shown until this build lands.
        previous: Option<Entity>,
    },
    Ready {
        entity: Option<Entity>,
        detailed: bool,
    },
}

impl CellState {
    fn entities(&self) -> Option<Entity> {
        match self {
            CellState::Building { previous, .. } => *previous,
            CellState::Ready { entity, .. } => *entity,
        }
    }

    fn detailed(&self) -> bool {
        match self {
            CellState::Building { detailed, .. } | CellState::Ready { detailed, .. } => *detailed,
        }
    }
}

/// ([`RADIUS`], [`DETAIL_RADIUS`]) for each `trees` setting; none when trees are off.
fn reach(quality: Quality) -> Option<(f64, f64)> {
    match quality {
        Quality::Off => None,
        Quality::Low => Some((1_000.0, 350.0)),
        Quality::Medium => Some((1_700.0, 600.0)),
        Quality::High => Some((RADIUS, DETAIL_RADIUS)),
        Quality::Ultra => Some((3_500.0, 1_300.0)),
    }
}

/// Whether a cell `distance` from the camera should have trees, and whether detailed
/// ones; `current` is the detail of the trees it has (or is building), if any, and
/// `(radius, detail_radius)` the setting's [`reach`].
fn wanted_detail(
    distance: f64,
    current: Option<bool>,
    (radius, detail_radius): (f64, f64),
) -> Option<bool> {
    let slack = |kept: bool| if kept { HYSTERESIS } else { 0.0 };
    (distance < radius + slack(current.is_some()))
        .then(|| distance < detail_radius + slack(current == Some(true)))
}

/// Cells within `reach` of `camera` (body-fixed, m) on a sphere of `radius`, with their
/// distances (roughly to the cell's near edge). Walks down the quadtree from the faces, so
/// every cell in reach is found whatever its size and orientation; probing a grid around
/// the camera skipped some, and their trees blinked out as the camera moved.
fn cells_near(camera: DVec3, radius: f64, reach: f64) -> Vec<(TileKey, f64)> {
    fn visit(camera: DVec3, radius: f64, reach: f64, key: TileKey, out: &mut Vec<(TileKey, f64)>) {
        let size = key.edge_length(radius);
        let distance = camera.distance(key.center_dir() * radius);
        if key.level == CELL_LEVEL {
            let distance = distance - size * 0.7;
            if distance < reach {
                out.push((key, distance));
            }
        } else if distance - 2.0 * size < reach {
            // Twice the edge covers the node's extent plus a cell's own slack.
            for child in key.children() {
                visit(camera, radius, reach, child, out);
            }
        }
    }
    let mut cells = Vec::new();
    for face in 0..6 {
        visit(camera, radius, reach, TileKey::root(face), &mut cells);
    }
    cells
}

#[derive(Resource)]
struct Trees {
    /// Replaced meshes, despawned once their successors have had time to reach the GPU.
    retiring: Vec<(Entity, u8)>,
    body: Option<BodyId>,
    cells: HashMap<TileKey, CellState>,
    material: Handle<StandardMaterial>,
}

struct TreeMesh {
    mesh: Mesh,
    center: DVec3,
}

fn setup(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(Trees {
        retiring: Vec::new(),
        body: None,
        cells: HashMap::default(),
        // Colour comes from the vertices; fronds are single sheets, so draw both sides.
        material: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.85,
            reflectance: 0.3,
            cull_mode: None,
            double_sided: true,
            ..default()
        }),
    });
}

fn update_trees(
    mut commands: Commands,
    mut trees: ResMut<Trees>,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    planets: Query<&Planet>,
    settings: Res<GraphicsSettings>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let trees = &mut *trees;
    let Some(body_id) = frame.origin_body else {
        return;
    };
    let body = sim.universe.system.body(body_id);
    let surface = planets
        .iter()
        .find(|p| p.body == body_id)
        .map(|p| p.surface.clone())
        .filter(|s| s.data.is_some());
    let camera_fixed =
        body.orientation(frame.time).inverse() * (rig.position - frame.body_offset(body_id));
    let altitude = camera_fixed.length() - body.radius;

    // Switching bodies (or losing data) clears everything.
    if trees.body != Some(body_id) {
        for (_, state) in trees.cells.drain() {
            if let Some(entity) = state.entities() {
                commands.entity(entity).despawn();
            }
        }
        trees.body = Some(body_id);
    }

    // Wanted cells and whether each should be detailed.
    let mut wanted: HashMap<TileKey, bool> = HashMap::default();
    if let Some(surface) = &surface
        && let Some(reach) = reach(settings.trees)
        && altitude < MAX_ALTITUDE
        && !rig.map_view
    {
        for (key, distance) in cells_near(camera_fixed, surface.radius, reach.0 + HYSTERESIS) {
            let current = trees.cells.get(&key).map(CellState::detailed);
            if let Some(detailed) = wanted_detail(distance, current, reach) {
                wanted.insert(key, detailed);
            }
        }
    }

    trees.retiring.retain_mut(|(entity, age)| {
        *age += 1;
        let done = *age > 3;
        if done {
            commands.entity(*entity).despawn();
        }
        !done
    });

    // Finished builds.
    for (_, state) in trees.cells.iter_mut() {
        let CellState::Building {
            task,
            detailed,
            previous,
        } = state
        else {
            continue;
        };
        let Some(result) = block_on(poll_once(task)) else {
            continue;
        };
        if let Some(previous) = previous.take() {
            trees.retiring.push((previous, 0));
        }
        let detailed = *detailed;
        let entity = result.map(|built| {
            let mut entity = commands.spawn((
                Mesh3d(meshes.add(built.mesh)),
                MeshMaterial3d(trees.material.clone()),
                SurfaceAnchor {
                    body: body_id,
                    center: built.center,
                },
                Transform::default(),
            ));
            if !detailed {
                entity.insert(NotShadowCaster);
            }
            entity.id()
        });
        *state = CellState::Ready { entity, detailed };
    }

    // Drop cells no longer wanted. A cell whose detail changed keeps its mesh until the
    // rebuild lands, and one that changes again mid-build finishes and is rebuilt after:
    // dropping it would take its old mesh with it and leave a gap.
    trees.cells.retain(|key, state| {
        let keep = wanted.contains_key(key);
        if !keep && let Some(entity) = state.entities() {
            commands.entity(entity).despawn();
        }
        keep
    });

    let Some(surface) = surface else {
        return;
    };
    let building = trees
        .cells
        .values()
        .filter(|s| matches!(s, CellState::Building { .. }))
        .count();
    let mut budget = MAX_BUILDS.saturating_sub(building);
    let pool = AsyncComputeTaskPool::get();
    // Nearest cells first.
    let mut todo: Vec<(TileKey, bool)> = wanted
        .iter()
        .filter(|(key, detailed)| match trees.cells.get(*key) {
            None => true,
            Some(CellState::Ready { detailed: d, .. }) => d != *detailed,
            Some(CellState::Building { .. }) => false,
        })
        .map(|(k, d)| (*k, *d))
        .collect();
    todo.sort_by(|a, b| {
        let da = camera_fixed.distance(a.0.center_dir() * body.radius);
        let db = camera_fixed.distance(b.0.center_dir() * body.radius);
        da.total_cmp(&db)
    });
    for (key, detailed) in todo {
        if budget == 0 {
            break;
        }
        budget -= 1;
        let surface: Arc<Surface> = surface.clone();
        let task = pool.spawn(async move { build_cell(&surface, key, detailed) });
        let previous = trees.cells.remove(&key).and_then(|s| s.entities());
        trees.cells.insert(
            key,
            CellState::Building {
                task,
                detailed,
                previous,
            },
        );
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Species {
    Pine,
    Broadleaf,
    Palm,
    Mangrove,
    Bush,
}

/// Fraction of candidate spots with a tree, and which species, for a land-cover class.
fn vegetation(class: Cover, latitude: f64, roll: f64) -> Option<(f64, Species)> {
    let subtropical = latitude.abs() < 32.0;
    use Species::*;
    let (density, species) = match class {
        Cover::NeedleleafForest => (0.75, Pine),
        Cover::TropicalForest => (0.8, if roll < 0.3 { Palm } else { Broadleaf }),
        Cover::DeciduousForest => (0.75, Broadleaf),
        Cover::MixedForest => (
            0.75,
            if subtropical && roll < 0.25 {
                Palm
            } else if roll < 0.6 {
                Pine
            } else {
                Broadleaf
            },
        ),
        Cover::Savanna => (
            0.2,
            if subtropical && roll < 0.35 {
                Palm
            } else {
                Broadleaf
            },
        ),
        Cover::Mangrove => (0.85, Mangrove),
        Cover::Shrubland => (0.35, if roll < 0.08 { Broadleaf } else { Bush }),
        Cover::Wetland => (
            0.08,
            if subtropical && roll < 0.5 {
                Palm
            } else {
                Bush
            },
        ),
        Cover::Urban => (
            0.04,
            if subtropical && roll < 0.4 {
                Palm
            } else {
                Broadleaf
            },
        ),
        Cover::Grassland | Cover::Cropland => (0.015, Broadleaf),
        Cover::Barren | Cover::Tundra => (0.02, Bush),
        _ => return None,
    };
    Some((density, species))
}

fn build_cell(surface: &Surface, key: TileKey, detailed: bool) -> Option<TreeMesh> {
    let cell = key.edge_length(surface.radius);
    let count = (cell / SPACING).ceil() as usize;
    let center_dir = key.center_dir();
    let center = center_dir * (surface.radius + surface.terrain.height(center_dir));
    let id = (
        ((key.face as i64) << 32) | key.x as i64,
        key.y as i64,
        key.level as i64,
    );
    let mut builder = MeshBuilder::default();
    for gj in 0..count {
        for gi in 0..count {
            let index = (gj * count + gi) as i64;
            let rand = |channel: i64| random(id, index * 16 + channel);
            let i = (gi as f64 + rand(0)) / count as f64 * TILE_CELLS as f64;
            let j = (gj as f64 + rand(1)) / count as f64 * TILE_CELLS as f64;
            let dir = key.sample_dir(i, j);
            let (class, water) = surface.cover_at(dir, u8::MAX);
            if water > 0.25 || surface.terrain.is_cleared(dir) {
                continue;
            }
            let latitude = lat_lon(dir).0.to_degrees();
            let Some((density, species)) = vegetation(class, latitude, rand(2)) else {
                continue;
            };
            // Clumps and clearings rather than an even spread.
            let clumping = 0.5 + 0.8 * fbm(dir * (surface.radius / 60.0), 1.0);
            if rand(3) > density * clumping.clamp(0.0, 1.6) {
                continue;
            }
            let ground = dir * (surface.radius + surface.terrain.height(dir));
            let base = to_render(ground - center);
            let (east, north) = east_north(dir);
            let up = to_render(dir);
            let heading = rand(4) * std::f64::consts::TAU;
            let right = to_render(east * heading.cos() + north * heading.sin());
            let forward = up.cross(right);
            let tree = Tree {
                base: base - up * 0.3,
                up,
                right,
                forward,
                scale: 0.75 + 0.5 * rand(5) as f32,
                tint: 0.8 + 0.4 * rand(6) as f32,
                lean: (rand(7) as f32 - 0.5) * 0.12,
                seed: rand(8) as f32,
            };
            tree.build(&mut builder, species, detailed);
        }
    }
    (!builder.is_empty()).then(|| TreeMesh {
        mesh: builder.build(),
        center,
    })
}

/// One tree's placement and random variation.
struct Tree {
    base: Vec3,
    up: Vec3,
    right: Vec3,
    forward: Vec3,
    scale: f32,
    tint: f32,
    lean: f32,
    seed: f32,
}

fn color(rgb: [f32; 3], tint: f32) -> LinearRgba {
    LinearRgba::rgb(rgb[0] * tint, rgb[1] * tint, rgb[2] * tint)
}

impl Tree {
    /// Point `x` right, `y` up, `z` forward of the tree base.
    fn at(&self, x: f32, y: f32, z: f32) -> Vec3 {
        self.base + self.right * x + self.up * y + self.forward * z
    }

    fn rotation(&self) -> Quat {
        Quat::from_mat3(&Mat3::from_cols(self.right, self.up, -self.forward))
    }

    fn build(&self, b: &mut MeshBuilder, species: Species, detailed: bool) {
        let s = self.scale;
        let bark = color([0.07, 0.05, 0.035], self.tint);
        let lumpy = |seed: f32| {
            move |d: Vec3| {
                1.0 + 0.18 * ((d.x * 5.1 + seed * 7.0).sin() * (d.z * 4.3 + d.y * 3.7).cos())
            }
        };
        match species {
            Species::Pine => {
                let h = 16.0 * s;
                let needles = color([0.025, 0.05, 0.025], self.tint);
                let lean = self.right * self.lean;
                let axis = (self.up + lean).normalize();
                if detailed {
                    b.frustum(
                        self.base,
                        axis * h * 0.5,
                        (0.22 * s, 0.12 * s),
                        5,
                        false,
                        bark,
                    );
                    for (k, (at, radius, height)) in
                        [(0.3, 0.26, 0.42), (0.48, 0.21, 0.36), (0.64, 0.15, 0.36)]
                            .iter()
                            .enumerate()
                    {
                        let shade =
                            color([0.025, 0.05, 0.025], self.tint * (0.9 + 0.08 * k as f32));
                        b.frustum(
                            self.base + axis * h * at,
                            axis * h * height,
                            (h * radius, 0.0),
                            7,
                            false,
                            shade,
                        );
                    }
                } else {
                    b.frustum(
                        self.base + axis * h * 0.2,
                        axis * h * 0.8,
                        (h * 0.24, 0.0),
                        5,
                        false,
                        needles,
                    );
                }
            }
            Species::Broadleaf => {
                let h = 11.0 * s;
                let leaves = color([0.045, 0.085, 0.03], self.tint);
                let top = self.at(self.lean * h, h * 0.62, 0.0);
                if detailed {
                    b.frustum(
                        self.base,
                        top - self.base,
                        (0.25 * s, 0.15 * s),
                        5,
                        false,
                        bark,
                    );
                    b.blob(
                        top,
                        Vec3::new(h * 0.36, h * 0.3, h * 0.36),
                        self.rotation(),
                        1,
                        leaves,
                        lumpy(self.seed),
                    );
                } else {
                    b.blob(
                        top,
                        Vec3::new(h * 0.36, h * 0.32, h * 0.36),
                        self.rotation(),
                        0,
                        leaves,
                        |_| 1.0,
                    );
                    b.beam(self.base, top, 0.4 * s, bark);
                }
            }
            Species::Palm => {
                let h = 10.0 * s;
                let fronds = color([0.06, 0.095, 0.03], self.tint);
                let trunk = color([0.11, 0.09, 0.07], self.tint);
                // Gently curved trunk.
                let mid = self.at(self.lean * h * 0.3, h * 0.5, 0.0);
                let top = self.at(self.lean * h, h, 0.0);
                let sides = if detailed { 6 } else { 4 };
                b.frustum(
                    self.base,
                    mid - self.base,
                    (0.2 * s, 0.17 * s),
                    sides,
                    false,
                    trunk,
                );
                b.frustum(mid, top - mid, (0.17 * s, 0.15 * s), sides, false, trunk);
                let count = if detailed { 10 } else { 5 };
                for k in 0..count {
                    let angle = std::f32::consts::TAU * (k as f32 + self.seed) / count as f32;
                    let out = (self.right * angle.cos() + self.forward * angle.sin()).normalize();
                    let length = h * (0.32 + 0.06 * ((k * 7) % 3) as f32);
                    b.frond(
                        top,
                        (out + self.up * 0.35).normalize(),
                        self.up,
                        length,
                        h * 0.13,
                        fronds,
                    );
                }
                if detailed {
                    b.blob(
                        top,
                        Vec3::splat(h * 0.06),
                        self.rotation(),
                        0,
                        fronds,
                        |_| 1.0,
                    );
                }
            }
            Species::Mangrove => {
                let h = 5.0 * s;
                let leaves = color([0.03, 0.06, 0.025], self.tint);
                let crown = self.at(0.0, h * 0.6, 0.0);
                if detailed {
                    for k in 0..3 {
                        let angle = std::f32::consts::TAU * (k as f32 / 3.0 + self.seed);
                        let foot = self.at(angle.cos() * h * 0.3, 0.0, angle.sin() * h * 0.3);
                        b.beam(foot, crown, 0.12 * s, bark);
                    }
                }
                b.blob(
                    crown,
                    Vec3::new(h * 0.6, h * 0.35, h * 0.6),
                    self.rotation(),
                    u32::from(detailed),
                    leaves,
                    lumpy(self.seed),
                );
            }
            Species::Bush => {
                let h = 1.8 * s;
                let leaves = color([0.05, 0.075, 0.03], self.tint);
                b.blob(
                    self.at(0.0, h * 0.4, 0.0),
                    Vec3::new(h * 0.7, h * 0.5, h * 0.7),
                    self.rotation(),
                    u32::from(detailed),
                    leaves,
                    lumpy(self.seed),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use osp_terrain::cube::sphere_to_cube;

    use super::*;

    const HIGH: (f64, f64) = (RADIUS, DETAIL_RADIUS);

    #[test]
    fn new_cells_use_the_plain_radii() {
        assert_eq!(wanted_detail(DETAIL_RADIUS - 1.0, None, HIGH), Some(true));
        assert_eq!(wanted_detail(DETAIL_RADIUS + 1.0, None, HIGH), Some(false));
        assert_eq!(wanted_detail(RADIUS + 1.0, None, HIGH), None);
    }

    #[test]
    fn shown_cells_keep_their_trees_and_detail_past_the_radii() {
        let near = DETAIL_RADIUS + HYSTERESIS - 1.0;
        assert_eq!(wanted_detail(near, Some(true), HIGH), Some(true));
        assert_eq!(wanted_detail(near, Some(false), HIGH), Some(false));
        assert_eq!(
            wanted_detail(DETAIL_RADIUS - 1.0, Some(false), HIGH),
            Some(true)
        );
        assert_eq!(
            wanted_detail(DETAIL_RADIUS + HYSTERESIS + 1.0, Some(true), HIGH),
            Some(false)
        );

        let far = RADIUS + HYSTERESIS - 1.0;
        assert_eq!(wanted_detail(far, Some(false), HIGH), Some(false));
        assert_eq!(
            wanted_detail(RADIUS + HYSTERESIS + 1.0, Some(false), HIGH),
            None
        );
    }

    #[test]
    fn every_cell_in_reach_is_found() {
        let radius = 6_371_000.0;
        let reach = RADIUS + HYSTERESIS;
        let corner = DVec3::ONE.normalize();
        // The launch site, a face centre, and a cube corner (cells most skewed).
        for ground in [
            osp_terrain::cube::from_lat_lon(28.6f64.to_radians(), -80.6f64.to_radians()),
            DVec3::X,
            corner,
        ] {
            let camera = ground * (radius + 150.0);
            let found: HashMap<TileKey, f64> =
                cells_near(camera, radius, reach).into_iter().collect();
            // Every cell touched by a dense sample around the camera.
            let (east, north) = east_north(ground);
            let step = 20.0;
            let n = ((reach + 500.0) / step) as i32;
            for dy in -n..=n {
                for dx in -n..=n {
                    let offset = (east * dx as f64 + north * dy as f64) * step;
                    let (face, s, t) = sphere_to_cube((ground * radius + offset).normalize());
                    let key = TileKey::containing(face, s, t, CELL_LEVEL);
                    let size = key.edge_length(radius);
                    let distance = camera.distance(key.center_dir() * radius) - size * 0.7;
                    if distance < reach {
                        assert_eq!(found.get(&key), Some(&distance), "{key:?} missing");
                    }
                }
            }
        }
    }
}
