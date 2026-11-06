//! Bodies' shadows on each other, the lens flare, and the light they throw on each other.
//!
//! The shadow of another body falls on whatever the depth buffer holds (`shaders/eclipse.wesl`), a
//! multiply over the opaque scene, before the sky goes over it. The sky and the clouds are lit
//! through the same test at every step of their marches (`eclipse_at` in
//! `shaders/atmosphere.wesl`), so totality on Earth comes out as it should: a dark sky overhead with
//! a ring of sunset all around the horizon, where the air is outside the Moon's shadow. Earth's
//! shadow on the Moon lets through the light bent round through its air, which reddens it.
//!
//! The flare (`shaders/flare.wesl`) is drawn over everything in the view, before the post effects.
//!
//! The vessel's light probe holds the planet it is near; what shines on a body from another one
//! (earthshine on the Moon, moonlight on Earth) is a second, weak directional light.

use bevy::{
    asset::embedded_asset,
    camera::Exposure,
    core_pipeline::{
        FullscreenShader,
        core_3d::{main_opaque_pass_3d, main_transparent_pass_3d},
        schedule::{Core3d, Core3dSystems},
    },
    light::{VolumetricLight, light_consts::lux},
    math::DVec3,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_resource::{
            binding_types::{sampler, texture_2d, texture_2d_multisampled, uniform_buffer},
            *,
        },
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        view::{
            Msaa, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms,
        },
    },
    shader::{ShaderDefVal, load_shader_library},
};
use osp_sim::{
    body::{Body, BodyId},
    system::SolarSystem,
};

use crate::{
    FrameSet,
    atmosphere::{BodyAirs, render_sky},
    camera::{CameraRig, MainCamera},
    clouds::CloudResult,
    frame::{RenderFrame, Sunlight, to_render},
    settings::GraphicsSettings,
    sim::Sim,
};

/// A body this small in the shadow caster's sky (as a share of the Sun's size there) casts no
/// shadow worth drawing.
const LEAST_OCCLUDER: f64 = 0.15;
/// What gets into an umbra when the occluder has air, by colour: light refracted through its air
/// (Earth's shadow on the Moon, about a thousandth of sunlight, far redder than it).
const AIR_UMBRA: [f32; 3] = [0.006, 0.0016, 0.0004];
/// A second light weaker than this, in lux, isn't worth a light.
const LEAST_SHINE: f64 = 0.01;
/// Exposure (EV100) on an airless body's night side, lit by nothing else than earthshine.
const AIRLESS_NIGHT_EV: f32 = 4.5;

pub struct EclipsePlugin;

impl Plugin for EclipsePlugin {
    fn build(&self, app: &mut App) {
        load_shader_library!(app, "shaders/eclipse_common.wesl");
        embedded_asset!(app, "shaders/eclipse.wesl");
        embedded_asset!(app, "shaders/flare.wesl");
        app.init_resource::<LensFrame>()
            .add_plugins(ExtractResourcePlugin::<LensFrame>::default())
            .add_systems(Startup, spawn_planetshine)
            .add_systems(
                Update,
                (place_shadows, shine_planets)
                    .after(crate::frame::update_sunlight)
                    .in_set(FrameSet::Render),
            );
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<LensBuffers>()
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(
                Render,
                prepare_lens.in_set(RenderSystems::PrepareBindGroups),
            )
            .add_systems(
                Core3d,
                (
                    render_shadows.after(main_opaque_pass_3d).before(render_sky),
                    render_flare
                        .after(main_transparent_pass_3d)
                        .in_set(Core3dSystems::MainPass),
                ),
            );
    }
}

/// `Eclipse` in `shaders/eclipse_common.wesl`.
#[derive(Debug, Clone, Copy, Default, ShaderType, Reflect)]
pub struct EclipseUniform {
    first: Vec4,
    second: Vec4,
    umbra: Vec4,
}

