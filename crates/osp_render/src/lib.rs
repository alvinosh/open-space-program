//! Everything the game looks like: planets and their terrain, the sky, clouds and sea,
//! vessels with their materials, plumes and trails, the launch site, trees, the navball and the
//! pictures of parts in the editor's parts list.
//!
//! It reads the game (`osp_game`: the simulation, the floating origin, the camera rig, the
//! settings) and never the other way round, so the game builds and runs without it, and
//! nothing here can change how anything flies. [`RenderPlugin`] adds it all: the full
//! graphics, or the dev version's plain look (`--dev`, see [`dev_graphics`]).

mod atmosphere;
mod cloud_puffs;
mod clouds;
pub mod dev_graphics;
mod eclipse;
mod exhaust_trail;
mod launch_site;
mod meshgen;
mod navball;
mod ocean;
mod paint_layers;
mod part_mesh;
mod part_pictures;
mod planet;
mod plasma;
mod plume;
mod sky_light;
mod space_centre;
mod stars;
mod surface;
mod terrain_layers;
mod terrain_material;
mod trees;
mod vessel_material;
mod vessel_render;
pub mod warmup;

use bevy::prelude::*;
// What is read from the game, under the same paths it has there.
use osp_game::{FrameSet, camera, content, frame, part_info, settings, sim};

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            warmup::WarmupPlugin,
            planet::PlanetPlugin,
            graphics,
            trees::TreesPlugin,
            launch_site::LaunchSitePlugin,
            space_centre::SpaceCentrePlugin,
            vessel_render::VesselRenderPlugin,
            part_pictures::PartPicturesPlugin,
            navball::NavballPlugin,
        ));
    }
}

/// Every custom pass and shader: the terrain's, the sky and haze, clouds, eclipses, the sea's
/// waves, re-entry glow, the sky's light, the stars, plume volumes and smoke trails. Or, in the
/// dev version (`cargo dev`), none of them and the plain look of `dev_graphics` instead. The
/// rest of the renderer must not need anything from them.
fn graphics(app: &mut App) {
    if dev_graphics::on() {
        app.add_plugins(dev_graphics::DevGraphicsPlugin);
        return;
    }
    app.add_plugins((
        (
            terrain_material::TerrainMaterialPlugin,
            terrain_layers::TerrainLayersPlugin,
        ),
        (
            atmosphere::AtmospherePlugin,
            clouds::CloudsPlugin,
            eclipse::EclipsePlugin,
            ocean::OceanPlugin,
            plasma::PlasmaPlugin,
            cloud_puffs::CloudPuffsPlugin,
            sky_light::SkyLightPlugin,
            stars::StarsPlugin,
        ),
        plume::PlumePlugin,
        exhaust_trail::ExhaustTrailPlugin,
    ));
}
