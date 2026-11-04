//! Scripted smoke test: `OSP_CAPTURE=<dir> cargo run` launches the rocket, saves a few
//! screenshots into `<dir>` and exits. Handy for checking rendering without a human.
//!
//! The camera renders into an offscreen image so captures work even when the window is
//! hidden or occluded (macOS stops presenting occluded windows).

use std::path::PathBuf;

use bevy::{
    camera::{ImageRenderTarget, NormalizedRenderTarget},
    ecs::system::SystemParam,
    math::{DVec2, DVec3},
    picking::{
        backend::prelude::PointerId,
        pointer::{Location, PointerAction, PointerButton, PointerInput},
    },
    prelude::*,
    render::{
        render_resource::TextureFormat,
        view::screenshot::{Screenshot, save_to_disk},
    },
    ui::UiGlobalTransform,
};

use osp_sim::{
    attitude::SasMode, prediction::PatchEnd, propulsion::Propulsion, staging::Activation,
};
use osp_ui::{
    ActionSource, ContextAction, DragPayload, DropAction, Formatters, Template, UiAction, UiData,
};

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera, ViewOutput},
    controls::InputLocked,
    editor::Editor,
    frame::{RenderFrame, to_render},
    map_labels::{body_spheres, hidden},
    menu::Menu,
    orbit_lines::{Prediction, ScriptedPointer, project},
    sim::{Sim, TimeWarp},
};

/// Size of the captured image; `OSP_CAPTURE_SIZE=<w>x<h>` overrides it (checking the UI
/// on small screens).
const CAPTURE_SIZE: (u32, u32) = (1600, 900);

fn capture_size() -> (u32, u32) {
    std::env::var("OSP_CAPTURE_SIZE")
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .unwrap_or(CAPTURE_SIZE)
}

pub struct SmokeTestPlugin;

impl Plugin for SmokeTestPlugin {
    fn build(&self, app: &mut App) {
        if let Some(dir) = std::env::var_os("OSP_CAPTURE") {
            let in_orbit = std::env::var("OSP_SCENARIO").is_ok_and(|s| s != "pad");
            let times: Vec<f64> = std::env::var("OSP_CAPTURE_AT")
                .unwrap_or_default()
                .split(',')
                .filter_map(|t| t.trim().parse().ok())
                .collect();
            let mut actions = if let Some(last) = times.last() {
                // Launch, then stills at the given times (debugging the ascent).
                let mut actions = vec![
                    (0.5, Action::MapView(false)),
                    (1.0, Action::Ui("ui.show:attitude_info")),
                    (1.0, Action::Ui("ui.show:vessel_info")),
                    (1.0, Action::Ui("ui.show:structure_info")),
                    (3.5, Action::Launch),
                ];
                actions.extend_from_slice(PITCH_KICK);
                // `OSP_CAPTURE_STAGE_AT=<s>,...`: stage at those times too.
                let stages = std::env::var("OSP_CAPTURE_STAGE_AT").unwrap_or_default();
                actions.extend(
                    stages
                        .split(',')
                        .filter_map(|t| t.trim().parse().ok())
                        .map(|at| (at, Action::Section)),
                );
                for (k, &at) in times.iter().enumerate() {
                    let name: &'static str = Box::leak(format!("view{k}.png").into_boxed_str());
                    actions.push((at, Action::Capture(name)));
                }
                actions.push((last + 3.0, Action::Exit));
                actions.sort_by(|a, b| a.0.total_cmp(&b.0));
                actions
            } else if std::env::var_os("OSP_CAPTURE_QUICK").is_some() {
                QUICK_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_PLAN").is_some() {
                PLAN_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_MAP_UI").is_some() {
                MAP_UI_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_RENDEZVOUS").is_some() {
                RENDEZVOUS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_SAS").is_some() {
                SAS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_DOCKING").is_some() {
                DOCKING_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_DEORBIT").is_some() {
                DEORBIT_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_TRANSFER").is_some() {
                TRANSFER_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_MANIFEST").is_some() {
                MANIFEST_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_MARKERS").is_some() {
                MARKERS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_MARS").is_some() {
                MARS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_DEBRIS").is_some() {
                DEBRIS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_PLAY").is_some() {
                PLAY_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_TUTORIAL").is_some() {
                TUTORIAL_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_CLEAR").is_some() {
                CLEAR_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_EDITOR_VIEW").is_some() {
                EDITOR_VIEW_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_MENUS").is_some() {
                MENUS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_MENU_TUTORIAL").is_some() {
                MENU_TUTORIAL_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_MISSION").is_some() {
                MISSION_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_HOVER").is_some() {
                HOVER_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_PARTS").is_some() {
                PARTS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_SETTINGS").is_some() {
                SETTINGS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_STAGING_POINTER").is_some() {
                STAGING_POINTER_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_STAGING").is_some() {
                STAGING_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_GLIDER_RUNWAY").is_some() {
                GLIDER_RUNWAY_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_MENU").is_some() {
                MENU_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_GRAPHICS").is_some() {
                GRAPHICS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_RUNWAY").is_some() {
                RUNWAY_SCRIPT.to_vec()
            } else if in_orbit {
                ORBIT_SCRIPT.to_vec()
            } else {
                PAD_SCRIPT.to_vec()
            };
            // `OSP_CAPTURE_PART=<vessel>-<part>`: that part's window open from the start.
            if let Ok(part) = std::env::var("OSP_CAPTURE_PART") {
                let show: &'static str =
                    Box::leak(format!("ui.show:part_info:{part}").into_boxed_str());
                actions.insert(0, (0.8, Action::Ui(show)));
            }
            // `OSP_CAPTURE_HIDE_UI=1`: the vehicle and world only (checking part models).
            if std::env::var_os("OSP_CAPTURE_HIDE_UI").is_some() {
                actions.insert(0, (0.6, Action::Ui("ui.hide_group:flight")));
            }
            app.insert_resource(Script {
                dir: PathBuf::from(dir),
                step: 0,
                target: Handle::default(),
                actions,
                ui_pointer: Vec2::ZERO,
            })
            .add_systems(PostStartup, render_offscreen)
            .add_systems(
                Update,
                run_script.after(FrameSet::Input).before(FrameSet::Simulate),
            );
        }
    }
}

#[derive(Resource)]
struct Script {
    dir: PathBuf,
    step: usize,
    target: Handle<Image>,
    actions: Vec<(f64, Action)>,
    /// Where the pointer the UI sees is, physical px.
    ui_pointer: Vec2,
}

fn render_offscreen(
    mut commands: Commands,
    mut script: ResMut<Script>,
    mut images: ResMut<Assets<Image>>,
    mut locked: ResMut<InputLocked>,
) {
    locked.0 = true;
    let (width, height) = capture_size();
    let image = Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
    script.target = images.add(image);
    // The view and the UI go into the image instead of the window (see `camera`), so
    // captures include the HUD and the render scale.
    commands.insert_resource(ViewOutput(Some(script.target.clone())));
}

