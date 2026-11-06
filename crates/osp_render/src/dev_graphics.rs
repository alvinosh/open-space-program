//! The dev version's graphics (`cargo dev`, or `--dev` or `OSP_DEV=1`): the game drawn with
//! Bevy's standard shading alone. The ground is big flat facets, each one colour, the hulls
//! are matte and blocky, plumes are plain glowing cones, the sky is one colour that fades
//! with height and night, and there is no haze, cloud, sea, glow, skylight or starfield.
//!
//! It is there to keep the renderer apart from the rest of the game. The full graphics are a
//! stack of custom passes and shaders (`main::graphics` adds them all, as one group); the dev
//! version leaves every one of them out. So the simulation and the game around it can be
//! worked on, run and captured quickly without them, and a change to them cannot break it.
//! What both share (the floating origin, the camera, the terrain's quadtree, the vessels'
//! parts and how they move) takes the full graphics' resources as optional, and must keep
//! working without them: launching the dev version is the test of that.
//!
//! The graphics settings start from [`crate::settings::GraphicsSettings::dev`] and changes to
//! them are not saved, so the full version's settings stay as they were.

use bevy::{
    asset::RenderAssetUsages, light::light_consts::lux, math::DVec3, mesh::PrimitiveTopology,
    prelude::*,
};
use osp_sim::{body::Body, geometry::draw::Detail};
use osp_terrain::{Cover, Layer, TILE_CELLS, TileKey};

use crate::{
    FrameSet,
    camera::CameraRig,
    frame::{RenderFrame, Sunlight, to_render},
    plume::PlumeDef,
    sim::Sim,
    surface::Surface,
    vessel_material::VesselMaterial,
};

/// How finely parts are drawn (see `part_mesh::detail`): a tank twelve-sided, a nose a few
/// rings, a truss of square bars.
pub const DETAIL: Detail = Detail {
    curve_share: 0.04,
    curve_tolerance: 0.3,
    angle_step: 30.0 * std::f64::consts::PI / 180.0,
    twist_step: 10.0 * std::f64::consts::PI / 180.0,
    member_sides: 4,
};
/// Quads along a ground chunk's edge, each two flat triangles of one colour. The full
/// terrain has 32, so the facets' corners are among its vertices.
const GROUND_QUADS: usize = 8;
const GROUND_STRIDE: usize = TILE_CELLS / GROUND_QUADS;
/// The sea's colour where the ground is water, linear, and how far it is mixed in.
const SEA: Vec3 = Vec3::new(0.01, 0.045, 0.12);
const SEA_MIX: f32 = 0.6;
/// Launch-site paving, linear.
const CONCRETE: f32 = 0.35;
const ASPHALT: f32 = 0.05;
/// Light that reaches everything from everywhere, cd/m²: a quarter or so of the sun's at the
/// sunlight exposure, so what faces away from the sun, and the night side, still show.
const AMBIENT: f32 = 8000.0;
/// The clear sky's brightness at the ground at noon, linear.
const SKY: f32 = 0.6;
/// Sunlight fades out over this share of a body's radius at the edge of its shadow.
const PENUMBRA: f64 = 0.01;

/// Whether this is the dev version (see `osp_game::settings::dev_version`).
pub fn on() -> bool {
    crate::settings::dev_version()
}

pub struct DevGraphicsPlugin;

impl Plugin for DevGraphicsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: AMBIENT,
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Update, (sky, sunlight).in_set(FrameSet::Render))
        .add_systems(PostUpdate, matte_hulls);
    }
}

/// The material every ground chunk shares: its colour is in the facets.
#[derive(Resource)]
pub struct Ground(pub Handle<StandardMaterial>);

fn setup(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    commands.insert_resource(Ground(materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 1.0,
        reflectance: 0.1,
        // Skirts face sideways, either way.
        cull_mode: None,
        ..default()
    })));
}