/// The bodies that can shade `body`, with the positions relative to `origin` (a point in `body`'s
/// frame, in f64 and sim axes) for the shader, largest first.
pub fn eclipse_for(
    system: &SolarSystem,
    frame: &RenderFrame,
    airs: &BodyAirs,
    body: BodyId,
    relative_to: DVec3,
) -> EclipseUniform {
    let star = system.root();
    let here = frame.body_offset(body);
    let to_star = frame.body_offset(star) - here;
    let sun_size = system.body(star).radius / to_star.length().max(1.0);
    let mut casters: Vec<(f64, &Body)> = system
        .bodies()
        .iter()
        .filter(|b| b.id != body && b.id != star)
        .filter_map(|b| {
            let along = (frame.body_offset(b.id) - here).dot(to_star.normalize());
            if along <= 0.0 {
                return None;
            }
            let distance = (frame.body_offset(b.id) - here).length();
            let size = b.radius / distance;
            let apart = (frame.body_offset(b.id) - here)
                .reject_from(to_star)
                .length();
            // Its shadow's reach at this body's distance, against how far off-axis this body is.
            let reach = system.body(body).radius + b.radius + along * sun_size;
            (size > sun_size * LEAST_OCCLUDER && apart < reach).then_some((size, b))
        })
        .collect();
    casters.sort_by(|a, b| b.0.total_cmp(&a.0));
    let place = |b: Option<&(f64, &Body)>| {
        b.map_or(Vec4::ZERO, |(_, b)| {
            to_render(frame.body_offset(b.id) - here - relative_to).extend(b.radius as f32)
        })
    };
    let umbra = casters.first().map_or([0.0; 3], |(_, b)| {
        if airs.get(b.id).is_some() {
            AIR_UMBRA
        } else {
            [0.0; 3]
        }
    });
    EclipseUniform {
        first: place(casters.first()),
        second: place(casters.get(1)),
        umbra: Vec4::new(umbra[0], umbra[1], umbra[2], sun_size as f32),
    }
}

/// The share of the Sun's disc in sight past an occluder. Mirrors the shader's `disc_in_sight`.
fn disc_in_sight(p: DVec3, sun: DVec3, centre: DVec3, radius: f64, sun_radius: f64) -> f64 {
    let to_it = centre - p;
    let d = to_it.length();
    if d <= radius || to_it.dot(sun) <= 0.0 {
        return 1.0;
    }
    let its = (radius / d).asin();
    let apart = to_it.cross(sun).length().atan2(to_it.dot(sun));
    let smooth = |a: f64, b: f64, x: f64| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    if its >= sun_radius {
        return smooth(its - sun_radius, its + sun_radius, apart);
    }
    let least = 1.0 - its * its / (sun_radius * sun_radius);
    least + (1.0 - least) * smooth(sun_radius - its, sun_radius + its, apart)
}

/// The share of the Sun in sight from a point `local` relative to `body`, past every other body but
/// the one it stands on (that shadow is the planet's own, handled with its air).
pub fn sunlight_left(system: &SolarSystem, frame: &RenderFrame, body: BodyId, local: DVec3) -> f64 {
    let star = system.root();
    let p = frame.relative(body, local);
    let to_star = frame.body_offset(star) - p;
    let sun = to_star.normalize_or(DVec3::X);
    let sun_radius = system.body(star).radius / to_star.length().max(1.0);
    system
        .bodies()
        .iter()
        .filter(|b| b.id != star && b.id != body)
        .map(|b| disc_in_sight(p, sun, frame.body_offset(b.id), b.radius, sun_radius))
        .fold(1.0, f64::min)
}

impl EclipseUniform {
    fn any(&self) -> bool {
        self.first.w > 0.0
    }
}

/// `Flare` in `shaders/flare.wesl`.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct FlareUniform {
    place: Vec4,
    tint: Vec4,
}

/// What the shadow and flare passes draw this frame.
#[derive(Resource, Clone, Default, ExtractResource)]
#[extract_app(RenderApp)]
struct LensFrame {
    /// The shadow in sight, relative to the camera, and the direction to the Sun.
    shadow: Option<(EclipseUniform, Vec4)>,
    flare: Option<FlareUniform>,
}

