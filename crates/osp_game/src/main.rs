//! Open Space Program: a realism-focused spaceflight game.
//!
//! The simulation lives in `osp_sim` (f64, engine-agnostic), content comes from mods via
//! `osp_mods`, and the UI is data-driven through `osp_ui`. This crate wires them
//! together: it loads content, steps the universe, renders it relative to a floating
//! origin and publishes state for the UI.

mod atmosphere;
mod camera;
mod content;
mod controls;
mod frame;
mod launch_site;
mod meshgen;
mod navball;
mod orbit_lines;
mod planet;
mod sim;
mod smoke_test;
mod surface;
mod terrain_material;
mod trees;
mod ui_data;
mod vessel_render;

use bevy::{
    app::{TaskPoolOptions, TaskPoolThreadAssignmentPolicy},
    prelude::*,
};
use osp_ui::{UiPlugin, UiSet};

/// Per-frame ordering. Everything that reads the floating origin runs after `Origin`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum FrameSet {
    Input,
    Simulate,
    Origin,
    Camera,
    Render,
}

fn main() -> AppExit {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Open Space Program".into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: content::asset_root().to_string_lossy().into_owned(),
                    ..default()
                })
                .set(TaskPoolPlugin {
                    task_pool_options: TaskPoolOptions {
                        // Terrain chunks and tree cells are built on the async compute pool;
                        // Bevy's default (a quarter of the cores, at most four) makes the
                        // ground load slowly.
                        async_compute: TaskPoolThreadAssignmentPolicy {
                            min_threads: 2,
                            max_threads: 8,
                            percent: 0.5,
                            on_thread_spawn: None,
                            on_thread_destroy: None,
                        },
                        ..default()
                    },
                }),
        )
        .add_plugins(UiPlugin)
        .configure_sets(
            Update,
            (
                UiSet::Input,
                FrameSet::Input,
                FrameSet::Simulate,
                FrameSet::Origin,
                FrameSet::Camera,
                FrameSet::Render,
                UiSet::Build,
                UiSet::Update,
            )
                .chain(),
        )
        // Content first: everything else is built from the loaded mods.
        .add_plugins(content::ContentPlugin)
        .add_plugins((
            sim::SimPlugin,
            frame::FramePlugin,
            camera::CameraPlugin,
            controls::ControlsPlugin,
            terrain_material::TerrainMaterialPlugin,
            planet::PlanetPlugin,
            atmosphere::AtmospherePlugin,
            trees::TreesPlugin,
            launch_site::LaunchSitePlugin,
            vessel_render::VesselRenderPlugin,
            navball::NavballPlugin,
            orbit_lines::OrbitLinesPlugin,
            ui_data::UiDataPlugin,
            smoke_test::SmokeTestPlugin,
        ))
        .add_plugins(diagnostics)
        .run()
}

/// `OSP_FPS=1` logs frame times every second.
fn diagnostics(app: &mut App) {
    if std::env::var_os("OSP_FPS").is_some() {
        app.add_plugins((
            bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
            bevy::diagnostic::LogDiagnosticsPlugin::default(),
        ));
    }
}
