//! Sky, haze and sunlight for bodies with air.
//!
//! How much light gets through the air, and how much it scatters, comes from
//! [`osp_sim::scattering`]: each body with `scattering` in its data has tables of it, built once
//! here and uploaded for the shaders (`shaders/atmosphere.wesl`).
//!
//! The sky and the haze are one pass (`shaders/sky.wesl`), drawn over the whole view for every
//! body with air in sight, between the opaque scene and anything transparent. Working from the
//! depth buffer, it treats everything in the opaque scene alike: the ground, the vessel, the
//! launch site and the trees all sit in the same haze. It is one draw per body, farthest first,
//! so the Moon seen through Earth's air, or Mars's glow from Earth's sky, come out right.
//!
//! Sunlight is dimmed and reddened for the air between the sun and the vessel the camera looks
//! at, so the vessel and everything around it is lit like the ground it stands on: orange at
//! sunset, dark at night. The terrain reads that dimming back from the light's colour, divides it
//! out and applies each fragment's own value.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    camera::Exposure,
    core_pipeline::{
        FullscreenShader,
        core_3d::{main_opaque_pass_3d, main_transparent_pass_3d},
        schedule::Core3d,
    },
    image::{
        CompressedImageFormats, ImageAddressMode, ImageFilterMode, ImageSampler,
        ImageSamplerDescriptor, ImageType,
    },
    light::light_consts::lux,
    math::DVec3,
    platform::collections::HashMap,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_component::ExtractComponentPlugin,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::{
            binding_types::{
                sampler, texture_2d, texture_2d_multisampled, texture_cube, uniform_buffer,
            },
            *,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        texture::{FallbackImage, GpuImage},
        view::{
            Msaa, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms,
        },
    },
    shader::ShaderDefVal,
};
use half::f16;
use osp_sim::{
    body::BodyId,
    clouds::{CloudMap, CloudsDef, WEATHER_SIZE, Weather},
    scattering::{Air, SKY_TABLE_SIZE, ScatteringDef, TRANSMITTANCE_SIZE},
};

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera},
    content::GameContent,
    eclipse::{EclipseUniform, eclipse_for},
    frame::{RenderFrame, Sunlight, rotation_to_render},
    settings::{GraphicsSettings, Quality},
    sim::Sim,
};

/// Lower bound of the light's dimming, so the shaders can divide it back out.
const MIN_TRANSMITTANCE: f64 = 1e-4;
/// Camera exposure (EV100) in daylight and in darkness: the view adapts like an eye or an
/// auto-exposure camera, so night scenes show floodlights and city lights.
const DAY_EV: f32 = 15.0;
const NIGHT_EV: f32 = 9.0;
/// Time constant of that adaptation, s.
const ADAPTATION: f32 = 1.5;
/// The least share of sunlight the exposure takes account of in an eclipse.
const MIN_ECLIPSE_LIGHT: f32 = 0.004;
/// How many stops the exposure opens up under full cloud.
const OVERCAST_EV: f32 = 1.6;
/// Air this small on screen is left out, as the tangent of the half-angle: under half a pixel.
const SMALLEST_ON_SCREEN: f64 = 3e-4;
/// Mip levels of a weather map, for far views and soft shadows.
const WEATHER_MIP_LEVELS: u32 = 4;
/// The cloud map of a body without one of its own.
const SHARED_CLOUD_MAP: &str = "clouds/fair";

/// A cloud map from a mod's files, or why not.
fn read_cloud_map(content: &osp_mods::Content, path: &str) -> Result<CloudMap, String> {
    let read = |file: String| {
        let found = content
            .resolve_file(&file)
            .ok_or_else(|| format!("no {file} in any mod"))?;
        let bytes = std::fs::read(found).map_err(|e| format!("{file}: {e}"))?;
        Image::from_buffer(
            &bytes,
            ImageType::Extension("png"),
            CompressedImageFormats::NONE,
            false,
            ImageSampler::Default,
            RenderAssetUsages::RENDER_WORLD,
        )
        .map_err(|e| format!("{file}: {e}"))
    };
    let (positive, negative) = (read(format!("{path}P.png"))?, read(format!("{path}N.png"))?);
    let size = positive.width() as usize;
    if positive.height() as usize != size || negative.size() != positive.size() {
        return Err(format!("{path}: its two images should be the same square"));
    }
    let (Some(positive), Some(negative)) = (&positive.data, &negative.data) else {
        return Err(format!("{path}: no data"));
    };
    Ok(CloudMap::from_two_textures(size, 4, positive, negative))
}

