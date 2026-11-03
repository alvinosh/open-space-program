//! Re-entry: the plasma around a vessel coming in hot, after Lindner's atmospheric entry shader
//! (`shaders/plasma.wesl` has how it is drawn).
//!
//! How strong it is and how it looks come from the air's density and the vessel's speed
//! (`osp_sim::plasma`). Its shape comes from the vessel's own meshes: they are gathered when it is
//! first needed and whenever the vessel changes, into one mesh of the hull's triangles and of strips
//! on points scattered over them. The way the air goes and a depth map of the hull looking along it
//! reach the shader from here as the flow turns. The light it throws on the vessel and anything else
//! near is a point light at the shock.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    camera::visibility::NoFrustumCulling,
    light::{NotShadowCaster, NotShadowReceiver},
    math::Affine3A,
    mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology, VertexAttributeValues},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    platform::collections::HashMap,
    prelude::*,
    render::render_resource::{
        AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
        TextureDimension, TextureFormat, TextureUsages,
    },
    shader::ShaderRef,
};
use osp_sim::{
    plasma::{
        AIRSTREAM_SIZE, Airstream, EntryLook, HullPoints, HullTriangle, NOTHING_THERE, airstream,
        entry_look, flow_in_vessel, nose_radius, scatter_points,
    },
    vessel::VesselId,
};

use crate::{
    FrameSet,
    frame::{RenderFrame, rotation_to_render},
    part_info::ShowsPart,
    part_mesh::axes,
    settings::{GraphicsSettings, Quality},
    sim::Sim,
    vessel_material::VesselMaterial,
    vessel_render::VesselVisual,
};

/// Vessels with effects at once.
const MOST_AT_ONCE: usize = 4;
/// Below this strength, nothing is drawn.
const LEAST_STRENGTH: f32 = 0.01;
/// How fast the strength may change, per second, so it swells rather than pops on a stage drop.
const CHANGE_RATE: f32 = 0.6;
/// How fast the rest of the look follows, per second.
const LOOK_RATE: f32 = 1.5;
/// The flow has to turn this far before the map is rebuilt, radians.
const REMAP_ANGLE: f32 = 0.015;
/// The most points on a hull per square metre, for a small one.
const POINTS_PER_SQUARE_METRE: f32 = 220.0;
/// Of the hull's points, every this many also gets a spark.
const SPARK_EVERY: usize = 2;
/// Strips in the long stream behind it all.
const WAKE_STRIPS: usize = 72;
/// A streak is this many times the distance between points wide, at the hull.
const STRIP_WIDTH: f32 = 1.6;
/// How wide a spark is, m.
const SPARK_WIDTH: f32 = 0.04;
/// Brightness of the gas at full strength, cd/m².
const GAS_BRIGHTNESS: f32 = 42_000.0;
/// Light the gas throws at full strength, lumens per square metre of hull.
const LIGHT_PER_AREA: f32 = 1.2e7;
/// How fast bits of hull are shown to leave, m/s, whatever the true speed: at the true 7 km/s they
/// would only be a blur.
const SHOWN_FLOW: f32 = 160.0;
/// What air burns like when the data doesn't say.
const AIR_PLASMA: [f32; 3] = [1.0, 0.42, 0.2];
/// Colours that don't depend on the air: hot blue gas, the violet of the thinnest air.
const BLUE: [f32; 3] = [0.36, 0.46, 1.0];
const VIOLET: [f32; 3] = [0.5, 0.32, 1.0];

/// Places along a strip: closer together near the hull, where more goes on.
const LONG_STRIP: [f32; 6] = [0.0, 0.05, 0.14, 0.3, 0.58, 1.0];
const SHORT_STRIP: [f32; 4] = [0.0, 0.2, 0.55, 1.0];
const STREAM_STRIP: [f32; 9] = [0.0, 0.06, 0.14, 0.24, 0.36, 0.5, 0.65, 0.82, 1.0];
const SPARK_STRIP: [f32; 2] = [0.0, 1.0];

/// The layers, in the order they are drawn (`GLOW` and so on in the shader).
#[derive(Clone, Copy)]
enum Layer {
    Glow = 0,
    Wake = 1,
    Wrap = 2,
    Trail = 3,
    Cone = 4,
    Spark = 5,
    Bow = 6,
}

pub struct PlasmaPlugin;

