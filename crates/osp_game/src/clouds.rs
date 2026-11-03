//! Clouds: a ray march through a shell around the nearest body that has any, at half resolution
//! (`shaders/clouds.wesl`). A frame takes three passes after the sky:
//!
//! 1. The march, into half-resolution targets: light to add, how much of the scene shows through,
//!    and how far away the light came from.
//! 2. Resolve (`shaders/clouds_resolve.wesl`): the march is dithered, so each pixel is averaged
//!    with where its cloud was last frame, found from that distance and how the camera moved. Two
//!    history textures take turns.
//! 3. Composite (`shaders/clouds_composite.wesl`): an upsample that respects the depth buffer, so
//!    a vessel's edge stays sharp against a cloud behind it, as a plain blend over the scene.
//!
//! The weather itself, and the shadows it casts on the ground and in the air, belong to
//! `atmosphere` and `osp_sim::clouds`.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    core_pipeline::{FullscreenShader, core_3d::main_transparent_pass_3d, schedule::Core3d},
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    light::light_consts::lux,
    math::DVec3,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        render_resource::{
            binding_types::{
                sampler, texture_2d, texture_2d_multisampled, texture_3d, texture_cube,
                uniform_buffer,
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
use osp_sim::{
    body::BodyId,
    clouds::{
        BASE_NOISE_SIZE, DETAIL_NOISE_SIZE, base_noise, detail_noise, tile_offset, with_mips,
    },
    time::SimTime,
};

use crate::{
    FrameSet,
    atmosphere::{AirBindings, AirUniform, BodyAirs, render_sky},
    camera::{CameraRig, MainCamera},
    eclipse::{EclipseUniform, eclipse_for},
    frame::{RenderFrame, rotation_to_render, to_render},
    settings::{GraphicsSettings, Quality},
    sim::Sim,
};

/// How far the fine noise is worn into the clouds, m: past it, a pixel is larger than the detail.
const DETAIL_DISTANCE: f32 = 70_000.0;
/// A camera jump larger than this is a cut, m.
const CUT_DISTANCE: f64 = 50_000.0;
/// Which way the wind moves the fine structure, in the body's fixed axes.
const WIND_DIRECTION: DVec3 = DVec3::new(0.86, 0.0, 0.5);

/// Most steps along a ray, a step's length as a share of the distance, steps towards the sun, and
/// whether there is fine detail. Past where the steps run out, a flat sheet takes over, so the
/// budget and the step set how far clouds keep their depth: from about 3 times the distance they
/// begin at (Low) to 7 times (Ultra).
fn march_settings(quality: Quality) -> (u32, f32, u32, bool) {
    match quality {
        Quality::Off | Quality::Low => (64, 0.018, 3, false),
        Quality::Medium => (96, 0.015, 4, true),
        Quality::High => (128, 0.012, 5, true),
        Quality::Ultra => (192, 0.01, 6, true),
    }
}

pub struct CloudsPlugin;

impl Plugin for CloudsPlugin {
    fn build(&self, app: &mut App) {
        load_shader_library!(app, "shaders/clouds_types.wesl");
        embedded_asset!(app, "shaders/clouds.wesl");
        embedded_asset!(app, "shaders/clouds_resolve.wesl");
        embedded_asset!(app, "shaders/clouds_composite.wesl");
        let (base, detail) = {
            let (base, detail) = std::thread::scope(|scope| {
                let base = scope.spawn(|| noise_image(&base_noise(), BASE_NOISE_SIZE));
                let detail = scope.spawn(|| noise_image(&detail_noise(), DETAIL_NOISE_SIZE));
                (
                    base.join().expect("building the cloud noise"),
                    detail.join().expect("building the cloud noise"),
                )
            });
            let mut images = app.world_mut().resource_mut::<Assets<Image>>();
            (images.add(base), images.add(detail))
        };
        app.insert_resource(CloudFrame {
            base,
            detail,
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

/// A cube of noise with its mips as a repeating 3D texture.
fn noise_image(top: &[[u8; 4]], size: usize) -> Image {
    let data: Vec<u8> = with_mips(top, size).into_iter().flatten().collect();
    let mut image = Image::default();
    image.texture_descriptor.size = Extent3d {
        width: size as u32,
        height: size as u32,
        depth_or_array_layers: size as u32,
    };
    image.texture_descriptor.dimension = TextureDimension::D3;
    image.texture_descriptor.format = TextureFormat::Rgba8Unorm;
    image.texture_descriptor.mip_level_count = size.ilog2() + 1;
    image.data = Some(data);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// `CloudDraw` in `shaders/clouds_types.wesl`.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct CloudDraw {
    eye: Vec4,
    sun: Vec4,
    local_from_render: Mat3,
    tile: Vec4,
    shell: Vec4,
    albedo: Vec4,
    prev_view_from_world: Mat3,
    prev_clip_from_view: Mat4,
    moved: Vec4,
    frame: Vec4,
}

/// The noise textures, and what to draw this frame.
#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
struct CloudFrame {
    base: Handle<Image>,
    detail: Handle<Image>,
    draw: Option<(AirBindings, CloudDraw, EclipseUniform, Mat3)>,
}

/// The camera last frame, for finding what it drew.
struct LastView {
    body: BodyId,
    eye: DVec3,
    view_from_world: Mat3,
    clip_from_view: Mat4,
}

fn place_clouds(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    settings: Res<GraphicsSettings>,
    airs: Res<BodyAirs>,
    camera: Single<(&Camera, &Transform), With<MainCamera>>,
    mut clouds: ResMut<CloudFrame>,
    mut last: Local<Option<LastView>>,
    mut count: Local<u32>,
) {
    clouds.draw = None;
    let (Some(_), true) = (frame.origin_body, settings.clouds != Quality::Off) else {
        *last = None;
        return;
    };
    let system = &sim.universe.system;
    let (camera, transform) = camera.into_inner();
    // The nearest body with clouds; any other is too far away for them to show.
    let Some((id, def)) = system
        .bodies()
        .iter()
        .filter_map(|body| Some((body.id, &airs.get(body.id)?.clouds.as_ref()?.def)))
        .min_by(|a, b| {
            let distance = |id: BodyId| (rig.position - frame.body_offset(id)).length();
            distance(a.0).total_cmp(&distance(b.0))
        })
    else {
        *last = None;
        return;
    };
    let body = system.body(id);
    let eye = rig.position - frame.body_offset(id);
    let view_from_world = Mat3::from_quat(transform.rotation.inverse());
    let clip_from_view = camera.clip_from_view();
    let (moved, was_cut) = match &*last {
        Some(last) if last.body == id => {
            let moved = eye - last.eye;
            (moved, moved.length() > CUT_DISTANCE)
        }
        _ => (DVec3::ZERO, true),
    };
    let (prev_view_from_world, prev_clip_from_view) = last
        .as_ref()
        .map_or((view_from_world, clip_from_view), |last| {
            (last.view_from_world, last.clip_from_view)
        });
    *last = Some(LastView {
        body: id,
        eye,
        view_from_world,
        clip_from_view,
    });
    *count = count.wrapping_add(1);

    let (steps, step_share, light_steps, detail) = march_settings(settings.clouds);
    let turned = body.orientation(frame.time).inverse();
    // How far the wind has carried the noise.
    let seconds = frame.time.seconds_since(SimTime::default());
    let tile = tile_offset(WIND_DIRECTION * def.wind * seconds);
    let star = frame.body_offset(system.root());
    let sun = to_render(star - frame.body_offset(id)).normalize_or(Vec3::Y);
    let radius = body.radius;
    let cirrus = if def.cirrus > 0.0 {
        (radius + def.cirrus_height) as f32
    } else {
        0.0
    };
    let bounce = body
        .scattering
        .as_ref()
        .map_or(0.0, |s| (s.ground_albedo.iter().sum::<f64>() / 3.0) as f32);
    let [r, g, b] = def.albedo.map(|c| c as f32);
    let draw = CloudDraw {
        eye: to_render(eye).extend(steps as f32),
        sun: sun.extend(lux::RAW_SUNLIGHT),
        local_from_render: Mat3::from_quat(rotation_to_render(turned)),
        tile: Vec4::new(
            tile.x as f32,
            tile.y as f32,
            tile.z as f32,
            light_steps as f32,
        ),
        shell: Vec4::new(
            (radius + def.base) as f32,
            (radius + def.top) as f32,
            def.density as f32,
            cirrus,
        ),
        albedo: Vec4::new(r, g, b, bounce),
        prev_view_from_world,
        prev_clip_from_view,
        moved: to_render(moved).extend(if was_cut { 1.0 } else { 0.0 }),
        frame: Vec4::new(
            (*count % 4096) as f32,
            DETAIL_DISTANCE,
            if detail { 1.0 } else { 0.0 },
            step_share,
        ),
    };
    let eclipse = eclipse_for(system, &frame, &airs, id, DVec3::ZERO);
    let local = Mat3::from_quat(rotation_to_render(turned));
    clouds.draw = Some((airs.bindings(id), draw, eclipse, local));
}

#[derive(Resource)]
struct CloudPipelines {
    march_layout: BindGroupLayoutDescriptor,
    march_layout_msaa: BindGroupLayoutDescriptor,
    resolve_layout: BindGroupLayoutDescriptor,
    composite_layout: BindGroupLayoutDescriptor,
    composite_layout_msaa: BindGroupLayoutDescriptor,
    march: CachedRenderPipelineId,
    march_msaa: CachedRenderPipelineId,
    resolve: CachedRenderPipelineId,
    composite: CachedRenderPipelineId,
    composite_msaa: CachedRenderPipelineId,
    noise_sampler: Sampler,
    history_sampler: Sampler,
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
    let march_layout = |label: &'static str, msaa: bool| {
        BindGroupLayoutDescriptor::new(
            label,
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::FRAGMENT,
                (
                    (0, uniform_buffer::<ViewUniform>(true)),
                    (1, uniform_buffer::<CloudDraw>(false)),
                    (2, depth_binding(msaa)),
                    (3, texture_3d(filterable)),
                    (4, texture_3d(filterable)),
                    (5, sampler(SamplerBindingType::Filtering)),
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
    let resolve_layout = BindGroupLayoutDescriptor::new(
        "clouds_resolve",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (0, uniform_buffer::<ViewUniform>(true)),
                (1, uniform_buffer::<CloudDraw>(false)),
                (
                    2,
                    texture_2d(TextureSampleType::Float { filterable: false }),
                ),
                (
                    3,
                    texture_2d(TextureSampleType::Float { filterable: false }),
                ),
                (4, texture_2d(filterable)),
                (5, sampler(SamplerBindingType::Filtering)),
            ),
        ),
    );
    let (march_plain, march_msaa) = (
        march_layout("clouds_march", false),
        march_layout("clouds_march_msaa", true),
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
    let march_target = |layout: &BindGroupLayoutDescriptor, msaa: bool| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("clouds_march".into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: asset_server.load("embedded://osp/shaders/clouds.wesl"),
                shader_defs: defs(msaa),
                targets: vec![
                    Some(TextureFormat::Rgba16Float.into()),
                    Some(TextureFormat::R32Float.into()),
                ],
                ..default()
            }),
            ..default()
        })
    };
    let march = march_target(&march_plain, false);
    let march_multisampled = march_target(&march_msaa, true);
    let resolve = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("clouds_resolve".into()),
        layout: vec![resolve_layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: asset_server.load("embedded://osp/shaders/clouds_resolve.wesl"),
            shader_defs: defs(false),
            targets: vec![Some(TextureFormat::Rgba16Float.into())],
            ..default()
        }),
        ..default()
    });
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
    let noise_sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("clouds_noise"),
        address_mode_u: AddressMode::Repeat,
        address_mode_v: AddressMode::Repeat,
        address_mode_w: AddressMode::Repeat,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        mipmap_filter: MipmapFilterMode::Linear,
        ..default()
    });
    let history_sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("clouds_history"),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    commands.insert_resource(CloudPipelines {
        march_layout: march_plain,
        march_layout_msaa: march_msaa,
        resolve_layout,
        composite_layout: composite_plain,
        composite_layout_msaa: composite_msaa,
        march,
        march_msaa: march_multisampled,
        resolve,
        composite,
        composite_msaa: composite_multisampled,
        noise_sampler,
        history_sampler,
    });
}