#[derive(Clone, Copy)]
enum Action {
    Capture(&'static str),
    /// Throttle up, brakes off, SAS on and stage.
    Launch,
    /// Fire the next activation sequence.
    Section,
    /// Rearrange the sequences as the staging panel does (a `staging.*` action).
    Sequences(&'static str),
    /// Pilot rotation input (pitch up, yaw right, roll right).
    Steer(DVec3),
    Sas(SasMode),
    /// RCS on and translating (right, down, forward), or off.
    Rcs(Option<DVec3>),
    /// Fire a UI action, e.g. to open a window.
    Ui(&'static str),
    MapView(bool),
    /// Map camera distance, in radii of the body it looks at.
    MapDistance(f64),
    /// Index into `WARP_RATES`.
    Warp(usize),
    /// Put the map's scripted pointer on the planned trajectory this many seconds ahead (or
    /// the first point in sight after that), or on planned burn `i`'s node.
    PointAt(f64),
    PointAtBurn(usize),
    /// Put the scripted pointer on the port the active vessel would dock with.
    PointAtTargetPort,
    /// Press or release a mouse button (where the scripted pointer is).
    Press(MouseButton),
    Release(MouseButton),
    /// Put the active vessel this far off its target's port, facing it, closing this fast.
    LineUpPorts(f64, f64),
    /// Let go of what has docked with the active vessel.
    Undock,
    /// Let the actions through, as they are when a player flies.
    Unlock,
    /// Move the pointer the UI sees onto the middle of the node with this action, `drag`
    /// payload, `drop` or `context` as they read now, or that start with this if it ends in
    /// `:` (the capture has no real pointer).
    UiPointAt(&'static str),
    /// The same, near its left edge: on a row, its number rather than one of its tiles.
    UiPointAtEdge(&'static str),
    /// Press or release that pointer's button.
    UiPress(PointerButton),
    UiRelease(PointerButton),
    /// Stop the capture with a failure unless the sequences start like this: `<kind>x<count>`
    /// for each group, `,` between groups, ` | ` between sequences, the next first.
    ExpectSequences(&'static str),
    /// Stop the capture with a failure unless the game holds this many vessels, debris left out,
    /// and the one being flown is called this.
    ExpectGame(usize, &'static str),
    /// Stop the capture with a failure unless the active vessel's sequences number this many.
    ExpectSequenceCount(usize),
    /// Stop the capture with a failure unless the menu is open on this screen, or closed for "".
    ExpectScreen(&'static str),
    /// Turn the editor's view by this many pixels of mouse movement, and zoom by these lines of
    /// the wheel.
    EditorLook(f32, f32, f32),
    /// Stop the capture with a failure unless the editor's list of vessels is out or away.
    ExpectList(bool),
    /// Stop the capture with a failure unless the editor is open with this craft on show, "" for
    /// an empty hangar, or closed for `None`.
    ExpectEditor(Option<&'static str>),
    Exit,
}

/// (real seconds since start, action)
const ORBIT_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (3.0, Action::Capture("orbit.png")),
    (3.1, Action::Rcs(Some(DVec3::Z))),
    (3.4, Action::Capture("orbit_rcs.png")),
    (3.5, Action::Rcs(None)),
    (3.5, Action::MapView(true)),
    (5.0, Action::Capture("orbit_map.png")),
    // On rails SAS snaps onto its target and follows it around the orbit.
    (5.2, Action::Sas(SasMode::Retrograde)),
    (5.5, Action::Warp(9)),
    (15.0, Action::Capture("orbit_warped.png")),
    (15.5, Action::MapView(false)),
    (16.5, Action::Capture("orbit_retrograde.png")),
    (17.0, Action::Exit),
];

/// `OSP_SCENARIO=glider_runway OSP_CAPTURE_GLIDER_RUNWAY=1`: the glider on its dolly, the
/// boosters lit, rotated off the runway, past burnout, then rid of boosters and wheels.
const GLIDER_RUNWAY_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (3.0, Action::Capture("parked.png")),
    (3.2, Action::Launch),
    (5.0, Action::Capture("lit.png")),
    (7.5, Action::Steer(DVec3::new(0.12, 0.0, 0.0))),
    (9.0, Action::Steer(DVec3::ZERO)),
    (10.0, Action::Capture("airborne.png")),
    (13.0, Action::Capture("burnout.png")),
    (14.5, Action::Section),
    (15.5, Action::Section),
    (17.5, Action::Capture("clean.png")),
    (18.0, Action::Exit),
];

/// How many sequences the Saturn V is built with.
const SATURN_V_SEQUENCES: usize = 9;

/// `OSP_CAPTURE_PLAY=1` (with `OSP_USER_DIR=<empty folder>`, for the saves): from the menu to flying
/// and back. New game goes to an empty hangar, which has nothing to launch. A vessel is chosen there,
/// its staging changed, another looked at instead; Back leaves the game as it was. Then one launches
/// from the pad with its staging, which saves the game. A second vessel goes in from the editor to
/// orbit, the first is flown again, and the save brings back the game as it was with one vessel in
/// it.
const PLAY_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.6, Action::Capture("play_start.png")),
    (1.65, Action::ExpectScreen("pause")),
    (1.7, Action::Ui("game.new")),
    (2.5, Action::Capture("editor_empty.png")),
    (2.55, Action::ExpectEditor(Some(""))),
    (2.56, Action::ExpectScreen("")),
    (2.58, Action::ExpectList(true)),
    (2.6, Action::Ui("editor.launch")),
    (2.8, Action::ExpectEditor(Some(""))),
    (2.9, Action::Capture("editor_nothing_to_launch.png")),
    (3.1, Action::Ui("editor.show:saturn_v")),
    (7.0, Action::Capture("editor_saturn_v.png")),
    (7.05, Action::ExpectEditor(Some("saturn_v"))),
    // Choosing puts the list away. It comes back, and goes, with its button.
    (7.06, Action::ExpectList(false)),
    (7.1, Action::Unlock),
    (7.2, Action::Ui("staging.add")),
    (7.8, Action::Capture("editor_staging.png")),
    (7.9, Action::ExpectSequenceCount(SATURN_V_SEQUENCES + 1)),
    (7.92, Action::Ui("editor.list")),
    (7.96, Action::Capture("editor_list_open.png")),
    (7.98, Action::ExpectList(true)),
    // Another one instead, then back: nothing has happened to the game.
    (8.0, Action::Ui("editor.show:glider")),
    (10.0, Action::Capture("editor_glider.png")),
    (10.05, Action::ExpectEditor(Some("glider"))),
    (10.06, Action::ExpectList(false)),
    (10.1, Action::Ui("menu.back")),
    (10.6, Action::Capture("editor_back.png")),
    (10.65, Action::ExpectScreen("pause")),
    (10.66, Action::ExpectEditor(None)),
    (10.7, Action::ExpectGame(1, "Pathfinder")),
    (10.8, Action::Ui("game.new")),
    (10.9, Action::Ui("editor.show:saturn_v")),
    (12.5, Action::Ui("staging.add")),
    (12.7, Action::Ui("editor.place:pad")),
    (13.0, Action::Ui("editor.launch")),
    (17.0, Action::Capture("launched_pad.png")),
    (17.1, Action::ExpectGame(1, "Saturn V")),
    (17.2, Action::ExpectSequenceCount(SATURN_V_SEQUENCES + 1)),
    (17.3, Action::Ui("game.menu")),
    (17.4, Action::Ui("menu.open:load")),
    (18.0, Action::Capture("play_load.png")),
    (18.1, Action::Ui("menu.back")),
    (18.2, Action::Ui("editor.open")),
    (19.0, Action::ExpectEditor(Some(""))),
    (19.1, Action::Ui("editor.show:glider")),
    (22.0, Action::Capture("editor_glider_second.png")),
    (22.1, Action::Ui("editor.place:orbit")),
    (22.2, Action::Ui("editor.launch")),
    (26.0, Action::Capture("launched_orbit.png")),
    (26.1, Action::ExpectGame(2, "Kestrel")),
    (26.2, Action::Ui("game.menu")),
    (26.3, Action::Ui("menu.open:vessels")),
    (27.0, Action::Capture("play_two_vessels.png")),
    // Every look in the hangar took a number: the scenario was 0, the first launch number 6.
    (27.1, Action::Ui("game.fly:6")),
    (31.0, Action::Capture("flying_first_again.png")),
    (31.1, Action::ExpectGame(2, "Saturn V")),
    (31.2, Action::Ui("game.menu")),
    (31.3, Action::Ui("game.save_as")),
    (31.5, Action::Ui("menu.open:load")),
    (32.0, Action::Capture("play_load_two.png")),
    (32.1, Action::Ui("game.load:game_1")),
    (36.0, Action::Capture("loaded.png")),
    (36.1, Action::ExpectGame(2, "Kestrel")),
    (36.5, Action::Exit),
];

/// `OSP_CAPTURE_EDITOR_VIEW=1`: as far out as the wheel goes in the editor, at the first view and
/// from above, for a small craft and a tall one: what is on show stays in sight, in the hangar.
const EDITOR_VIEW_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.new")),
    (1.5, Action::Ui("editor.show:sounder")),
    (3.0, Action::Capture("sounder_start.png")),
    (3.1, Action::EditorLook(0.0, 0.0, -100.0)),
    (3.6, Action::Capture("sounder_far.png")),
    (3.7, Action::EditorLook(0.0, 150.0, -100.0)),
    (4.2, Action::Capture("sounder_far_above.png")),
    (4.3, Action::EditorLook(0.0, -600.0, -100.0)),
    (4.8, Action::Capture("sounder_far_below.png")),
    (4.9, Action::Ui("editor.show:saturn_v")),
    (7.5, Action::EditorLook(0.0, 0.0, -100.0)),
    (8.0, Action::Capture("saturn_far.png")),
    (8.1, Action::EditorLook(0.0, 150.0, -100.0)),
    (8.6, Action::Capture("saturn_far_above.png")),
    (8.7, Action::EditorLook(0.0, 500.0, -100.0)),
    (9.2, Action::Capture("saturn_far_top.png")),
    (9.5, Action::Exit),
];

/// `OSP_CAPTURE_CLEAR=1`: the pad is taken, so a launch is refused until it is cleared; then the
/// Vessels screen destroying one that is not being flown, then the one that is, and then the last,
/// which leaves the hangar and from there the title screen.
const CLEAR_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.1, Action::Ui("editor.open")),
    (1.6, Action::Ui("editor.show:sounder")),
    (3.0, Action::Ui("editor.launch")),
    (3.3, Action::ExpectEditor(Some("sounder"))),
    (3.4, Action::Capture("pad_occupied.png")),
    (3.5, Action::Ui("editor.clear")),
    (3.9, Action::Capture("pad_clear_asked.png")),
    (4.0, Action::ExpectEditor(Some("sounder"))),
    (4.1, Action::Ui("editor.clear")),
    (4.6, Action::Capture("pad_cleared.png")),
    (4.7, Action::Ui("editor.launch")),
    (6.5, Action::Capture("launched_after_clearing.png")),
    (6.6, Action::ExpectEditor(None)),
    (6.7, Action::ExpectGame(1, "Sounder")),
    // A plane from the runway and a satellite in orbit: the pad stays taken by the Sounder.
    (6.8, Action::Ui("game.menu")),
    (6.9, Action::Ui("editor.open")),
    (7.0, Action::Ui("editor.show:glider")),
    (8.5, Action::Ui("editor.place:runway")),
    (8.7, Action::Ui("editor.launch")),
    (10.0, Action::ExpectGame(2, "Kestrel")),
    (10.1, Action::Ui("game.menu")),
    (10.2, Action::Ui("editor.open")),
    (10.3, Action::Ui("editor.show:satellite")),
    (11.5, Action::Ui("editor.place:runway")),
    (11.8, Action::Capture("runway_occupied.png")),
    (11.9, Action::Ui("editor.launch")),
    (12.1, Action::ExpectEditor(Some("satellite"))),
    (12.2, Action::Ui("editor.place:orbit")),
    (12.4, Action::Ui("editor.launch")),
    (14.0, Action::ExpectGame(3, "Satellite")),
    (14.1, Action::Ui("game.menu")),
    (14.2, Action::Ui("menu.open:vessels")),
    (14.7, Action::Capture("vessels_three.png")),
    // The numbers: the scenario 0; hangar, shown, launched 1, 2, 3; 4, 5, 6; 7, 8, 9.
    (14.8, Action::Ui("vessel.destroy:3")),
    (15.2, Action::Capture("vessel_destroy_asked.png")),
    (15.3, Action::ExpectGame(3, "Satellite")),
    (15.4, Action::Ui("vessel.destroy:3")),
    (15.8, Action::Capture("vessel_destroyed.png")),
    (15.9, Action::ExpectGame(2, "Satellite")),
    (16.0, Action::ExpectScreen("vessels")),
    // The one being flown: another is flown instead.
    (16.1, Action::Ui("vessel.destroy:9")),
    (16.3, Action::Ui("vessel.destroy:9")),
    (16.8, Action::ExpectGame(1, "Kestrel")),
    // The last: only the hangar is left, and back from there the title screen.
    (16.9, Action::Ui("vessel.destroy:6")),
    (17.1, Action::Ui("vessel.destroy:6")),
    (18.0, Action::Capture("nothing_left.png")),
    (18.1, Action::ExpectEditor(Some(""))),
    (18.2, Action::Ui("menu.back")),
    (19.2, Action::ExpectEditor(None)),
    (19.3, Action::ExpectScreen("title")),
    (19.4, Action::Capture("title_after_the_last.png")),
    (19.8, Action::Exit),
];

/// `OSP_CAPTURE_MENUS=1`: every screen of the menu, walked with Esc and Back in turn so a difference
/// between them stops the run, and leaving for the title screen.
const MENUS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.5, Action::Capture("menu_pause.png")),
    (1.55, Action::ExpectScreen("pause")),
    (1.6, Action::Ui("editor.open")),
    (2.1, Action::Capture("menu_editor.png")),
    (2.15, Action::ExpectEditor(Some(""))),
    (2.2, Action::Ui("game.menu")),
    (2.3, Action::ExpectScreen("pause")),
    (2.35, Action::ExpectEditor(None)),
    (2.4, Action::Ui("menu.open:vessels")),
    (2.9, Action::Capture("menu_vessels.png")),
    (2.95, Action::ExpectScreen("vessels")),
    (3.0, Action::Ui("menu.back")),
    (3.1, Action::ExpectScreen("pause")),
    (3.2, Action::Ui("menu.open:load")),
    (3.7, Action::Capture("menu_load.png")),
    (3.8, Action::Ui("game.menu")),
    (3.9, Action::ExpectScreen("pause")),
    (4.0, Action::Ui("menu.open:settings")),
    (4.5, Action::Capture("menu_graphics.png")),
    (4.6, Action::Ui("ui.tab:settings:sound")),
    (5.1, Action::Capture("menu_sound.png")),
    (5.2, Action::Ui("ui.tab:settings:controls")),
    (5.7, Action::Capture("menu_controls.png")),
    (5.8, Action::Ui("ui.tab:settings:interface")),
    (6.3, Action::Capture("menu_interface.png")),
    (6.4, Action::Ui("ui.tab:settings:graphics")),
    (6.5, Action::ExpectScreen("settings")),
    (6.6, Action::Ui("menu.back")),
    (6.7, Action::ExpectScreen("pause")),
    // Back from the root resumes, Esc opens it again on the root and Resume leaves it.
    (6.8, Action::Ui("menu.back")),
    (6.9, Action::ExpectScreen("")),
    (7.0, Action::Ui("game.menu")),
    (7.1, Action::ExpectScreen("pause")),
    (7.2, Action::Ui("game.menu")),
    (7.3, Action::ExpectScreen("")),
    (7.4, Action::Ui("game.menu")),
    (7.5, Action::Ui("menu.open:load")),
    (7.6, Action::Ui("game.resume")),
    (7.7, Action::ExpectScreen("")),
    // A screen left open is not where the menu opens next.
    (7.8, Action::Ui("game.menu")),
    (7.9, Action::ExpectScreen("pause")),
    (8.0, Action::Ui("game.save")),
    (8.6, Action::Capture("menu_saved.png")),
    (8.7, Action::Ui("game.exit")),
    (9.7, Action::Capture("menu_title.png")),
    (9.75, Action::ExpectScreen("title")),
    // Neither goes anywhere from the title screen.
    (9.8, Action::Ui("game.menu")),
    (9.9, Action::ExpectScreen("title")),
    (10.0, Action::Ui("menu.back")),
    (10.1, Action::ExpectScreen("title")),
    (10.2, Action::Ui("menu.open:tutorials")),
    (10.7, Action::Capture("menu_tutorials.png")),
    (10.8, Action::Ui("game.menu")),
    (10.9, Action::ExpectScreen("title")),
    (11.0, Action::Ui("game.new")),
    (11.1, Action::ExpectEditor(Some(""))),
    (11.2, Action::Ui("menu.back")),
    (11.3, Action::ExpectScreen("title")),
    (11.35, Action::ExpectEditor(None)),
    // From the title screen to flying, by way of the editor.
    (11.4, Action::Ui("game.new")),
    (11.5, Action::Ui("editor.show:sounder")),
    (12.5, Action::Capture("menu_editor_list.png")),
    (12.6, Action::Ui("editor.launch")),
    (13.2, Action::ExpectEditor(None)),
    (13.3, Action::ExpectScreen("")),
    (13.5, Action::Exit),
];