/// A cloud map as a filterable cubemap with all its mips, one channel.
fn cloud_map_image(map: &CloudMap) -> Image {
    let levels = map.size.ilog2() + 1;
    let mut data = Vec::with_capacity(map.data.len() * 4 / 3 + 64);
    for face in map.data.chunks(map.size * map.size) {
        let (mut level, mut size) = (face.to_vec(), map.size);
        data.extend_from_slice(&level);
        while size > 1 {
            let half = size / 2;
            let mut next = vec![0u8; half * half];
            for y in 0..half {
                for x in 0..half {
                    let at = |dx: usize, dy: usize| level[(2 * y + dy) * size + 2 * x + dx] as u32;
                    next[y * half + x] = ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / 4) as u8;
                }
            }
            data.extend_from_slice(&next);
            level = next;
            size = half;
        }
    }
    let mut image = Image::default();
    image.texture_descriptor.size = Extent3d {
        width: map.size as u32,
        height: map.size as u32,
        depth_or_array_layers: 6,
    };
    image.texture_descriptor.format = TextureFormat::R8Unorm;
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// The cloud map's axes from a body's fixed axes, and its second copy's from those.
pub fn map_turns(def: &CloudsDef) -> (Mat3, Mat3) {
    let (first, second) = osp_sim::clouds::map_turns(def.seed);
    (
        Mat3::from_quat(first.as_quat()),
        Mat3::from_quat(second.as_quat()),
    )
}

pub struct AtmospherePlugin;

impl Plugin for AtmospherePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/sky.wesl");
        // Built here rather than at startup so terrain and skies can rely on them. Each takes a
        // few tenths of a second.
        let tables: Vec<(BodyId, Air, Option<BodyClouds>)> = {
            let system = &app.world().resource::<Sim>().universe.system;
            std::thread::scope(|scope| {
                let builds: Vec<_> = system
                    .bodies()
                    .iter()
                    .filter_map(|body| {
                        Some((
                            body.id,
                            body.scattering.as_ref()?,
                            body.clouds.as_ref(),
                            body.radius,
                        ))
                    })
                    .map(|(id, def, clouds, radius)| {
                        scope.spawn(move || {
                            let clouds = clouds.map(|def| BodyClouds {
                                weather: Weather::new(def),
                                def: def.clone(),
                            });
                            (id, Air::new(def, radius), clouds)
                        })
                    })
                    .collect();
                builds
                    .into_iter()
                    .map(|build| build.join().expect("building the air tables"))
                    .collect()
            })
        };
        // Each map once, whoever else uses it. What a map says also goes for the shade under it.
        let mut maps: HashMap<String, Option<CloudMap>> = HashMap::new();
        {
            let content = &app.world().resource::<GameContent>().0;
            for (_, _, clouds) in &tables {
                let Some(clouds) = clouds else { continue };
                let path = clouds.def.map.as_deref().unwrap_or(SHARED_CLOUD_MAP);
                maps.entry(path.to_owned()).or_insert_with(|| {
                    match read_cloud_map(content, path) {
                        Ok(map) => Some(map),
                        Err(why) => {
                            warn!("cloud map: {why}");
                            None
                        }
                    }
                });
            }
        }
        let mut tables = tables;
        for (_, _, clouds) in &mut tables {
            if let Some(clouds) = clouds
                && let Some(Some(map)) = clouds.def.map.as_deref().and_then(|path| maps.get(path))
            {
                clouds.weather.take_cover(map, clouds.def.seed);
            }
        }
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let map_images: HashMap<String, Handle<Image>> = maps
            .into_iter()
            .filter_map(|(path, map)| Some((path, images.add(cloud_map_image(&map?)))))
            .collect();
        let no_weather = images.add(clear_cubemap());
        let airs = tables
            .into_iter()
            .map(|(id, air, clouds)| {
                let map = clouds
                    .as_ref()
                    .and_then(|c| map_images.get(c.def.map.as_deref().unwrap_or(SHARED_CLOUD_MAP)))
                    .cloned()
                    .unwrap_or_else(|| no_weather.clone());
                (id, BodyAir::new(air, clouds, map, &mut images))
            })
            .collect();
        app.insert_resource(BodyAirs { airs, no_weather })
            .init_resource::<SkyDraws>()
            .add_plugins(ExtractResourcePlugin::<SkyDraws>::default())
            .add_plugins(ExtractComponentPlugin::<MainCamera>::default())
            .add_systems(
                Update,
                (place_skies, dim_sunlight)
                    .after(crate::frame::update_sunlight)
                    .in_set(FrameSet::Render),
            );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<SkyBuffers>()
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(Render, prepare_sky.in_set(RenderSystems::PrepareBindGroups))
            .add_systems(
                Core3d,
                render_sky
                    .after(main_opaque_pass_3d)
                    .before(main_transparent_pass_3d),
            );
    }
}

