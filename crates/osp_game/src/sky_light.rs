//! Image-based lighting from the sky: what lights a vessel's surfaces besides the sun, and
//! what they reflect.
//!
//! Each frame `shaders/sky_light.wesl` renders the sky and the ground around the floating
//! origin (the vessel) into a small cubemap, with the same scattering as the visible sky
//! and the ground lit by the sun through the air. Bevy's `GeneratedEnvironmentMapLight`
//! filters it into the diffuse and glossy light every PBR material on the main camera
//! picks up: blue from the sky above and the ground's colour from below on the pad, the
//! bright planet underneath and black space above in orbit. The terrain lights itself
//! instead (`shaders/terrain.wesl`): the probe only holds near the vessel.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    light::{EnvironmentMapLight, GeneratedEnvironmentMapLight, light_consts::lux},
    pbr::generate::downsampling_system,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::{
            binding_types::{sampler, texture_2d, texture_storage_2d_array, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        texture::{FallbackImage, GpuImage},
    },
    shader::ShaderDefVal,
};

use crate::{
    FrameSet,
    atmosphere::{TransmittanceLuts, view_steps},
    camera::MainCamera,
    frame::{RenderFrame, to_render},
    settings::{GraphicsSettings, Quality},
    sim::Sim,
    surface::Surface,
};

/// Albedo of the ground under an Earth-like atmosphere: vegetation, soil and sand.
const EARTH_GROUND: Vec3 = Vec3::new(0.11, 0.12, 0.08);
/// Airless bodies' ground albedo as a share of their display colour.
const AIRLESS_GROUND: f32 = 0.25;

pub struct SkyLightPlugin;

impl Plugin for SkyLightPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/sky_light.wesl");
        app.init_resource::<SkyLight>()
            .add_plugins(ExtractResourcePlugin::<SkyLight>::default())
            .add_systems(
                Update,
                apply_effects_setting.run_if(resource_changed::<GraphicsSettings>),
            )
            .add_systems(
                Update,
                place_probe
                    .after(crate::frame::update_sunlight)
                    .in_set(FrameSet::Render),
            );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<SkyLightBindGroup>()
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(
                Render,
                (
                    prepare_bind_group.in_set(RenderSystems::PrepareBindGroups),
                    // Before Bevy filters the cubemap for this frame.
                    render_probe
                        .after(RenderSystems::PrepareBindGroups)
                        .before(downsampling_system),
                ),
            );
    }
}

#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct SkyLightParams {
    /// xyz: planet centre relative to the probe (render axes, m); w: mean radius.
    planet: Vec4,
    /// xyz: direction to the sun; w: its illuminance above the atmosphere.
    sun: Vec4,
    /// rgb: ground albedo; w: 1 for an Earth-like atmosphere.
    ground: Vec4,
    /// x: ray-marching steps.
    options: Vec4,
}

/// The probe's cubemap and what the shader needs to fill it this frame.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub struct SkyLight {
    /// The cubemap and its size, once the graphics settings are known.
    image: Option<(Handle<Image>, u32)>,
    /// The origin body's sunlight transmittance table.
    transmittance: Handle<Image>,
    params: SkyLightParams,
}

/// Cubemap size at each `effects` setting: sharp enough at the top for glossy paint to
/// mirror the horizon.
fn probe_size(quality: Quality) -> u32 {
    match quality {
        Quality::Off => 32,
        Quality::Low => 64,
        Quality::Medium => 128,
        Quality::High => 256,
        Quality::Ultra => 512,
    }
}

fn cubemap(size: u32) -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        &[0; 8],
        TextureFormat::Rgba16Float,
        // Bevy reads the source's size in the main world when it sets up the filtering.
        RenderAssetUsages::all(),
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::STORAGE_BINDING | TextureUsages::COPY_SRC;
    image
}

fn apply_effects_setting(
    mut commands: Commands,
    settings: Res<GraphicsSettings>,
    mut sky_light: ResMut<SkyLight>,
    mut images: ResMut<Assets<Image>>,
    camera: Single<Entity, With<MainCamera>>,
) {
    let size = probe_size(settings.effects);
    if sky_light
        .image
        .as_ref()
        .is_some_and(|(_, current)| *current == size)
    {
        return;
    }
    let image = images.add(cubemap(size));
    // Bevy sets up the filtered maps when the source is attached, so replace both.
    commands
        .entity(*camera)
        .remove::<EnvironmentMapLight>()
        .insert(GeneratedEnvironmentMapLight {
            environment_map: image.clone(),
            // The cubemap holds luminance already.
            intensity: 1.0,
            ..default()
        });
    sky_light.image = Some((image, size));
}