/// `OSP_CAPTURE_MENU=1`: open the menu, launch the rocket plane from the runway through the editor,
/// take off and climb out.
/// `OSP_CAPTURE_SETTINGS=1`: the menu's pages of settings, then the flight view and the map at a
/// bigger interface, the map's labels still on what they name.
const SETTINGS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.1, Action::Ui("menu.open:settings")),
    (1.2, Action::Ui("ui.tab:settings:sound")),
    (1.8, Action::Capture("settings_sound.png")),
    (1.9, Action::Ui("ui.tab:settings:controls")),
    (2.5, Action::Capture("settings_controls.png")),
    (2.6, Action::Ui("ui.tab:settings:interface")),
    (3.2, Action::Capture("settings_interface.png")),
    (3.3, Action::Ui("interface.set:scale:125")),
    (3.4, Action::Ui("game.resume")),
    (4.2, Action::Capture("settings_flight_125.png")),
    (4.3, Action::MapView(true)),
    (6.0, Action::Capture("settings_map_125.png")),
    (6.5, Action::Exit),
];

const MENU_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.6, Action::Capture("menu.png")),
    (1.7, Action::Ui("game.new")),
    (1.8, Action::Ui("editor.show:comet")),
    (2.4, Action::Ui("editor.place:runway")),
    (2.5, Action::Capture("menu_runway.png")),
    (2.9, Action::Ui("editor.launch")),
    (5.0, Action::Capture("runway.png")),
    (5.2, Action::Launch),
    (15.0, Action::Capture("rolling.png")),
    (22.5, Action::Steer(DVec3::new(0.3, 0.0, 0.0))),
    (26.0, Action::Steer(DVec3::ZERO)),
    (30.0, Action::Capture("climb.png")),
    (31.5, Action::Exit),
];

