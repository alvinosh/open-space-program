//! The star background: NASA's Deep Star Map (stars and the Milky Way) as the main
//! camera's skybox, baked by `cargo run -p osp_bake --release -- --sky`. Without the file
//! space stays black.
//!
//! The stars are shown at a fixed screen brightness rather than their real one, which no
//! exposure for a sunlit vessel would show. The sky is drawn over the skybox and fades them by
//! its own brightness (`shaders/sky.wesl`), wherever in the sky you look and whatever the air.

use bevy::{
    camera::Exposure,
    light::Skybox,
    math::DQuat,
    prelude::*,
    render::render_resource::{TextureViewDescriptor, TextureViewDimension},
};
use osp_sim::constants::OBLIQUITY_J2000;

use crate::{FrameSet, camera::MainCamera, content, frame::rotation_to_render};

/// The baked cubemap, relative to the asset root.
const PATH: &str = "core/sky/starmap.png";
/// Screen brightness of the map's white, after exposure.
const BRIGHTNESS: f32 = 1.0;

pub struct StarsPlugin;

impl Plugin for StarsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_starmap).add_systems(
            Update,
            (
                attach_skybox.run_if(resource_exists::<Starmap>),
                keep_stars_steady.in_set(FrameSet::Render),
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

/// Bevy multiplies the skybox by the exposure; undo it so the stars keep their look as the view
/// adapts between day and night.
fn keep_stars_steady(camera: Single<(&mut Skybox, &Exposure), With<MainCamera>>) {
    let (mut skybox, exposure) = camera.into_inner();
    skybox.brightness = BRIGHTNESS / exposure.exposure();
}
