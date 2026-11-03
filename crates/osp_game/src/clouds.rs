//! Clouds, drawn the way EnvironmentalVisualEnhancements draws them (`assets/core/clouds/LICENSE.txt`):
//! a cloud map on a sheet at one height over the planet, a tiling detail texture on it, and close to
//! the camera a grid of soft puffs that you fly through (`crate::cloud_puffs`).
//!
//! The sheet takes two passes after the sky: drawn at half resolution (`shaders/clouds.wesl`), then
//! laid over the scene by an upsample that respects the depth buffer, so a vessel's edge stays sharp
//! against a cloud behind it (`shaders/clouds_composite.wesl`).
//!
//! The map, and the shadows it casts on the ground and in the air, belong to `atmosphere` and
//! `osp_sim::clouds`.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    core_pipeline::{FullscreenShader, core_3d::main_transparent_pass_3d, schedule::Core3d},
    image::{
        CompressedImageFormats, ImageAddressMode, ImageFilterMode, ImageSampler,
        ImageSamplerDescriptor, ImageType,
    },
    light::light_consts::lux,
    math::DVec3,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
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
            ExtractedView, Msaa, ViewDepthStencilTexture, ViewTarget, ViewUniform,
            ViewUniformOffset, ViewUniforms,
        },
    },
    shader::{ShaderDefVal, load_shader_library},
};
use osp_sim::{body::BodyId, time::SimTime};

use crate::{
    FrameSet,
    atmosphere::{AirBindings, AirUniform, BodyAirs, render_sky},
    camera::{CameraRig, MainCamera},
    content::GameContent,
    eclipse::{EclipseUniform, eclipse_for},
    frame::{RenderFrame, rotation_to_render, to_render},
    settings::{GraphicsSettings, Quality},
    sim::Sim,
};

/// How often the detail tiles, m: its big copy and its fine one (`DETAIL_TILE` and `FINE_TILE` in the
/// shader).
const DETAIL_TILES: [f64; 2] = [40_000.0, 5_300.0];
/// Which way the wind moves the detail, in the body's fixed axes.
const WIND_DIRECTION: DVec3 = DVec3::new(0.86, 0.0, 0.5);
/// How fast the UV noise moves, in its periods per second: its `_UVNoiseAnimation`, by the
/// `_UniversalTime.x` of a 20th of a second.
const WOBBLE_RATE: [f64; 2] = [0.002 / 20.0, 0.001 / 20.0];

/// A picture from a mod's files, as a texture with its mip chain, for anything drawn near and far.
pub fn picture(
    content: &osp_mods::Content,
    path: &str,
    address: ImageAddressMode,
) -> Option<Image> {
    let bytes = content
        .resolve_file(path)
        .and_then(|file| std::fs::read(file).ok())?;
    let top = Image::from_buffer(
        &bytes,
        ImageType::Extension("png"),
        CompressedImageFormats::NONE,
        false,
        ImageSampler::Default,
        RenderAssetUsages::RENDER_WORLD,
    )
    .ok()?;
    let (mut size, mut level) = (top.width() as usize, top.data?);
    let mut data = level.clone();
    let mut levels = 1;
    while size > 1 {
        let half = size / 2;
        let mut next = vec![0u8; half * half * 4];
        for y in 0..half {
            for x in 0..half {
                for c in 0..4 {
                    let at = |dx: usize, dy: usize| {
                        level[((2 * y + dy) * size + 2 * x + dx) * 4 + c] as u32
                    };
                    let sum = at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1);
                    next[(y * half + x) * 4 + c] = (sum / 4) as u8;
                }
            }
        }
        data.extend_from_slice(&next);
        level = next;
        size = half;
        levels += 1;
    }
    let mut image = Image::default();
    image.texture_descriptor.size = Extent3d {
        width: top.texture_descriptor.size.width,
        height: top.texture_descriptor.size.height,
        depth_or_array_layers: 1,
    };
    image.texture_descriptor.format = TextureFormat::Rgba8Unorm;
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: address,
        address_mode_v: address,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    Some(image)
}