/// `OSP_CAPTURE_MENU_TUTORIAL=1`: the title screen's Tutorials, and a tutorial started from them.
const MENU_TUTORIAL_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.1, Action::Ui("game.exit")),
    (1.2, Action::Ui("menu.open:tutorials")),
    (2.0, Action::Capture("menu_tutorials.png")),
    (4.5, Action::Ui("game.start_tutorial:controls")),
    (7.0, Action::Capture("tutorial_from_menu.png")),
    (10.0, Action::Exit),
];

/// `OSP_CAPTURE_GRAPHICS=1`: the main menu's Graphics page with the performance overlay,
/// then the Low preset and back to High.
const GRAPHICS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.1, Action::Ui("menu.open:settings")),
    (1.2, Action::Ui("ui.show:perf")),
    (4.0, Action::Capture("graphics.png")),
    (4.2, Action::Ui("graphics.preset:low")),
    (7.0, Action::Capture("graphics_low.png")),
    (7.2, Action::Ui("graphics.preset:high")),
    (7.4, Action::Ui("game.resume")),
    (10.0, Action::Capture("graphics_flight.png")),
    (10.5, Action::Exit),
];

/// `OSP_SCENARIO=runway OSP_CAPTURE_RUNWAY=1`: the rocket plane at the threshold, its
/// engine lit, then a burst of frames during the fast part of the takeoff roll (watching
/// the scenery go by).
const RUNWAY_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (3.0, Action::Capture("parked.png")),
    (3.2, Action::Launch),
    (5.0, Action::Capture("lit.png")),
    (16.0, Action::Capture("roll_0.png")),
    (16.1, Action::Capture("roll_1.png")),
    (16.2, Action::Capture("roll_2.png")),
    (16.3, Action::Capture("roll_3.png")),
    (16.4, Action::Capture("roll_4.png")),
    (16.5, Action::Capture("roll_5.png")),
    (17.5, Action::Exit),
];

/// In orbit (`OSP_CAPTURE_PLAN=1`): plan a 200 m/s prograde burn at periapsis, let the
/// executor fly it after warping there, and capture the plan, the burn and the result.
const PLAN_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("ui.show:flight_plan")),
    (1.5, Action::Ui("plan.add:pe")),
    (1.7, Action::Ui("plan.dv_step:100")),
    (1.8, Action::Ui("plan.dv:prograde:1")),
    (1.9, Action::Ui("plan.dv:prograde:1")),
    (2.0, Action::MapView(true)),
    (4.0, Action::Capture("plan_map.png")),
    (4.5, Action::Ui("plan.execute")),
    (4.6, Action::Ui("warp.to:burn")),
    (4.7, Action::MapView(false)),
    (32.0, Action::Capture("plan_burning.png")),
    (58.0, Action::MapView(true)),
    (60.0, Action::Capture("plan_after.png")),
    (61.0, Action::Exit),
];

/// `OSP_SCENARIO=rendezvous OSP_CAPTURE_MAP_UI=1`: planning in map view with the pointer, as
/// a player would. The map with the station targeted; the pointer on the trajectory (a
/// ghost node); a click there and a prograde burn (the selected node's handles and panel,
/// the plan dashed); a right-click further on (the menu) and a second burn from it; the
/// pointer on the first node; then that node dragged along its orbit.
const MAP_UI_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(true)),
    (0.6, Action::Ui("ui.show:flight_plan")),
    (3.0, Action::Capture("map_ui.png")),
    (3.1, Action::PointAt(600.0)),
    (3.6, Action::Capture("map_ui_hover.png")),
    (3.7, Action::Press(MouseButton::Left)),
    (3.8, Action::Release(MouseButton::Left)),
    (3.9, Action::Ui("plan.dv_step:100")),
    (4.0, Action::Ui("plan.dv:prograde:1")),
    (4.1, Action::Ui("plan.dv:prograde:1")),
    (4.2, Action::Ui("plan.dv:prograde:1")),
    (4.8, Action::Capture("map_ui_node.png")),
    (4.9, Action::PointAt(3_000.0)),
    (5.0, Action::Press(MouseButton::Right)),
    (5.1, Action::Release(MouseButton::Right)),
    (5.7, Action::Capture("map_ui_menu.png")),
    (5.8, Action::Ui("map.add_burn")),
    (5.9, Action::Ui("plan.dv:normal:1")),
    (6.6, Action::Capture("map_ui_two.png")),
    (6.7, Action::PointAtBurn(0)),
    (7.2, Action::Capture("map_ui_node_hover.png")),
    (7.3, Action::Press(MouseButton::Left)),
    (7.4, Action::PointAt(2_400.0)),
    (7.6, Action::Release(MouseButton::Left)),
    (8.2, Action::Capture("map_ui_dragged.png")),
    (8.5, Action::Exit),
];

