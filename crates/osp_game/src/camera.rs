//! Orbit camera around the active vessel (flight view) or its parent body (map view), or
//! around what the player follows instead (from the Universe Manifest): another vessel, or
//! in map view a body. Following ends when the player flies another vessel, the followed
//! vessel is gone, or (for a body) the map closes. The flight camera stays above the ground
//! (or the sea): looking down from low, it stops at the ground instead of going under.
//!
//! The view goes to the window, or to an image for scripted captures ([`ViewOutput`]).
//! Below 100% render scale (see `settings`) it renders into an image that much smaller,
//! and a 2D camera stretches that over the output under the UI, which stays at full
//! resolution.

use bevy::{
    anti_alias::{fxaa::Fxaa, taa::TemporalAntiAliasing},
    camera::{Exposure, Hdr, ImageRenderTarget, RenderTarget, visibility::RenderLayers},
    core_pipeline::prepass::{DepthPrepass, MotionVectorPrepass, NormalPrepass},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    math::DVec3,
    pbr::{ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel},
    post_process::bloom::Bloom,
    prelude::*,
    render::{
        camera::{MipBias, TemporalJitter},
        render_resource::TextureFormat,
        view::{Msaa, Tonemapping},
    },
    window::PrimaryWindow,
};

use osp_sim::{body::BodyId, vessel::Vessel, vessel::VesselId};
use osp_ui::PointerOverUi;

use crate::{
    FrameSet,
    controls::InputLocked,
    frame::{RenderFrame, to_render},
    settings::{Antialiasing, GraphicsSettings, Quality},
    sim::Sim,
};

/// Render layer of the scaled view's composite (keeps gizmos and the scene out of it).
const COMPOSITE_LAYER: usize = 8;
/// Steepest the camera looks down or up, rad.
const MAX_PITCH: f64 = 1.55;
/// The flight camera stays at least this high above the ground, m.
const GROUND_CLEARANCE: f64 = 2.0;
/// How far ambient occlusion looks for occluders, m: about a landing leg's gap to the hull.
const AO_RADIUS: f32 = 1.2;

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
                    apply_view_settings,
                    reset_history_on_cuts,
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

/// What the camera looks at instead of the active vessel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Follow {
    Vessel(VesselId),
    /// In map view only.
    Body(BodyId),
}

