//! The sea's waves on the GPU: from the spectrum in `osp_sim::ocean` to a map of slopes and foam,
//! every frame. A compute chain over three cascades of different sizes:
//!
//! 1. Spectrum (`shaders/ocean_spectrum.wesl`): every wave mode at this instant.
//! 2. FFT (`shaders/ocean_fft.wesl`): back to a height field, a dispatch per stage of the transform,
//!    along the rows and then the columns, between two textures in turn.
//! 3. Assemble (`shaders/ocean_assemble.wesl`): slope, its square, foam.
//! 4. Mips (`shaders/ocean_mips.wesl`): averages, so a coarse pixel sees the slope's variance.
//!
//! The terrain reads the result (`shaders/terrain.wesl`), as an array image shared by every chunk.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::{
            binding_types::{texture_2d_array, texture_storage_2d_array, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue},
        texture::GpuImage,
    },
    shader::ShaderDefVal,
};
use osp_sim::ocean::{CASCADES, OCEAN_SIZE, OceanDef};

use crate::{FrameSet, sim::Sim};

/// Levels in the maps' mip chain: 256 down to 1.
const MIP_LEVELS: u32 = 9;
/// How long foam takes to fade by a factor of e, s.
const FOAM_LIFETIME: f32 = 2.5;

pub struct OceanPlugin;

impl Plugin for OceanPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/ocean_spectrum.wesl");
        embedded_asset!(app, "shaders/ocean_fft.wesl");
        embedded_asset!(app, "shaders/ocean_assemble.wesl");
        embedded_asset!(app, "shaders/ocean_mips.wesl");
        let def = app
            .world()
            .resource::<Sim>()
            .universe
            .system
            .bodies()
            .iter()
            .find_map(|body| body.ocean.clone());
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let maps = images.add(maps_image());
        let spectrum = def.as_ref().map(|def| images.add(amplitudes_image(def)));
        let gravity = def.as_ref().map_or(9.81, |d| d.gravity as f32);
        app.insert_resource(OceanMaps(maps.clone()))
            .insert_resource(OceanFrame {
                maps,
                spectrum,
                gravity,
                seconds: 0.0,
                delta: 0.0,
            })
            .add_plugins(ExtractResourcePlugin::<OceanFrame>::default())
            .add_systems(Update, advance.in_set(FrameSet::Render));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<OceanTargets>()
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(
                Render,
                (
                    prepare.in_set(RenderSystems::PrepareBindGroups),
                    run_chain
                        .after(RenderSystems::PrepareBindGroups)
                        .before(RenderSystems::Render),
                ),
            );
    }
}

/// The waves' maps, for the terrain.
#[derive(Resource)]
pub struct OceanMaps(pub Handle<Image>);

fn maps_image() -> Image {
    let n = OCEAN_SIZE as u32;
    let mut image = Image::default();
    image.texture_descriptor.size = Extent3d {
        width: n,
        height: n,
        depth_or_array_layers: CASCADES.len() as u32,
    };
    image.texture_descriptor.format = TextureFormat::Rgba16Float;
    image.texture_descriptor.mip_level_count = MIP_LEVELS;
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::STORAGE_BINDING | TextureUsages::COPY_DST;
    image.data = None;
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    });
    image
}