pub struct CloudsPlugin;

impl Plugin for CloudsPlugin {
    fn build(&self, app: &mut App) {
        load_shader_library!(app, "shaders/clouds_types.wesl");
        embedded_asset!(app, "shaders/clouds.wesl");
        embedded_asset!(app, "shaders/clouds_composite.wesl");
        let (detail, wobble) = {
            let content = &app.world().resource::<GameContent>().0;
            let read = |path| {
                let image = picture(content, path, ImageAddressMode::Repeat);
                if image.is_none() {
                    warn!("no cloud texture `{path}` in any mod");
                }
                image.unwrap_or_default()
            };
            (read("clouds/detail.png"), read("clouds/uvnoise.png"))
        };
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let (detail, wobble) = (images.add(detail), images.add(wobble));
        app.insert_resource(CloudFrame {
            detail,
            wobble,
            draw: None,
        })
        .add_plugins(ExtractResourcePlugin::<CloudFrame>::default())
        .add_systems(
            Update,
            place_clouds
                .after(crate::frame::update_sunlight)
                .in_set(FrameSet::Render),
        );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<CloudTargets>()
            .init_resource::<CloudResult>()
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(
                Render,
                prepare_clouds.in_set(RenderSystems::PrepareBindGroups),
            )
            .add_systems(
                Core3d,
                render_clouds
                    .after(render_sky)
                    .before(main_transparent_pass_3d),
            );
    }
}

/// `CloudDraw` in `shaders/clouds_types.wesl`.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct CloudDraw {
    eye: Vec4,
    sun: Vec4,
    local_from_render: Mat3,
    map_from_local: Mat3,
    copy_from_map: Mat3,
    shell: Vec4,
    albedo: Vec4,
    detail_wind: Vec4,
    wobble: Vec4,
}

/// The tiling textures, and what to draw this frame.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
struct CloudFrame {
    detail: Handle<Image>,
    wobble: Handle<Image>,
    draw: Option<(AirBindings, CloudDraw, EclipseUniform, Mat3)>,
}

/// The nearest body with clouds: any other is too far away for them to show.
pub fn nearest_cloudy_body(
    sim: &Sim,
    frame: &RenderFrame,
    rig: &CameraRig,
    airs: &BodyAirs,
) -> Option<BodyId> {
    let system = &sim.universe.system;
    system
        .bodies()
        .iter()
        .filter(|body| airs.get(body.id).is_some_and(|air| air.clouds.is_some()))
        .map(|body| body.id)
        .min_by(|&a, &b| {
            let distance = |id: BodyId| (rig.position - frame.body_offset(id)).length();
            distance(a).total_cmp(&distance(b))
        })
}

