//! Orbit camera around the active vessel (flight view) or its parent body (map view).
//!
//! The view goes to the window, or to an image for scripted captures ([`ViewOutput`]).
//! Below 100% render scale (see `settings`) it renders into an image that much smaller,
//! and a 2D camera stretches that over the output under the UI, which stays at full
//! resolution.

use bevy::{
    anti_alias::fxaa::Fxaa,
    camera::{Exposure, Hdr, ImageRenderTarget, RenderTarget, visibility::RenderLayers},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    math::DVec3,
    post_process::bloom::Bloom,
    prelude::*,
    render::{
        render_resource::TextureFormat,
        view::{Msaa, Tonemapping},
    },
    window::PrimaryWindow,
};

use osp_ui::PointerOverUi;

use crate::{
    FrameSet,
    controls::InputLocked,
    frame::{RenderFrame, to_render},
    settings::{Antialiasing, GraphicsSettings},
    sim::Sim,
};

/// Render layer of the scaled view's composite (keeps gizmos and the scene out of it).
const COMPOSITE_LAYER: usize = 8;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraRig>()
            .init_resource::<ViewOutput>()
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, update_camera.in_set(FrameSet::Camera))
            .add_systems(
                Update,
                (
                    apply_view_settings.run_if(resource_changed::<GraphicsSettings>),
                    apply_render_scale,
                )
                    .in_set(FrameSet::Render),
            );
    }
}

/// Where the view and the UI end up: the primary window, or an image (scripted captures).
#[derive(Resource, Default)]
pub struct ViewOutput(pub Option<Handle<Image>>);

#[derive(Component)]
pub struct MainCamera;

#[derive(Resource)]
pub struct CameraRig {
    pub map_view: bool,
    /// Radians around the local vertical.
    yaw: f64,
    /// Elevation above the local horizon, radians.
    pitch: f64,
    flight_distance: f64,
    /// Map distance as a multiple of the focused body's radius.
    map_distance: f64,
    /// Flight view: look at this height above the base of the stack instead of its middle
    /// (debugging, `OSP_CAMERA`).
    focus_height: Option<f64>,
    /// Camera position relative to the render origin (f64, sim axes), for systems such as
    /// planet LOD that need more precision than the f32 transform.
    pub position: DVec3,
}

impl Default for CameraRig {
    fn default() -> Self {
        let mut rig = Self {
            map_view: false,
            // South of the vessel: the sun is behind the camera for northern launch sites.
            yaw: std::f64::consts::PI,
            pitch: 0.15,
            flight_distance: 120.0,
            map_distance: 4.0,
            focus_height: None,
            position: DVec3::ZERO,
        };
        // Debugging and screenshots: `OSP_CAMERA=<yaw°>,<pitch°>,<distance m>[,<focus m>]`,
        // the last being the height above the base of the stack to look at.
        if let Ok(spec) = std::env::var("OSP_CAMERA") {
            let values: Vec<f64> = spec
                .split(',')
                .filter_map(|v| v.trim().parse().ok())
                .collect();
            if let [yaw, pitch, distance, ..] = values[..] {
                rig.yaw = yaw.to_radians();
                rig.pitch = pitch.to_radians();
                rig.flight_distance = distance;
                rig.focus_height = values.get(3).copied();
            }
        }
        rig
    }
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        MainCamera,
        Camera3d::default(),
        // Lighting, sky and haze are physically scaled; tonemap once at the end.
        Hdr,
        Projection::Perspective(PerspectiveProjection {
            fov: 60f32.to_radians(),
            near: 0.1,
            // Bevy uses an infinite reverse-Z projection; `far` only affects culling.
            far: 1e15,
            ..default()
        }),
        Exposure::SUNLIGHT,
        Transform::default(),
    ));
}

/// Anti-aliasing and bloom.
fn apply_view_settings(
    mut commands: Commands,
    settings: Res<GraphicsSettings>,
    camera: Single<Entity, With<MainCamera>>,
) {
    let mut camera = commands.entity(*camera);
    match settings.antialiasing {
        Antialiasing::Off => camera.insert(Msaa::Off).remove::<Fxaa>(),
        Antialiasing::Fxaa => camera.insert((Msaa::Off, Fxaa::default())),
        Antialiasing::Msaa => camera.insert(Msaa::Sample4).remove::<Fxaa>(),
    };
    if settings.bloom {
        // Glow around what is much brighter than the scene: plumes, the sun on metal.
        camera.insert(Bloom {
            intensity: 0.08,
            ..Bloom::NATURAL
        });
    } else {
        camera.remove::<Bloom>();
    }
}

/// The 2D camera and sprite that show the scaled view on the window.
#[derive(Component)]
struct ViewComposite;

