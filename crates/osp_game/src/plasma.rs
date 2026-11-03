//! Re-entry: the shock and the wake around a vessel coming in hot (`shaders/plasma.wesl`).
//!
//! How hot comes from the air's density and the vessel's speed (`osp_sim::plasma`). Its shape comes
//! from the vessel: a small map of its hull looking along the airflow, built on the CPU whenever
//! the flow or the vessel changes, and uploaded in place. The light it throws on the vessel and
//! anything else near is a point light at the shock.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    light::NotShadowCaster,
    math::{DQuat, DVec3},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    platform::collections::HashMap,
    prelude::*,
    render::render_resource::{
        AsBindGroup, CompareFunction, Extent3d, Face, RenderPipelineDescriptor, ShaderType,
        SpecializedMeshPipelineError, TextureDimension, TextureFormat,
    },
    shader::ShaderRef,
};
use half::f16;
use osp_sim::{
    plasma::{HULL_MAP_SIZE, HullMap, flow_frame, glow_level, hull_map, stagnation_heating},
    vessel::VesselId,
};

use crate::{
    FrameSet,
    frame::RenderFrame,
    settings::{GraphicsSettings, Quality},
    sim::Sim,
};

/// How many blurred copies of the hull map there are (`HULL_LEVELS` in the shader).
const HULL_LEVELS: usize = 6;
/// Vessels with effects at once.
const MOST_AT_ONCE: usize = 4;
/// Below this strength, nothing is drawn.
const LEAST_STRENGTH: f32 = 0.01;
/// How fast the effect may change, per second, so it swells rather than pops on a stage drop.
const CHANGE_RATE: f32 = 0.6;
/// The flow has to turn this far before the map is rebuilt, radians.
const REMAP_ANGLE: f64 = 0.015;
/// Brightness of the gas at full strength, cd/m², per hull radius of it in the line of sight.
const GAS_BRIGHTNESS: f32 = 45_000.0;
/// Light the gas throws at full strength, lumens per square metre of hull.
const LIGHT_PER_AREA: f32 = 1.2e7;
/// How long the wake is at full strength, in hull radii.
const WAKE_LENGTH: f64 = 22.0;
/// How far the box reaches upstream of the hull, in radii, to hold the shock.
const UPSTREAM: f64 = 1.5;
/// How far the wake is shown to be moving, m/s, whatever the true speed: at the true 7 km/s the
/// streaks would only be a blur.
const SHOWN_FLOW: f32 = 90.0;
/// What air burns like when the data doesn't say.
const AIR_PLASMA: [f32; 3] = [1.0, 0.42, 0.2];

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
    box_size: Vec4,
    color: Vec4,
    hull: Vec4,
    look: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
struct PlasmaMaterial {
    #[uniform(0)]
    params: PlasmaParams,
    #[texture(1, dimension = "3d")]
    #[sampler(2)]
    hull: Handle<Image>,
}

impl Material for PlasmaMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://osp/shaders/plasma.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
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
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // The inside of the box, whatever is in the way: the ray stops at what the depth says, and
        // the camera may well be in the box.
        descriptor.primitive.cull_mode = Some(Face::Front);
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_compare = CompareFunction::Always.into();
        }
        Ok(())
    }
}

/// One vessel's effect.
struct Effect {
    entity: Entity,
    light: Entity,
    material: Handle<PlasmaMaterial>,
    image: Handle<Image>,
    strength: f32,
    /// The flow in the vessel's axes the map was built for, and the hull's size then.
    mapped_flow: DVec3,
    mapped_stations: usize,
    map: Option<HullMap>,
}

#[derive(Resource, Default)]
struct PlasmaEffects(HashMap<VesselId, Effect>);