fn place_clouds(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    settings: Res<GraphicsSettings>,
    airs: Res<BodyAirs>,
    mut clouds: ResMut<CloudFrame>,
) {
    clouds.draw = None;
    let (Some(_), true) = (frame.origin_body, settings.clouds != Quality::Off) else {
        return;
    };
    let Some(id) = nearest_cloudy_body(&sim, &frame, &rig, &airs) else {
        return;
    };
    let Some(def) = airs
        .get(id)
        .and_then(|air| air.clouds.as_ref())
        .map(|c| &c.def)
    else {
        return;
    };
    let system = &sim.universe.system;
    let body = system.body(id);
    let eye = rig.position - frame.body_offset(id);
    let turned = body.orientation(frame.time).inverse();
    let local = Mat3::from_quat(rotation_to_render(turned));
    let seconds = frame.time.seconds_since(SimTime::default());
    // How far the wind has carried each copy of the detail, wrapped to stay accurate in f32.
    let carried = WIND_DIRECTION * def.wind * seconds;
    let wind = |tile: f64| {
        Vec2::new(
            (carried.x / tile).rem_euclid(1.0) as f32,
            (carried.z / tile).rem_euclid(1.0) as f32,
        )
    };
    let (big, fine) = (wind(DETAIL_TILES[0]), wind(DETAIL_TILES[1]));
    let phase = |rate: f64| (seconds * rate).rem_euclid(1.0) as f32;
    let star = frame.body_offset(system.root());
    let sun = to_render(star - frame.body_offset(id)).normalize_or(Vec3::Y);
    let radius = body.radius;
    let high = if def.cirrus > 0.0 {
        (radius + def.cirrus_height) as f32
    } else {
        0.0
    };
    let own_map = def.map.is_some();
    let (map_from_local, copy_from_map) = crate::atmosphere::map_turns(def);
    let [r, g, b] = def.albedo.map(|c| c as f32);
    let draw = CloudDraw {
        eye: to_render(eye).extend(0.0),
        sun: sun.extend(lux::RAW_SUNLIGHT),
        local_from_render: local,
        map_from_local,
        copy_from_map,
        shell: Vec4::new(
            (radius + def.shadow_height()) as f32,
            high,
            osp_sim::clouds::map_texel(radius),
            if own_map { 1.0 } else { 0.0 },
        ),
        albedo: Vec4::new(r, g, b, def.cirrus as f32),
        detail_wind: Vec4::new(big.x, big.y, fine.x, fine.y),
        wobble: Vec4::new(
            phase(WOBBLE_RATE[0]),
            phase(WOBBLE_RATE[1]),
            phase(WOBBLE_RATE[1]),
            phase(WOBBLE_RATE[0] * 0.6),
        ),
    };
    let eclipse = eclipse_for(system, &frame, &airs, id, DVec3::ZERO);
    clouds.draw = Some((airs.bindings(id), draw, eclipse, local));
}

#[derive(Resource)]
struct CloudPipelines {
    sheet_layout: BindGroupLayoutDescriptor,
    sheet_layout_msaa: BindGroupLayoutDescriptor,
    composite_layout: BindGroupLayoutDescriptor,
    composite_layout_msaa: BindGroupLayoutDescriptor,
    sheet: CachedRenderPipelineId,
    sheet_msaa: CachedRenderPipelineId,
    composite: CachedRenderPipelineId,
    composite_msaa: CachedRenderPipelineId,
    tile_sampler: Sampler,
}