/// Ray-marching steps through the air for the sky and the haze at each `effects` setting.
/// Against a fine march (see `osp_sim::scattering`) the worst ray in the sky is 16% off at 16
/// steps, 4% at 24, 2.2% at 32 and 1% at 48, and that only on a few rays through the shadow's
/// edge at dusk.
pub fn view_steps(quality: Quality) -> u32 {
    match quality {
        Quality::Off | Quality::Low => 16,
        Quality::Medium => 24,
        Quality::High => 32,
        Quality::Ultra => 48,
    }
}

/// A body's air as the shaders take it (`Air` in `shaders/atmosphere.wesl`). All zero for a body
/// without any, whose shaders never read it.
#[derive(Debug, Clone, Copy, Default, ShaderType, Reflect)]
pub struct AirUniform {
    rayleigh: Vec4,
    mie: Vec4,
    mie_extinction: Vec4,
    mie_g: Vec4,
    ozone: Vec4,
    shape: Vec4,
}

impl AirUniform {
    fn new(def: &ScatteringDef, radius: f64) -> Self {
        let rgb = |c: [f64; 3], w: f64| Vec4::new(c[0] as f32, c[1] as f32, c[2] as f32, w as f32);
        Self {
            rayleigh: rgb(def.rayleigh, 1.0 / def.rayleigh_height),
            mie: rgb(def.mie, 1.0 / def.mie_height),
            mie_extinction: rgb(
                std::array::from_fn(|c| def.mie[c] + def.mie_absorption[c]),
                0.0,
            ),
            mie_g: rgb(def.mie_g, 0.0),
            ozone: rgb(def.ozone, def.ozone_height),
            shape: Vec4::new(
                radius as f32,
                (radius + def.height) as f32,
                (1.0 / def.ozone_width) as f32,
                0.0,
            ),
        }
    }
}

/// A body's clouds: its data and its map of them.
pub struct BodyClouds {
    pub def: CloudsDef,
    pub weather: Weather,
}

/// A body's air: its tables, and their GPU copies.
pub struct BodyAir {
    pub air: Air,
    pub uniform: AirUniform,
    pub transmittance: Handle<Image>,
    pub multiple: Handle<Image>,
    pub irradiance: Handle<Image>,
    pub clouds: Option<BodyClouds>,
    /// The clouds' weather map; a placeholder without clouds.
    pub weather: Handle<Image>,
    /// The clouds' map of what they look like.
    pub cloud_map: Handle<Image>,
}