/// The hull map, blurred more at each level, as the slices of a 3D texture. Not mip levels: an image
/// updated in place only gets its top level rewritten. A box blur of twice the width each time,
/// from a summed-area table.
fn hull_image(map: Option<&HullMap>) -> Image {
    let n = HULL_MAP_SIZE;
    let top: Vec<[f32; 3]> = map.map_or_else(
        || vec![[0.0; 3]; n * n],
        // Front and back times coverage, so they average properly as they blur.
        |m| {
            m.texels
                .iter()
                .map(|t| [t[0] * t[2], t[1] * t[2], t[2]])
                .collect()
        },
    );
    let mut table = vec![[0.0f32; 3]; (n + 1) * (n + 1)];
    for j in 0..n {
        for i in 0..n {
            let (left, up, both) = (
                table[(j + 1) * (n + 1) + i],
                table[j * (n + 1) + i + 1],
                table[j * (n + 1) + i],
            );
            table[(j + 1) * (n + 1) + i + 1] =
                std::array::from_fn(|c| top[j * n + i][c] + left[c] + up[c] - both[c]);
        }
    }
    let mut data = Vec::with_capacity(n * n * HULL_LEVELS * 8);
    for level in 0..HULL_LEVELS {
        let reach = if level == 0 { 0 } else { 1usize << (level - 1) };
        for j in 0..n {
            for i in 0..n {
                let (x0, x1) = (i.saturating_sub(reach), (i + reach + 1).min(n));
                let (y0, y1) = (j.saturating_sub(reach), (j + reach + 1).min(n));
                // What lies past the map's edge counts as nothing there.
                let area = ((2 * reach + 1) * (2 * reach + 1)) as f32;
                for c in 0..3 {
                    let sum = table[y1 * (n + 1) + x1][c]
                        - table[y0 * (n + 1) + x1][c]
                        - table[y1 * (n + 1) + x0][c]
                        + table[y0 * (n + 1) + x0][c];
                    data.extend_from_slice(&f16::from_f32(sum / area).to_le_bytes());
                }
                data.extend_from_slice(&f16::from_f32(1.0).to_le_bytes());
            }
        }
    }
    let mut image = Image::default();
    image.texture_descriptor.size = Extent3d {
        width: n as u32,
        height: n as u32,
        depth_or_array_layers: HULL_LEVELS as u32,
    };
    image.texture_descriptor.dimension = TextureDimension::D3;
    image.texture_descriptor.format = TextureFormat::Rgba16Float;
    image.data = Some(data);
    // Rebuilt as the flow turns, so the main world keeps its copy to write into.
    image.asset_usage = RenderAssetUsages::default();
    image.texture_descriptor.usage |= bevy::render::render_resource::TextureUsages::COPY_DST;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

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
    mut boxes: Query<&mut Transform, Without<PointLight>>,
    mut lights: Query<(&mut PointLight, &mut Transform), With<PointLight>>,
    mut unit_cube: Local<Option<Handle<Mesh>>>,
) {
    if frame.origin_body.is_none() {
        return;
    }
    let system = &sim.universe.system;
    let cube = unit_cube
        .get_or_insert_with(|| meshes.add(Cuboid::new(1.0, 1.0, 1.0)))
        .clone();
    let steps = match settings.effects {
        Quality::Off | Quality::Low => 24.0,
        Quality::Medium => 40.0,
        Quality::High => 56.0,
        Quality::Ultra => 80.0,
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
        let speed = airflow.length();
        let outline = vessel.propulsion.shape();
        let radius = (outline.reference_area() / std::f64::consts::PI)
            .sqrt()
            .max(0.3);
        let heating = stagnation_heating(air.sample(altitude).density, speed, radius);
        let target = glow_level(heating) as f32;
        if target <= 0.0 && !effects.0.contains_key(&vessel.id()) {
            continue;
        }
        if seen.len() >= MOST_AT_ONCE {
            continue;
        }
        seen.push(vessel.id());
        // In the vessel's axes: which way the air goes over it.
        let attitude = vessel.attitude_at(system, frame.time);
        let flow_local = attitude.inverse() * (-airflow).normalize_or(-DVec3::Z);
        let stations = outline.outline().len();
        let effect = effects.0.entry(vessel.id()).or_insert_with(|| {
            let image = images.add(hull_image(None));
            let material = materials.add(PlasmaMaterial {
                params: PlasmaParams::default(),
                hull: image.clone(),
            });
            let entity = commands
                .spawn((
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::default(),
                    NotShadowCaster,
                    // The box follows the flow and its size changes with it.
                    bevy::camera::visibility::NoFrustumCulling,
                    Name::new("Re-entry"),
                ))
                .id();
            let light = commands
                .spawn((
                    PointLight {
                        intensity: 0.0,
                        range: 500.0,
                        radius: 2.0,
                        ..default()
                    },
                    Transform::default(),
                ))
                .id();
            Effect {
                entity,
                light,
                material,
                image,
                strength: 0.0,
                mapped_flow: DVec3::ZERO,
                mapped_stations: 0,
                map: None,
            }
        });

        // Swells and fades instead of jumping when a stage goes, or the view changes vessel.
        let most = CHANGE_RATE * time.delta_secs();
        effect.strength += (target - effect.strength).clamp(-most, most);

        // Middle of the hull as the box's centre: rings are from the middle of the base.
        let middle = vessel.propulsion.length() * 0.5;
        let remap = effect.map.is_none()
            || effect.mapped_stations != stations
            || effect.mapped_flow.angle_between(flow_local) > REMAP_ANGLE;
        if remap && effect.strength > 0.0 {
            let rings: Vec<(DVec3, f64)> = outline
                .rings()
                .map(|(centre, reach)| (centre - DVec3::Z * middle, reach))
                .collect();
            let map = hull_map(&rings, DVec3::Z, flow_local);
            if let Some(mut image) = images.get_mut(&effect.image) {
                *image = hull_image(Some(&map));
            }
            effect.mapped_flow = flow_local;
            effect.mapped_stations = stations;
            effect.map = Some(map);
        }

        let show = effect.strength > LEAST_STRENGTH;
        let Some(map) = effect.map.as_ref().filter(|_| show) else {
            if let Ok(mut transform) = boxes.get_mut(effect.entity) {
                transform.scale = Vec3::ZERO;
            }
            if let Ok((mut light, _)) = lights.get_mut(effect.light) {
                light.intensity = 0.0;
            }
            continue;
        };

        // The box: z along the flow, from just ahead of the shock to the end of the wake.
        let hull_radius = map.reach;
        let wake = WAKE_LENGTH * hull_radius * (0.35 + 0.65 * effect.strength as f64);
        let start = map.front - UPSTREAM * hull_radius;
        let end = map.back + wake;
        let half_width = map.half_width;
        let to_flow: DQuat = flow_frame(flow_local);
        let box_rotation = attitude * to_flow.inverse();
        let base = vessel.base_at(system, frame.time);
        let centre_body = base + attitude * (DVec3::Z * middle);
        let box_centre = centre_body + box_rotation * (DVec3::Z * ((start + end) * 0.5));
        if let Ok(mut transform) = boxes.get_mut(effect.entity) {
            transform.translation = frame.to_render(vessel.parent(), box_centre);
            // The box's own axes are in the sim's convention, flow along its Z, unlike a model's:
            // only the world side of the rotation changes axes.
            transform.rotation =
                (DQuat::from_rotation_x(-std::f64::consts::FRAC_PI_2) * box_rotation).as_quat();
            transform.scale = Vec3::new(
                (2.0 * half_width) as f32,
                (2.0 * half_width) as f32,
                (end - start) as f32,
            );
        }
        if let Some(mut material) = materials.get_mut(&effect.material) {
            let tint = body
                .scattering
                .as_ref()
                .and_then(|s| s.plasma)
                .unwrap_or(AIR_PLASMA.map(f64::from))
                .map(|c| c as f32);
            material.params = PlasmaParams {
                box_size: Vec4::new(half_width as f32, start as f32, end as f32, steps),
                color: Vec4::new(tint[0], tint[1], tint[2], effect.strength),
                hull: Vec4::new(
                    hull_radius as f32,
                    map.front as f32,
                    map.back as f32,
                    SHOWN_FLOW,
                ),
                look: Vec4::new(GAS_BRIGHTNESS, map.half_width as f32, 0.0, 0.0),
            };
        }
        if let Ok((mut light, mut transform)) = lights.get_mut(effect.light) {
            let tint = body
                .scattering
                .as_ref()
                .and_then(|s| s.plasma)
                .unwrap_or(AIR_PLASMA.map(f64::from));
            light.color = Color::linear_rgb(tint[0] as f32, tint[1] as f32, tint[2] as f32);
            let area = (hull_radius * hull_radius) as f32;
            light.intensity = LIGHT_PER_AREA * area * effect.strength * effect.strength;
            let ahead = box_rotation * (DVec3::Z * (map.front - hull_radius * 0.6));
            transform.translation = frame.to_render(vessel.parent(), centre_body + ahead);
        }
    }
    // Gone, or cooled right down.
    effects.0.retain(|id, effect| {
        let keep = seen.contains(id);
        if !keep {
            commands.entity(effect.entity).despawn();
            commands.entity(effect.light).despawn();
            images.remove(&effect.image);
            materials.remove(&effect.material);
        }
        keep
    });
}
