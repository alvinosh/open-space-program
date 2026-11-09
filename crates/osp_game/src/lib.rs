//! Open Space Program: a realism-focused spaceflight game.
//!
//! The simulation lives in `osp_sim` (f64, engine-agnostic), content comes from mods via
//! `osp_mods`, and the UI is data-driven through `osp_ui`. This crate is the game around
//! them: it loads content, steps the universe, keeps the floating origin and the camera rig,
//! takes the player's controls, runs missions and saves, and publishes state for the UI.
//!
//! What it all looks like is `osp_render`'s, and the editor is `osp_editor`'s; both read this
//! crate and are never read by it (the editor's side of things is in [`editing`]). Nothing here
//! may name a render type. The `osp` binary puts them together.

mod alarm_clock;
mod alerts;
pub mod camera;
pub mod content;
pub mod controls;
pub mod dialog;
mod docking_view;
pub mod editing;
mod flight_plan;
pub mod frame;
mod manifest;
mod map_labels;
mod markers;
pub mod menu;
mod mission;
pub mod orbit_lines;
pub mod part_highlight;
pub mod part_info;
pub mod part_pictures;
pub mod profiler;
mod recorder;
pub mod saves;
pub mod settings;
pub mod sim;
pub mod smoke_test;
mod sound;
mod targeting;
mod transfer_planner;
mod tutorial;
mod ui_data;
mod windows;

use bevy::prelude::*;
use osp_ui::UiSet;

/// Per-frame ordering. Everything that reads the floating origin runs after `Origin`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum FrameSet {
    Input,
    Simulate,
    Origin,
    Camera,
    Render,
}

/// The whole game but what it looks like. Add `osp_ui::UiPlugin` before it.
pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
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
        .init_resource::<part_pictures::PartPictures>()
        .init_resource::<part_pictures::PartInHand>()
        .add_plugins((
            settings::SettingsPlugin,
            profiler::ProfilerPlugin,
            recorder::RecorderPlugin,
        ))
        .add_plugins((
            sim::SimPlugin,
            frame::FramePlugin,
            camera::CameraPlugin,
            controls::ControlsPlugin,
            (sound::SoundPlugin, alerts::AlertsPlugin),
            (
                part_info::PartInfoPlugin,
                part_highlight::PartHighlightPlugin,
            ),
            ui_data::UiDataPlugin,
            smoke_test::SmokeTestPlugin,
            (
                menu::MenuPlugin,
                saves::SavesPlugin,
                dialog::DialogPlugin,
                editing::EditingPlugin,
                windows::WindowsPlugin,
                mission::MissionPlugin,
                tutorial::TutorialPlugin,
            ),
        ))
        // Map view and flight planning.
        .add_plugins((
            orbit_lines::OrbitLinesPlugin,
            flight_plan::FlightPlanPlugin,
            map_labels::MapLabelsPlugin,
            markers::MarkersPlugin,
            targeting::TargetingPlugin,
            docking_view::DockingViewPlugin,
            manifest::ManifestPlugin,
            transfer_planner::TransferPlannerPlugin,
            alarm_clock::AlarmClockPlugin,
        ));
    }
}
