//! The smoke an engine leaves in the air: a column that stays where the exhaust was let go and
//! turns with the planet, while the plume (see [`crate::plume`]) hangs off the vessel.
//!
//! How it looks follows from the exhaust. Each metre flown lets go of `mass flow / airspeed`
//! of it, which mixes with the air until it is about as dense as the air: so a rocket leaving
//! the pad stands in a wide, opaque cloud, and the same motor 50 km up leaves a much wider and
//! fainter column, and by the edge of the air nothing that shows. The particles in the exhaust
//! (a `trail` share of it: a solid's alumina) are what hides the sky; they stay in the column,
//! which goes on spreading for minutes, so it thins as it ages.
//!
//! It is drawn as a ribbon facing the camera, along which `shaders/exhaust_trail.wesl` shades a
//! soft round column with billows at its edge.

use std::collections::VecDeque;

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    camera::visibility::NoFrustumCulling,
    light::NotShadowCaster,
    math::DVec3,
    mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};
use osp_sim::{body::BodyId, time::SimTime};

use crate::{
    camera::{CameraRig, MainCamera},
    frame::{RenderFrame, to_render},
    sim::Sim,
};

/// Extinction by alumina a few microns across, m²/kg (3 Q / 4 ρ d, with Q = 2, ρ = 3950 kg/m³
/// and d = 3 µm).
const EXTINCTION: f64 = 126.0;
/// How many times its own mass of air the exhaust has taken in by the time it hangs in the air
/// as a column.
const DILUTION: f64 = 80.0;
/// The slowest airspeed that counts: slower than this the exhaust is blown away from under the
/// vessel by its own momentum, not left behind it.
const SLOWEST: f64 = 20.0;
/// Air too thin to stop the exhaust, kg/m³ (about 80 km on Earth): it just flies apart.
const THINNEST_AIR: f64 = 1.5e-5;
/// Widest a fresh column gets, m.
const WIDEST: f64 = 2_000.0;
/// How fast a column spreads in air at sea level, m²/s (turbulent diffusion in the open air),
/// and how fast thin air lets it spread as well.
const SPREAD: f64 = 30.0;
const MOST_SPREAD: f64 = 600.0;
/// Air at sea level, kg/m³, to compare with.
const SEA_LEVEL_AIR: f64 = 1.2;
/// A column fainter than this cannot be seen.
const FAINTEST: f32 = 0.004;
/// How long it is kept, sim seconds, and over what last share of that it fades.
const LIFE: f64 = 900.0;
const LIFE_FADE: f64 = 0.3;
/// A new point every this many of the column's radii, with the most points kept.
const SPACING: f64 = 1.0;
const MOST_POINTS: usize = 800;
/// The ribbon reaches this many radii out, for the billows to push the edge about in.
const MARGIN: f32 = 1.5;
/// Seconds for the smoke to fill the place of a plume that has gone out.
const CLEARS: f64 = 1.0;

pub struct ExhaustTrailPlugin;

impl Plugin for ExhaustTrailPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/exhaust_trail.wesl");
        app.add_plugins(MaterialPlugin::<TrailMaterial>::default())
            .init_resource::<Emissions>();
    }
}

/// What the plumes let go of this frame.
#[derive(Resource, Default)]
pub struct Emissions(pub Vec<Emission>);

#[derive(Debug, Clone)]
pub struct Emission {
    /// The plume it comes from: a plume makes one column for as long as it burns.
    pub plume: Entity,
    pub body: BodyId,
    /// Where, relative to the body (sim axes), m.
    pub at: DVec3,
    /// Exhaust let go, kg/s, and the share of it that is smoke.
    pub flow: f64,
    pub smoke: f64,
    /// The air's speed past the vessel, m/s, and its density, kg/m³.
    pub airspeed: f64,
    pub air: f64,
    /// Colour of the smoke in daylight.
    pub colour: [f32; 3],
    /// How far behind the nozzles the plume is still flame: the smoke starts beyond it, m.
    pub clear: f32,
}

/// A stretch of a column, as it was let go.
#[derive(Debug, Clone, Copy)]
struct Point {
    /// Where, in the body's own axes, so that it turns with the planet, m.
    fixed: DVec3,
    born: SimTime,
    /// Radius when it was let go, m, and how fast it spreads, m²/s.
    radius: f32,
    spread: f32,
    /// Smoke per metre, kg/m.
    smoke: f32,
    /// How far along the column, in radii: the billows are this size wherever they are.
    along: f32,
}