impl BodyAir {
    fn new(
        air: Air,
        clouds: Option<BodyClouds>,
        cloud_map: Handle<Image>,
        images: &mut Assets<Image>,
    ) -> Self {
        Self {
            cloud_map,
            uniform: AirUniform::new(&air.def, air.radius),
            transmittance: images.add(table_image(air.transmittance_table(), TRANSMITTANCE_SIZE)),
            multiple: images.add(table_image(air.multiple_table(), SKY_TABLE_SIZE)),
            irradiance: images.add(table_image(air.irradiance_table(), SKY_TABLE_SIZE)),
            weather: clouds
                .as_ref()
                .map_or_else(Handle::default, |c| images.add(weather_image(&c.weather))),
            clouds,
            air,
        }
    }

    /// `Clouds` in `shaders/atmosphere.wesl`, with clouds if `on`.
    fn clouds_uniform(&self, on: bool) -> Vec4 {
        match &self.clouds {
            Some(clouds) if on => Vec4::new(
                (self.air.radius + clouds.def.shadow_height()) as f32,
                1.0,
                0.0,
                0.0,
            ),
            _ => Vec4::ZERO,
        }
    }
}

/// What a material or pass that imports `shaders/atmosphere.wesl` binds. Placeholders for a body
/// without air, whose shaders never sample them.
#[derive(Clone, Default)]
pub struct AirBindings {
    pub transmittance: Handle<Image>,
    pub multiple: Handle<Image>,
    pub irradiance: Handle<Image>,
    pub uniform: AirUniform,
    pub weather: Handle<Image>,
    pub cloud_map: Handle<Image>,
    /// `Clouds` in `shaders/atmosphere.wesl`.
    pub clouds: Vec4,
}

/// The tables of every body with air.
#[derive(Resource)]
pub struct BodyAirs {
    airs: HashMap<BodyId, BodyAir>,
    /// A clear cubemap for bodies without weather: the binding wants a cube whatever is in it.
    no_weather: Handle<Image>,
}

impl BodyAirs {
    pub fn get(&self, body: BodyId) -> Option<&BodyAir> {
        self.airs.get(&body)
    }

    pub fn bindings(&self, body: BodyId) -> AirBindings {
        self.get(body).map_or_else(
            || AirBindings {
                weather: self.no_weather.clone(),
                cloud_map: self.no_weather.clone(),
                ..default()
            },
            |air| AirBindings {
                transmittance: air.transmittance.clone(),
                multiple: air.multiple.clone(),
                irradiance: air.irradiance.clone(),
                uniform: air.uniform,
                weather: if air.clouds.is_some() {
                    air.weather.clone()
                } else {
                    self.no_weather.clone()
                },
                cloud_map: air.cloud_map.clone(),
                clouds: air.clouds_uniform(true),
            },
        )
    }
}

/// A table as a filterable texture for the shaders.
fn table_image(table: &[[f32; 3]], (width, height): (usize, usize)) -> Image {
    let data = table
        .iter()
        .flat_map(|d| [d[0], d[1], d[2], 0.0])
        .flat_map(|x| f16::from_f32(x).to_le_bytes())
        .collect();
    let mut image = Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// The weather map as a filterable cubemap, with a few mips for far views and soft shadows.
fn weather_image(weather: &Weather) -> Image {
    let n = WEATHER_SIZE;
    let mut data = Vec::with_capacity(6 * n * n * 4 * 4 / 3);
    for face in weather.data.chunks(n * n) {
        let (mut level, mut size) = (face.to_vec(), n);
        data.extend(level.iter().flatten());
        for _ in 1..WEATHER_MIP_LEVELS {
            let half = size / 2;
            let mut next = vec![[0u8; 4]; half * half];
            for y in 0..half {
                for x in 0..half {
                    let mut sum = [0u32; 4];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let texel = level[(2 * x + dx) + (2 * y + dy) * size];
                        for (s, v) in sum.iter_mut().zip(texel) {
                            *s += v as u32;
                        }
                    }
                    next[x + y * half] = sum.map(|s| (s / 4) as u8);
                }
            }
            data.extend(next.iter().flatten());
            level = next;
            size = half;
        }
    }
    let mut image = Image::default();
    image.texture_descriptor.size = Extent3d {
        width: n as u32,
        height: n as u32,
        depth_or_array_layers: 6,
    };
    image.texture_descriptor.format = TextureFormat::Rgba8Unorm;
    image.texture_descriptor.mip_level_count = WEATHER_MIP_LEVELS;
    image.data = Some(data);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

fn clear_cubemap() -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        &[0; 4],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image
}