fn place_shadows(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    settings: Res<GraphicsSettings>,
    airs: Res<BodyAirs>,
    camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut out: ResMut<LensFrame>,
) {
    out.shadow = None;
    out.flare = None;
    let Some(_) = frame.origin_body else {
        return;
    };
    let system = &sim.universe.system;
    let (camera, transform) = camera.into_inner();
    // Shadows on the body nearest the camera; anything else is a dot.
    let nearest = system
        .bodies()
        .iter()
        .filter(|b| b.parent.is_some())
        .min_by(|a, b| {
            let d = |x: &Body| (rig.position - frame.body_offset(x.id)).length() - x.radius;
            d(a).total_cmp(&d(b))
        });
    let star = frame.body_offset(system.root());
    if let Some(body) = nearest {
        let from_centre = rig.position - frame.body_offset(body.id);
        let eclipse = eclipse_for(system, &frame, &airs, body.id, from_centre);
        if eclipse.any() {
            let sun = to_render(star - frame.body_offset(body.id)).normalize_or(Vec3::Y);
            out.shadow = Some((eclipse, sun.extend(0.0)));
        }
    }

    // The flare: only with the Sun in front of the camera. Lens effects go with the bloom.
    if !settings.bloom {
        return;
    }
    let to_sun_world = to_render(star - rig.position);
    let to_sun = to_sun_world.normalize_or(Vec3::Y);
    let in_view = transform.affine().inverse().transform_vector3(to_sun);
    if in_view.z >= -0.05 {
        return;
    }
    let clip = camera.clip_from_view();
    let (x, y) = (
        clip.x_axis.x * in_view.x / -in_view.z,
        clip.y_axis.y * in_view.y / -in_view.z,
    );
    if x.abs() > 1.6 || y.abs() > 1.6 {
        return;
    }
    let aspect = clip.y_axis.y / clip.x_axis.x;
    let sun_radius = (system.body(system.root()).radius / star.distance(rig.position)) as f32;
    let near = clip.w_axis.z;
    // The air in the way, from where the camera is.
    let tint = nearest
        .and_then(|body| Some((body, airs.get(body.id)?)))
        .map_or(Vec3::ONE, |(body, air)| {
            let eye = rig.position - frame.body_offset(body.id);
            let sun = (star - frame.body_offset(body.id)).normalize();
            let [r, g, b] = air.air.sun_transmittance(eye, sun).map(|t| t as f32);
            Vec3::new(r, g, b)
        });
    out.flare = Some(FlareUniform {
        place: Vec4::new(
            x * 0.5 + 0.5,
            0.5 - y * 0.5,
            sun_radius * clip.y_axis.y * 0.5,
            aspect,
        ),
        tint: tint.extend(near / to_sun_world.length()),
    });
}

/// The weak second light: what shines here from the brightest other body.
#[derive(Component)]
struct Planetshine;

fn spawn_planetshine(mut commands: Commands) {
    commands.spawn((
        Planetshine,
        DirectionalLight {
            illuminance: 0.0,
            shadow_maps_enabled: false,
            ..default()
        },
        // The shaders take the first directional light to be the Sun. Bevy sorts lights with this
        // after those without, and there is no fog for it to do anything else.
        VolumetricLight,
        Transform::default(),
    ));
}

/// Sunlight reflected by `body` onto a point `at` (relative to the origin): albedo, how much of the
/// disc is lit as seen from there, and the square of its apparent size.
fn shine_from(body: &Body, frame: &RenderFrame, star: DVec3, at: DVec3) -> f64 {
    let centre = frame.body_offset(body.id);
    let to_it = centre - at;
    let lit = (star - centre).normalize().dot(-to_it.normalize());
    let phase = 0.5 + 0.5 * lit;
    lux::RAW_SUNLIGHT as f64 * body.albedo * phase * (body.radius / to_it.length()).powi(2)
}