/// `OSP_SCENARIO=rendezvous OSP_CAPTURE_RENDEZVOUS=1`: the target windows, the map with
/// the station, closest approaches and plane nodes, then a planned plane match.
/// In `apollo11_docking` (`OSP_CAPTURE_DOCKING=1`): the guides, what the two collide with drawn
/// over them, the view out of the port, a bump, and a dock let go again.
const DOCKING_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::LineUpPorts(6.0, 0.0)),
    (3.0, Action::Capture("docking_guides.png")),
    (3.1, Action::Ui("debug.colliders")),
    (3.6, Action::Capture("docking_colliders.png")),
    (3.7, Action::Ui("debug.colliders")),
    (3.8, Action::Ui("camera.port_view")),
    (4.4, Action::Capture("docking_port_view.png")),
    (4.5, Action::Ui("camera.port_view")),
    // Too fast: it bumps and bounces.
    (4.7, Action::LineUpPorts(0.8, 0.8)),
    (7.0, Action::Capture("docking_bumped.png")),
    (7.2, Action::LineUpPorts(0.6, 0.2)),
    (9.5, Action::Capture("docking_soft_capture.png")),
    (16.0, Action::Capture("docking_docked.png")),
    (16.5, Action::Undock),
    (22.0, Action::Capture("docking_undocked.png")),
    (22.5, Action::Exit),
];

/// In `apollo11_docking` (`OSP_CAPTURE_SAS=1`), all by clicks: the SAS buttons with no target or
/// burn and with both, kill rotation on and off, the tabs under the staging stack, and a
/// right-click on the target's port to make that port the target.
const SAS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (0.6, Action::Unlock),
    (1.0, Action::LineUpPorts(6.0, 0.0)),
    (1.2, Action::Ui("target.clear")),
    (2.5, Action::Capture("sas_no_target.png")),
    (2.6, Action::UiPointAt("vessel.sas_mode:stability")),
    (2.8, Action::UiPress(PointerButton::Primary)),
    (2.9, Action::UiRelease(PointerButton::Primary)),
    (3.4, Action::Capture("sas_kill_rot_on.png")),
    (3.5, Action::UiPress(PointerButton::Primary)),
    (3.6, Action::UiRelease(PointerButton::Primary)),
    (4.1, Action::Capture("sas_kill_rot_off.png")),
    (4.2, Action::UiPointAt("ui.tab:staging:inputs")),
    (4.4, Action::UiPress(PointerButton::Primary)),
    (4.5, Action::UiRelease(PointerButton::Primary)),
    (5.0, Action::Steer(DVec3::new(1.0, -0.5, 0.0))),
    (5.5, Action::Capture("sas_inputs_tab.png")),
    (5.6, Action::Steer(DVec3::ZERO)),
    (5.7, Action::Ui("target.vessel:1")),
    (6.0, Action::PointAtTargetPort),
    (6.2, Action::Press(MouseButton::Right)),
    (6.3, Action::Release(MouseButton::Right)),
    (7.0, Action::Capture("sas_part_window.png")),
    (7.1, Action::UiPointAt("target.part:")),
    (7.3, Action::UiPress(PointerButton::Primary)),
    (7.4, Action::UiRelease(PointerButton::Primary)),
    (7.5, Action::Ui("ui.show:target_info")),
    (7.6, Action::Ui("vessel.sas_mode:target")),
    (8.5, Action::Capture("sas_part_targeted.png")),
    (8.7, Action::Ui("plan.add:pe")),
    (9.5, Action::Capture("sas_all_rows.png")),
    (9.7, Action::Exit),
];

const RENDEZVOUS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("ui.show:target_info")),
    (1.0, Action::Ui("ui.show:manifest")),
    (3.0, Action::Capture("rendezvous_flight.png")),
    (3.2, Action::Ui("ui.hide:manifest")),
    (3.5, Action::MapView(true)),
    (5.5, Action::Capture("rendezvous_map.png")),
    (6.0, Action::Ui("ui.show:flight_plan")),
    (6.0, Action::Ui("plan.match_plane")),
    (8.0, Action::Capture("rendezvous_plan.png")),
    (8.5, Action::Exit),
];

/// In orbit (`OSP_CAPTURE_DEORBIT=1`): plan a 150 m/s retrograde burn and capture the
/// map, where the path turns red into the atmosphere and ends at an impact marker.
const DEORBIT_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(true)),
    (1.0, Action::Ui("plan.add:pe")),
    (1.2, Action::Ui("plan.dv_step:100")),
    (1.3, Action::Ui("plan.dv:prograde:-1")),
    (1.4, Action::Ui("plan.dv_step:10")),
    (1.5, Action::Ui("plan.dv:prograde:-1")),
    (1.6, Action::Ui("plan.dv:prograde:-1")),
    (1.7, Action::Ui("plan.dv:prograde:-1")),
    (1.8, Action::Ui("plan.dv:prograde:-1")),
    (1.9, Action::Ui("plan.dv:prograde:-1")),
    (4.0, Action::Capture("deorbit_map.png")),
    (4.5, Action::Exit),
];

/// In LEO (`OSP_CAPTURE_TRANSFER=1`): target the Moon, plot transfers, make the cheapest
/// one's burn and show the trajectory it leads to.
const TRANSFER_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("target.body:Moon")),
    (1.2, Action::Ui("ui.show:transfer_planner")),
    (1.5, Action::Ui("transfer.compute")),
    (5.0, Action::Capture("transfer_plot.png")),
    (5.2, Action::Ui("transfer.create")),
    (5.4, Action::Ui("ui.hide:transfer_planner")),
    (5.4, Action::Ui("ui.show:flight_plan")),
    (5.6, Action::Ui("plan.aim:100000")),
    (6.2, Action::Ui("plan.circularize:pe")),
    (6.4, Action::MapView(true)),
    (8.0, Action::Capture("transfer_map.png")),
    (8.5, Action::Exit),
];

/// On the pad (`OSP_CAPTURE_PARTS=1`): part windows with their colours, one opened by a
/// right-click on a staging tile (every part of the tile tinted), another on a single part.
const PARTS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (2.0, Action::Capture("parts_start.png")),
    (2.2, Action::Ui("part.group:0:20,21,22,23,24,25,26,27,28")),
    (2.4, Action::Ui("ui.show:part_info:0-1")),
    (3.5, Action::Capture("parts_marked.png")),
    (3.6, Action::Ui("ui.hide:part_info:0-20")),
    (4.5, Action::Capture("parts_closed.png")),
    (4.7, Action::Exit),
];