/// This frame's clouds, for anything else that needs to see through them (alpha is how much shows).
#[derive(Resource, Default)]
pub struct CloudResult(pub Option<TextureView>);

/// The half-resolution targets, and this frame's bind groups.
#[derive(Resource, Default)]
struct CloudTargets {
    size: UVec2,
    light: Option<TextureView>,
    distance: Option<TextureView>,
    /// Two histories taking turns: `next` is written this frame.
    history: Vec<TextureView>,
    next: usize,
    air: UniformBuffer<AirUniform>,
    clouds: UniformBuffer<Vec4>,
    draw: UniformBuffer<CloudDraw>,
    eclipse: UniformBuffer<EclipseUniform>,
    local: UniformBuffer<Mat3>,
    sun: UniformBuffer<Vec4>,
    groups: Option<CloudGroups>,
}

struct CloudGroups {
    march: BindGroup,
    resolve: BindGroup,
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
    let (Some(base), Some(detail)) = (images.get(&frame.base), images.get(&frame.detail)) else {
        return;
    };
    let full = extracted.viewport.zw();
    let size = (full + 1) / 2;
    if size.min_element() == 0 {
        return;
    }
    let targets = &mut *targets;
    let mut draw = *draw;
    if targets.size != size {
        targets.size = size;
        targets.light = Some(target(&device, size, TextureFormat::Rgba16Float));
        targets.distance = Some(target(&device, size, TextureFormat::R32Float));
        targets.history = (0..2)
            .map(|_| target(&device, size, TextureFormat::Rgba16Float))
            .collect();
        // New targets have nothing in them.
        draw.moved.w = 1.0;
    }
    targets.next = 1 - targets.next;
    let (Some(light), Some(distance)) = (&targets.light, &targets.distance) else {
        return;
    };
    targets.air.set(air.uniform);
    targets.air.write_buffer(&device, &queue);
    targets.clouds.set(air.clouds);
    targets.clouds.write_buffer(&device, &queue);
    targets.draw.set(draw);
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
    let march = device.create_bind_group(
        "clouds_march",
        &pipeline_cache.get_bind_group_layout(if multi {
            &pipelines.march_layout_msaa
        } else {
            &pipelines.march_layout
        }),
        &BindGroupEntries::with_indices((
            (0, view.clone()),
            (1, &targets.draw),
            (2, depth),
            (3, &base.texture_view),
            (4, &detail.texture_view),
            (5, &pipelines.noise_sampler),
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
    let resolve = device.create_bind_group(
        "clouds_resolve",
        &pipeline_cache.get_bind_group_layout(&pipelines.resolve_layout),
        &BindGroupEntries::with_indices((
            (0, view),
            (1, &targets.draw),
            (2, light),
            (3, distance),
            (4, &targets.history[1 - targets.next]),
            (5, &pipelines.history_sampler),
        )),
    );
    let composite = device.create_bind_group(
        "clouds_composite",
        &pipeline_cache.get_bind_group_layout(if multi {
            &pipelines.composite_layout_msaa
        } else {
            &pipelines.composite_layout
        }),
        &BindGroupEntries::with_indices(((0, &targets.history[targets.next]), (1, depth))),
    );
    result.0 = Some(targets.history[targets.next].clone());
    targets.groups = Some(CloudGroups {
        march,
        resolve,
        composite,
    });
}

fn attachment(view: &TextureView) -> Option<RenderPassColorAttachment<'_>> {
    Some(RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: Operations {
            load: LoadOp::Clear(LinearRgba::BLACK.into()),
            store: StoreOp::Store,
        },
    })
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
    let (Some(light), Some(distance)) = (&targets.light, &targets.distance) else {
        return;
    };
    let (view_target, view_offset, msaa) = view.into_inner();
    let multi = msaa.samples() > 1;
    let (Some(march), Some(resolve), Some(composite)) = (
        pipeline_cache.get_render_pipeline(if multi {
            pipelines.march_msaa
        } else {
            pipelines.march
        }),
        pipeline_cache.get_render_pipeline(pipelines.resolve),
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
            label: Some("clouds_march"),
            color_attachments: &[attachment(light), attachment(distance)],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(march);
        pass.set_bind_group(0, &groups.march, &[view_offset.offset]);
        pass.draw(0..3, 0..1);
    }
    {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("clouds_resolve"),
            color_attachments: &[attachment(&targets.history[targets.next])],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(resolve);
        pass.set_bind_group(0, &groups.resolve, &[view_offset.offset]);
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