/// The sky's colour where the camera is: the body's air scattering sunlight, as thick as the
/// air is there, in daylight; black in space and at night.
fn sky(sim: Res<Sim>, frame: Res<RenderFrame>, rig: Res<CameraRig>, mut clear: ResMut<ClearColor>) {
    let Some(id) = frame.origin_body else {
        return;
    };
    let system = &sim.universe.system;
    let body = system.body(id);
    let from_centre = rig.position - frame.body_offset(id);
    let air = body.atmosphere.as_ref().map_or(0.0, |air| {
        let ground = air.sample(0.0).pressure;
        let here = air.sample(from_centre.length() - body.radius).pressure;
        if ground > 0.0 { here / ground } else { 0.0 }
    });
    let up = from_centre.normalize_or(DVec3::Z);
    let to_sun = (frame.body_offset(system.root()) - rig.position).normalize_or(DVec3::Z);
    let day = smoothstep(-0.15, 0.1, up.dot(to_sun));
    let colour = sky_tint(body) * SKY * (air.powf(0.4) * day) as f32;
    let colour = Color::linear_rgb(colour.x, colour.y, colour.z);
    if clear.0 != colour {
        clear.0 = colour;
    }
}

/// The colour a body's air scatters, brightest channel 1: from its scattering if it has
/// any (molecules blue, dust by how much of it scatters rather than absorbs), otherwise its
/// own colour; black without air.
fn sky_tint(body: &Body) -> Vec3 {
    let scatter = match (&body.scattering, &body.atmosphere) {
        (Some(s), _) => {
            let mie = DVec3::from(s.mie);
            let kept = mie / (mie + DVec3::from(s.mie_absorption)).max(DVec3::splat(1e-12));
            (DVec3::from(s.rayleigh) + mie * kept).as_vec3()
        }
        (None, Some(_)) => Vec3::from(body.color),
        (None, None) => return Vec3::ZERO,
    };
    scatter / scatter.max_element().max(1e-12)
}

/// Full sunlight, except where the viewed vessel's body stands between it and the sun (the
/// map view is lit throughout, and so is anything round the star itself).
fn sunlight(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    mut light: Single<&mut DirectionalLight, With<Sunlight>>,
) {
    let Some(id) = frame.vessel_body else {
        return;
    };
    let system = &sim.universe.system;
    let at = frame.vessel_offset;
    let to_sun = (frame.body_offset(system.root()) - frame.relative(id, at)).normalize_or(DVec3::Z);
    // Nothing shades the editor's display spot over the star (see `Vessel::empty_display`).
    let lit = if rig.map_view || id == system.root() {
        1.0
    } else {
        sunlit(at, to_sun, system.body(id).radius)
    };
    let illuminance = lux::RAW_SUNLIGHT * lit as f32;
    if light.illuminance != illuminance {
        light.illuminance = illuminance;
    }
}

/// How much of the sun is seen from `at` (relative to a body's centre) past that body,
/// `radius` round: 0 in its shadow, 1 outside it, with a narrow ramp between.
fn sunlit(at: DVec3, to_sun: DVec3, radius: f64) -> f64 {
    let along = at.dot(to_sun);
    if along >= 0.0 {
        return 1.0;
    }
    let off_axis = (at - to_sun * along).length();
    ((off_axis - radius) / (PENUMBRA * radius)).clamp(0.0, 1.0)
}

/// Hulls as Bevy shades them, matte: no metal, gloss or clear coat, and no normal or
/// occlusion maps. The finish shader is not used at all (see `vessel_material`).
fn matte_hulls(
    mut events: MessageReader<AssetEvent<VesselMaterial>>,
    mut materials: ResMut<Assets<VesselMaterial>>,
) {
    for event in events.read() {
        if let AssetEvent::Added { id } = event
            && let Some(mut material) = materials.get_mut(*id)
        {
            let base = &mut material.base;
            base.metallic = 0.0;
            base.perceptual_roughness = base.perceptual_roughness.max(0.7);
            base.reflectance = 0.3;
            base.clearcoat = 0.0;
            base.metallic_roughness_texture = None;
            base.normal_map_texture = None;
            base.occlusion_texture = None;
        }
    }
}

/// A mesh drawn in facets: each triangle on its own vertices, facing the way it does.
pub fn faceted(mesh: Mesh) -> Mesh {
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return mesh;
    }
    mesh.with_duplicated_vertices().with_computed_flat_normals()
}