#[derive(Resource)]
pub struct CameraRig {
    pub map_view: bool,
    /// Looking at something other than the active vessel.
    pub follow: Option<Follow>,
    /// The active vessel when following started.
    flown: Option<VesselId>,
    /// Radians around the local vertical.
    yaw: f64,
    /// Elevation above the local horizon, radians.
    pitch: f64,
    flight_distance: f64,
    /// Map distance as a multiple of the focused body's radius.
    pub map_distance: f64,
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
            follow: None,
            flown: None,
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

impl CameraRig {
    /// The vessel the view is centred on: the one followed, or else the active one.
    pub fn viewed_vessel<'a>(&self, sim: &'a Sim) -> &'a Vessel {
        match self.follow {
            Some(Follow::Vessel(id)) => sim.universe.vessel(id),
            _ => None,
        }
        .unwrap_or_else(|| sim.active_vessel())
    }

    /// The body the map view is centred on: the one followed, or else the viewed vessel's
    /// parent. `None` in flight view.
    pub fn map_centre(&self, sim: &Sim) -> Option<BodyId> {
        self.map_view.then(|| match self.follow {
            Some(Follow::Body(body)) => body,
            _ => self.viewed_vessel(sim).parent(),
        })
    }

    /// Follow `what` (`None`: the active vessel again); following a body opens the map.
    pub fn set_follow(&mut self, what: Option<Follow>, sim: &Sim) {
        self.follow = what.filter(|&f| f != Follow::Vessel(sim.active_id()));
        self.flown = Some(sim.active_id());
        if matches!(what, Some(Follow::Body(_))) {
            self.map_view = true;
        }
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

/// Anti-aliasing, ambient occlusion and bloom. The map view uses FXAA instead of TAA: its
/// orbit lines are a pixel wide, and TAA would smear them as the camera turns. Ambient
/// occlusion needs multisampling off.
///
/// No contact shadows: on the faceted scenery they shadow the next facet over wherever the
/// sun grazes it, and the shadow maps are fine enough near the vessel anyway.
fn apply_view_settings(
    mut commands: Commands,
    settings: Res<GraphicsSettings>,
    rig: Res<CameraRig>,
    camera: Single<Entity, With<MainCamera>>,
    mut applied_for_map: Local<Option<bool>>,
) {
    if !settings.is_changed() && *applied_for_map == Some(rig.map_view) {
        return;
    }
    *applied_for_map = Some(rig.map_view);
    let mut antialiasing = settings.antialiasing;
    if rig.map_view && antialiasing == Antialiasing::Taa {
        antialiasing = Antialiasing::Fxaa;
    }
    let msaa = antialiasing == Antialiasing::Msaa;

    let mut camera = commands.entity(*camera);
    // Start from none of it: removing TAA or AO leaves behind the jitter and prepasses they
    // brought in.
    camera.remove::<(
        Fxaa,
        TemporalAntiAliasing,
        TemporalJitter,
        MipBias,
        ScreenSpaceAmbientOcclusion,
        DepthPrepass,
        NormalPrepass,
        MotionVectorPrepass,
    )>();
    camera.insert(if msaa { Msaa::Sample4 } else { Msaa::Off });
    match antialiasing {
        Antialiasing::Fxaa => {
            camera.insert(Fxaa::default());
        }
        Antialiasing::Taa => {
            camera.insert(TemporalAntiAliasing::default());
        }
        Antialiasing::Off | Antialiasing::Msaa => {}
    }
    let ao_level = match settings.ambient_occlusion {
        Quality::Off => None,
        Quality::Low => Some(ScreenSpaceAmbientOcclusionQualityLevel::Low),
        Quality::Medium => Some(ScreenSpaceAmbientOcclusionQualityLevel::Medium),
        Quality::High => Some(ScreenSpaceAmbientOcclusionQualityLevel::High),
        Quality::Ultra => Some(ScreenSpaceAmbientOcclusionQualityLevel::Ultra),
    };
    if let Some(quality_level) = ao_level.filter(|_| !msaa) {
        camera.insert(ScreenSpaceAmbientOcclusion {
            quality_level,
            radius: AO_RADIUS,
            ..default()
        });
    }
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

/// Start TAA's history afresh after a cut, when what's in view jumps: another vessel, a
/// followed body, or the origin moving to another body.
fn reset_history_on_cuts(
    sim: Res<Sim>,
    rig: Res<CameraRig>,
    frame: Res<RenderFrame>,
    mut taa: Query<&mut TemporalAntiAliasing, With<MainCamera>>,
    mut last: Local<Option<(VesselId, Option<Follow>, Option<BodyId>)>>,
) {
    let view = (sim.active_id(), rig.follow, frame.origin_body);
    if last.replace(view).is_some_and(|last| last != view) {
        for mut taa in &mut taa {
            taa.reset = true;
        }
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
        rig.yaw += motion.delta.x as f64 * 0.005;
        rig.pitch = (rig.pitch + motion.delta.y as f64 * 0.005).clamp(-MAX_PITCH, MAX_PITCH);
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

    let follow_ends = match rig.follow {
        Some(Follow::Vessel(id)) => sim.universe.vessel(id).is_none(),
        Some(Follow::Body(_)) => !rig.map_view,
        None => false,
    };
    if follow_ends || rig.flown != Some(sim.active_id()) {
        rig.follow = None;
        rig.flown = Some(sim.active_id());
    }
    let Some(origin_body) = frame.origin_body else {
        return;
    };
    let system = &sim.universe.system;
    let vessel = rig.viewed_vessel(&sim);
    let centre = rig.map_centre(&sim).unwrap_or(origin_body);
    let body = system.body(centre);

    // Positions here are relative to the render origin, in f64: the vessel in flight view.
    let (focus, up, distance) = if rig.map_view {
        rig.map_distance = (rig.map_distance * zoom).clamp(1.2, 5_000.0);
        (
            frame.body_offset(centre),
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
    let yaw = rig.yaw;
    let place = |pitch: f64| {
        focus + distance * (pitch.cos() * (yaw.cos() * north + yaw.sin() * east) + pitch.sin() * up)
    };
    rig.position = place(rig.pitch);
    if !rig.map_view {
        // Height above the ground of a point relative to the origin.
        let to_fixed = body.orientation(frame.time).inverse();
        let clearance = |at: DVec3| {
            let at = frame.origin_offset + at;
            at.length() - body.radius - body.terrain_height(to_fixed * at.normalize())
        };
        rig.pitch = pitch_above_ground(rig.pitch, |pitch| clearance(place(pitch)));
        rig.position = place(rig.pitch);
        // Nothing clears it (the vessel is down in a hole): straight up out of the ground.
        let short = GROUND_CLEARANCE - clearance(rig.position);
        if short > 0.0 {
            let vertical = (frame.origin_offset + rig.position).normalize();
            rig.position += vertical * short;
        }
    }

    let eye = to_render(rig.position);
    let target = to_render(focus);
    **camera = Transform::from_translation(eye).looking_at(target, to_render(up));
}

/// The lowest pitch from `pitch` up to [`MAX_PITCH`] at which the camera is
/// [`GROUND_CLEARANCE`] above the ground, `clearance` giving its height above the ground at
/// a pitch: `pitch` itself if it is high enough already, or if no pitch is.
fn pitch_above_ground(pitch: f64, clearance: impl Fn(f64) -> f64) -> f64 {
    if clearance(pitch) >= GROUND_CLEARANCE || clearance(MAX_PITCH) < GROUND_CLEARANCE {
        return pitch;
    }
    // Bisect between too low and high enough, to a few hundredths of a degree.
    let (mut low, mut high) = (pitch, MAX_PITCH);
    for _ in 0..16 {
        let middle = (low + high) / 2.0;
        if clearance(middle) >= GROUND_CLEARANCE {
            high = middle;
        } else {
            low = middle;
        }
    }
    high
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_camera_stops_at_the_ground() {
        // Looking at a point 10 m up from 100 m away over flat ground.
        let clearance = |pitch: f64| 10.0 + 100.0 * pitch.sin();
        // Above the ground already: left alone.
        assert_eq!(pitch_above_ground(0.3, clearance), 0.3);
        assert_eq!(pitch_above_ground(-0.05, clearance), -0.05);
        // Below it: raised until it just clears.
        let pitch = pitch_above_ground(-1.2, clearance);
        let lowest = ((GROUND_CLEARANCE - 10.0) / 100.0).asin();
        assert!(
            pitch >= lowest && pitch - lowest < 1e-3,
            "{pitch} vs {lowest}"
        );
        // In a hole nothing clears: unchanged (the caller lifts the camera instead).
        assert_eq!(
            pitch_above_ground(-0.4, |p: f64| -50.0 + 10.0 * p.sin()),
            -0.4
        );
    }
}
