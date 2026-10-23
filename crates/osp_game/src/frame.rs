//! Floating origin and the mapping from simulation space to render space.
//!
//! The simulation works in f64, Z-up, body-relative frames. Bevy renders in f32, Y-up.
//! Each frame we choose an origin: the vessel the camera looks at (the active one unless
//! another is followed), expressed as (body, offset) rather than in heliocentric
//! coordinates. Every body's centre is then computed relative to that origin through the
//! frame tree, in f64, and only the final small differences are converted to f32.
//! Precision near the camera is therefore the same at Earth, Neptune or anywhere in
//! between.
//!
//! The sky itself is drawn by `atmosphere`; the clear colour is black space.

use std::f64::consts::FRAC_PI_2;

use bevy::{
    light::{
        CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLightShadowMap,
        light_consts::lux,
    },
    math::{DQuat, DVec3},
    prelude::*,
};
use osp_sim::{body::BodyId, time::SimTime};

use crate::{
    FrameSet,
    camera::CameraRig,
    settings::{GraphicsSettings, Quality},
    sim::Sim,
};

pub struct FramePlugin;

impl Plugin for FramePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderFrame>()
            .insert_resource(ClearColor(Color::BLACK))
            .insert_resource(GlobalAmbientLight {
                color: Color::WHITE,
                brightness: 1_500.0,
                ..default()
            })
            .add_systems(Startup, spawn_sunlight)
            .add_systems(Update, update_frame.in_set(FrameSet::Origin))
            .add_systems(Update, update_sunlight.in_set(FrameSet::Render))
            .add_systems(
                Update,
                apply_shadow_settings.run_if(resource_changed::<GraphicsSettings>),
            );
    }
}

/// Everything needed to place simulation objects in render space this frame.
#[derive(Resource, Default)]
pub struct RenderFrame {
    /// The instant being rendered.
    pub time: SimTime,
    /// Body whose frame the origin is expressed in.
    pub origin_body: Option<BodyId>,
    /// Origin position relative to `origin_body`.
    pub origin_offset: DVec3,
    /// Each body's centre relative to the origin (sim axes, f64), indexed by `BodyId`.
    body_offsets: Vec<DVec3>,
}

impl RenderFrame {
    /// Position of a body's centre relative to the origin, f64 sim axes.
    pub fn body_offset(&self, body: BodyId) -> DVec3 {
        self.body_offsets[body.0]
    }

    /// Point given relative to `body`, relative to the origin (f64 sim axes).
    pub fn relative(&self, body: BodyId, local: DVec3) -> DVec3 {
        self.body_offsets[body.0] + local
    }

    /// Point given relative to `body`, in render space.
    pub fn to_render(&self, body: BodyId, local: DVec3) -> Vec3 {
        to_render(self.relative(body, local))
    }
}

/// Sim (Z-up) vector to render (Y-up) vector.
pub fn to_render(v: DVec3) -> Vec3 {
    Vec3::new(v.x as f32, v.z as f32, -v.y as f32)
}

/// Sim rotation to render rotation.
pub fn rotation_to_render(q: DQuat) -> Quat {
    let basis = DQuat::from_rotation_x(-FRAC_PI_2);
    (basis * q * basis.inverse()).as_quat()
}

fn update_frame(sim: Res<Sim>, rig: Res<CameraRig>, mut frame: ResMut<RenderFrame>) {
    let time = sim.render_time;
    let system = &sim.universe.system;
    let vessel = rig.viewed_vessel(&sim);
    let origin_body = vessel.parent();
    let origin_offset = vessel.state_at(system, time).position;

    frame.time = time;
    frame.origin_body = Some(origin_body);
    frame.origin_offset = origin_offset;
    frame.body_offsets.clear();
    frame.body_offsets.extend(
        system
            .bodies()
            .iter()
            .map(|body| system.relative_state(body.id, origin_body, time).position - origin_offset),
    );
}

#[derive(Component)]
pub struct Sunlight;

fn spawn_sunlight(mut commands: Commands) {
    commands.spawn((
        Sunlight,
        DirectionalLight {
            illuminance: lux::RAW_SUNLIGHT,
            ..default()
        },
        Transform::default(),
    ));
}

/// Sun shadows for the `shadows` setting: shadow map size, cascades and how far they
/// reach. Shadows only matter near the camera: the vessel, the pad and the trees around it.
fn apply_shadow_settings(
    settings: Res<GraphicsSettings>,
    mut map: ResMut<DirectionalLightShadowMap>,
    sun: Single<(&mut DirectionalLight, &mut CascadeShadowConfig), With<Sunlight>>,
) {
    let (size, num_cascades, maximum_distance) = match settings.shadows {
        Quality::Off => (map.size, 1, 0.0),
        Quality::Low => (1024, 2, 1_000.0),
        Quality::Medium => (2048, 3, 2_500.0),
        Quality::High => (2048, 4, 4_000.0),
        Quality::Ultra => (4096, 4, 4_000.0),
    };
    let (mut light, mut cascades) = sun.into_inner();
    light.shadow_maps_enabled = settings.shadows != Quality::Off;
    if map.size != size {
        map.size = size;
    }
    if light.shadow_maps_enabled {
        *cascades = CascadeShadowConfigBuilder {
            num_cascades,
            minimum_distance: 0.5,
            first_cascade_far_bound: 40.0,
            maximum_distance,
            overlap_proportion: 0.2,
        }
        .build();
    }
}

pub fn update_sunlight(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    mut light: Single<&mut Transform, With<Sunlight>>,
) {
    // The root body is the star; light travels from it towards the origin.
    let sun = sim.universe.system.root();
    let direction = to_render(-frame.body_offset(sun)).normalize_or(Vec3::NEG_Y);
    let up = if direction.y.abs() > 0.99 {
        Vec3::X
    } else {
        Vec3::Y
    };
    **light = Transform::default().looking_to(direction, up);
}