/// `SkyDraw` in `shaders/sky.wesl`.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct SkyDrawUniform {
    /// xyz: the camera relative to the body's centre (render axes, m); w: ray-marching steps.
    eye: Vec4,
    /// xyz: direction to the sun from the body; w: its illuminance above the air.
    sun: Vec4,
}

/// One body's air to draw this frame.
#[derive(Clone)]
struct SkyDraw {
    air: AirBindings,
    draw: SkyDrawUniform,
    /// The body's fixed axes from render axes.
    local_from_render: Mat3,
    /// Other bodies' shadows, relative to this body's centre.
    eclipse: EclipseUniform,
    /// The direction to the Sun.
    sun: Vec4,
}

/// This frame's draws, farthest first.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
struct SkyDraws(Vec<SkyDraw>);

fn place_skies(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    settings: Res<GraphicsSettings>,
    airs: Res<BodyAirs>,
    camera: Single<&Transform, With<MainCamera>>,
    mut draws: ResMut<SkyDraws>,
) {
    draws.0.clear();
    if frame.origin_body.is_none() {
        return;
    }
    let system = &sim.universe.system;
    let steps = view_steps(settings.effects) as f32;
    let star = frame.body_offset(system.root());
    let mut sorted = Vec::new();
    for (&id, air) in &airs.airs {
        let centre = frame.to_render(id, DVec3::ZERO);
        let eye = camera.translation - centre;
        let distance = eye.length() as f64;
        if air.air.top() / distance.max(1.0) < SMALLEST_ON_SCREEN {
            continue;
        }
        let sun = crate::frame::to_render(star - frame.body_offset(id)).normalize_or(Vec3::Y);
        let mut bindings = airs.bindings(id);
        bindings.clouds = air.clouds_uniform(settings.clouds != Quality::Off);
        let turned = rotation_to_render(system.body(id).orientation(frame.time).inverse());
        sorted.push((
            distance,
            SkyDraw {
                air: bindings,
                draw: SkyDrawUniform {
                    eye: eye.extend(steps),
                    sun: sun.extend(lux::RAW_SUNLIGHT),
                },
                local_from_render: Mat3::from_quat(turned),
                eclipse: eclipse_for(system, &frame, &airs, id, DVec3::ZERO),
                sun: sun.extend(0.0),
            },
        ));
    }
    sorted.sort_by(|a, b| b.0.total_cmp(&a.0));
    draws.0.extend(sorted.into_iter().map(|(_, draw)| draw));
}

#[derive(Resource)]
pub(crate) struct SkyPipelines {
    layout: BindGroupLayoutDescriptor,
    layout_msaa: BindGroupLayoutDescriptor,
    pipeline: CachedRenderPipelineId,
    pipeline_msaa: CachedRenderPipelineId,
}