/// Point the camera at the output, or at an image `render_scale` of its size shown by
/// [`ViewComposite`]. The image target's scale factor makes its logical size the output's,
/// so cursor positions and projected points stay in window coordinates. The UI goes with
/// whichever camera draws the output.
#[allow(clippy::too_many_arguments)]
fn apply_render_scale(
    mut commands: Commands,
    settings: Res<GraphicsSettings>,
    output: Res<ViewOutput>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Single<
        (
            Entity,
            &mut RenderTarget,
            &mut Projection,
            Has<IsDefaultUiCamera>,
        ),
        With<MainCamera>,
    >,
    mut sprites: Query<&mut Sprite, With<ViewComposite>>,
    composite: Query<Entity, (With<ViewComposite>, With<Camera>)>,
    composite_parts: Query<Entity, With<ViewComposite>>,
    mut images: ResMut<Assets<Image>>,
) {
    // The output's size in physical and logical pixels.
    let (output_target, physical, logical) = match &output.0 {
        Some(image) => {
            let size = images.get(image).map_or(UVec2::ZERO, Image::size);
            (RenderTarget::from(image.clone()), size, size.as_vec2())
        }
        None => (
            RenderTarget::default(),
            window.physical_size(),
            window.size(),
        ),
    };
    let size = (physical.as_vec2() * settings.render_scale as f32 / 100.0)
        .round()
        .as_uvec2();
    let (main_camera, target, projection, has_ui) = &mut *camera;
    // Bevy recomputes a camera's size when its target resizes or its projection changes,
    // not when the target is swapped for another; touching the projection makes it.
    if settings.render_scale >= 100 || size.min_element() == 0 {
        let at_output = match (&**target, &output_target) {
            // Only ever the primary window.
            (RenderTarget::Window(_), RenderTarget::Window(_)) => true,
            (RenderTarget::Image(a), RenderTarget::Image(b)) => a == b,
            _ => false,
        };
        if !at_output {
            **target = output_target;
            projection.set_changed();
        }
        if !*has_ui {
            commands.entity(*main_camera).insert(IsDefaultUiCamera);
        }
        for entity in &composite_parts {
            commands.entity(entity).despawn();
        }
        return;
    }
    let scale_factor = size.x as f32 / logical.x.max(1.0);
    let current = match &**target {
        RenderTarget::Image(target) => images
            .get(&target.handle)
            .filter(|image| image.size() == size && target.scale_factor == scale_factor)
            .map(|_| target.handle.clone()),
        _ => None,
    };
    let handle = match current {
        Some(handle) => handle,
        None => {
            let handle = images.add(Image::new_target_texture(
                size.x,
                size.y,
                TextureFormat::Rgba8UnormSrgb,
                None,
            ));
            **target = RenderTarget::Image(ImageRenderTarget {
                handle: handle.clone(),
                scale_factor,
            });
            projection.set_changed();
            handle
        }
    };
    if *has_ui {
        commands.entity(*main_camera).remove::<IsDefaultUiCamera>();
    }
    if composite.is_empty() {
        commands.spawn((
            ViewComposite,
            Camera2d,
            Camera {
                order: 1,
                ..default()
            },
            output_target,
            IsDefaultUiCamera,
            Msaa::Off,
            Tonemapping::None,
            RenderLayers::layer(COMPOSITE_LAYER),
        ));
        commands.spawn((
            ViewComposite,
            Sprite {
                image: handle,
                custom_size: Some(logical),
                ..default()
            },
            RenderLayers::layer(COMPOSITE_LAYER),
            Pickable::IGNORE,
        ));
        return;
    }
    for mut sprite in &mut sprites {
        if sprite.image != handle {
            sprite.image = handle.clone();
        }
        if sprite.custom_size != Some(logical) {
            sprite.custom_size = Some(logical);
        }
    }
}

fn update_camera(
    mut rig: ResMut<CameraRig>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    locked: Res<InputLocked>,
    over_ui: Res<PointerOverUi>,
    mut camera: Single<&mut Transform, With<MainCamera>>,
) {
    if !locked.0 && buttons.pressed(MouseButton::Right) {
        rig.yaw -= motion.delta.x as f64 * 0.005;
        rig.pitch = (rig.pitch + motion.delta.y as f64 * 0.005).clamp(-1.55, 1.55);
    }
    let scroll_lines = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y as f64,
        MouseScrollUnit::Pixel => scroll.delta.y as f64 / 40.0,
    };
    // The wheel over a window scrolls it instead.
    let zoom = if locked.0 || over_ui.0 {
        1.0
    } else {
        0.88f64.powf(scroll_lines)
    };

    let Some(origin_body) = frame.origin_body else {
        return;
    };
    let system = &sim.universe.system;
    let vessel = sim.active_vessel();
    let body = system.body(origin_body);

    // Positions here are relative to the render origin (the vessel), in f64.
    let (focus, up, distance) = if rig.map_view {
        rig.map_distance = (rig.map_distance * zoom).clamp(1.2, 5_000.0);
        (
            frame.body_offset(origin_body),
            DVec3::Z,
            rig.map_distance * body.radius,
        )
    } else {
        rig.flight_distance = (rig.flight_distance * zoom).clamp(5.0, 2_000_000.0);
        // The origin is the centre of mass; look a little below the middle of the stack, so
        // the vessel sits above the navball with room for its plume.
        let nose = vessel.attitude_at(system, frame.time) * DVec3::Z;
        let com = vessel.mass_properties().center_of_mass;
        let height = rig.focus_height.unwrap_or(vessel.propulsion.length() * 0.3);
        let focus = nose * (height - com);
        (focus, frame.origin_offset.normalize(), rig.flight_distance)
    };

    // Horizon basis around `up`, anchored to the body's pole so yaw is stable.
    let pole = body.pole();
    let east = pole.cross(up).try_normalize().unwrap_or(DVec3::X);
    let north = up.cross(east);
    let offset = distance
        * (rig.pitch.cos() * (rig.yaw.cos() * north + rig.yaw.sin() * east) + rig.pitch.sin() * up);
    rig.position = focus + offset;

    let eye = to_render(rig.position);
    let target = to_render(focus);
    **camera = Transform::from_translation(eye).looking_at(target, to_render(up));
}
