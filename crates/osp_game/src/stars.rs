//! The star background: NASA's Deep Star Map (stars and the Milky Way) as the main
//! camera's skybox, baked by `cargo run -p osp_bake --release -- --sky`. Without the file
//! space stays black.
//!
//! The stars are shown at a fixed screen brightness rather than their real one, which no
//! exposure for a sunlit vessel would show. They fade out under a daylit sky; the sky's
//! own haze also covers them, since it is drawn over the skybox.

use bevy::{
    camera::Exposure,
    light::Skybox,
    math::DQuat,
    prelude::*,
    render::render_resource::{TextureViewDescriptor, TextureViewDimension},
};
use osp_sim::constants::OBLIQUITY_J2000;

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera},
    content,
    frame::{RenderFrame, rotation_to_render},
    sim::Sim,
    surface::Surface,
};

/// The baked cubemap, relative to the asset root.
const PATH: &str = "core/sky/starmap.png";
/// Screen brightness of the map's white, after exposure.
const BRIGHTNESS: f32 = 1.0;
/// Heights over which a daylit sky fades to black, m.
const AIR: (f64, f64) = (20_000.0, 70_000.0);

pub struct StarsPlugin;

impl Plugin for StarsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_starmap).add_systems(
            Update,
            (
                attach_skybox.run_if(resource_exists::<Starmap>),
                fade_stars.in_set(FrameSet::Render),
            ),
        );
    }
}

#[derive(Resource)]
struct Starmap(Handle<Image>);

fn load_starmap(mut commands: Commands, assets: Res<AssetServer>) {
    if !content::asset_root().join(PATH).is_file() {
        info!(
            "no star background at {PATH}; bake it with `cargo run -p osp_bake --release -- --sky`"
        );
        return;
    }
    commands.insert_resource(Starmap(assets.load(PATH)));
}

/// Once loaded, turn the six stacked faces into a cube and give it to the camera.
fn attach_skybox(
    mut commands: Commands,
    starmap: Res<Starmap>,
    mut images: ResMut<Assets<Image>>,
    camera: Single<Entity, With<MainCamera>>,
) {
    let Some(mut image) = images.get_mut(&starmap.0) else {
        return;
    };
    if let Err(e) = image.reinterpret_stacked_2d_as_array(6) {
        warn!("star background: {e}");
    }
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    commands.entity(*camera).insert(Skybox {
        image: Some(starmap.0.clone()),
        brightness: 0.0,
        // The map is in equatorial (ICRF) axes; the sim's are ecliptic.
        rotation: rotation_to_render(DQuat::from_rotation_x(-OBLIQUITY_J2000)),
    });
    commands.remove_resource::<Starmap>();
}

/// Full stars in the map view and in space; none under a daylit sky.
fn fade_stars(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    camera: Single<(&mut Skybox, &Exposure), With<MainCamera>>,
) {
    let (mut skybox, exposure) = camera.into_inner();
    let mut visible = 1.0;
    if let (false, Some(origin_body)) = (rig.map_view, frame.origin_body) {
        let body = sim.universe.system.body(origin_body);
        if Surface::is_earthlike(body) {
            let origin = -frame.body_offset(origin_body);
            let to_sun = frame.body_offset(sim.universe.system.root()).normalize();
            let up = origin.normalize();
            let daylight = smoothstep(-0.12, 0.1, up.dot(to_sun));
            let air = 1.0 - smoothstep(AIR.0, AIR.1, origin.length() - body.radius);
            visible = 1.0 - daylight * air;
        }
    }
    // Bevy multiplies the skybox by the exposure; undo it so the stars keep their look as
    // the view adapts between day and night.
    skybox.brightness = BRIGHTNESS * visible as f32 / exposure.exposure();
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