fn init_pipelines(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
    fullscreen: Res<FullscreenShader>,
    device: Res<RenderDevice>,
) {
    let layout = |label: &'static str, msaa: bool| {
        let lut = || texture_2d(TextureSampleType::Float { filterable: true });
        let depth = if msaa {
            texture_2d_multisampled(TextureSampleType::Depth)
        } else {
            texture_2d(TextureSampleType::Depth)
        };
        BindGroupLayoutDescriptor::new(
            label,
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::FRAGMENT,
                (
                    (0, uniform_buffer::<ViewUniform>(true)),
                    (1, uniform_buffer::<SkyDrawUniform>(false)),
                    (2, depth),
                    (110, lut()),
                    (111, sampler(SamplerBindingType::Filtering)),
                    (112, lut()),
                    (113, lut()),
                    (114, uniform_buffer::<AirUniform>(false)),
                    (
                        115,
                        texture_cube(TextureSampleType::Float { filterable: true }),
                    ),
                    (116, uniform_buffer::<Vec4>(false)),
                    (117, uniform_buffer::<Mat3>(false)),
                    (118, uniform_buffer::<EclipseUniform>(false)),
                    (119, uniform_buffer::<Vec4>(false)),
                ),
            ),
        )
    };
    let (plain, msaa) = (layout("sky", false), layout("sky_msaa", true));
    let dual_source = device
        .features()
        .contains(WgpuFeatures::DUAL_SOURCE_BLENDING);
    let queue = |layout: &BindGroupLayoutDescriptor, samples: u32| {
        // `atmosphere.wesl` binds its tables in the material group; here that is group 0.
        let mut shader_defs = vec![
            ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 0),
            "CLOUD_RAYS".into(),
        ];
        if samples > 1 {
            shader_defs.push("MULTISAMPLED".into());
        }
        if dual_source {
            shader_defs.push("DUAL_SOURCE_BLENDING".into());
        }
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("sky".into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: asset_server.load("embedded://osp/shaders/sky.wesl"),
                shader_defs,
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::Rgba16Float,
                    blend: Some(BlendState {
                        color: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: if dual_source {
                                BlendFactor::Src1
                            } else {
                                BlendFactor::SrcAlpha
                            },
                            operation: BlendOperation::Add,
                        },
                        alpha: BlendComponent {
                            src_factor: BlendFactor::Zero,
                            dst_factor: BlendFactor::One,
                            operation: BlendOperation::Add,
                        },
                    }),
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            multisample: MultisampleState {
                count: samples,
                ..default()
            },
            ..default()
        })
    };
    let (pipeline, pipeline_msaa) = (queue(&plain, 1), queue(&msaa, Msaa::Sample4.samples()));
    commands.insert_resource(SkyPipelines {
        layout: plain,
        layout_msaa: msaa,
        pipeline,
        pipeline_msaa,
    });
}

/// The buffers and bind groups of this frame's draws; the buffers are reused.
#[derive(Resource, Default)]
pub(crate) struct SkyBuffers {
    buffers: Vec<SkyDrawBuffers>,
    groups: Vec<BindGroup>,
}

#[derive(Default)]
struct SkyDrawBuffers {
    air: UniformBuffer<AirUniform>,
    params: UniformBuffer<SkyDrawUniform>,
    clouds: UniformBuffer<Vec4>,
    local_from_render: UniformBuffer<Mat3>,
    eclipse: UniformBuffer<EclipseUniform>,
    sun: UniformBuffer<Vec4>,
}