impl Plugin for PlasmaPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/plasma.wesl");
        app.add_plugins(MaterialPlugin::<PlasmaMaterial>::default())
            .init_resource::<PlasmaEffects>()
            .add_systems(Update, update_effects.in_set(FrameSet::Render));
    }
}

#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct PlasmaParams {
    /// x: how many of the hull's points streaks can come off.
    edge: Vec4,
    flow: Vec4,
    across_x: Vec4,
    across_y: Vec4,
    centre: Vec4,
    look: Vec4,
    extent: Vec4,
    hot: Vec4,
    mid: Vec4,
    far: Vec4,
    wrap: Vec4,
    shock: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
struct PlasmaMaterial {
    #[uniform(0)]
    params: PlasmaParams,
    #[texture(1, sample_type = "float", filterable = false)]
    airstream: Handle<Image>,
}

impl Material for PlasmaMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://osp/shaders/plasma.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://osp/shaders/plasma.wesl".into()
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

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(3),
        ])?];
        // Strips are seen from both sides, and models don't all wind their triangles the same way.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The vessel's hull, in the axes of its root (the base of the stack, render axes).
#[derive(Default)]
struct Gathered {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    indices: Vec<u32>,
    /// How many meshes it came from, and how many of those weren't loaded yet.
    meshes: usize,
    waiting: usize,
}

impl Gathered {
    fn add(&mut self, mesh: &Mesh, transform: Affine3A) {
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            return;
        };
        let base = self.positions.len() as u32;
        self.positions.extend(
            positions
                .iter()
                .map(|p| transform.transform_point3(Vec3::from(*p))),
        );
        match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
            Some(VertexAttributeValues::Float32x3(normals)) => {
                self.normals.extend(normals.iter().map(|n| {
                    transform
                        .transform_vector3(Vec3::from(*n))
                        .normalize_or_zero()
                }));
            }
            _ => self
                .normals
                .extend(std::iter::repeat_n(Vec3::ZERO, positions.len())),
        }
        match mesh.indices() {
            Some(indices) => self.indices.extend(indices.iter().map(|i| base + i as u32)),
            None => self.indices.extend(base..base + positions.len() as u32),
        }
    }

    fn triangles(&self) -> Vec<HullTriangle> {
        self.indices
            .chunks_exact(3)
            .map(|t| {
                let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
                let positions = [self.positions[a], self.positions[b], self.positions[c]];
                // A mesh without normals gets its triangles' own.
                let flat = (positions[1] - positions[0])
                    .cross(positions[2] - positions[0])
                    .normalize_or_zero();
                let normal = |n: Vec3| if n == Vec3::ZERO { flat } else { n };
                HullTriangle {
                    positions,
                    normals: [
                        normal(self.normals[a]),
                        normal(self.normals[b]),
                        normal(self.normals[c]),
                    ],
                }
            })
            .collect()
    }
}

