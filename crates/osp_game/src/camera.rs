//! Orbit camera around the active vessel (flight view) or its parent body (map view).

use bevy::{
    camera::{Exposure, Hdr},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    math::DVec3,
    post_process::bloom::Bloom,
    prelude::*,
};

use osp_ui::PointerOverUi;

use crate::{
    FrameSet,
    controls::InputLocked,
    frame::{RenderFrame, to_render},
    sim::Sim,
};

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraRig>()
            .add_systems(Startup, spawn_camera)
            .add_systems(Update, update_camera.in_set(FrameSet::Camera));
    }
}

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
        // Glow around what is much brighter than the scene: plumes, the sun on metal.
        Bloom {
            intensity: 0.08,
            ..Bloom::NATURAL
        },
        Transform::default(),
    ));
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