fn prepare_sky(
    draws: Res<SkyDraws>,
    pipelines: Res<SkyPipelines>,
    pipeline_cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    view_uniforms: Res<ViewUniforms>,
    views: Query<(&ViewDepthStencilTexture, &Msaa), With<MainCamera>>,
    mut sky: ResMut<SkyBuffers>,
) {
    sky.groups.clear();
    let Some(view) = view_uniforms.uniforms.binding() else {
        return;
    };
    let Ok((depth, msaa)) = views.single() else {
        return;
    };
    let Some(depth) = depth.attachment.depth_stencil_views().depth_only_view() else {
        return;
    };
    let layout = if msaa.samples() > 1 {
        &pipelines.layout_msaa
    } else {
        &pipelines.layout
    };
    let layout = pipeline_cache.get_bind_group_layout(layout);
    let sky = &mut *sky;
    sky.buffers.resize_with(draws.0.len(), Default::default);
    for (draw, buffers) in draws.0.iter().zip(&mut sky.buffers) {
        buffers.air.set(draw.air.uniform);
        buffers.air.write_buffer(&device, &queue);
        buffers.params.set(draw.draw);
        buffers.params.write_buffer(&device, &queue);
        buffers.clouds.set(draw.air.clouds);
        buffers.clouds.write_buffer(&device, &queue);
        buffers.local_from_render.set(draw.local_from_render);
        buffers.local_from_render.write_buffer(&device, &queue);
        buffers.eclipse.set(draw.eclipse);
        buffers.eclipse.write_buffer(&device, &queue);
        buffers.sun.set(draw.sun);
        buffers.sun.write_buffer(&device, &queue);
        let image = |handle: &Handle<Image>| images.get(handle).unwrap_or(&fallback.d2);
        let weather = images.get(&draw.air.weather).unwrap_or(&fallback.cube);
        let transmittance = image(&draw.air.transmittance);
        sky.groups.push(device.create_bind_group(
            "sky",
            &layout,
            &BindGroupEntries::with_indices((
                (0, view.clone()),
                (1, &buffers.params),
                (2, depth),
                (110, &transmittance.texture_view),
                (111, &transmittance.sampler),
                (112, &image(&draw.air.multiple).texture_view),
                (113, &image(&draw.air.irradiance).texture_view),
                (114, &buffers.air),
                (115, &weather.texture_view),
                (116, &buffers.clouds),
                (117, &buffers.local_from_render),
                (118, &buffers.eclipse),
                (119, &buffers.sun),
            )),
        ));
    }
}

pub(crate) fn render_sky(
    view: ViewQuery<(&ViewTarget, &ViewUniformOffset, &Msaa), With<MainCamera>>,
    sky: Res<SkyBuffers>,
    pipelines: Res<SkyPipelines>,
    pipeline_cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    if sky.groups.is_empty() {
        return;
    }
    let (target, view_offset, msaa) = view.into_inner();
    let pipeline = if msaa.samples() > 1 {
        pipelines.pipeline_msaa
    } else {
        pipelines.pipeline
    };
    let Some(pipeline) = pipeline_cache.get_render_pipeline(pipeline) else {
        return;
    };
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("sky"),
            color_attachments: &[Some(target.get_color_attachment())],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(pipeline);
    for group in &sky.groups {
        pass.set_bind_group(0, group, &[view_offset.offset]);
        pass.draw(0..3, 0..1);
    }
}