/// `OSP_TUTORIAL=controls OSP_CAPTURE_TUTORIAL=1`: the first step with its control outlined, the
/// hint once the player has been idle on it (20 s of game time), the launch that does the
/// next steps, the navball step waiting for Continue, the climb, the map step, and the end.
const TUTORIAL_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (2.0, Action::Capture("tutorial_step.png")),
    (22.8, Action::Capture("tutorial_hint.png")),
    (23.0, Action::Launch),
    (26.0, Action::Capture("tutorial_continue.png")),
    (26.5, Action::Ui("tutorial.next")),
    (29.0, Action::Capture("tutorial_climb.png")),
    (46.0, Action::Capture("tutorial_map.png")),
    (47.0, Action::MapView(true)),
    (49.5, Action::Capture("tutorial_done.png")),
    (53.0, Action::Exit),
];

/// On the pad (`OSP_CAPTURE_MISSION=1`, a scenario with goals such as `pad`): the goals window,
/// a launch until "Leave the pad" is met (the notice and the tick), then the results window
/// (opened by hand: the mission is not over).
const MISSION_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (2.0, Action::Capture("mission_start.png")),
    (2.2, Action::Launch),
    (9.5, Action::Capture("mission_goal.png")),
    (10.0, Action::Ui("ui.show:mission_results")),
    (11.0, Action::Capture("mission_results.png")),
    (12.0, Action::Exit),
];

/// On the pad (`OSP_CAPTURE_HOVER=1`): the engines that light next are teal; a staging tile
/// under the pointer (`part.hover` stands in for it) tints its nine engines blue; one engine
/// split off into a sequence of its own is blue alone. A camera that sees the engines:
/// `OSP_CAMERA=200,22,13,-14 OSP_SCENARIO=pad`.
const HOVER_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (2.0, Action::Capture("hover_next.png")),
    (2.2, Action::Ui("part.hover:0:18,19,20,21,22,23,24,25,26")),
    (3.0, Action::Capture("hover_tile.png")),
    (3.2, Action::Ui("part.hover:")),
    (3.3, Action::Sequences("staging.drop_at:one:0:0:0")),
    (3.6, Action::Ui("part.hover:0:26")),
    (4.4, Action::Capture("hover_split.png")),
    (4.6, Action::Exit),
];

/// In LEO (`OSP_CAPTURE_STAGING=1`): the sequence stack, rearranged (RCS moved in with the
/// fairing, an empty sequence added), then the engine armed and the fairing jettisoned.
const STAGING_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (2.0, Action::Capture("staging_start.png")),
    (2.2, Action::Sequences("staging.drop:item:0:1:1")),
    (2.3, Action::Sequences("staging.add")),
    (3.0, Action::Capture("staging_edited.png")),
    (3.2, Action::Section),
    (3.4, Action::Section),
    (5.0, Action::Capture("staging_fairing.png")),
    (5.5, Action::Exit),
];

/// On the pad and lit (`OSP_SCENARIO=apollo11 OSP_CAPTURE_STAGING_POINTER=1`): the staging
/// window with the pointer, as a player would use it, while the data under it changes every
/// frame. Swap two rows by dragging, undo that; drag one engine of a cluster onto the strip
/// under its row, so it fires first; put a row with its arrow; use a row's menu.
const STAGING_POINTER_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (0.6, Action::Unlock),
    (2.5, Action::Launch),
    (3.0, Action::ExpectSequences(STAGED)),
    (3.0, Action::Capture("pointer_start.png")),
    (3.2, Action::UiPointAtEdge("seq:1")),
    (3.4, Action::UiPress(PointerButton::Primary)),
    (3.6, Action::UiPointAt("staging.drop:$drag:0")),
    (4.0, Action::Capture("pointer_dragging.png")),
    (4.2, Action::UiRelease(PointerButton::Primary)),
    (4.6, Action::ExpectSequences(SWAPPED)),
    (4.6, Action::Capture("pointer_swapped.png")),
    (4.8, Action::UiPointAt("staging.undo")),
    (5.0, Action::UiPress(PointerButton::Primary)),
    (5.1, Action::UiRelease(PointerButton::Primary)),
    (5.4, Action::ExpectSequences(STAGED)),
    (5.6, Action::UiPointAt("one:1:0")),
    (5.8, Action::UiPress(PointerButton::Primary)),
    (6.0, Action::UiPointAt("staging.drop_at:$drag:1")),
    (6.3, Action::Capture("pointer_splitting.png")),
    (6.5, Action::UiRelease(PointerButton::Primary)),
    (6.9, Action::ExpectSequences(SPLIT)),
    (6.9, Action::Capture("pointer_split.png")),
    (7.0, Action::UiPointAt("staging.undo")),
    (7.2, Action::UiPress(PointerButton::Primary)),
    (7.3, Action::UiRelease(PointerButton::Primary)),
    (7.6, Action::ExpectSequences(STAGED)),
    (7.8, Action::UiPointAt("staging.drop:seq:0:1")),
    (8.0, Action::UiPress(PointerButton::Primary)),
    (8.1, Action::UiRelease(PointerButton::Primary)),
    (8.4, Action::ExpectSequences(SWAPPED)),
    (8.5, Action::UiPointAt("staging.undo")),
    (8.7, Action::UiPress(PointerButton::Primary)),
    (8.8, Action::UiRelease(PointerButton::Primary)),
    (9.1, Action::ExpectSequences(STAGED)),
    (9.3, Action::UiPointAtEdge("staging.menu:0")),
    (9.5, Action::UiPress(PointerButton::Secondary)),
    (9.6, Action::UiRelease(PointerButton::Secondary)),
    (10.0, Action::Capture("pointer_menu.png")),
    (10.1, Action::UiPointAt("staging.add_at:0")),
    (10.3, Action::UiPress(PointerButton::Primary)),
    (10.4, Action::UiRelease(PointerButton::Primary)),
    (10.8, Action::ExpectSequences(ADDED)),
    (10.8, Action::Capture("pointer_added.png")),
    (11.0, Action::UiPointAt("staging.undo")),
    (11.2, Action::UiPress(PointerButton::Primary)),
    (11.3, Action::UiRelease(PointerButton::Primary)),
    (11.6, Action::ExpectSequences(STAGED)),
    // A tile onto another of its row goes just before it.
    (11.8, Action::UiPointAt("item:0:2")),
    (12.0, Action::UiPress(PointerButton::Primary)),
    (12.2, Action::UiPointAt("staging.drop_before:$drag:0:0")),
    (12.5, Action::UiRelease(PointerButton::Primary)),
    (12.8, Action::ExpectSequences(REORDERED)),
    (12.9, Action::UiPointAt("staging.undo")),
    (13.1, Action::UiPress(PointerButton::Primary)),
    (13.2, Action::UiRelease(PointerButton::Primary)),
    (13.5, Action::ExpectSequences(STAGED)),
    // A right-click on a tile opens its part's window, whose button does it by hand and takes
    // it out of its sequence.
    (13.7, Action::UiPointAt("item:0:1")),
    (13.9, Action::UiPress(PointerButton::Secondary)),
    (14.0, Action::UiRelease(PointerButton::Secondary)),
    (14.5, Action::Capture("pointer_window.png")),
    (14.6, Action::UiPointAt("part.do:0:decoupler:")),
    (14.8, Action::UiPress(PointerButton::Primary)),
    (14.9, Action::UiRelease(PointerButton::Primary)),
    (15.4, Action::ExpectSequences(BY_HAND)),
    (15.4, Action::Capture("pointer_by_hand.png")),
    (15.6, Action::Exit),
];