/// The attributes of the effect's mesh.
#[derive(Default)]
struct Pieces {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    places: Vec<[f32; 2]>,
    strips: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

/// A number in 0..1 that goes with `n`.
fn random(n: usize, salt: u32) -> f32 {
    let mut h = (n as u32).wrapping_mul(0x9E37_79B1) ^ salt.wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 13;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

impl Pieces {
    /// The hull's triangles, as one layer.
    fn hull(&mut self, hull: &Gathered, layer: Layer) {
        let base = self.positions.len() as u32;
        for (index, p) in hull.positions.iter().enumerate() {
            self.positions.push(p.to_array());
            // A mesh without normals of its own gets no glow to speak of, rather than none at all.
            self.normals
                .push(hull.normals[index].normalize_or(Vec3::Y).to_array());
            self.places.push([0.0, 0.0]);
            self.strips.push([layer as u32 as f32, 0.0, 0.0, 0.0]);
        }
        self.indices.extend(hull.indices.iter().map(|i| base + i));
    }

    /// A strip on a point of the hull, the vertex shader to shape it. Its two random numbers are
    /// `n`'s, `width` its width at the hull.
    fn strip(
        &mut self,
        (position, normal): (Vec3, Vec3),
        layer: Layer,
        rows: &[f32],
        n: usize,
        width: f32,
    ) {
        let base = self.positions.len() as u32;
        let (r1, r2) = (random(n, 1), random(n, 2));
        for &t in rows {
            for side in [-1.0, 1.0] {
                self.positions.push(position.to_array());
                self.normals.push(normal.to_array());
                self.places.push([t, side]);
                self.strips.push([layer as u32 as f32, r1, r2, width]);
            }
        }
        for row in 0..rows.len() as u32 - 1 {
            let (a, b, c, d) = (
                base + 2 * row,
                base + 2 * row + 1,
                base + 2 * row + 2,
                base + 2 * row + 3,
            );
            self.indices.extend([a, b, c, b, d, c]);
        }
    }

    fn into_mesh(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.places)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.strips)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}

/// The effect's whole mesh, for a hull and its points, in the order it draws.
fn effect_mesh(hull: &Gathered, points: &HullPoints) -> Mesh {
    let mut pieces = Pieces::default();
    let width = points.spacing * STRIP_WIDTH;
    let at = |k: usize| {
        let point = &points.points[k];
        (point.position, point.normal)
    };
    let count = points.points.len();
    pieces.hull(hull, Layer::Glow);
    for k in 0..WAKE_STRIPS {
        pieces.strip((Vec3::ZERO, Vec3::Z), Layer::Wake, &STREAM_STRIP, k, 0.0);
    }
    for k in 0..count {
        pieces.strip(at(k), Layer::Wrap, &SHORT_STRIP, k, width * 2.0);
    }
    for k in 0..count {
        pieces.strip(at(k), Layer::Trail, &LONG_STRIP, k, width);
    }
    for k in 0..count {
        pieces.strip(at(k), Layer::Cone, &SHORT_STRIP, k, width * 2.5);
    }
    for k in (0..count).step_by(SPARK_EVERY) {
        pieces.strip(at(k), Layer::Spark, &SPARK_STRIP, k, SPARK_WIDTH);
    }
    pieces.hull(hull, Layer::Bow);
    pieces.into_mesh()
}

/// One vessel's effect.
struct Effect {
    /// Nothing is spawned until the hull has been gathered.
    drawn: Option<Drawn>,
    light: Entity,
    strength: f32,
    look: EntryLook,
    /// What the hull was gathered from, to tell when it changes.
    gathered: (usize, usize),
    points: HullPoints,
    /// The map, for the flow it was built for.
    map: Option<Airstream>,
}

struct Drawn {
    entity: Entity,
    material: Handle<PlasmaMaterial>,
    image: Handle<Image>,
    mesh: Handle<Mesh>,
}

#[derive(Resource, Default)]
struct PlasmaEffects(HashMap<VesselId, Effect>);

/// The map as a texture, its depth and how far inside the outline: read texel by texel in both
/// stages, so it needs no filtering.
fn airstream_image(map: Option<&Airstream>) -> Image {
    let n = AIRSTREAM_SIZE;
    let mut data = Vec::with_capacity(n * n * 8);
    for k in 0..n * n {
        let (depth, inside) = map.map_or((NOTHING_THERE, 0.0), |m| (m.depth[k], m.inside[k]));
        data.extend_from_slice(&depth.to_le_bytes());
        data.extend_from_slice(&inside.to_le_bytes());
    }
    let mut image = Image::new(
        Extent3d {
            width: n as u32,
            height: n as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rg32Float,
        // Rebuilt as the flow turns, so the main world keeps its copy to write into.
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_DST;
    image
}

fn mix3(a: [f32; 3], b: [f32; 3], k: f32) -> Vec3 {
    Vec3::from(a).lerp(Vec3::from(b), k)
}

/// The gas's colours for its look: `tint` is what the body's air burns like.
fn colours(tint: [f32; 3], look: &EntryLook, strength: f32) -> [Vec3; 5] {
    let (thin, white) = (look.thin as f32, look.white as f32);
    // At the hull, between yellow and blue with how strong it is and how thin the air.
    let yellow = mix3(tint, [1.0, 0.86, 0.62], 0.7).to_array();
    let hot = mix3(
        yellow,
        BLUE,
        (thin * thin * 0.5 + (strength - 0.8).max(0.0)).min(0.8),
    );
    let mid = Vec3::from(tint);
    // A pale red, going violet only in the thinnest air.
    let pale = Vec3::from(tint) * Vec3::new(0.85, 0.55, 0.5);
    let far = mix3(pale.to_array(), VIOLET, thin * thin * thin * 0.75);
    let wrap = mix3(BLUE, VIOLET, thin * 0.5);
    let shock = Vec3::from(BLUE);
    // A Mach cone's white over it all.
    [hot, mid, far, wrap, shock].map(|c| c.lerp(Vec3::ONE, white))
}

#[allow(clippy::too_many_arguments)]
fn update_effects(
    mut commands: Commands,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    settings: Res<GraphicsSettings>,
    time: Res<Time>,
    mut effects: ResMut<PlasmaEffects>,
    mut materials: ResMut<Assets<PlasmaMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut transforms: Query<&mut Transform, Without<PointLight>>,
    mut lights: Query<(&mut PointLight, &mut Transform)>,
    visuals: Query<(Entity, &VesselVisual, &GlobalTransform)>,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    parts: Query<(), With<ShowsPart>>,
    hull_meshes: Query<
        (&Mesh3d, &GlobalTransform),
        Or<(
            With<MeshMaterial3d<VesselMaterial>>,
            With<MeshMaterial3d<StandardMaterial>>,
        )>,
    >,
) {
    if frame.origin_body.is_none() {
        return;
    }
    let system = &sim.universe.system;
    let most_points = match settings.effects {
        Quality::Off | Quality::Low => 3_000,
        Quality::Medium => 6_000,
        Quality::High => 9_000,
        Quality::Ultra => 12_000,
    };
    let mut seen = Vec::new();
    for vessel in &sim.universe.vessels {
        let body = system.body(vessel.parent());
        let Some(air) = body.atmosphere.as_ref() else {
            continue;
        };
        let state = vessel.state_at(system, frame.time);
        let altitude = state.position.length() - body.radius;
        if altitude > air.height {
            continue;
        }
        let airflow = state.velocity - body.surface_velocity(state.position);
        let radius = nose_radius(vessel.propulsion.shape().reference_area());
        let climbing = airflow.dot(state.position.normalize_or_zero());
        let target = entry_look(
            air.sample(altitude).density,
            airflow.length(),
            radius,
            climbing,
        );
        if target.strength <= 0.0 && !effects.0.contains_key(&vessel.id()) {
            continue;
        }
        if seen.len() >= MOST_AT_ONCE {
            continue;
        }
        seen.push(vessel.id());
        let effect = effects.0.entry(vessel.id()).or_insert_with(|| Effect {
            drawn: None,
            light: commands
                .spawn((
                    PointLight {
                        intensity: 0.0,
                        range: 500.0,
                        radius: 2.0,
                        ..default()
                    },
                    Transform::default(),
                ))
                .id(),
            strength: 0.0,
            look: target,
            gathered: (0, 0),
            points: HullPoints::default(),
            map: None,
        });

        // Swells and fades instead of jumping when a stage goes, or the view changes vessel.
        let most = CHANGE_RATE * time.delta_secs();
        effect.strength += (target.strength as f32 - effect.strength).clamp(-most, most);
        let follow = (LOOK_RATE * time.delta_secs()).min(1.0) as f64;
        effect.look.length += (target.length - effect.look.length) * follow;
        effect.look.spread += (target.spread - effect.look.spread) * follow;
        effect.look.thin += (target.thin - effect.look.thin) * follow;
        effect.look.white += (target.white - effect.look.white) * follow;

        // The vessel's own meshes, loaded or not yet, whenever they change.
        let attitude = vessel.attitude_at(system, frame.time);
        if effect.strength > 0.0
            && let Some((root, _, root_transform)) = visuals
                .iter()
                .find(|(_, visual, _)| visual.id == vessel.id())
        {
            let parts_of = |entity: Entity| {
                parts.contains(entity) || parents.iter_ancestors(entity).any(|a| parts.contains(a))
            };
            let mut hull = Gathered::default();
            let to_root = root_transform.affine().inverse();
            let found: Vec<_> = children
                .iter_descendants(root)
                .filter_map(|e| hull_meshes.get(e).ok().filter(|_| parts_of(e)))
                .collect();
            hull.waiting = found
                .iter()
                .filter(|(mesh, _)| meshes.get(&mesh.0).is_none())
                .count();
            hull.meshes = found.len();
            if (hull.meshes, hull.waiting) != effect.gathered {
                for (mesh, transform) in &found {
                    if let Some(mesh) = meshes.get(&mesh.0) {
                        hull.add(mesh, to_root * transform.affine());
                    }
                }
                effect.gathered = (hull.meshes, hull.waiting);
                effect.points =
                    scatter_points(&hull.triangles(), most_points, POINTS_PER_SQUARE_METRE);
                effect.map = None;
                if !effect.points.points.is_empty() {
                    let mesh = effect_mesh(&hull, &effect.points);
                    match &mut effect.drawn {
                        Some(drawn) => {
                            if let Some(mut old) = meshes.get_mut(&drawn.mesh) {
                                *old = mesh;
                            }
                        }
                        None => {
                            let image = images.add(airstream_image(None));
                            let material = materials.add(PlasmaMaterial {
                                params: PlasmaParams::default(),
                                airstream: image.clone(),
                            });
                            let mesh = meshes.add(mesh);
                            let entity = commands
                                .spawn((
                                    Mesh3d(mesh.clone()),
                                    MeshMaterial3d(material.clone()),
                                    Transform::default(),
                                    NotShadowCaster,
                                    NotShadowReceiver,
                                    // Its strips go where the flow takes them.
                                    NoFrustumCulling,
                                    Name::new("Re-entry"),
                                ))
                                .id();
                            effect.drawn = Some(Drawn {
                                entity,
                                material,
                                image,
                                mesh,
                            });
                        }
                    }
                }
            }
        }

        // The way the air goes, in the mesh's axes.
        let flow = flow_in_vessel(attitude, airflow);
        let flow = axes([flow.x as f32, flow.y as f32, flow.z as f32]);
        let remap = effect
            .map
            .as_ref()
            .is_none_or(|map| map.flow.angle_between(flow) > REMAP_ANGLE);
        if remap && effect.strength > 0.0 && !effect.points.points.is_empty() {
            let map = airstream(&effect.points, flow);
            if let Some(drawn) = &effect.drawn
                && let Some(mut image) = images.get_mut(&drawn.image)
            {
                *image = airstream_image(Some(&map));
            }
            effect.map = Some(map);
        }

        let (Some(drawn), Some(map)) = (&effect.drawn, &effect.map) else {
            continue;
        };
        let show = effect.strength > LEAST_STRENGTH;
        // The same place as the vessel's own visual.
        let root = Transform {
            translation: frame.to_render(vessel.parent(), vessel.base_at(system, frame.time)),
            rotation: rotation_to_render(attitude),
            scale: if show { Vec3::ONE } else { Vec3::ZERO },
        };
        if let Ok(mut transform) = transforms.get_mut(drawn.entity) {
            *transform = root;
        }
        let tint = body
            .scattering
            .as_ref()
            .and_then(|s| s.plasma)
            .map_or(AIR_PLASMA, |c| c.map(|v| v as f32));
        if show && let Some(mut material) = materials.get_mut(&drawn.material) {
            let [hot, mid, far, wrap, shock] = colours(tint, &effect.look, effect.strength);
            material.params = PlasmaParams {
                edge: Vec4::new(map.on_edge as f32, 0.0, 0.0, 0.0),
                flow: map.flow.extend(map.half_width),
                across_x: map.across[0].extend(map.reach),
                across_y: map.across[1].extend(map.back),
                centre: map.centre.extend(map.front),
                look: Vec4::new(
                    effect.strength,
                    effect.look.length as f32,
                    effect.look.thin as f32,
                    effect.look.white as f32,
                ),
                extent: Vec4::new(
                    effect.look.spread as f32,
                    effect.points.spacing,
                    GAS_BRIGHTNESS,
                    SHOWN_FLOW,
                ),
                hot: hot.extend(1.0),
                mid: mid.extend(1.0),
                far: far.extend(1.0),
                wrap: wrap.extend(1.0),
                shock: shock.extend(1.0),
            };
        }
        if let Ok((mut light, mut transform)) = lights.get_mut(effect.light) {
            light.color = Color::linear_rgb(tint[0], tint[1], tint[2]);
            let area = map.reach * map.reach;
            light.intensity = LIGHT_PER_AREA * area * effect.strength * effect.strength;
            if !show {
                light.intensity = 0.0;
            }
            let ahead = map.centre + map.flow * (map.front - map.reach * 0.6);
            transform.translation = root.translation + root.rotation * ahead;
        }
    }
    // Gone, or cooled right down.
    effects.0.retain(|id, effect| {
        let keep = seen.contains(id);
        if !keep {
            commands.entity(effect.light).despawn();
            if let Some(drawn) = &effect.drawn {
                commands.entity(drawn.entity).despawn();
                images.remove(&drawn.image);
                materials.remove(&drawn.material);
                meshes.remove(&drawn.mesh);
            }
        }
        keep
    });
}