fn amplitudes_image(def: &OceanDef) -> Image {
    let n = OCEAN_SIZE as u32;
    let mut data = Vec::with_capacity(OCEAN_SIZE * OCEAN_SIZE * CASCADES.len() * 16);
    for cascade in 0..CASCADES.len() {
        for texel in def.amplitudes(cascade) {
            for value in texel {
                data.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    let mut image = Image::default();
    image.texture_descriptor.size = Extent3d {
        width: n,
        height: n,
        depth_or_array_layers: CASCADES.len() as u32,
    };
    image.texture_descriptor.format = TextureFormat::Rgba32Float;
    image.data = Some(data);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image
}

/// What the render world needs for this frame's waves.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
struct OceanFrame {
    maps: Handle<Image>,
    spectrum: Option<Handle<Image>>,
    gravity: f32,
    /// Real time, not the game's: waves shouldn't whip by under time warp.
    seconds: f32,
    delta: f32,
}

fn advance(time: Res<Time<Real>>, mut frame: ResMut<OceanFrame>) {
    frame.delta = time.delta_secs().min(0.1);
    // A float loses a wave's phase slowly: an hour's precision is plenty, and long-period modes
    // don't jump visibly when it wraps once in that time.
    frame.seconds = (frame.seconds + frame.delta) % 3600.0;
}

#[derive(Resource)]
struct OceanPipelines {
    spectrum_layout: BindGroupLayoutDescriptor,
    fft_layout: BindGroupLayoutDescriptor,
    assemble_layout: BindGroupLayoutDescriptor,
    mips_layout: BindGroupLayoutDescriptor,
    spectrum: CachedComputePipelineId,
    fft: CachedComputePipelineId,
    assemble: CachedComputePipelineId,
    mips: CachedComputePipelineId,
}

fn init_pipelines(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
) {
    let not_filtered = TextureSampleType::Float { filterable: false };
    let float_store = |access| texture_storage_2d_array(TextureFormat::Rgba32Float, access);
    let write = StorageTextureAccess::WriteOnly;
    let spectrum_layout = BindGroupLayoutDescriptor::new(
        "ocean_spectrum",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<Vec4>(false),
                texture_2d_array(not_filtered),
                float_store(write),
                float_store(write),
            ),
        ),
    );
    let fft_layout = BindGroupLayoutDescriptor::new(
        "ocean_fft",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<UVec4>(false),
                texture_2d_array(not_filtered),
                float_store(write),
            ),
        ),
    );
    let assemble_layout = BindGroupLayoutDescriptor::new(
        "ocean_assemble",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                uniform_buffer::<Vec4>(false),
                texture_2d_array(not_filtered),
                texture_2d_array(not_filtered),
                texture_2d_array(not_filtered),
                texture_storage_2d_array(TextureFormat::R32Float, write),
                texture_storage_2d_array(TextureFormat::Rgba16Float, write),
            ),
        ),
    );
    let mips_layout = BindGroupLayoutDescriptor::new(
        "ocean_mips",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::COMPUTE,
            (
                texture_2d_array(not_filtered),
                texture_storage_2d_array(TextureFormat::Rgba16Float, write),
            ),
        ),
    );
    let queue = |label: &'static str,
                 path: &'static str,
                 layout: &BindGroupLayoutDescriptor,
                 defs: Vec<ShaderDefVal>| {
        pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(label.into()),
            layout: vec![layout.clone()],
            shader: asset_server.load(path),
            shader_defs: defs,
            ..default()
        })
    };
    let pipelines = OceanPipelines {
        spectrum: queue(
            "ocean_spectrum",
            "embedded://osp_render/shaders/ocean_spectrum.wesl",
            &spectrum_layout,
            vec![],
        ),
        fft: queue(
            "ocean_fft",
            "embedded://osp_render/shaders/ocean_fft.wesl",
            &fft_layout,
            vec![],
        ),
        assemble: queue(
            "ocean_assemble",
            "embedded://osp_render/shaders/ocean_assemble.wesl",
            &assemble_layout,
            vec![],
        ),
        mips: queue(
            "ocean_mips",
            "embedded://osp_render/shaders/ocean_mips.wesl",
            &mips_layout,
            vec![],
        ),
        spectrum_layout,
        fft_layout,
        assemble_layout,
        mips_layout,
    };
    commands.insert_resource(pipelines);
}

/// Scratch textures of a frame's chain, and its bind groups.
#[derive(Resource, Default)]
struct OceanTargets {
    fields: Option<Fields>,
    next_foam: usize,
    spectrum_params: UniformBuffer<Vec4>,
    assemble_params: UniformBuffer<Vec4>,
    groups: Option<Groups>,
}

struct Fields {
    /// Two fields, each with two textures the stages go back and forth between. An even number of
    /// stages leaves the result where the spectrum was written.
    first: [TextureView; 2],
    second: [TextureView; 2],
    foam: [TextureView; 2],
    /// Each stage's size and direction.
    stages: Vec<UniformBuffer<UVec4>>,
}

struct Groups {
    spectrum: BindGroup,
    transforms: Vec<BindGroup>,
    assemble: BindGroup,
    mips: Vec<(BindGroup, u32)>,
}

fn array_target(device: &RenderDevice, format: TextureFormat, label: &'static str) -> TextureView {
    device
        .create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width: OCEAN_SIZE as u32,
                height: OCEAN_SIZE as u32,
                depth_or_array_layers: CASCADES.len() as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        })
        .create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            ..default()
        })
}