fn place_probe(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    settings: Res<GraphicsSettings>,
    luts: Res<TransmittanceLuts>,
    mut sky_light: ResMut<SkyLight>,
) {
    let Some(origin_body) = frame.origin_body else {
        return;
    };
    let system = &sim.universe.system;
    let body = system.body(origin_body);
    let earthlike = Surface::is_earthlike(body);
    let ground = if body.parent.is_none() {
        Vec3::ZERO
    } else if earthlike {
        EARTH_GROUND
    } else {
        Vec3::from(body.color) * AIRLESS_GROUND
    };
    let sun = to_render(frame.body_offset(system.root())).normalize_or(Vec3::Y);
    sky_light.params = SkyLightParams {
        planet: to_render(frame.body_offset(origin_body)).extend(body.radius as f32),
        sun: sun.extend(lux::RAW_SUNLIGHT),
        ground: ground.extend(if earthlike { 1.0 } else { 0.0 }),
        options: Vec4::new(view_steps(settings.effects) as f32, 0.0, 0.0, 0.0),
    };
    sky_light.transmittance = luts.texture(origin_body);
}

#[derive(Resource)]
struct SkyLightPipeline {
    layout: BindGroupLayoutDescriptor,
    pipeline: CachedComputePipelineId,
}

fn init_pipeline(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "sky_light",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::COMPUTE,
            (
                (0, uniform_buffer::<SkyLightParams>(false)),
                (
                    1,
                    texture_storage_2d_array(
                        TextureFormat::Rgba16Float,
                        StorageTextureAccess::WriteOnly,
                    ),
                ),
                (
                    110,
                    texture_2d(TextureSampleType::Float { filterable: true }),
                ),
                (111, sampler(SamplerBindingType::Filtering)),
            ),
        ),
    );
    let pipeline = pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
        label: Some("sky_light".into()),
        layout: vec![layout.clone()],
        shader: asset_server.load("embedded://osp/shaders/sky_light.wesl"),
        // `atmosphere.wesl` binds its table in the material group; here that is group 0.
        shader_defs: vec![ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 0)],
        ..default()
    });
    commands.insert_resource(SkyLightPipeline { layout, pipeline });
}

/// This frame's bind group and the cubemap's size, while there is one to fill.
#[derive(Resource, Default)]
struct SkyLightBindGroup(Option<(BindGroup, u32)>);

fn prepare_bind_group(
    sky_light: Res<SkyLight>,
    pipeline: Res<SkyLightPipeline>,
    pipeline_cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut bind_group: ResMut<SkyLightBindGroup>,
) {
    bind_group.0 = None;
    let Some((target, size)) = sky_light
        .image
        .as_ref()
        .and_then(|(image, size)| Some((images.get(image)?, *size)))
    else {
        return;
    };
    // Bodies without an atmosphere have no table; the shader doesn't sample it then.
    let lut = images.get(&sky_light.transmittance).unwrap_or(&fallback.d2);
    let mut params = UniformBuffer::from(sky_light.params);
    params.write_buffer(&device, &queue);
    let output = target.texture.create_view(&TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    let group = device.create_bind_group(
        "sky_light",
        &pipeline_cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::with_indices((
            (0, &params),
            (1, &output),
            (110, &lut.texture_view),
            (111, &lut.sampler),
        )),
    );
    bind_group.0 = Some((group, size));
}

fn render_probe(
    bind_group: Res<SkyLightBindGroup>,
    pipeline: Res<SkyLightPipeline>,
    pipeline_cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let Some((group, size)) = &bind_group.0 else {
        return;
    };
    let Some(compute) = pipeline_cache.get_compute_pipeline(pipeline.pipeline) else {
        return;
    };
    let mut pass = ctx
        .command_encoder()
        .begin_compute_pass(&ComputePassDescriptor {
            label: Some("sky_light"),
            timestamp_writes: None,
        });
    pass.set_pipeline(compute);
    pass.set_bind_group(0, group, &[]);
    let groups = size.div_ceil(8);
    pass.dispatch_workgroups(groups, groups, 6);
}