/// The sequences of Apollo 11's Saturn V once the first has fired, and what the pointer script
/// makes of them.
const STAGED: &str = "enginesx4,decouplerx1,enginesx4 | enginesx5 | decouplerx1 | ";
const SWAPPED: &str = "enginesx5 | enginesx4,decouplerx1,enginesx4 | decouplerx1 | ";
const SPLIT: &str = "enginesx4,decouplerx1,enginesx4 | enginesx1 | enginesx4 | decouplerx1 | ";
const ADDED: &str = " | enginesx4,decouplerx1,enginesx4 | enginesx5 | decouplerx1 | ";
const REORDERED: &str = "enginesx4,enginesx4,decouplerx1 | enginesx5 | decouplerx1 | ";
// The S-IC's retro motors went with it.
const BY_HAND: &str = "enginesx4 | enginesx5 | decouplerx1 | ";

/// In LEO (`OSP_CAPTURE_DEBRIS=1`): warp into daylight, jettison the fairing and watch the
/// halves swing open, release the payload from the upper stage, then the Universe Manifest
/// and the map with the debris in them.
const DEBRIS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (0.6, Action::Warp(7)),
    (3.0, Action::Warp(0)),
    (4.0, Action::Section),
    (4.2, Action::Section),
    (5.2, Action::Capture("debris_fairing.png")),
    (8.0, Action::Capture("debris_fairing_open.png")),
    (8.2, Action::Section),
    (11.0, Action::Capture("debris_stage.png")),
    (11.2, Action::Ui("ui.show:manifest")),
    (11.6, Action::Capture("debris_manifest.png")),
    (11.8, Action::Ui("ui.hide:manifest")),
    (11.8, Action::MapView(true)),
    (13.5, Action::Capture("debris_map.png")),
    (14.0, Action::Exit),
];

/// In LEO (`OSP_CAPTURE_MANIFEST=1`): shed three pieces of debris (vessels 1 to 3), then in
/// the Universe Manifest hide the bodies, follow the spent upper stage, destroy a fairing
/// half, clear the rest of the debris and follow the Moon.
/// In LEO (`OSP_CAPTURE_MARKERS=1`): body markers in map view with the Moon targeted, close
/// to Earth and then zoomed out past the Moon; untargeted, far enough out that the Moon's
/// marker tucks into Earth's; and around the Sun.
const MARKERS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(true)),
    (0.6, Action::Ui("target.body:Moon")),
    (3.0, Action::Capture("markers.png")),
    (3.2, Action::MapDistance(150.0)),
    (5.0, Action::Capture("markers_moon.png")),
    (5.2, Action::Ui("target.clear")),
    (5.2, Action::MapDistance(4000.0)),
    (7.0, Action::Capture("markers_crowded.png")),
    (7.2, Action::Ui("manifest.follow_body:Sun")),
    (7.2, Action::MapDistance(800.0)),
    (9.0, Action::Capture("markers_sun.png")),
    (9.5, Action::Exit),
];

/// In LEO (`OSP_CAPTURE_MARS=1`): the map centred on Mars, close enough that its limb spans
/// the view, twice half a second apart. The vessel moves kilometres between them; Mars must
/// not (centred on the vessel, it jumped by f32 steps of kilometres).
const MARS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::Ui("manifest.follow_body:Mars")),
    (0.6, Action::MapDistance(2.0)),
    (6.0, Action::Capture("mars_a.png")),
    (6.5, Action::Capture("mars_b.png")),
    (7.0, Action::Exit),
];

const MANIFEST_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (0.6, Action::Warp(7)),
    (3.0, Action::Warp(0)),
    (4.0, Action::Section),
    (4.2, Action::Section),
    (4.4, Action::Section),
    (6.0, Action::Ui("ui.show:manifest")),
    (6.2, Action::Ui("manifest.filter:bodies")),
    (6.6, Action::Capture("manifest.png")),
    (6.8, Action::Ui("manifest.follow:3")),
    (7.0, Action::Ui("manifest.remove:1")),
    (8.0, Action::Capture("manifest_follow.png")),
    (8.2, Action::Ui("manifest.confirm")),
    (8.4, Action::Ui("manifest.clear_debris")),
    (8.8, Action::Capture("manifest_clear.png")),
    (9.0, Action::Ui("manifest.confirm")),
    (9.2, Action::Ui("manifest.filter:bodies")),
    (9.4, Action::Ui("manifest.follow_body:Moon")),
    (11.0, Action::Capture("manifest_moon.png")),
    (11.5, Action::Exit),
];

/// Pitch a few degrees towards the east after clearing the tower, then let SAS hold.
const PITCH_KICK: &[(f64, Action)] = &[
    (10.0, Action::Steer(DVec3::new(-0.3, 0.0, 0.0))),
    (12.0, Action::Steer(DVec3::ZERO)),
];

/// One still once the terrain has loaded (`OSP_CAPTURE_QUICK=1`), or with
/// `OSP_CAPTURE_AT=<s>,<s>,...` a launch and stills `view<n>.png` at those times (staging
/// at `OSP_CAPTURE_STAGE_AT=<s>,...`). Combine with `OSP_CAMERA=<yaw°>,<pitch°>,<distance
/// m>` to frame them.
const QUICK_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (8.0, Action::Capture("view.png")),
    (10.0, Action::Exit),
];

const PAD_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (3.0, Action::Capture("pad.png")),
    (3.5, Action::Launch),
    (8.0, Action::Capture("liftoff.png")),
    (10.0, PITCH_KICK[0].1),
    (11.5, Action::Capture("pitch.png")),
    (12.0, PITCH_KICK[1].1),
    (40.0, Action::Capture("ascent.png")),
    (40.5, Action::MapView(true)),
    (42.0, Action::Capture("map.png")),
    (44.0, Action::Exit),
];

