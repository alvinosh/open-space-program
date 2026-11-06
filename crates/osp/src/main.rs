//! Open Space Program: the game (`osp_game`), its editor (`osp_editor`) and what it looks like
//! (`osp_render`), in a window. `--dev` (or `OSP_DEV=1`, or `cargo dev`) draws it with the dev version's plain
//! graphics. `--watch <mission>` flies one of the smoke tests' missions in it, to watch (see
//! [`watch`]).

mod watch;

use bevy::{
    app::{TaskPoolOptions, TaskPoolThreadAssignmentPolicy},
    asset::io::AssetSourceBuilder,
    prelude::*,
};
use osp_editor::EditorPlugin;
use osp_game::{GamePlugin, content, settings};
use osp_render::RenderPlugin;
use osp_ui::UiPlugin;

fn main() -> AppExit {
    let watched = watch::requested();
    if let Some(mission) = watched {
        // The game starts in the mission's scenario, straight into flight, while its script loads.
        // SAFETY: nothing has started yet that could read the environment at the same time.
        unsafe { std::env::set_var("OSP_SCENARIO", mission.scenario) };
    }
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
                        title: if settings::dev_version() {
                            "Open Space Program (dev)"
                        } else {
                            "Open Space Program"
                        }
                        .into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::log::LogPlugin {
                    // `--features profile`: time every system for the profiler.
                    #[cfg(feature = "profile")]
                    custom_layer: osp_game::profiler::spans::layer,
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
        .add_plugins(GamePlugin)
        .add_plugins(EditorPlugin)
        .add_plugins(RenderPlugin)
        .add_plugins(diagnostics)
        .add_plugins(watch::WatchPlugin(watched))
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