/// An engine plume's flame: a cone from the nozzle exit (y = 0) narrowing to y = −1, for
/// `update_engines` to scale to the flame's hot core (`PlumeCluster::flame_glow`).
pub fn plume_mesh() -> Mesh {
    Mesh::from(
        ConicalFrustum {
            radius_top: 0.8,
            radius_bottom: 0.3,
            height: 1.0,
        }
        .mesh()
        .resolution(8),
    )
    .translated_by(Vec3::NEG_Y * 0.5)
}

/// A plume's glow, in its flame's colour.
pub fn plume_material(look: &PlumeDef) -> StandardMaterial {
    let flame = Vec3::from(look.flame);
    let colour = flame / flame.max_element().max(1e-6);
    StandardMaterial {
        base_color: Color::linear_rgba(colour.x, colour.y, colour.z, 0.5),
        alpha_mode: AlphaMode::Add,
        unlit: true,
        cull_mode: None,
        ..default()
    }
}

/// A ground chunk as the dev version draws it (on a worker thread): [`GROUND_QUADS`] quads
/// along each edge, every triangle flat and the colour of the ground at its middle, with
/// skirts to hide the cracks between levels. Its vertices are relative to its centre
/// (render axes), and that centre is body-fixed, m.
pub fn ground_chunk(surface: &Surface, key: TileKey) -> (Mesh, DVec3) {
    let n = GROUND_QUADS;
    let stride = GROUND_STRIDE as f64;
    let data = surface.data.as_deref();
    let heights = data.and_then(|d| d.view(Layer::Height, key));
    let colours = data.and_then(|d| d.view(Layer::Color, key));
    let cover = surface.cover_view(key);
    // On the ground the full terrain's mesh has there, from the same heights.
    let point = |i: f64, j: f64| {
        let dir = key.sample_dir(i, j);
        let height = match &heights {
            Some(view) => surface
                .terrain
                .height_over(dir, view.height(i, j) as f64, key.level),
            None => surface.terrain.height_lod(dir, key.level),
        };
        dir * (surface.radius + height)
    };
    let colour = |i: f64, j: f64| {
        let dir = key.sample_dir(i, j);
        let srgb = colours
            .as_ref()
            .map_or(surface.fallback_color.map(f32::from), |v| v.rgb(i, j));
        let linear = Color::srgb(srgb[0] / 255.0, srgb[1] / 255.0, srgb[2] / 255.0).to_linear();
        let natural = (cover.as_ref())
            .map(|v| surface.cover_from_view(v, i, j, dir))
            .unwrap_or((surface.fallback_cover, 0.0));
        let (class, water) = surface.paved(dir, natural);
        let ground = match class {
            Cover::Concrete => Vec3::splat(CONCRETE),
            Cover::Asphalt => Vec3::splat(ASPHALT),
            _ => Vec3::new(linear.red, linear.green, linear.blue),
        };
        let c = ground.lerp(SEA, water.clamp(0.0, 1.0) * SEA_MIX);
        [c.x, c.y, c.z, 1.0]
    };
    let corners: Vec<DVec3> = (0..=n)
        .flat_map(|j| (0..=n).map(move |i| (i, j)))
        .map(|(i, j)| point(i as f64 * stride, j as f64 * stride))
        .collect();
    let corner = |i: usize, j: usize| corners[j * (n + 1) + i];
    let center = corner(n / 2, n / 2);

    let (mut positions, mut normals, mut colors) = (Vec::new(), Vec::new(), Vec::new());
    let mut triangle = |mut p: [DVec3; 3], colour: [f32; 4]| {
        let mut normal = (p[1] - p[0])
            .cross(p[2] - p[0])
            .normalize_or(p[0].normalize());
        // Counter-clockwise from above, whatever the cube face's axes.
        if normal.dot(p[0] + p[1] + p[2]) < 0.0 {
            p.swap(1, 2);
            normal = -normal;
        }
        for q in p {
            positions.push(to_render(q - center).to_array());
            normals.push(to_render(normal).to_array());
            colors.push(colour);
        }
    };
    for j in 0..n {
        for i in 0..n {
            let (a, b) = (corner(i, j), corner(i + 1, j));
            let (c, d) = (corner(i, j + 1), corner(i + 1, j + 1));
            let (fi, fj) = (i as f64, j as f64);
            let at = |di: f64, dj: f64| colour((fi + di) * stride, (fj + dj) * stride);
            triangle([a, b, d], at(2.0 / 3.0, 1.0 / 3.0));
            triangle([a, d, c], at(1.0 / 3.0, 2.0 / 3.0));
        }
    }
    // Skirts: down from each edge, lit as the ground is.
    let depth = (key.edge_length(surface.radius) / n as f64 * 2.0).max(1.0);
    let edge = |side: usize, k: usize| match side {
        0 => (k, 0),
        1 => (k, n),
        2 => (0, k),
        _ => (n, k),
    };
    for side in 0..4 {
        for k in 0..n {
            let ((i0, j0), (i1, j1)) = (edge(side, k), edge(side, k + 1));
            let (top0, top1) = (corner(i0, j0), corner(i1, j1));
            let (low0, low1) = (
                top0 - top0.normalize() * depth,
                top1 - top1.normalize() * depth,
            );
            let mid = colour(
                (i0 + i1) as f64 / 2.0 * stride,
                (j0 + j1) as f64 / 2.0 * stride,
            );
            let up = to_render((top0 + top1).normalize()).to_array();
            for q in [top0, low0, low1, top0, low1, top1] {
                positions.push(to_render(q - center).to_array());
                normals.push(up);
                colors.push(mid);
            }
        }
    }
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    (mesh, center)
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy::mesh::VertexAttributeValues;
    use osp_sim::terrain::Terrain;

    use super::*;

    fn smooth_sphere(radius: f64) -> Surface {
        Surface {
            radius,
            terrain: Arc::new(Terrain::new(radius, None, &[])),
            data: None,
            fallback_color: [128, 100, 80],
            fallback_cover: Cover::Barren,
            has_air: false,
        }
    }

    #[test]
    fn a_ground_chunk_is_flat_facets_facing_out_on_the_ground() {
        let radius = 600_000.0;
        let surface = smooth_sphere(radius);
        let key = TileKey::root(2).children()[1];
        let (mesh, center) = ground_chunk(&surface, key);
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no positions");
        };
        let Some(VertexAttributeValues::Float32x3(normals)) =
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            panic!("no normals");
        };
        let tops = GROUND_QUADS * GROUND_QUADS * 2 * 3;
        assert_eq!(positions.len(), tops + 4 * GROUND_QUADS * 6);
        let up = to_render(center.normalize());
        for (tri, faces) in positions[..tops].chunks(3).zip(normals[..tops].chunks(3)) {
            let p = tri.iter().map(|p| Vec3::from(*p)).collect::<Vec<_>>();
            let n = Vec3::from(faces[0]);
            // One normal per facet, out of the ground and square to the facet.
            assert!(faces.iter().all(|f| Vec3::from(*f) == n));
            assert!(n.dot(up) > 0.5, "{n} against {up}");
            assert!((p[1] - p[0]).cross(p[2] - p[0]).normalize().dot(n) > 0.999);
            // The corners are on the ground.
            for q in &p {
                // Render axes back to sim axes (`frame::to_render` the other way).
                let q = DVec3::new(q.x as f64, -q.z as f64, q.y as f64);
                let height = (center + q).length() - radius;
                assert!(height.abs() < 1.0, "{height}");
            }
        }
    }

    #[test]
    fn the_sun_is_hidden_only_behind_the_body() {
        let to_sun = DVec3::X;
        let r = 1000.0;
        assert_eq!(sunlit(DVec3::new(1100.0, 0.0, 0.0), to_sun, r), 1.0);
        assert_eq!(sunlit(DVec3::new(-1100.0, 0.0, 0.0), to_sun, r), 0.0);
        assert_eq!(sunlit(DVec3::new(-1100.0, 1100.0, 0.0), to_sun, r), 1.0);
        let edge = sunlit(
            DVec3::new(-1100.0, r * (1.0 + PENUMBRA / 2.0), 0.0),
            to_sun,
            r,
        );
        assert!((edge - 0.5).abs() < 1e-6, "{edge}");
    }
}