fn shine_planets(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    mut lights: Query<
        (&mut DirectionalLight, &mut Transform),
        (With<Planetshine>, Without<Sunlight>),
    >,
    mut exposure: Single<&mut Exposure, With<MainCamera>>,
    time: Res<Time>,
) {
    let Ok((mut light, mut transform)) = lights.single_mut() else {
        return;
    };
    let Some(vessel_body) = frame.vessel_body else {
        return;
    };
    let system = &sim.universe.system;
    let here = system.body(vessel_body);
    let at = frame.relative(vessel_body, frame.vessel_offset);
    let star = frame.body_offset(system.root());
    // Parent and children: the vessel's own body is in the light probe already.
    let brightest = here
        .parent
        .iter()
        .chain(here.children.iter())
        .filter(|&&id| id != system.root())
        .map(|&id| {
            let body = system.body(id);
            (shine_from(body, &frame, star, at), body)
        })
        .max_by(|a, b| a.0.total_cmp(&b.0));
    match brightest.filter(|_| !rig.map_view) {
        Some((shine, body)) if shine > LEAST_SHINE => {
            let [r, g, b] = body.color;
            light.color = Color::linear_rgb(r, g, b);
            light.illuminance = shine as f32;
            let toward = to_render(frame.body_offset(body.id) - at).normalize_or(Vec3::Y);
            *transform = Transform::default().looking_to(-toward, Vec3::Y);
        }
        _ => light.illuminance = 0.0,
    }

    // With no air to scatter anything, a night side is lit by that alone.
    if here.atmosphere.is_none() && here.parent.is_some() && !rig.map_view {
        let up = frame.vessel_offset.normalize_or(DVec3::Z);
        let sun_up = up.dot((star - at).normalize()) as f32;
        let night = 1.0 - ((sun_up + 0.1) / 0.2).clamp(0.0, 1.0);
        if night > 0.0 {
            let target = exposure.ev100 + (AIRLESS_NIGHT_EV - exposure.ev100) * night;
            let blend = 1.0 - (-time.delta_secs() / 1.5).exp();
            exposure.ev100 += (target - exposure.ev100) * blend;
        }
    }
}

#[derive(Resource)]
struct LensPipelines {
    shadow_layout: BindGroupLayoutDescriptor,
    shadow_layout_msaa: BindGroupLayoutDescriptor,
    flare_layout: BindGroupLayoutDescriptor,
    flare_layout_msaa: BindGroupLayoutDescriptor,
    shadow: CachedRenderPipelineId,
    shadow_msaa: CachedRenderPipelineId,
    flare: CachedRenderPipelineId,
    flare_msaa: CachedRenderPipelineId,
    linear: Sampler,
    /// Stands in for the clouds' result when there are none: nothing in the way.
    no_clouds: TextureView,
}

