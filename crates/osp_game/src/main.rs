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
mod flight_plan;
mod frame;
mod hull;
mod launch_site;
mod map_labels;
mod menu;
mod meshgen;
mod navball;
mod orbit_lines;
mod planet;
mod plume;
mod profiler;
mod settings;
mod sim;
mod smoke_test;
mod surface;
mod targeting;
mod terrain_material;
mod transfer_planner;
mod trees;
mod ui_data;
mod vessel_render;

use bevy::{
    app::{TaskPoolOptions, TaskPoolThreadAssignmentPolicy},
    asset::io::AssetSourceBuilder,
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
        // Models and other files that mods ship (`mods://<mod>/...`); must exist before the
        // asset plugin.
        .register_asset_source(
            "mods",
            AssetSourceBuilder::platform_default(&content::mods_root().to_string_lossy(), None),
        )
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Open Space Program".into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::log::LogPlugin {
                    // `--features profile`: time every system for the profiler.
                    #[cfg(feature = "profile")]
                    custom_layer: profiler::spans::layer,
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
        .add_plugins((settings::SettingsPlugin, profiler::ProfilerPlugin))
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
            plume::PlumePlugin,
            vessel_render::VesselRenderPlugin,
            navball::NavballPlugin,
            ui_data::UiDataPlugin,
            smoke_test::SmokeTestPlugin,
            menu::MenuPlugin,
        ))
        // Map view and flight planning.
        .add_plugins((
            orbit_lines::OrbitLinesPlugin,
            flight_plan::FlightPlanPlugin,
            map_labels::MapLabelsPlugin,
            targeting::TargetingPlugin,
            transfer_planner::TransferPlannerPlugin,
        ))
        .add_plugins(diagnostics)
        .run()
}

/// `OSP_FPS=1` logs frame times every second. `OSP_UNCAPPED=1` runs as fast as it can, for
/// measuring: no vsync, and no 60 Hz cap while the window is unfocused.
fn diagnostics(app: &mut App) {
    if std::env::var_os("OSP_FPS").is_some() {
        app.add_plugins((
            bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
            bevy::diagnostic::LogDiagnosticsPlugin::default(),
        ));
    }
    if std::env::var_os("OSP_UNCAPPED").is_some() {
        app.insert_resource(bevy::winit::WinitSettings {
            focused_mode: bevy::winit::UpdateMode::Continuous,
            unfocused_mode: bevy::winit::UpdateMode::Continuous,
        })
        .add_systems(Startup, |mut window: Single<&mut Window>| {
            window.present_mode = bevy::window::PresentMode::AutoNoVsync;
        });
    }
}