fn prepare(
    frame: Res<OceanFrame>,
    pipelines: Res<OceanPipelines>,
    pipeline_cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut targets: ResMut<OceanTargets>,
) {
    let targets = &mut *targets;
    targets.groups = None;
    let (Some(spectrum), Some(maps)) = (
        frame
            .spectrum
            .as_ref()
            .and_then(|handle| images.get(handle)),
        images.get(&frame.maps),
    ) else {
        return;
    };
    let fields = targets.fields.get_or_insert_with(|| {
        let store = |label| array_target(&device, TextureFormat::Rgba32Float, label);
        let stages = OCEAN_SIZE.ilog2();
        Fields {
            first: [store("ocean_first"), store("ocean_first")],
            second: [store("ocean_second"), store("ocean_second")],
            foam: [
                array_target(&device, TextureFormat::R32Float, "ocean_foam"),
                array_target(&device, TextureFormat::R32Float, "ocean_foam"),
            ],
            stages: (0..2 * stages)
                .map(|k| {
                    let size = 1u32 << (k % stages);
                    let mut buffer =
                        UniformBuffer::from(UVec4::new(size, u32::from(k >= stages), 0, 0));
                    buffer.write_buffer(&device, &queue);
                    buffer
                })
                .collect(),
        }
    });
    targets.next_foam = 1 - targets.next_foam;
    targets
        .spectrum_params
        .set(Vec4::new(frame.seconds, frame.gravity, 0.0, 0.0));
    targets.spectrum_params.write_buffer(&device, &queue);
    targets
        .assemble_params
        .set(Vec4::new(frame.delta, FOAM_LIFETIME, 0.0, 0.0));
    targets.assemble_params.write_buffer(&device, &queue);

    let spectrum_group = device.create_bind_group(
        "ocean_spectrum",
        &pipeline_cache.get_bind_group_layout(&pipelines.spectrum_layout),
        &BindGroupEntries::sequential((
            &targets.spectrum_params,
            &spectrum.texture_view,
            &fields.first[0],
            &fields.second[0],
        )),
    );
    let layout = pipeline_cache.get_bind_group_layout(&pipelines.fft_layout);
    let transforms: Vec<BindGroup> = [&fields.first, &fields.second]
        .into_iter()
        .flat_map(|field| {
            let layout = &layout;
            let device = &device;
            fields.stages.iter().enumerate().map(move |(k, stage)| {
                device.create_bind_group(
                    "ocean_fft",
                    layout,
                    &BindGroupEntries::sequential((stage, &field[k % 2], &field[1 - k % 2])),
                )
            })
        })
        .collect();
    let level = |mip: u32| {
        maps.texture.create_view(&TextureViewDescriptor {
            dimension: Some(TextureViewDimension::D2Array),
            base_mip_level: mip,
            mip_level_count: Some(1),
            ..default()
        })
    };
    let assemble = device.create_bind_group(
        "ocean_assemble",
        &pipeline_cache.get_bind_group_layout(&pipelines.assemble_layout),
        &BindGroupEntries::sequential((
            &targets.assemble_params,
            &fields.first[0],
            &fields.second[0],
            &fields.foam[1 - targets.next_foam],
            &fields.foam[targets.next_foam],
            &level(0),
        )),
    );
    let mips = (1..MIP_LEVELS)
        .map(|mip| {
            let group = device.create_bind_group(
                "ocean_mips",
                &pipeline_cache.get_bind_group_layout(&pipelines.mips_layout),
                &BindGroupEntries::sequential((&level(mip - 1), &level(mip))),
            );
            (group, (OCEAN_SIZE as u32 >> mip).max(1))
        })
        .collect();
    targets.groups = Some(Groups {
        spectrum: spectrum_group,
        transforms,
        assemble,
        mips,
    });
}

fn run_chain(
    targets: Res<OceanTargets>,
    pipelines: Res<OceanPipelines>,
    pipeline_cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let Some(groups) = &targets.groups else {
        return;
    };
    let (Some(spectrum), Some(fft), Some(assemble), Some(mips)) = (
        pipeline_cache.get_compute_pipeline(pipelines.spectrum),
        pipeline_cache.get_compute_pipeline(pipelines.fft),
        pipeline_cache.get_compute_pipeline(pipelines.assemble),
        pipeline_cache.get_compute_pipeline(pipelines.mips),
    ) else {
        return;
    };
    let layers = CASCADES.len() as u32;
    let n = OCEAN_SIZE as u32;
    let mut pass = ctx
        .command_encoder()
        .begin_compute_pass(&ComputePassDescriptor {
            label: Some("ocean"),
            timestamp_writes: None,
        });
    pass.set_pipeline(spectrum);
    pass.set_bind_group(0, &groups.spectrum, &[]);
    pass.dispatch_workgroups(n.div_ceil(16), n.div_ceil(16), layers);
    pass.set_pipeline(fft);
    for group in &groups.transforms {
        pass.set_bind_group(0, group, &[]);
        pass.dispatch_workgroups(n.div_ceil(16), n.div_ceil(16), layers);
    }
    pass.set_pipeline(assemble);
    pass.set_bind_group(0, &groups.assemble, &[]);
    pass.dispatch_workgroups(n.div_ceil(16), n.div_ceil(16), layers);
    pass.set_pipeline(mips);
    for (group, size) in &groups.mips {
        pass.set_bind_group(0, group, &[]);
        pass.dispatch_workgroups(size.div_ceil(8), size.div_ceil(8), layers);
    }
}