impl Point {
    fn radius_at(&self, age: f64) -> f32 {
        (self.radius * self.radius + 2.0 * self.spread * age.max(0.0) as f32).sqrt()
    }
}

/// Radius of a column made of `exhaust` kg/m, once it has mixed with air of density `air`.
fn fresh_radius(exhaust: f64, air: f64) -> f64 {
    (DILUTION * exhaust / (std::f64::consts::PI * air))
        .sqrt()
        .min(WIDEST)
}

/// Optical depth through the middle of a round column of `radius`, holding `smoke` kg/m.
fn optical_depth(smoke: f32, radius: f32) -> f32 {
    2.0 * EXTINCTION as f32 * smoke / (std::f32::consts::PI * radius.max(1e-3))
}

/// A column of smoke, which outlives the plume and the vessel that made it.
#[derive(Component)]
pub struct Trail {
    /// The plume still adding to it.
    plume: Option<Entity>,
    body: BodyId,
    points: VecDeque<Point>,
    /// Where the nozzles are now, while they add to it.
    head: Option<DVec3>,
    clear: f32,
}

#[derive(Debug, Clone, Copy, Default, ShaderType)]
pub struct TrailParams {
    /// rgb: the smoke's albedo, w: seed of its billows.
    pub smoke: Vec4,
    /// xyz: the local vertical (world axes).
    pub up: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct TrailMaterial {
    #[uniform(0)]
    pub params: TrailParams,
}

impl Material for TrailMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://osp/shaders/exhaust_trail.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    /// Behind every plume, whichever mesh origin is the nearer: the plume is the brighter.
    fn depth_bias(&self) -> f32 {
        -100_000.0
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Add what the plumes let go to their columns, let the columns age and shape their ribbons to
/// the camera.
#[allow(clippy::too_many_arguments)]
pub fn update_trails(
    mut commands: Commands,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    camera: Single<&Transform, With<MainCamera>>,
    mut emissions: ResMut<Emissions>,
    mut trails: Query<(
        Entity,
        &mut Trail,
        &Mesh3d,
        &MeshMaterial3d<TrailMaterial>,
        &mut Visibility,
    )>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TrailMaterial>>,
    mut last: Local<Option<(SimTime, u32)>>,
) {
    let system = &sim.universe.system;
    let (before, mut seeds) = last.unwrap_or((frame.time, 0));
    // Time went backwards: a flight started afresh, whose smoke has not been made yet.
    let restarted = frame.time.seconds_since(before) < -1.0;
    let dt = frame.time.seconds_since(before).max(0.0);
    let emissions = std::mem::take(&mut emissions.0);
    let mut used = vec![false; emissions.len()];
    for (entity, mut trail, mesh, material, mut visibility) in &mut trails {
        if restarted {
            commands.entity(entity).despawn();
            continue;
        }
        let trail = &mut *trail;
        // What its plume lets go, if it still burns on the same body.
        let found = trail.plume.and_then(|plume| {
            emissions
                .iter()
                .position(|e| e.plume == plume && e.body == trail.body && e.air > THINNEST_AIR)
        });
        let body = system.body(trail.body);
        let orientation = body.orientation(frame.time);
        if let Some(found) = found {
            used[found] = true;
            add(trail, &emissions[found], frame.time, orientation.inverse());
        } else {
            trail.plume = None;
            trail.head = None;
            trail.clear *= (-dt / CLEARS).exp() as f32;
        }
        while trail.points.front().is_some_and(|p| {
            let age = frame.time.seconds_since(p.born);
            age >= LIFE || optical_depth(p.smoke, p.radius_at(age)) < FAINTEST
        }) {
            trail.points.pop_front();
        }
        if trail.points.is_empty() && trail.plume.is_none() {
            commands.entity(entity).despawn();
            continue;
        }
        *visibility = if rig.map_view {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if rig.map_view {
            continue;
        }
        if let Some(mut material) = materials.get_mut(&material.0) {
            let up = to_render(frame.vessel_offset).normalize_or(Vec3::Y);
            material.params.up = up.extend(0.0);
        }
        if let Some(mut mesh) = meshes.get_mut(&mesh.0) {
            let places: Vec<Vec3> = trail
                .points
                .iter()
                .map(|p| frame.to_render(trail.body, orientation * p.fixed))
                .collect();
            let head = trail.head.map(|at| frame.to_render(trail.body, at));
            shape(
                &mut mesh,
                trail,
                &places,
                head,
                camera.translation,
                frame.time,
            );
        }
    }
    if !restarted {
        for (emission, _) in emissions
            .iter()
            .zip(&used)
            .filter(|(e, used)| !**used && e.air > THINNEST_AIR)
        {
            seeds += 1;
            let mut trail = Trail {
                plume: Some(emission.plume),
                body: emission.body,
                points: VecDeque::new(),
                head: None,
                clear: emission.clear,
            };
            let orientation = system.body(emission.body).orientation(frame.time);
            add(&mut trail, emission, frame.time, orientation.inverse());
            let [r, g, b] = emission.colour;
            commands.spawn((
                trail,
                Mesh3d(meshes.add(Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::default(),
                ))),
                MeshMaterial3d(materials.add(TrailMaterial {
                    params: TrailParams {
                        smoke: Vec4::new(r, g, b, 3.17 * seeds as f32),
                        up: Vec4::Y,
                    },
                })),
                Transform::default(),
                Visibility::Hidden,
                NoFrustumCulling,
                NotShadowCaster,
            ));
        }
    }
    *last = Some((frame.time, seeds));
}

/// Let go of what `emission` says, if the vessel has come far enough to need a new point.
fn add(trail: &mut Trail, emission: &Emission, now: SimTime, into_fixed: bevy::math::DQuat) {
    let exhaust = emission.flow / emission.airspeed.max(SLOWEST);
    let radius = fresh_radius(exhaust, emission.air);
    trail.head = Some(emission.at);
    trail.clear = emission.clear;
    let fixed = into_fixed * emission.at;
    let along = match trail.points.back() {
        Some(last) => {
            let moved = last.fixed.distance(fixed);
            if moved < SPACING * radius {
                return;
            }
            last.along + (moved / radius) as f32
        }
        None => 0.0,
    };
    let thin = (SEA_LEVEL_AIR / emission.air).sqrt();
    trail.points.push_back(Point {
        fixed,
        born: now,
        radius: radius as f32,
        spread: (SPREAD * thin).min(MOST_SPREAD) as f32,
        smoke: (exhaust * emission.smoke) as f32,
        along,
    });
    if trail.points.len() > MOST_POINTS {
        trail.points.pop_front();
    }
}

/// Shape the ribbon. Every point has two vertices across the column as the camera sees it:
/// `uv` is how far across (-1 to 1) and how far along it is, in radii; the `normal` points
/// across; `colour` is its optical depth, age and radius.
fn shape(
    mesh: &mut Mesh,
    trail: &Trail,
    places: &[Vec3],
    head: Option<Vec3>,
    camera: Vec3,
    now: SimTime,
) {
    // From the nozzles back: (place, optical depth, age, radius, along).
    let mut stations = Vec::with_capacity(places.len() + 1);
    let newest = trail.points.back();
    if let (Some(head), Some(point)) = (head, newest) {
        stations.push((
            head,
            optical_depth(point.smoke, point.radius),
            0.0,
            point.radius,
            point.along,
        ));
    }
    for (point, &place) in trail.points.iter().zip(places).rev() {
        let age = now.seconds_since(point.born);
        let radius = point.radius_at(age);
        let fade = ((LIFE - age) / (LIFE_FADE * LIFE)).clamp(0.0, 1.0) as f32;
        let depth = optical_depth(point.smoke, radius) * fade;
        stations.push((place, depth, age as f32, radius, point.along));
    }
    let n = stations.len();
    let mut positions = Vec::with_capacity(2 * n);
    let mut normals = Vec::with_capacity(2 * n);
    let mut uvs = Vec::with_capacity(2 * n);
    let mut colours = Vec::with_capacity(2 * n);
    let mut indices = Vec::with_capacity(6 * n);
    let mut behind = 0.0f32;
    let mut across = Vec3::ZERO;
    for i in 0..n {
        let (place, depth, age, radius, along) = stations[i];
        if i > 0 {
            behind += place.distance(stations[i - 1].0);
        }
        let tangent =
            (stations[(i + 1).min(n - 1)].0 - stations[i.saturating_sub(1)].0).normalize_or_zero();
        let side = tangent.cross(camera - place);
        // Looking along the column the side is ill-defined: keep the last one.
        if side.length_squared() > 1e-6 * (camera - place).length_squared() {
            let side = side.normalize();
            across = if side.dot(across) < 0.0 { -side } else { side };
        }
        // The plume is still flame here, and the far end tapers off.
        let clear = trail.clear.max(1e-3);
        let shown = if head.is_some() || trail.clear > 0.01 {
            smooth(0.35 * clear, clear, behind)
        } else {
            1.0
        };
        let depth = if i + 1 == n { 0.0 } else { depth * shown };
        for u in [-1.0f32, 1.0] {
            positions.push(place + across * (u * MARGIN * radius));
            normals.push(across);
            uvs.push([u, along]);
            colours.push([depth, age, radius, 1.0]);
        }
        if i + 1 < n {
            let k = 2 * i as u32;
            indices.extend([k, k + 1, k + 2, k + 1, k + 3, k + 2]);
        }
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    mesh.insert_indices(Indices::U32(indices));
}

fn smooth(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pair of big solids, 5 t/s of exhaust of which a third is alumina.
    fn column(airspeed: f64, air: f64) -> (f64, f32) {
        let exhaust = 5_000.0 / airspeed;
        let radius = fresh_radius(exhaust, air);
        (radius, optical_depth((exhaust * 0.3) as f32, radius as f32))
    }

    #[test]
    fn low_down_the_column_hides_the_sky_and_high_up_it_is_a_veil() {
        let (near_ground, thick) = column(300.0, 1.2);
        assert!((5.0..30.0).contains(&near_ground), "{near_ground} m");
        assert!(thick > 5.0, "{thick}");
        let (high_up, thin) = column(1_500.0, 3e-4);
        assert!(high_up > 5.0 * near_ground, "{high_up} m");
        assert!((0.02..1.0).contains(&thin), "{thin}");
    }

    #[test]
    fn it_thins_as_it_spreads() {
        let point = Point {
            fixed: DVec3::ZERO,
            born: SimTime::J2000,
            radius: 15.0,
            spread: SPREAD as f32,
            smoke: 5.0,
            along: 0.0,
        };
        let fresh = optical_depth(point.smoke, point.radius_at(0.0));
        let old = optical_depth(point.smoke, point.radius_at(600.0));
        assert!(point.radius_at(600.0) > 150.0, "{}", point.radius_at(600.0));
        assert!(old < 0.15 * fresh && old > FAINTEST, "{fresh} then {old}");
    }

    #[test]
    fn the_ribbon_faces_the_camera_and_starts_clear_of_the_flame() {
        let point = |x: f64, along: f32| Point {
            fixed: DVec3::new(x, 0.0, 0.0),
            born: SimTime::J2000,
            radius: 10.0,
            spread: 0.0,
            smoke: 5.0,
            along,
        };
        let trail = Trail {
            plume: None,
            body: BodyId(0),
            points: VecDeque::from([point(0.0, 0.0), point(100.0, 10.0), point(200.0, 20.0)]),
            head: None,
            clear: 150.0,
        };
        let places = [Vec3::ZERO, Vec3::X * 100.0, Vec3::X * 200.0];
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        let head = Some(Vec3::X * 300.0);
        shape(
            &mut mesh,
            &trail,
            &places,
            head,
            Vec3::new(100.0, 0.0, 500.0),
            SimTime::J2000,
        );
        let colours = mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap();
        let bevy::mesh::VertexAttributeValues::Float32x4(colours) = colours else {
            panic!("colours as they were written");
        };
        // Head, 200 m, 100 m and 0 m along the x axis: flame, partly clear, clear, the taper.
        let depths: Vec<f32> = colours.chunks(2).map(|c| c[0][0]).collect();
        assert_eq!(depths[0], 0.0);
        assert!(depths[1] > 0.0 && depths[1] < depths[2], "{depths:?}");
        assert_eq!(depths[3], 0.0);
        // Across the ribbon is across the line of sight: along y, not towards the camera.
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap();
        let bevy::mesh::VertexAttributeValues::Float32x3(positions) = positions else {
            panic!("positions as they were written");
        };
        let width = Vec3::from(positions[5]) - Vec3::from(positions[4]);
        assert!(width.y.abs() > 0.99 * width.length(), "{width}");
        assert!((width.length() - 2.0 * MARGIN * 10.0).abs() < 1e-3);
    }
}