fn init_pipelines(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    pipeline_cache: Res<PipelineCache>,
    fullscreen: Res<FullscreenShader>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let depth = |msaa: bool| {
        if msaa {
            texture_2d_multisampled(TextureSampleType::Depth)
        } else {
            texture_2d(TextureSampleType::Depth)
        }
    };
    let shadow_layout = |msaa: bool| {
        BindGroupLayoutDescriptor::new(
            "eclipse",
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::FRAGMENT,
                (
                    (0, uniform_buffer::<ViewUniform>(true)),
                    (1, uniform_buffer::<EclipseUniform>(false)),
                    (2, uniform_buffer::<Vec4>(false)),
                    (3, depth(msaa)),
                ),
            ),
        )
    };
    let flare_layout = |msaa: bool| {
        BindGroupLayoutDescriptor::new(
            "flare",
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    (0, uniform_buffer::<FlareUniform>(false)),
                    (1, depth(msaa)),
                    (2, texture_2d(TextureSampleType::Float { filterable: true })),
                    (3, sampler(SamplerBindingType::Filtering)),
                ),
            ),
        )
    };
    let (shadow_plain, shadow_multi) = (shadow_layout(false), shadow_layout(true));
    let (flare_plain, flare_multi) = (flare_layout(false), flare_layout(true));
    let defs = |msaa: bool| {
        if msaa {
            vec![ShaderDefVal::from("MULTISAMPLED")]
        } else {
            vec![]
        }
    };
    let shadow_pipeline = |layout: &BindGroupLayoutDescriptor, samples: u32| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("eclipse".into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: asset_server.load("embedded://osp_render/shaders/eclipse.wesl"),
                shader_defs: defs(samples > 1),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::Rgba16Float,
                    // Multiply the scene by the output.
                    blend: Some(BlendState {
                        color: BlendComponent {
                            src_factor: BlendFactor::Dst,
                            dst_factor: BlendFactor::Zero,
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
    let flare_shader: Handle<Shader> =
        asset_server.load("embedded://osp_render/shaders/flare.wesl");
    let flare_pipeline = |layout: &BindGroupLayoutDescriptor, samples: u32| {
        pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("flare".into()),
            layout: vec![layout.clone()],
            vertex: VertexState {
                shader: flare_shader.clone(),
                shader_defs: defs(samples > 1),
                entry_point: Some("vertex".into()),
                buffers: vec![],
                ..default()
            },
            fragment: Some(FragmentState {
                shader: flare_shader.clone(),
                shader_defs: defs(samples > 1),
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::Rgba16Float,
                    blend: Some(BlendState {
                        color: BlendComponent {
                            src_factor: BlendFactor::One,
                            dst_factor: BlendFactor::One,
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
    let shadow = shadow_pipeline(&shadow_plain, 1);
    let shadow_msaa = shadow_pipeline(&shadow_multi, Msaa::Sample4.samples());
    let flare = flare_pipeline(&flare_plain, 1);
    let flare_msaa = flare_pipeline(&flare_multi, Msaa::Sample4.samples());
    let linear = device.create_sampler(&SamplerDescriptor {
        label: Some("flare"),
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    // One texel that lets everything through.
    let no_clouds = device
        .create_texture_with_data(
            &queue,
            &TextureDescriptor {
                label: Some("no_clouds"),
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rgba8Unorm,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                view_formats: &[],
            },
            TextureDataOrder::LayerMajor,
            &[0, 0, 0, 255],
        )
        .create_view(&TextureViewDescriptor::default());
    commands.insert_resource(LensPipelines {
        shadow_layout: shadow_plain,
        shadow_layout_msaa: shadow_multi,
        flare_layout: flare_plain,
        flare_layout_msaa: flare_multi,
        shadow,
        shadow_msaa,
        flare,
        flare_msaa,
        linear,
        no_clouds,
    });
}

#[derive(Resource, Default)]
struct LensBuffers {
    eclipse: UniformBuffer<EclipseUniform>,
    sun: UniformBuffer<Vec4>,
    flare: UniformBuffer<FlareUniform>,
    shadow_group: Option<BindGroup>,
    flare_group: Option<BindGroup>,
}

fn prepare_lens(
    frame: Res<LensFrame>,
    pipelines: Res<LensPipelines>,
    pipeline_cache: Res<PipelineCache>,
    clouds: Res<CloudResult>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    view_uniforms: Res<ViewUniforms>,
    views: Query<(&ViewDepthStencilTexture, &Msaa), With<MainCamera>>,
    mut buffers: ResMut<LensBuffers>,
) {
    buffers.shadow_group = None;
    buffers.flare_group = None;
    let Ok((depth, msaa)) = views.single() else {
        return;
    };
    let Some(depth) = depth.attachment.depth_stencil_views().depth_only_view() else {
        return;
    };
    let multi = msaa.samples() > 1;
    let buffers = &mut *buffers;
    if let (Some((eclipse, sun)), Some(view)) = (&frame.shadow, view_uniforms.uniforms.binding()) {
        buffers.eclipse.set(*eclipse);
        buffers.eclipse.write_buffer(&device, &queue);
        buffers.sun.set(*sun);
        buffers.sun.write_buffer(&device, &queue);
        buffers.shadow_group = Some(device.create_bind_group(
            "eclipse",
            &pipeline_cache.get_bind_group_layout(if multi {
                &pipelines.shadow_layout_msaa
            } else {
                &pipelines.shadow_layout
            }),
            &BindGroupEntries::with_indices((
                (0, view),
                (1, &buffers.eclipse),
                (2, &buffers.sun),
                (3, depth),
            )),
        ));
    }
    if let Some(flare) = &frame.flare {
        buffers.flare.set(*flare);
        buffers.flare.write_buffer(&device, &queue);
        let cloud_view = clouds.0.as_ref().unwrap_or(&pipelines.no_clouds);
        buffers.flare_group = Some(device.create_bind_group(
            "flare",
            &pipeline_cache.get_bind_group_layout(if multi {
                &pipelines.flare_layout_msaa
            } else {
                &pipelines.flare_layout
            }),
            &BindGroupEntries::with_indices((
                (0, &buffers.flare),
                (1, depth),
                (2, cloud_view),
                (3, &pipelines.linear),
            )),
        ));
    }
}

fn render_shadows(
    view: ViewQuery<(&ViewTarget, &ViewUniformOffset, &Msaa), With<MainCamera>>,
    buffers: Res<LensBuffers>,
    pipelines: Res<LensPipelines>,
    pipeline_cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let Some(group) = &buffers.shadow_group else {
        return;
    };
    let (target, view_offset, msaa) = view.into_inner();
    let id = if msaa.samples() > 1 {
        pipelines.shadow_msaa
    } else {
        pipelines.shadow
    };
    let Some(pipeline) = pipeline_cache.get_render_pipeline(id) else {
        return;
    };
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("eclipse"),
            color_attachments: &[Some(target.get_color_attachment())],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[view_offset.offset]);
    pass.draw(0..3, 0..1);
}

fn render_flare(
    view: ViewQuery<(&ViewTarget, &Msaa), With<MainCamera>>,
    buffers: Res<LensBuffers>,
    pipelines: Res<LensPipelines>,
    pipeline_cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let Some(group) = &buffers.flare_group else {
        return;
    };
    let (target, msaa) = view.into_inner();
    let id = if msaa.samples() > 1 {
        pipelines.flare_msaa
    } else {
        pipelines.flare
    };
    let Some(pipeline) = pipeline_cache.get_render_pipeline(id) else {
        return;
    };
    let mut pass = ctx
        .command_encoder()
        .begin_render_pass(&RenderPassDescriptor {
            label: Some("flare"),
            color_attachments: &[Some(target.get_color_attachment())],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totality_goes_dark_and_clear_sky_stays_lit() {
        let sun = DVec3::X;
        let sun_radius = 0.00465;
        // Near its perigee: from further off its disc doesn't quite cover the Sun's.
        let moon = DVec3::new(3.56e8, 0.0, 0.0);
        let radius = 1.7374e6;
        // On the axis: the Moon's disc covers the Sun's.
        assert!(disc_in_sight(DVec3::ZERO, sun, moon, radius, sun_radius) < 1e-9);
        // Far off the axis: nothing in the way.
        let away = DVec3::new(0.0, 6.3e6, 0.0);
        assert_eq!(disc_in_sight(away, sun, moon, radius, sun_radius), 1.0);
        // In the penumbra, part of the Sun shows.
        let part = DVec3::new(0.0, 2.2e6, 0.0);
        let seen = disc_in_sight(part, sun, moon, radius, sun_radius);
        assert!(seen > 0.05 && seen < 0.95, "{seen}");
        // Facing away from the Sun, the Moon can't shade.
        assert_eq!(
            disc_in_sight(DVec3::ZERO, -sun, moon, radius, sun_radius),
            1.0
        );
    }

    #[test]
    fn a_small_occluder_only_takes_a_bite() {
        // A transit: Venus's size against the Sun's, seen from Earth.
        let sun = DVec3::X;
        let seen = disc_in_sight(DVec3::ZERO, sun, DVec3::new(4e10, 0.0, 0.0), 6e6, 0.00465);
        assert!(seen > 0.99 && seen < 1.0, "{seen}");
    }
}