/// Dim the sun for the air above the viewed vessel (in flight view), and adapt the exposure
/// to how much light there is. The sky's own light comes from `sky_light`. The light's illuminance
/// stays `RAW_SUNLIGHT` and its colour is the transmittance, so the shaders can divide it
/// back out (`light_transmittance` in `shaders/atmosphere.wesl`).
fn dim_sunlight(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    time: Res<Time>,
    mut light: Single<&mut DirectionalLight, With<Sunlight>>,
    mut exposure: Single<&mut Exposure, With<MainCamera>>,
    rig: Res<CameraRig>,
    settings: Res<GraphicsSettings>,
    airs: Res<BodyAirs>,
) {
    let Some(vessel_body) = frame.vessel_body else {
        return;
    };
    let system = &sim.universe.system;
    let vessel = frame.vessel_offset;
    let to_sun =
        (frame.body_offset(system.root()) - frame.relative(vessel_body, vessel)).normalize();
    // The map camera is above all air: what it sees is lit by the full sun, not the light
    // that reaches the vessel (in Earth's shadow, that left the Moon and Mars black).
    let air = airs.get(vessel_body).filter(|_| !rig.map_view);
    let cloud_shadow = |air: &BodyAir| {
        let Some(clouds) = air
            .clouds
            .as_ref()
            .filter(|_| settings.clouds != Quality::Off)
        else {
            return 1.0;
        };
        // The map's axes: the body's fixed axes as the renderer turns them, Y to the pole.
        let to_fixed = system.body(vessel_body).orientation(frame.time).inverse();
        let turn = |v: DVec3| {
            let v = to_fixed * v;
            DVec3::new(v.x, v.z, -v.y)
        };
        let radius = air.air.radius;
        clouds
            .weather
            .sunlight(&clouds.def, radius, turn(vessel), turn(to_sun))
    };
    let shade = air.map_or(1.0, cloud_shadow);
    let transmittance = air.map_or([1.0; 3], |air| {
        air.air
            .sun_transmittance(vessel, to_sun)
            .map(|t| (t * shade).max(MIN_TRANSMITTANCE))
    });
    let [r, g, b] = transmittance.map(|t| t as f32);
    light.color = Color::linear_rgb(r, g, b);
    light.illuminance = lux::RAW_SUNLIGHT;
    let up = vessel.normalize_or(DVec3::Z);
    let daylight = smoothstep(-0.12, 0.1, up.dot(to_sun)) as f32;

    // Expose for the ground below: the sunlit disc in the map view, otherwise day or night
    // where the vessel is (in orbit over the night side, that shows the city lights).
    let scene = if rig.map_view { 1.0 } else { daylight };
    // Under cloud there is less light, and the eye opens up to it.
    let overcast = OVERCAST_EV * (1.0 - shade as f32);
    // In another body's shadow the eye opens up, but only makes up for half the stops: an eclipse
    // should look dark.
    let eclipsed =
        crate::eclipse::sunlight_left(system, &frame, vessel_body, frame.vessel_offset) as f32;
    let day_ev = DAY_EV + 0.5 * eclipsed.max(MIN_ECLIPSE_LIGHT).log2();
    let target = NIGHT_EV + (day_ev - NIGHT_EV) * scene.sqrt() - overcast;
    let blend = 1.0 - (-time.delta_secs() / ADAPTATION).exp();
    exposure.ev100 += (target - exposure.ev100) * blend;
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use osp_sim::scattering::sphere_hits;

    use super::*;

    fn shipped_bodies() -> Vec<osp_sim::data::BodyDef> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/core");
        let content = osp_mods::load(&root, None);
        content.registry.all("bodies", &mut Vec::new())
    }

    #[test]
    fn every_shipped_body_with_air_has_a_sky_and_it_looks_like_one() {
        let bodies = shipped_bodies();
        assert!(!bodies.is_empty());
        for body in &bodies {
            // Air that is not drawn would look like a bug in the game.
            let Some(def) = &body.scattering else {
                assert!(body.atmosphere.is_none(), "{} has no scattering", body.id);
                continue;
            };
            let air = Air::new(def, body.radius);
            let uniform = AirUniform::new(def, body.radius);
            assert!(uniform.shape.y > uniform.shape.x, "{}", body.id);
            let eye = DVec3::new(0.0, 0.0, body.radius + 2.0);
            let sun = DVec3::new(0.6, 0.0, 0.8);
            for look in [DVec3::Z, DVec3::new(1.0, 0.0, 0.05).normalize()] {
                let exit = sphere_hits(eye, look, air.top()).unwrap().1;
                let ray = air.scatter(
                    eye,
                    look,
                    0.0,
                    exit,
                    sun,
                    view_steps(Quality::Ultra) as usize,
                );
                for c in 0..3 {
                    let (light, through) = (ray.inscatter[c], ray.transmittance[c]);
                    assert!(light > 1e-4 && light < 1.0, "{}: {light}", body.id);
                    assert!((0.0..=1.0).contains(&through), "{}: {through}", body.id);
                }
            }
        }
    }

    #[test]
    fn the_uniform_gives_the_shader_what_the_tables_were_built_from() {
        let bodies = shipped_bodies();
        let earth = bodies.iter().find(|b| b.id == "Earth").unwrap();
        let def = earth.scattering.as_ref().unwrap();
        let uniform = AirUniform::new(def, earth.radius);
        assert_eq!(uniform.shape.x, earth.radius as f32);
        assert_eq!(uniform.shape.y, (earth.radius + def.height) as f32);
        assert_eq!(uniform.rayleigh.w, (1.0 / def.rayleigh_height) as f32);
        // Extinction is scattering plus absorption.
        let extinction = def.mie[1] + def.mie_absorption[1];
        assert_eq!(uniform.mie_extinction.y, extinction as f32);
    }
}