/// The sequences as [`Action::ExpectSequences`] reads them.
fn sequences_summary(propulsion: &Propulsion) -> String {
    (propulsion.sequences.list().iter())
        .map(|seq| {
            (propulsion.groups(seq).iter())
                .map(|g| format!("{}x{}", g.kind, g.members.len()))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// The pointer a capture moves about the UI, through the picking the real one goes through.
#[derive(SystemParam)]
struct UiPointer<'w, 's> {
    input: MessageWriter<'w, PointerInput>,
    data: Res<'w, UiData>,
    formatters: Res<'w, Formatters>,
    nodes: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            &'static InheritedVisibility,
            Option<&'static ActionSource>,
            Option<&'static DragPayload>,
            Option<&'static DropAction>,
            Option<&'static ContextAction>,
        ),
    >,
}

impl UiPointer<'_, '_> {
    /// The middle and size of the shown node that says `what`, physical px.
    fn find(&self, what: &str) -> Option<(Vec2, Vec2)> {
        self.nodes
            .iter()
            .find(|(node, _, visible, click, drag, drop, context)| {
                let says = |template: Option<&Template>| {
                    template.is_some_and(|t| {
                        let said = t.render(&self.data, &self.formatters);
                        said == what || (what.ends_with(':') && said.starts_with(what))
                    })
                };
                visible.get()
                    && node.size().min_element() > 0.0
                    && (says(click.map(|t| &t.0))
                        || says(drag.map(|t| &t.0))
                        || says(drop.map(|t| &t.0))
                        || says(context.map(|t| &t.0)))
            })
            .map(|(node, transform, ..)| (transform.translation, node.size()))
    }

    fn send(&mut self, target: &Handle<Image>, position: Vec2, action: PointerAction) {
        let target = NormalizedRenderTarget::Image(ImageRenderTarget {
            handle: target.clone(),
            scale_factor: 1.0,
        });
        self.input.write(PointerInput::new(
            PointerId::Mouse,
            Location { target, position },
            action,
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn run_script(
    mut commands: Commands,
    time: Res<Time>,
    mut script: ResMut<Script>,
    mut sim: ResMut<Sim>,
    mut rig: ResMut<CameraRig>,
    mut warp: ResMut<TimeWarp>,
    mut ui: MessageWriter<UiAction>,
    mut exit: MessageWriter<AppExit>,
    (prediction, frame, camera): (
        Res<Prediction>,
        Res<RenderFrame>,
        Single<(&Camera, &Transform), With<MainCamera>>,
    ),
    mut pointer: ResMut<ScriptedPointer>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut ui_pointer: UiPointer,
    mut locked: ResMut<InputLocked>,
    menu: Res<Menu>,
    mut editor: ResMut<Editor>,
) {
    let Some(&(at, action)) = script.actions.get(script.step) else {
        return;
    };
    if time.elapsed_secs_f64() < at {
        return;
    }
    script.step += 1;
    match action {
        Action::Capture(name) => {
            let path = script.dir.join(name);
            info!("smoke test: capturing {}", path.display());
            commands
                .spawn(Screenshot::image(script.target.clone()))
                .observe(save_to_disk(path));
        }
        Action::Launch => {
            let ut = sim.universe.time();
            let (vessel, system) = sim.active_vessel_in_system();
            vessel.brakes = false;
            vessel.set_throttle(1.0);
            vessel.set_sas(system, ut, true);
            sim.stage_active();
        }
        Action::Section => sim.stage_active(),
        Action::Sequences(action) => {
            let propulsion = &mut sim.active_vessel_mut().propulsion;
            if !crate::controls::edit_sequences(propulsion, &UiAction(action.into())) {
                warn!("smoke test: `{action}` did not apply");
            }
        }
        Action::Steer(input) => sim.active_vessel_mut().rotation_input = input,
        Action::Sas(mode) => {
            let ut = sim.universe.time();
            let (vessel, system) = sim.active_vessel_in_system();
            vessel.set_sas_mode(system, ut, mode);
        }
        Action::Rcs(translation) => {
            let vessel = sim.active_vessel_mut();
            vessel.set_rcs(translation.is_some());
            vessel.translation_input = translation.unwrap_or(DVec3::ZERO);
        }
        Action::Ui(action) => {
            ui.write(UiAction(action.into()));
        }
        Action::ExpectGame(count, flying) => {
            let vessels = sim.universe.vessels.iter();
            let found = vessels.filter(|v| !v.is_debris()).count();
            let name = &sim.active_vessel().name;
            if found != count || name != flying {
                error!("smoke test: {found} vessels, flying {name}: wanted {count}, {flying}");
                exit.write(AppExit::error());
            }
        }
        Action::ExpectScreen(wanted) => {
            let showing = if menu.open { menu.screen() } else { "" };
            if showing != wanted {
                error!("smoke test: the menu shows `{showing}`, wanted `{wanted}`");
                exit.write(AppExit::error());
            }
        }
        Action::EditorLook(x, y, zoom) => {
            editor.look(DVec2::new(x as f64, y as f64), zoom as f64);
        }
        Action::ExpectList(wanted) => {
            if editor.list_open() != wanted {
                error!("smoke test: the list of vessels should be open: {wanted}");
                exit.write(AppExit::error());
            }
        }
        Action::ExpectEditor(wanted) => {
            let found = editor.craft();
            if found != wanted {
                error!("smoke test: the editor has {found:?}, wanted {wanted:?}");
                exit.write(AppExit::error());
            }
        }
        Action::ExpectSequenceCount(count) => {
            let found = sim.active_vessel().propulsion.sequences.list().len();
            if found != count {
                error!("smoke test: {found} sequences, wanted {count}");
                exit.write(AppExit::error());
            }
        }
        Action::LineUpPorts(gap, closing) => {
            let index = sim.active_index();
            if !sim.universe.line_up_ports(index, gap, closing) {
                error!("smoke test: no ports to line up");
                exit.write(AppExit::error());
            }
        }
        Action::Undock => {
            let id = sim.active_id();
            let port = sim.active_vessel().propulsion.joined_parts().next();
            let done = port.is_some_and(|port| sim.perform(id, Activation::undock(port)));
            if !done {
                error!("smoke test: nothing to undock");
                exit.write(AppExit::error());
            }
        }
        Action::MapView(on) => rig.map_view = on,
        Action::MapDistance(radii) => rig.map_distance = radii,
        Action::Warp(index) => warp.set_index(index, &sim, time.elapsed_secs_f64()),
        Action::PointAt(ahead) => {
            // The first point from there on that is in sight.
            let spheres = body_spheres(&sim, &frame);
            let eye = to_render(rig.position);
            pointer.0 = (0..720).find_map(|k| {
                let time = sim.universe.time().add_seconds(ahead + 15.0 * k as f64);
                let patch = prediction
                    .patches
                    .iter()
                    .find(|p| p.start <= time && time <= p.end)?;
                let at = frame.to_render(patch.frame, patch.orbit.state_at(time).position);
                project(camera.0, camera.1, at).filter(|_| !hidden(eye, at, &spheres))
            });
        }
        Action::PointAtBurn(index) => {
            pointer.0 = prediction.burn(index).and_then(|(patch, end)| match end {
                PatchEnd::Burn { state, .. } => project(
                    camera.0,
                    camera.1,
                    frame.to_render(patch.frame, state.position),
                ),
                PatchEnd::Event(_) => None,
            });
        }
        Action::PointAtTargetPort => {
            let approach = sim
                .universe
                .port_approach(sim.active_index(), sim.render_time);
            let parent = sim.active_vessel().parent();
            // A little behind the port's face, to be on the part rather than just off its edge.
            pointer.0 = approach.and_then(|approach| {
                let behind = approach.far.at.position - approach.far.out * 0.3;
                project(camera.0, camera.1, frame.to_render(parent, behind))
            });
        }
        Action::Press(button) => buttons.press(button),
        Action::Release(button) => buttons.release(button),
        Action::Unlock => locked.0 = false,
        Action::UiPointAt(what) | Action::UiPointAtEdge(what) => {
            let Some((middle, size)) = ui_pointer.find(what) else {
                panic!("smoke test: nothing on screen is `{what}`");
            };
            let to = match action {
                Action::UiPointAtEdge(_) => middle - Vec2::new(0.5 * size.x - 6.0, 0.0),
                _ => middle,
            };
            let delta = to - script.ui_pointer;
            script.ui_pointer = to;
            let target = script.target.clone();
            ui_pointer.send(&target, to, PointerAction::Move { delta });
        }
        Action::UiPress(button) => {
            let (target, at) = (script.target.clone(), script.ui_pointer);
            ui_pointer.send(&target, at, PointerAction::Press(button));
        }
        Action::UiRelease(button) => {
            let (target, at) = (script.target.clone(), script.ui_pointer);
            ui_pointer.send(&target, at, PointerAction::Release(button));
        }
        Action::ExpectSequences(expected) => {
            let got = sequences_summary(&sim.active_vessel().propulsion);
            info!("smoke test: sequences {got}");
            assert!(
                got.starts_with(expected),
                "the sequences are not as they should be: {got}"
            );
        }
        Action::Exit => {
            let system = &sim.universe.system;
            let v = sim.active_vessel();
            let state = v.state_at(system, sim.universe.time());
            info!(
                "smoke test: warp {}x, UT {:?}",
                warp.rate(),
                sim.universe.time()
            );
            info!(
                "smoke test: altitude {:.0} m, speed {:.0} m/s, log: {:?}",
                state.position.length() - system.body(v.parent()).radius,
                state.velocity.length(),
                v.log.iter().map(|e| &e.message).collect::<Vec<_>>()
            );
            exit.write(AppExit::Success);
        }
    }
}