fn init_pipelines(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
    fullscreen: Res<FullscreenShader>,
    device: Res<RenderDevice>,
) {
    let filterable = TextureSampleType::Float { filterable: true };
    let depth_binding = |msaa: bool| {
        if msaa {
            texture_2d_multisampled(TextureSampleType::Depth)
        } else {
            texture_2d(TextureSampleType::Depth)
        }
    };
    let sheet_layout = |label: &'static str, msaa: bool| {
        BindGroupLayoutDescriptor::new(
            label,
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::FRAGMENT,
                (
                    (0, uniform_buffer::<ViewUniform>(true)),
                    (1, uniform_buffer::<CloudDraw>(false)),
                    (2, depth_binding(msaa)),
                    (3, texture_cube(filterable)),
                    (4, texture_2d(filterable)),
                    (5, sampler(SamplerBindingType::Filtering)),
                    (6, texture_2d(filterable)),
                    (110, texture_2d(filterable)),
                    (111, sampler(SamplerBindingType::Filtering)),
                    (112, texture_2d(filterable)),
                    (113, texture_2d(filterable)),
                    (114, uniform_buffer::<AirUniform>(false)),
                    (115, texture_cube(filterable)),
                    (116, uniform_buffer::<Vec4>(false)),
                    (117, uniform_buffer::<Mat3>(false)),
                    (118, uniform_buffer::<EclipseUniform>(false)),
                    (119, uniform_buffer::<Vec4>(false)),
                ),
            ),
        )
    };
    let composite_layout = |label: &'static str, msaa: bool| {
        BindGroupLayoutDescriptor::new(
            label,
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::FRAGMENT,
                (
                    (
                        0,
                        texture_2d(TextureSampleType::Float { filterable: false }),
                    ),
                    (1, depth_binding(msaa)),
                ),
            ),
        )
    };
    let (sheet_plain, sheet_msaa) = (
        sheet_layout("clouds_sheet", false),
        sheet_layout("clouds_sheet_msaa", true),
    );
    let (composite_plain, composite_msaa) = (
        composite_layout("clouds_composite", false),
        composite_layout("clouds_composite_msaa", true),
    );
    let defs = |msaa: bool| {
        let mut defs = vec![
            ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 0),
            "CLOUD_RAYS".into(),
        ];
        if msaa {
            defs.push("MULTISAMPLED".into());
        }
        defs
    };
    let sheet_target = |layout: &BindGroupLayoutDescriptor, msaa: bool| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("clouds_sheet".into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: asset_server.load("embedded://osp/shaders/clouds.wesl"),
                shader_defs: defs(msaa),
                targets: vec![Some(TextureFormat::Rgba16Float.into())],
                ..default()
            }),
            ..default()
        })
    };
    let sheet = sheet_target(&sheet_plain, false);
    let sheet_multisampled = sheet_target(&sheet_msaa, true);
    let composite_pipeline = |layout: &BindGroupLayoutDescriptor, samples: u32| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("clouds_composite".into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: asset_server.load("embedded://osp/shaders/clouds_composite.wesl"),
                shader_defs: defs(samples > 1),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::Rgba16Float,
                    blend: Some(BlendState {
                        color: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::SrcAlpha,
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
    let composite = composite_pipeline(&composite_plain, 1);
    let composite_multisampled = composite_pipeline(&composite_msaa, Msaa::Sample4.samples());
    let tile_sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("clouds_tile"),
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        address_mode_w: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        ..default()
    });
    commands.insert_resource(CloudPipelines {
        sheet_layout: sheet_plain,
        sheet_layout_msaa: sheet_msaa,
        composite_layout: composite_plain,
        composite_layout_msaa: composite_msaa,
        sheet,
        sheet_msaa: sheet_multisampled,
        composite,
        composite_msaa: composite_multisampled,
        tile_sampler,
    });
}

/// This frame's clouds, for anything else that needs to see through them (alpha is how much shows).
#[derive(Resource, Default)]
pub struct CloudResult(pub Option<TextureView>);

/// The half-resolution target, and this frame's bind groups.
#[derive(Resource, Default)]
struct CloudTargets {
    size: UVec2,
    light: Option<TextureView>,
    air: UniformBuffer<AirUniform>,
    clouds: UniformBuffer<Vec4>,
    draw: UniformBuffer<CloudDraw>,
    eclipse: UniformBuffer<EclipseUniform>,
    local: UniformBuffer<Mat3>,
    sun: UniformBuffer<Vec4>,
    groups: Option<CloudGroups>,
}

struct CloudGroups {
    sheet: BindGroup,
    composite: BindGroup,
}

fn target(device: &RenderDevice, size: UVec2, format: TextureFormat) -> TextureView {
    device
        .create_texture(&TextureDescriptor {
            label: Some("clouds"),
            size: Extent3d {
                width: size.x,
                height: size.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&TextureViewDescriptor::default())
}

fn prepare_clouds(
    frame: Res<CloudFrame>,
    pipelines: Res<CloudPipelines>,
    pipeline_cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    fallback: Res<FallbackImage>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    view_uniforms: Res<ViewUniforms>,
    views: Query<(&ExtractedView, &ViewDepthStencilTexture, &Msaa), With<MainCamera>>,
    mut targets: ResMut<CloudTargets>,
    mut result: ResMut<CloudResult>,
) {
    targets.groups = None;
    result.0 = None;
    let Some((air, draw, eclipse, local)) = &frame.draw else {
        return;
    };
    let Some(view) = view_uniforms.uniforms.binding() else {
        return;
    };
    let Ok((extracted, depth, msaa)) = views.single() else {
        return;
    };
    let Some(depth) = depth.attachment.depth_stencil_views().depth_only_view() else {
        return;
    };
    let (Some(detail), Some(wobble), Some(map)) = (
        images.get(&frame.detail),
        images.get(&frame.wobble),
        images.get(&air.cloud_map),
    ) else {
        return;
    };
    let full = extracted.viewport.zw();
    let size = (full + 1) / 2;
    if size.min_element() == 0 {
        return;
    }
    let targets = &mut *targets;
    if targets.size != size {
        targets.size = size;
        targets.light = Some(target(&device, size, TextureFormat::Rgba16Float));
    }
    let Some(light) = &targets.light else {
        return;
    };
    targets.air.set(air.uniform);
    targets.air.write_buffer(&device, &queue);
    targets.clouds.set(air.clouds);
    targets.clouds.write_buffer(&device, &queue);
    targets.draw.set(*draw);
    targets.draw.write_buffer(&device, &queue);
    targets.eclipse.set(*eclipse);
    targets.eclipse.write_buffer(&device, &queue);
    targets.local.set(*local);
    targets.local.write_buffer(&device, &queue);
    targets.sun.set(draw.sun);
    targets.sun.write_buffer(&device, &queue);

    let image = |handle: &Handle<Image>| images.get(handle).unwrap_or(&fallback.d2);
    let weather = images.get(&air.weather).unwrap_or(&fallback.cube);
    let transmittance = image(&air.transmittance);
    let multi = msaa.samples() > 1;
    let sheet = device.create_bind_group(
        "clouds_sheet",
        &pipeline_cache.get_bind_group_layout(if multi {
            &pipelines.sheet_layout_msaa
        } else {
            &pipelines.sheet_layout
        }),
        &BindGroupEntries::with_indices((
            (0, view),
            (1, &targets.draw),
            (2, depth),
            (3, &map.texture_view),
            (4, &detail.texture_view),
            (5, &pipelines.tile_sampler),
            (6, &wobble.texture_view),
            (110, &transmittance.texture_view),
            (111, &transmittance.sampler),
            (112, &image(&air.multiple).texture_view),
            (113, &image(&air.irradiance).texture_view),
            (114, &targets.air),
            (115, &weather.texture_view),
            (116, &targets.clouds),
            (117, &targets.local),
            (118, &targets.eclipse),
            (119, &targets.sun),
        )),
    );
    let composite = device.create_bind_group(
        "clouds_composite",
        &pipeline_cache.get_bind_group_layout(if multi {
            &pipelines.composite_layout_msaa
        } else {
            &pipelines.composite_layout
        }),
        &BindGroupEntries::with_indices(((0, light), (1, depth))),
    );
    result.0 = Some(light.clone());
    targets.groups = Some(CloudGroups { sheet, composite });
}

fn render_clouds(
    view: ViewQuery<(&ViewTarget, &ViewUniformOffset, &Msaa), With<MainCamera>>,
    targets: Res<CloudTargets>,
    pipelines: Res<CloudPipelines>,
    pipeline_cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let Some(groups) = &targets.groups else {
        return;
    };
    let Some(light) = &targets.light else {
        return;
    };
    let (view_target, view_offset, msaa) = view.into_inner();
    let multi = msaa.samples() > 1;
    let (Some(sheet), Some(composite)) = (
        pipeline_cache.get_render_pipeline(if multi {
            pipelines.sheet_msaa
        } else {
            pipelines.sheet
        }),
        pipeline_cache.get_render_pipeline(if multi {
            pipelines.composite_msaa
        } else {
            pipelines.composite
        }),
    ) else {
        return;
    };
    let encoder = ctx.command_encoder();
    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("clouds_sheet"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: light,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(LinearRgba::BLACK.into()),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(sheet);
        pass.set_bind_group(0, &groups.sheet, &[view_offset.offset]);
        pass.draw(0..3, 0..1);
    }
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("clouds_composite"),
        color_attachments: &[Some(view_target.get_color_attachment())],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(composite);
    pass.set_bind_group(0, &groups.composite, &[]);
    pass.draw(0..3, 0..1);
}
