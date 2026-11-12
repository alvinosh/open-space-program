//! Scripted smoke test: `OSP_CAPTURE=<dir> cargo run` launches the rocket, saves a few
//! screenshots into `<dir>` and exits. Handy for checking rendering without a human.
//!
//! The camera renders into an offscreen image so captures work even when the window is
//! hidden or occluded (macOS stops presenting occluded windows).
//!
//! The steps that look inside the editor are the editor's to take (`osp_editor`'s `capture`): the
//! script sends them on as [`EditorStep`]s.

use std::path::PathBuf;

use bevy::{
    camera::{ImageRenderTarget, NormalizedRenderTarget},
    ecs::system::SystemParam,
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput, NativeKey, NativeKeyCode},
    },
    math::DVec3,
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
    attitude::SasMode, prediction::PatchEnd, propulsion::Propulsion, rendezvous::Target,
    staging::Activation,
};
use osp_ui::{
    ActionSource, ContextAction, DragPayload, DropAction, Formatters, LayoutVisibility, Slider,
    Template, TextInput, UiAction, UiData,
};

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera, ViewOutput},
    controls::InputLocked,
    dialog::Dialog,
    editing::LastLaunch,
    frame::{RenderFrame, to_render},
    map_labels::{body_spheres, hidden},
    markers::MarkerBox,
    menu::Menu,
    orbit_lines::{Prediction, ScriptedPointer, project},
    saves::Saves,
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

/// A capture's steps: the script's, then the editor's ([`EditorStep`]), then the UI's pointer goes
/// where they put the view's. All after the frame's input and before the simulation.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum CaptureSet {
    Script,
    Editor,
    Pointer,
}

/// A step of a script that only the editor (`osp_editor`) can take, as it looks inside it. Taken
/// in [`CaptureSet::Editor`] the frame it is sent; a step that fails ends the capture with an
/// error.
#[derive(Message, Clone, Copy)]
pub struct EditorStep(pub Action);

impl Plugin for SmokeTestPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<EditorStep>().configure_sets(
            Update,
            (CaptureSet::Script, CaptureSet::Editor, CaptureSet::Pointer)
                .chain()
                .after(FrameSet::Input)
                .before(FrameSet::Simulate),
        );
        if let Some(dir) = std::env::var_os("OSP_CAPTURE") {
            let in_orbit = std::env::var("OSP_SCENARIO").is_ok_and(|s| s != "pad");
            let times: Vec<f64> = std::env::var("OSP_CAPTURE_AT")
                .unwrap_or_default()
                .split(',')
                .filter_map(|t| t.trim().parse().ok())
                .collect();
            let mut actions = if let Some(last) = times.last() {
                // Launch, then stills at the given times (debugging the ascent). With
                // `OSP_CAPTURE_BARE=1` the flight UI is hidden, to see the vessel whole.
                let mut actions = vec![(0.5, Action::MapView(false)), (3.5, Action::Launch)];
                if std::env::var_os("OSP_CAPTURE_BARE").is_some() {
                    actions.push((1.0, Action::Ui("ui.hide_group:flight")));
                } else {
                    actions.extend([
                        (1.0, Action::Ui("ui.show:attitude_info")),
                        (1.0, Action::Ui("ui.show:vessel_info")),
                        (1.0, Action::Ui("ui.show:structure_info")),
                    ]);
                }
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
            } else if std::env::var_os("OSP_CAPTURE_NODE_INPUT").is_some() {
                NODE_INPUT_SCRIPT.to_vec()
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
            } else if std::env::var_os("OSP_CAPTURE_EDITOR_SHAPE").is_some() {
                EDITOR_SHAPE_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_EDITOR_BUILD").is_some() {
                EDITOR_BUILD_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_EDITOR_PARTS").is_some() {
                EDITOR_PARTS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_EDITOR_DRAG").is_some() {
                EDITOR_DRAG_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_FUEL_LINE").is_some() {
                FUEL_LINE_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_PROBE").is_some() {
                PROBE_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_EDITOR_MOVE").is_some() {
                EDITOR_MOVE_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_EDITOR_CRAFT").is_some() {
                EDITOR_CRAFT_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_SATURN_PAD").is_some() {
                SATURN_PAD_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_CHUTES").is_some() {
                CHUTES_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_SPLASH").is_some() {
                SPLASH_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_TRANSPOSITION").is_some() {
                TRANSPOSITION_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_BOXES").is_some() {
                BOXES_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_LEAVING").is_some() {
                LEAVING_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_NIGHT").is_some() {
                NIGHT_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_ASCENT").is_some() {
                ASCENT_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_FALLING").is_some() {
                FALLING_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_POWER").is_some() {
                POWER_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_WINDOWS").is_some() {
                WINDOWS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_WARP").is_some() {
                WARP_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_MODS").is_some() {
                MODS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_CAREER").is_some() {
                CAREER_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_SCIENCE").is_some() {
                SCIENCE_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_CAREER_REALISTIC").is_some() {
                CAREER_REALISTIC_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_SAVE_MODS").is_some() {
                SAVE_MODS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_BASE_ENGINES").is_some() {
                BASE_ENGINES_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_NO_POD").is_some() {
                NO_POD_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_SAVE").is_some() {
                SAVE_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_QUICKSAVE").is_some() {
                QUICKSAVE_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_ALARMS").is_some() {
                ALARMS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_LAUNCH_TIMES").is_some() {
                LAUNCH_TIMES_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_REVERT").is_some() {
                REVERT_SCRIPT.to_vec()
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
                move_ui_pointer: false,
            })
            .add_systems(PostStartup, render_offscreen)
            .add_systems(
                Update,
                (
                    run_script.in_set(CaptureSet::Script),
                    move_ui_pointer.in_set(CaptureSet::Pointer),
                ),
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
    /// The step taken this frame moved the view's pointer, so the UI's goes there too.
    move_ui_pointer: bool,
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

/// What a capture's script does at a moment: each is taken in one frame.
#[derive(Clone, Copy)]
pub enum Action {
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
    /// payload, `drop`, `context` or slider action as they read now, or that start with this if it
    /// ends in `:` (the capture has no real pointer).
    UiPointAt(&'static str),
    /// The same, this far along it from its left edge (0 to 1): pressed, that drags a slider.
    UiPointAlong(&'static str, f32),
    /// The same, near its left edge: on a row, its number rather than one of its tiles.
    UiPointAtEdge(&'static str),
    /// Type this, as a keyboard would (`\n` for Enter, `\x1b` for Escape).
    Type(&'static str),
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
    /// Move the pointer the UI sees onto the box round what is called this. In the map the view's
    /// scripted pointer goes there too, so what a click on the map would do gets its chance.
    UiPointAtMarker(&'static str),
    /// Take the view's scripted pointer away.
    ClearPointer,
    /// Turn the flight view so the vessel flown is between the camera and its target.
    LookAtTarget,
    /// Turn the map so the camera is over the vessel flown, with nothing in the way of what is near.
    MapOverFlown,
    /// Put the flight view on the sunny side of what it is looking at, this far off, m.
    LookFromSun(f64),
    /// Look from this far off, m, at this bearing and height above the horizon, degrees.
    LookFrom(f64, f64, f64),
    /// Stop the capture with a failure unless the active vessel plans this many burns.
    ExpectBurns(usize),
    /// Stop the capture with a failure unless the active vessel's target is called this, "" for none.
    ExpectTarget(&'static str),
    /// Stop the capture with a failure unless what is called this has a box on screen, or has none.
    ExpectMarker(&'static str, bool),
    /// Stop the capture with a failure unless the view is the map, or is not.
    ExpectMapView(bool),
    /// Stop the capture with a failure unless this question is being asked, or none for "".
    ExpectDialog(&'static str),
    /// Stop the capture with a failure unless there are this many saves.
    ExpectSaves(usize),
    /// Stop the capture with a failure unless the save the game goes to holds this many quicksaves.
    ExpectQuicksaves(usize),
    /// Press or release a key, as a player would.
    Key(KeyCode, bool),
    /// Stop the capture with a failure unless this condition on the UI's data holds.
    ExpectData(&'static str),
    /// Stop the capture with a failure unless this layout is open or closed.
    ExpectOpen(&'static str, bool),
    /// Stop the capture with a failure unless `windows.json` says this about a window.
    ExpectRemembered(&'static str, bool),
    /// Stop the capture with a failure unless the game can revert to the launch of what is called
    /// this, or cannot for `None`.
    ExpectRevert(Option<&'static str>),
    /// Stop the capture with a failure unless the game's clock is this many seconds past the last
    /// launch, from the first to the second.
    ExpectSinceLaunch(f64, f64),
    /// Turn the editor's view by this many pixels of mouse movement, and zoom by these lines of
    /// the wheel.
    EditorLook(f32, f32, f32),
    /// Stop the capture with a failure unless the editor's list of vessels is out or away.
    ExpectList(bool),
    /// Stop the capture with a failure unless the editor is open with this craft on show, "" for
    /// an empty hangar, or closed for `None`.
    ExpectEditor(Option<&'static str>),
    /// Put the view's scripted pointer this many pixels right and down of where the handle for this
    /// size of the part picked in the editor was drawn.
    EditorPointAtGrip(&'static str, f32, f32),
    /// Stop the capture with a failure unless this size of this part of the craft in the editor is
    /// between these, m.
    ExpectSize(&'static str, &'static str, f64, f64),
    /// Put the view's scripted pointer here, px from the top left.
    PointerAt(f32, f32),
    /// Put the view's scripted pointer on the ring of this free node of this part of the craft in
    /// the editor, as drawn last.
    EditorPointAtNode(&'static str, &'static str),
    /// Put the view's scripted pointer on this surface of this part of the craft in the editor, at
    /// these coordinates (u round it, v along it).
    EditorPointAtSurface(&'static str, &'static str, f64, f64),
    /// Stop the capture with a failure unless the craft in the editor has this many parts.
    ExpectPartCount(usize),
    /// Put the view's scripted pointer on the middle of this part of the craft in the editor.
    EditorPointAtPart(&'static str),
    /// Stop the capture with a failure unless this field (`osp_sim::properties`) of this part of
    /// the craft in the editor is this: a number, a choice's id, `true` or `false`, or text.
    ExpectField(&'static str, &'static str, &'static str),
    /// Stop the capture with a failure unless this layout is open, or is not.
    ExpectShown(&'static str, bool),
    /// Stop the capture with a failure unless the editor has this part picked, "" for none.
    ExpectPicked(&'static str),
    /// Put the view's scripted pointer on the editor's move or turn handle on this axis (0 to 2) of
    /// the part picked, as drawn last: along an arrow from -1 to 1, or this share of a turn round a
    /// ring.
    EditorPointAtHandle(usize, f32),
    /// Move the view's scripted pointer this many pixels right and down.
    PointerBy(f32, f32),
    /// Open the window of the flown vessel's part of this name, as a right click on it does.
    PartWindow(&'static str),
    /// Stop the capture with a failure unless this part of the craft in the editor is held by this
    /// node of its own.
    ExpectHeldBy(&'static str, &'static str),
    /// Stop the capture with a failure unless this part of the craft in the editor is held on a
    /// surface between these shares of the way along it.
    ExpectAlong(&'static str, f64, f64),
    /// Stop the capture with a failure unless the root of the craft in the editor is at least this
    /// far over the hangar's floor, m.
    ExpectRootAbove(f64),
    /// Stop the capture with a failure unless the root of the craft in the editor points this way
    /// in its frame (within a degree): `x`, `y`, `z`, `-x`...
    ExpectRootPoints(&'static str),
    Exit,
}

impl Action {
    /// Whether this looks inside the editor, so that only it can take it ([`EditorStep`]).
    pub fn in_the_editor(&self) -> bool {
        matches!(
            self,
            Self::EditorLook(..)
                | Self::ExpectList(..)
                | Self::ExpectEditor(..)
                | Self::EditorPointAtGrip(..)
                | Self::ExpectSize(..)
                | Self::EditorPointAtNode(..)
                | Self::EditorPointAtSurface(..)
                | Self::ExpectPartCount(..)
                | Self::EditorPointAtPart(..)
                | Self::ExpectField(..)
                | Self::ExpectPicked(..)
                | Self::EditorPointAtHandle(..)
                | Self::ExpectHeldBy(..)
                | Self::ExpectAlong(..)
                | Self::ExpectRootAbove(..)
                | Self::ExpectRootPoints(..)
        )
    }
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
const SATURN_V_SEQUENCES: usize = 11;
/// And the sounding rocket.
const SOUNDER_SEQUENCES: usize = 2;

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
    (27.1, Action::Ui("vessel.fly:6")),
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

/// `OSP_CAPTURE_EDITOR_BUILD=1`: a rocket built from nothing with the pointer, as a player would.
/// A new vessel; a capsule from the list, which goes straight on; a decoupler, a tank and an engine
/// each taken from the list and put on the free node under the last, the list staying out; the tank
/// picked, its settings shown in the rail in place of the list and set there (with no engine yet it
/// has nothing to be filled with), and the list brought back with Parts; the engine, after which the
/// tank holds what it burns; four fins round the tank's side; one taken off and put back with undo;
/// one dragged off out into the hangar, which takes its copies too, and put back with undo; then
/// it is launched and flown.
const EDITOR_BUILD_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.new")),
    (1.5, Action::Ui("editor.new")),
    (2.2, Action::Capture("build_new.png")),
    (2.3, Action::ExpectPartCount(0)),
    (2.4, Action::Unlock),
    (2.5, Action::Ui("editor.group:pods")),
    (2.6, Action::Ui("editor.add:pod:C-1 Capsule")),
    (3.4, Action::ExpectPartCount(1)),
    (3.45, Action::ExpectData("editor.rail == parts")),
    (3.5, Action::Capture("build_pod.png")),
    (3.6, Action::Ui("editor.group:structure")),
    // The decoupler dragged from its tile onto the ring under the capsule.
    (3.7, Action::UiPointAt("editor.add:decoupler")),
    (3.8, Action::UiPress(PointerButton::Primary)),
    (3.81, Action::Press(MouseButton::Left)),
    (3.9, Action::PointerAt(800.0, 600.0)),
    (4.3, Action::EditorPointAtNode("pod", "bottom")),
    (5.0, Action::Capture("build_drag_preview.png")),
    (5.1, Action::UiRelease(PointerButton::Primary)),
    (5.11, Action::Release(MouseButton::Left)),
    (5.8, Action::ExpectPartCount(2)),
    (5.9, Action::Ui("editor.add:tank")),
    (6.3, Action::EditorPointAtNode("decoupler", "bottom")),
    (7.0, Action::Press(MouseButton::Left)),
    (7.1, Action::Release(MouseButton::Left)),
    (7.7, Action::ExpectPartCount(3)),
    (7.72, Action::ExpectData("editor.rail == parts")),
    (7.74, Action::Ui("editor.pick:tank")),
    (7.82, Action::ExpectData("editor.rail == part")),
    (7.86, Action::Capture("build_part_settings.png")),
    // Everything about the tank, set in the rail.
    (7.9, Action::Ui("editor.set:shape:cone")),
    (8.4, Action::ExpectField("tank", "shape", "cone")),
    (8.5, Action::Ui("editor.set:top_diameter:1.2")),
    // It feeds no engine yet, so it has nothing to be filled with.
    (9.0, Action::Ui("editor.expand:contents")),
    (9.5, Action::ExpectField("tank", "contents", "custom")),
    (9.55, Action::Capture("build_tank_no_engine.png")),
    (9.6, Action::Ui("editor.set:utilization:50")),
    (10.1, Action::ExpectField("tank", "utilization", "50")),
    (10.2, Action::Ui("editor.expand:tank_type")),
    (10.7, Action::Capture("build_tank_types.png")),
    (10.8, Action::Ui("editor.set:shape:cylinder")),
    (11.8, Action::Ui("editor.set:utilization:90")),
    (12.35, Action::ExpectField("tank", "shape", "cylinder")),
    (12.4, Action::Capture("build_tank_set.png")),
    // Parts brings the list back in place of the settings, and lets go of the tank.
    (12.42, Action::Ui("editor.parts")),
    (12.52, Action::ExpectData("editor.rail == parts")),
    (12.53, Action::ExpectPicked("")),
    (12.55, Action::Ui("editor.group:engines")),
    (12.6, Action::Ui("editor.add:engine:K-1 Booster")),
    (13.0, Action::EditorPointAtNode("tank", "bottom")),
    (13.7, Action::Press(MouseButton::Left)),
    (13.8, Action::Release(MouseButton::Left)),
    (14.4, Action::ExpectPartCount(4)),
    // The tank feeds it now, and holds what it burns.
    (14.45, Action::ExpectField("tank", "contents", "kerolox")),
    (14.5, Action::Ui("editor.pick:tank")),
    (14.6, Action::Ui("editor.expand:contents")),
    (15.2, Action::Capture("build_tank_filled.png")),
    (15.3, Action::Ui("editor.parts")),
    (15.5, Action::Ui("editor.group:aero")),
    (15.6, Action::Ui("editor.add:fin")),
    (15.7, Action::Ui("editor.symmetry")),
    (15.75, Action::Ui("editor.symmetry")),
    (15.8, Action::Ui("editor.symmetry")),
    (
        16.0,
        Action::EditorPointAtSurface("tank", "barrel", 0.625, 0.12),
    ),
    (16.9, Action::Capture("build_fins_preview.png")),
    (17.0, Action::Press(MouseButton::Left)),
    (17.1, Action::Release(MouseButton::Left)),
    (17.7, Action::ExpectPartCount(8)),
    // A fin turned in its settings, then picked up by dragging it and put further up the tank.
    (17.75, Action::Ui("editor.pick:fin")),
    (17.8, Action::Ui("editor.set:roll:30")),
    (18.3, Action::ExpectField("fin", "roll", "30")),
    (18.4, Action::Ui("editor.set:roll:0")),
    (18.9, Action::Ui("editor.pick:")),
    (19.0, Action::EditorPointAtPart("fin")),
    (19.3, Action::Press(MouseButton::Left)),
    (
        19.5,
        Action::EditorPointAtSurface("tank", "barrel", 0.55, 0.6),
    ),
    (20.3, Action::Capture("build_move_preview.png")),
    (20.4, Action::Release(MouseButton::Left)),
    (21.0, Action::ExpectPartCount(8)),
    (21.1, Action::Capture("build_moved.png")),
    (21.2, Action::Ui("editor.undo")),
    (21.8, Action::ExpectPartCount(8)),
    (21.9, Action::ClearPointer),
    (22.0, Action::Ui("editor.pick:")),
    (22.1, Action::ExpectData("editor.rail == parts")),
    (22.2, Action::Capture("build_rocket.png")),
    (22.3, Action::Ui("editor.pick:fin")),
    (22.4, Action::Ui("editor.remove")),
    (23.0, Action::ExpectPartCount(7)),
    (23.1, Action::Ui("editor.undo")),
    (23.7, Action::ExpectPartCount(8)),
    (23.8, Action::Ui("editor.pick:")),
    // A fin dragged off the craft out into the hangar: shown gone while it is there, it comes off
    // with its copies when let go, and undo puts them back.
    (23.85, Action::EditorPointAtPart("fin")),
    (24.1, Action::Press(MouseButton::Left)),
    (24.2, Action::PointerBy(0.0, -40.0)),
    (24.4, Action::PointerAt(1150.0, 120.0)),
    (25.0, Action::ExpectData("editor.holding")),
    (25.1, Action::Capture("build_drag_off.png")),
    (25.2, Action::Release(MouseButton::Left)),
    (25.8, Action::ExpectPartCount(4)),
    (25.9, Action::Ui("editor.undo")),
    (26.5, Action::ExpectPartCount(8)),
    (26.6, Action::ClearPointer),
    (27.0, Action::Ui("editor.launch")),
    (28.0, Action::ExpectEditor(None)),
    (29.0, Action::Launch),
    (37.0, Action::Capture("build_flying.png")),
    (37.5, Action::Exit),
];

/// `OSP_CAPTURE_EDITOR_CRAFT=1`: what the craft itself can do in the editor. A tank as the root is
/// carried up off the floor by dragging it, and another goes under it; four decouplers go round the
/// lower one's side with radial symmetry, a tank on one's free node stands upright beside the core
/// and goes on all four, and a nose cone on one booster goes on all four; the root is turned with
/// its ring, taking the whole craft with it, and undone. Esc would leave and lose the rocket, so it
/// asks first, and Cancel keeps it; so do another craft from the list, and a new plane in its
/// place, which Discard goes on to.
/// Then the new plane: a capsule lying as on
/// the runway, a tank behind it, a pair of wings on the tank's sides with mirror symmetry, and a
/// tank put on one wing's tip goes on both.
const EDITOR_CRAFT_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.new")),
    (1.5, Action::Ui("editor.new")),
    (2.0, Action::Unlock),
    (2.1, Action::Ui("editor.group:tanks")),
    (2.2, Action::Ui("editor.add:tank")),
    (3.0, Action::ExpectPartCount(1)),
    (3.1, Action::Capture("craft_root.png")),
    // Carried up by its root.
    (3.2, Action::EditorPointAtPart("tank")),
    (3.4, Action::Press(MouseButton::Left)),
    (3.5, Action::PointerBy(0.0, -20.0)),
    (3.7, Action::PointerBy(0.0, -260.0)),
    (4.2, Action::Capture("craft_carried.png")),
    (4.3, Action::Release(MouseButton::Left)),
    (4.5, Action::ExpectRootAbove(8.0)),
    (4.6, Action::ClearPointer),
    (4.7, Action::Ui("editor.add:tank")),
    (5.0, Action::EditorPointAtNode("tank", "bottom")),
    (5.7, Action::Press(MouseButton::Left)),
    (5.8, Action::Release(MouseButton::Left)),
    (6.4, Action::ExpectPartCount(2)),
    (6.45, Action::ExpectRootAbove(8.0)),
    // The lower tank picked up and put on top of the root, by its other end; then back.
    (6.46, Action::EditorPointAtPart("tank_2")),
    (6.5, Action::Press(MouseButton::Left)),
    (6.55, Action::PointerBy(30.0, -30.0)),
    (6.8, Action::EditorPointAtNode("tank", "top")),
    (7.5, Action::Capture("craft_moved_on_top_preview.png")),
    (7.52, Action::Release(MouseButton::Left)),
    (8.1, Action::ExpectPartCount(2)),
    (8.12, Action::ExpectHeldBy("tank_2", "bottom")),
    (8.14, Action::ClearPointer),
    (8.16, Action::Capture("craft_moved_on_top.png")),
    (8.18, Action::Ui("editor.undo")),
    (8.4, Action::ExpectHeldBy("tank_2", "top")),
    // Four radial decouplers round the lower tank, boosters on them, a cone on one booster.
    (8.5, Action::EditorLook(0.0, 0.0, -4.0)),
    (8.6, Action::Ui("editor.add:decoupler")),
    (8.7, Action::Ui("editor.symmetry")),
    (8.75, Action::Ui("editor.symmetry")),
    (8.8, Action::Ui("editor.symmetry")),
    (8.9, Action::ExpectData("editor.symmetry == x4")),
    (
        7.0,
        Action::EditorPointAtSurface("tank_2", "barrel", 0.625, 0.5),
    ),
    (9.9, Action::Capture("craft_decouplers_preview.png")),
    (10.0, Action::Press(MouseButton::Left)),
    (10.1, Action::Release(MouseButton::Left)),
    (10.8, Action::ExpectPartCount(6)),
    (10.9, Action::Ui("editor.add:tank")),
    (11.0, Action::EditorPointAtNode("decoupler", "bottom")),
    (11.9, Action::Capture("craft_boosters_preview.png")),
    (12.0, Action::Press(MouseButton::Left)),
    (12.1, Action::Release(MouseButton::Left)),
    (12.8, Action::ExpectPartCount(10)),
    (12.85, Action::ExpectHeldBy("tank_3", "held_side")),
    (12.9, Action::Ui("editor.add:nose_cone")),
    (13.0, Action::EditorPointAtNode("tank_3", "top")),
    (13.9, Action::Press(MouseButton::Left)),
    (14.0, Action::Release(MouseButton::Left)),
    (14.7, Action::ExpectPartCount(14)),
    (14.8, Action::ClearPointer),
    (14.9, Action::Capture("craft_boosters.png")),
    // The root turned a quarter turn about +X by its ring: everything goes with it.
    (15.0, Action::Ui("editor.pick:tank")),
    (15.1, Action::Ui("editor.tool:turn")),
    (15.6, Action::Capture("craft_root_rings.png")),
    (15.7, Action::EditorPointAtHandle(0, 0.0)),
    (15.8, Action::Press(MouseButton::Left)),
    (15.9, Action::EditorPointAtHandle(0, 0.125)),
    (16.1, Action::EditorPointAtHandle(0, 0.25)),
    (17.0, Action::Capture("craft_root_turning.png")),
    (17.1, Action::Release(MouseButton::Left)),
    (17.8, Action::ClearPointer),
    (17.9, Action::Capture("craft_root_turned.png")),
    (18.0, Action::ExpectRootAbove(0.0)),
    (18.1, Action::Ui("editor.undo")),
    (18.6, Action::ExpectRootPoints("z")),
    (18.7, Action::Ui("editor.tool:shape")),
    (18.8, Action::Ui("editor.pick:")),
    // Leaving would lose the rocket, so Esc asks first, and Cancel keeps it.
    (18.82, Action::Ui("game.menu")),
    (
        18.9,
        Action::ExpectDialog("Discard the vessel you are building?"),
    ),
    (18.92, Action::Capture("craft_discard_asked.png")),
    (18.95, Action::Ui("dialog.answer:1")),
    (19.0, Action::ExpectDialog("")),
    // Another craft from the list asks too.
    (19.05, Action::Ui("editor.show:sounder")),
    (
        19.15,
        Action::ExpectDialog("Discard the vessel you are building?"),
    ),
    (19.2, Action::Ui("dialog.answer:1")),
    (19.25, Action::ExpectDialog("")),
    (19.3, Action::ExpectEditor(Some("new_vessel"))),
    (19.35, Action::ExpectPartCount(14)),
    // So does a new plane in its place; Discard goes on to it.
    (19.4, Action::Ui("editor.new:plane")),
    (
        19.5,
        Action::ExpectDialog("Discard the vessel you are building?"),
    ),
    (19.55, Action::Ui("dialog.answer:0")),
    // A plane: lying as it will on the runway, wings in mirror pairs.
    (20.1, Action::ExpectData("editor.place == runway")),
    (20.2, Action::ExpectData("editor.mirror")),
    (20.3, Action::Ui("editor.group:pods")),
    (20.4, Action::Ui("editor.add:pod:C-1 Capsule")),
    (21.2, Action::ExpectPartCount(1)),
    (21.3, Action::Ui("editor.group:aero")),
    (21.4, Action::Ui("editor.add:tank")),
    (21.6, Action::EditorPointAtNode("pod", "bottom")),
    (22.3, Action::Press(MouseButton::Left)),
    (22.4, Action::Release(MouseButton::Left)),
    (23.0, Action::ExpectPartCount(2)),
    (23.1, Action::Ui("editor.add:wing")),
    (
        23.2,
        Action::EditorPointAtSurface("tank", "barrel", 0.0, 0.5),
    ),
    (24.1, Action::Capture("plane_wings_preview.png")),
    (24.2, Action::Press(MouseButton::Left)),
    (24.3, Action::Release(MouseButton::Left)),
    (24.9, Action::ExpectPartCount(4)),
    (25.0, Action::ClearPointer),
    (25.1, Action::EditorLook(2.0, 300.0, 0.0)),
    (25.6, Action::Capture("plane_wings.png")),
    (25.7, Action::Ui("editor.place:pad")),
    (26.3, Action::Capture("plane_standing.png")),
    (26.4, Action::Ui("editor.place:runway")),
    (27.0, Action::Capture("plane_lying.png")),
    // Wheels under its belly: one that steers under the nose, a mirrored pair further back.
    (27.1, Action::EditorLook(2.0, -420.0, 0.0)),
    (27.2, Action::Ui("editor.add:nose_wheel")),
    (
        27.3,
        Action::EditorPointAtSurface("tank", "barrel", 0.25, 0.85),
    ),
    (28.2, Action::Press(MouseButton::Left)),
    (28.3, Action::Release(MouseButton::Left)),
    (28.9, Action::ExpectPartCount(5)),
    (29.0, Action::Ui("editor.add:wheel")),
    (
        29.1,
        Action::EditorPointAtSurface("tank", "barrel", 0.2, 0.25),
    ),
    (30.0, Action::Capture("plane_wheels_preview.png")),
    (30.1, Action::Press(MouseButton::Left)),
    (30.2, Action::Release(MouseButton::Left)),
    (30.8, Action::ExpectPartCount(7)),
    (30.9, Action::ClearPointer),
    (31.0, Action::Capture("plane_wheels.png")),
    (31.1, Action::Ui("editor.launch")),
    (32.0, Action::ExpectEditor(None)),
    (36.0, Action::Capture("plane_on_runway.png")),
    (36.5, Action::Exit),
];

/// `OSP_CAPTURE_EDITOR_PARTS=1`: the parts list as a grid of pictures. A new vessel, and each tab
/// of the list; the engines narrowed to those that burn hydrolox, then to those 3 to 4 m across,
/// each with a click on its chip; a tile pointed at, its figures under the grid; a search over every
/// tab; and a tab's tooltip.
const EDITOR_PARTS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.new")),
    (1.5, Action::Ui("editor.new")),
    (2.0, Action::Unlock),
    (2.1, Action::ExpectData("editor.rail == parts")),
    (2.2, Action::Ui("editor.group:pods")),
    (4.5, Action::Capture("parts_pods.png")),
    (4.6, Action::Ui("editor.group:tanks")),
    (5.6, Action::Capture("parts_tanks.png")),
    (5.7, Action::Ui("editor.group:engines")),
    (8.5, Action::Capture("parts_engines.png")),
    (8.6, Action::Ui("editor.group:structure")),
    (9.6, Action::Capture("parts_structure.png")),
    (9.7, Action::Ui("editor.group:aero")),
    (10.7, Action::Capture("parts_aero.png")),
    (10.8, Action::Ui("editor.group:control")),
    (12.0, Action::Capture("parts_control.png")),
    // Hydrolox engines 3 to 4 m across: two clicks once on the engines.
    (12.1, Action::Ui("editor.group:engines")),
    (12.6, Action::UiPointAt("editor.propellant:hydrolox")),
    (12.7, Action::UiPress(PointerButton::Primary)),
    (12.75, Action::UiRelease(PointerButton::Primary)),
    (13.0, Action::ExpectData("editor.propellant == hydrolox")),
    (14.5, Action::Capture("parts_hydrolox.png")),
    (14.6, Action::UiPointAt("editor.size:3_4")),
    (14.7, Action::UiPress(PointerButton::Primary)),
    (14.75, Action::UiRelease(PointerButton::Primary)),
    (15.0, Action::ExpectData("editor.size == 3_4")),
    (16.0, Action::Capture("parts_hydrolox_3_4.png")),
    // The pointer on the first tile: what it is, under the grid.
    (16.1, Action::UiPointAt("editor.add:")),
    (16.6, Action::ExpectData("editor.pointed")),
    (16.7, Action::Capture("parts_pointed.png")),
    (16.8, Action::Ui("editor.size:any")),
    (16.85, Action::Ui("editor.propellant:any")),
    (16.9, Action::Ui("editor.search:capsule")),
    (16.95, Action::ExpectData("editor.group == search")),
    (18.5, Action::Capture("parts_search.png")),
    (18.6, Action::UiPointAt("editor.group:aero")),
    (19.2, Action::Capture("parts_tab_tooltip.png")),
    (19.5, Action::Exit),
];

/// `OSP_CAPTURE_EDITOR_MOVE=1`: sliding and turning parts with the Move and Turn tools. A tank,
/// another under it and four fins round the first, built with the pointer; a fin picked, slid up
/// the tank by its arrow with the other three going along, and turned by a ring; both undone. The
/// lower tank turned about the stack by going across its ring, seen edge on, then slid down off
/// its joint, which is refused. Then it is launched as built.
const EDITOR_MOVE_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.new")),
    (1.5, Action::Ui("editor.new")),
    (2.0, Action::Unlock),
    (2.1, Action::Ui("editor.group:tanks")),
    (2.2, Action::Ui("editor.add:tank")),
    (3.0, Action::ExpectPartCount(1)),
    (3.1, Action::Ui("editor.add:tank")),
    (3.5, Action::EditorPointAtNode("tank", "bottom")),
    (4.2, Action::Press(MouseButton::Left)),
    (4.3, Action::Release(MouseButton::Left)),
    (5.0, Action::ExpectPartCount(2)),
    (5.1, Action::Ui("editor.add:fin")),
    (5.2, Action::Ui("editor.symmetry")),
    (5.25, Action::Ui("editor.symmetry")),
    (5.3, Action::Ui("editor.symmetry")),
    (
        5.5,
        Action::EditorPointAtSurface("tank", "barrel", 0.625, 0.5),
    ),
    (6.4, Action::Press(MouseButton::Left)),
    (6.5, Action::Release(MouseButton::Left)),
    (7.2, Action::ExpectPartCount(6)),
    (7.25, Action::EditorLook(0.0, 0.0, -6.0)),
    (7.3, Action::ClearPointer),
    (7.4, Action::Ui("editor.pick:fin")),
    (7.5, Action::Ui("editor.tool:move")),
    (7.6, Action::ExpectData("editor.tool == move")),
    (8.0, Action::Capture("move_arrows.png")),
    (8.1, Action::EditorPointAtHandle(1, 0.8)),
    (8.4, Action::Capture("move_pointed.png")),
    (8.5, Action::Press(MouseButton::Left)),
    (8.6, Action::PointerBy(0.0, -40.0)),
    (9.4, Action::Capture("move_dragging.png")),
    (9.5, Action::Release(MouseButton::Left)),
    (10.2, Action::ExpectAlong("fin", 0.55, 1.0)),
    (10.25, Action::ExpectAlong("fin_3", 0.55, 1.0)),
    (10.27, Action::ExpectHeldBy("fin", "root")),
    (10.3, Action::ClearPointer),
    (10.4, Action::Capture("move_moved.png")),
    (10.5, Action::Ui("editor.tool:turn")),
    (11.0, Action::Capture("turn_rings.png")),
    (11.1, Action::EditorPointAtHandle(2, 0.25)),
    (11.2, Action::Press(MouseButton::Left)),
    (11.3, Action::EditorPointAtHandle(2, 0.375)),
    (12.1, Action::Capture("turn_dragging.png")),
    (12.2, Action::Release(MouseButton::Left)),
    (12.9, Action::ClearPointer),
    (13.0, Action::Capture("turn_turned.png")),
    // Undone, the turn and then the slide.
    (13.05, Action::ExpectHeldBy("fin_4", "held_root")),
    (13.1, Action::Ui("editor.undo")),
    (13.5, Action::ExpectHeldBy("fin", "root")),
    (13.55, Action::ExpectAlong("fin", 0.55, 1.0)),
    (13.6, Action::Ui("editor.undo")),
    (14.1, Action::ExpectAlong("fin", 0.49, 0.51)),
    (14.2, Action::ExpectAlong("fin_4", 0.49, 0.51)),
    // The lower tank turned about the stack, then slid off its joint, which is refused.
    (14.3, Action::Ui("editor.pick:tank_2")),
    (14.4, Action::Ui("editor.tool:turn")),
    (14.9, Action::Capture("turn_tank.png")),
    // Seen edge on, the ring turns as the pointer goes across it.
    (15.0, Action::EditorPointAtHandle(2, 0.125)),
    (15.1, Action::Press(MouseButton::Left)),
    (15.2, Action::PointerBy(40.0, 0.0)),
    (16.0, Action::Capture("turn_tank_dragging.png")),
    (16.1, Action::Release(MouseButton::Left)),
    (16.8, Action::ExpectHeldBy("tank_2", "held_top")),
    (16.9, Action::Ui("editor.tool:move")),
    (17.0, Action::EditorPointAtHandle(2, 0.5)),
    (17.2, Action::Press(MouseButton::Left)),
    (17.3, Action::PointerBy(0.0, 60.0)),
    (18.0, Action::Capture("move_tank_dragging.png")),
    (18.1, Action::Release(MouseButton::Left)),
    (18.8, Action::ClearPointer),
    (18.9, Action::Capture("move_tank_refused.png")),
    (19.0, Action::ExpectData("editor.notice")),
    (19.1, Action::Ui("editor.pick:")),
    (19.2, Action::Ui("editor.tool:shape")),
    (19.3, Action::Ui("editor.launch")),
    (20.5, Action::ExpectEditor(None)),
    (24.0, Action::Capture("move_launched.png")),
    (24.5, Action::Exit),
];

/// `OSP_CAPTURE_EDITOR_SHAPE=1`: shaping parts in the editor with the mouse. The S-II lit under the
/// pointer and picked with a click, and let go with a click on nothing. The Saturn V's S-IVB
/// picked, its length handle pointed at and dragged up, then let go; undone and done again, then
/// undone. Its diameter dragged out and let go; undone; dragged a little, so it keeps the Instrument
/// Unit's width; dragged as far as it goes, and undone. An S-IC fin, too small from far off for more
/// than one handle; then the Sounder, with no question as nothing done to the Saturn V is left, a
/// fin's tip dragged out and back, and Esc to let go of it.
const EDITOR_SHAPE_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.new")),
    (1.5, Action::Ui("editor.show:saturn_v")),
    (5.0, Action::EditorLook(0.0, 0.0, 4.0)),
    (5.1, Action::Unlock),
    (5.2, Action::PointerAt(800.0, 500.0)),
    (5.6, Action::Capture("shape_hover.png")),
    (5.65, Action::Press(MouseButton::Left)),
    (5.7, Action::Release(MouseButton::Left)),
    (5.8, Action::ExpectPicked("s_ii")),
    (5.9, Action::PointerAt(1100.0, 250.0)),
    (5.95, Action::Press(MouseButton::Left)),
    (6.0, Action::Release(MouseButton::Left)),
    (6.05, Action::ExpectPicked("")),
    (6.1, Action::Ui("editor.pick:s_ivb")),
    (6.4, Action::Capture("shape_picked.png")),
    (6.45, Action::EditorPointAtGrip("length", 0.0, 0.0)),
    (6.8, Action::Capture("shape_pointed.png")),
    (6.9, Action::Press(MouseButton::Left)),
    (7.1, Action::EditorPointAtGrip("length", 0.0, -40.0)),
    (7.6, Action::Capture("shape_dragging.png")),
    (7.7, Action::EditorPointAtGrip("length", 0.0, -40.0)),
    (8.3, Action::Capture("shape_dragged.png")),
    (8.4, Action::Release(MouseButton::Left)),
    (9.2, Action::Capture("shape_longer.png")),
    (9.3, Action::ExpectSize("s_ivb", "length", 12.0, 40.0)),
    (9.4, Action::Ui("editor.undo")),
    (10.0, Action::Capture("shape_undone.png")),
    (10.1, Action::ExpectSize("s_ivb", "length", 11.34, 11.35)),
    (10.2, Action::Ui("editor.redo")),
    (10.8, Action::ExpectSize("s_ivb", "length", 12.0, 40.0)),
    (10.9, Action::Ui("editor.undo")),
    (11.5, Action::EditorPointAtGrip("diameter", 0.0, 0.0)),
    (12.0, Action::Press(MouseButton::Left)),
    (12.2, Action::EditorPointAtGrip("diameter", 25.0, 0.0)),
    (12.8, Action::Capture("shape_wider.png")),
    (12.9, Action::Release(MouseButton::Left)),
    (13.6, Action::ExpectSize("s_ivb", "diameter", 7.0, 16.0)),
    (13.7, Action::Ui("editor.undo")),
    (14.3, Action::EditorPointAtGrip("diameter", 0.0, 0.0)),
    (14.4, Action::Press(MouseButton::Left)),
    (14.6, Action::EditorPointAtGrip("diameter", 4.0, 0.0)),
    (15.0, Action::Capture("shape_matched.png")),
    (15.1, Action::Release(MouseButton::Left)),
    (15.8, Action::ExpectSize("s_ivb", "diameter", 6.6, 6.6)),
    (15.9, Action::EditorPointAtGrip("diameter", 0.0, 0.0)),
    (16.0, Action::Press(MouseButton::Left)),
    (16.2, Action::EditorPointAtGrip("diameter", 1000.0, 0.0)),
    (16.8, Action::Capture("shape_as_wide_as_it_goes.png")),
    (16.9, Action::Release(MouseButton::Left)),
    (17.6, Action::ExpectSize("s_ivb", "diameter", 16.0, 16.1)),
    (17.7, Action::Ui("editor.undo")),
    (18.3, Action::Ui("editor.pick:s_ic_fin_3")),
    (18.4, Action::EditorLook(0.0, 0.0, -10.0)),
    (19.0, Action::Capture("shape_fin_far.png")),
    (19.1, Action::Ui("editor.show:sounder")),
    // Every change to the Saturn V was undone or came to nothing, so nothing asks.
    (19.2, Action::ExpectDialog("")),
    (20.5, Action::Ui("editor.pick:sounder_fin_1")),
    (21.0, Action::Capture("shape_fin.png")),
    (21.1, Action::EditorPointAtGrip("span", 0.0, 0.0)),
    (21.3, Action::Press(MouseButton::Left)),
    (21.5, Action::EditorPointAtGrip("span", 30.0, 20.0)),
    (22.2, Action::Capture("shape_fin_dragging.png")),
    (22.3, Action::Release(MouseButton::Left)),
    (
        23.0,
        Action::ExpectSize("sounder_fin_1", "span", 1.15, 20.0),
    ),
    (23.1, Action::Capture("shape_fin_dragged.png")),
    (23.2, Action::Ui("game.menu")),
    (23.7, Action::Capture("shape_let_go.png")),
    (23.8, Action::ExpectEditor(Some("sounder"))),
    (24.0, Action::Exit),
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
    (3.5, Action::Ui("ask.clear")),
    (3.9, Action::Capture("pad_clear_asked.png")),
    (4.0, Action::ExpectEditor(Some("sounder"))),
    (4.05, Action::ExpectDialog("Destroy Pathfinder?")),
    (4.1, Action::Ui("dialog.answer:0")),
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
    (14.8, Action::Ui("ask.destroy:3")),
    (15.2, Action::Capture("vessel_destroy_asked.png")),
    (15.3, Action::ExpectGame(3, "Satellite")),
    (15.35, Action::ExpectDialog("Destroy Sounder?")),
    (15.4, Action::Ui("dialog.answer:0")),
    (15.8, Action::Capture("vessel_destroyed.png")),
    (15.9, Action::ExpectGame(2, "Satellite")),
    (16.0, Action::ExpectScreen("vessels")),
    // The one being flown: another is flown instead.
    (16.1, Action::Ui("ask.destroy:9")),
    (16.3, Action::Ui("dialog.answer:0")),
    (16.8, Action::ExpectGame(1, "Kestrel")),
    // The last: only the hangar is left, and back from there the title screen.
    (16.9, Action::Ui("ask.destroy:6")),
    (17.1, Action::Ui("dialog.answer:0")),
    (18.0, Action::Capture("nothing_left.png")),
    (18.1, Action::ExpectEditor(Some(""))),
    (18.2, Action::Ui("menu.back")),
    (19.2, Action::ExpectEditor(None)),
    (19.3, Action::ExpectScreen("title")),
    (19.4, Action::Capture("title_after_the_last.png")),
    (19.8, Action::Exit),
];

/// `OSP_SCENARIO=apollo11 OSP_CAPTURE_SATURN_PAD=1`: the whole stack on the pad with its tower, and
/// the spacecraft's window.
const SATURN_PAD_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (0.6, Action::Ui("ui.show:spacecraft_info")),
    (0.7, Action::LookFromSun(130.0)),
    (4.0, Action::Capture("saturn_pad.png")),
    (4.5, Action::Exit),
];

/// `OSP_SCENARIO=apollo11_parachutes OSP_CAPTURE_CHUTES=1`: the drogues open, then the mains.
const CHUTES_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (0.6, Action::LookFrom(70.0, 90.0, 5.0)),
    (5.0, Action::Capture("chutes_drogues.png")),
    (22.0, Action::LookFrom(130.0, 90.0, 3.0)),
    (24.0, Action::Capture("chutes_mains.png")),
    (24.5, Action::Exit),
];

/// `OSP_SCENARIO=apollo11_splashdown OSP_CAPTURE_SPLASH=1`: down on the water, and afloat.
const SPLASH_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (0.6, Action::LookFrom(45.0, 90.0, 5.0)),
    (6.0, Action::Capture("splash_coming.png")),
    (20.0, Action::LookFrom(18.0, 90.0, 12.0)),
    (22.0, Action::Capture("splash_afloat.png")),
    (22.5, Action::Exit),
];

/// `OSP_SCENARIO=apollo11_transposition OSP_CAPTURE_TRANSPOSITION=1`: the adapter closed on the
/// S-IVB, its panels opening as the CSM lets go, and the lunar module left standing in what remains.
const TRANSPOSITION_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (0.6, Action::LookFromSun(70.0)),
    // Its ullage motors, lit by the sequences that went before, burn out first.
    (8.0, Action::Capture("transposition_stack.png")),
    (8.1, Action::Section),
    (8.3, Action::Ui("manifest.follow:5")),
    (8.4, Action::LookFromSun(70.0)),
    (10.0, Action::Capture("transposition_panels_opening.png")),
    (14.0, Action::LookFromSun(28.0)),
    (16.0, Action::Capture("transposition_lander.png")),
    (16.1, Action::ExpectGame(2, "Saturn V")),
    (16.5, Action::Exit),
];

/// `OSP_SCENARIO=rendezvous OSP_CAPTURE_BOXES=1`: the other vessel's box, clicked with the pointer.
/// In the map it sets the target, without dropping a burn on the trajectory running beside it. In
/// the flight view the box is there because it is the target, however far, and clears it; then it is
/// too far for a box. Back in the map a right-click switches to it, staying in the map. Then a
/// planet's box, by a right-click.
const BOXES_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(true)),
    (0.6, Action::MapDistance(3.0)),
    (0.7, Action::MapOverFlown),
    // The scenario starts with it targeted.
    (0.8, Action::Ui("target.clear")),
    (2.5, Action::Capture("boxes_map.png")),
    (2.6, Action::ExpectMarker("Station", true)),
    (2.7, Action::UiPointAtMarker("Station")),
    (2.9, Action::Press(MouseButton::Left)),
    (2.9, Action::UiPress(PointerButton::Primary)),
    (3.0, Action::Release(MouseButton::Left)),
    (3.0, Action::UiRelease(PointerButton::Primary)),
    (3.5, Action::Capture("boxes_map_menu.png")),
    (3.6, Action::ExpectBurns(0)),
    (3.65, Action::ClearPointer),
    (3.7, Action::UiPointAt("target.vessel:")),
    (3.9, Action::UiPress(PointerButton::Primary)),
    (4.0, Action::UiRelease(PointerButton::Primary)),
    (4.3, Action::ExpectTarget("Station")),
    (4.4, Action::MapView(false)),
    (4.5, Action::LookAtTarget),
    (6.0, Action::Capture("boxes_flight_target.png")),
    (6.1, Action::ExpectMarker("Station", true)),
    (6.2, Action::UiPointAtMarker("Station")),
    (6.4, Action::UiPress(PointerButton::Primary)),
    (6.5, Action::UiRelease(PointerButton::Primary)),
    (7.0, Action::Capture("boxes_flight_menu.png")),
    (7.1, Action::UiPointAt("target.clear")),
    (7.3, Action::UiPress(PointerButton::Primary)),
    (7.4, Action::UiRelease(PointerButton::Primary)),
    (7.7, Action::ExpectTarget("")),
    (7.8, Action::ExpectMarker("Station", false)),
    (8.0, Action::Capture("boxes_flight_far.png")),
    (8.1, Action::MapView(true)),
    (8.2, Action::MapOverFlown),
    (9.5, Action::UiPointAtMarker("Station")),
    (9.7, Action::UiPress(PointerButton::Secondary)),
    (9.8, Action::UiRelease(PointerButton::Secondary)),
    (9.85, Action::ClearPointer),
    (10.3, Action::Capture("boxes_map_right_click.png")),
    (10.4, Action::UiPointAt("vessel.fly:")),
    (10.6, Action::UiPress(PointerButton::Primary)),
    (10.7, Action::UiRelease(PointerButton::Primary)),
    (11.0, Action::ExpectGame(2, "Station")),
    (11.1, Action::ExpectMapView(true)),
    (11.2, Action::ExpectMarker("Station", false)),
    (11.3, Action::ExpectBurns(0)),
    (12.0, Action::Capture("boxes_switched.png")),
    // A planet's box: a right-click gives the menu, which targets it.
    (12.1, Action::MapDistance(150.0)),
    (14.0, Action::ExpectMarker("Moon", true)),
    (14.1, Action::UiPointAtMarker("Moon")),
    (14.2, Action::ClearPointer),
    (14.3, Action::UiPress(PointerButton::Secondary)),
    (14.4, Action::UiRelease(PointerButton::Secondary)),
    (14.9, Action::Capture("boxes_moon_menu.png")),
    (15.0, Action::UiPointAt("target.body:Moon")),
    (15.2, Action::UiPress(PointerButton::Primary)),
    (15.3, Action::UiRelease(PointerButton::Primary)),
    (15.6, Action::ExpectTarget("Moon")),
    (16.0, Action::Exit),
];

/// `OSP_CAPTURE_LEAVING=1` (with `OSP_USER_DIR=<empty folder>`, for the saves): every way out of a
/// game. Esc and Cancel keep it going, Save and continue writes the save, Continue brings it back,
/// Don't save leaves the save alone, and with nothing new to lose nothing is asked. Then what else
/// asks through the same dialog, and a new game from the title screen.
const LEAVING_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.1, Action::ExpectSaves(0)),
    // Never saved. Esc closes the question and does nothing else.
    (1.2, Action::Ui("ask.leave:game.exit")),
    (1.6, Action::Capture("leave_asked.png")),
    (1.7, Action::ExpectDialog("Save this game first?")),
    (1.8, Action::Ui("game.menu")),
    (1.9, Action::ExpectDialog("")),
    (2.0, Action::ExpectScreen("pause")),
    // Cancel leaves the game as it was.
    (2.1, Action::Ui("ask.leave:game.exit")),
    (2.2, Action::Ui("dialog.answer:2")),
    (2.3, Action::ExpectDialog("")),
    (2.4, Action::ExpectScreen("pause")),
    (2.45, Action::ExpectSaves(0)),
    // Save and continue writes the save and goes on out.
    (2.5, Action::Ui("ask.leave:game.exit")),
    (2.6, Action::Ui("dialog.answer:0")),
    (3.2, Action::ExpectDialog("")),
    (3.3, Action::ExpectSaves(1)),
    (3.4, Action::ExpectScreen("title")),
    (3.9, Action::Capture("title_continue.png")),
    // Continue brings it back, and time passes, so there is something to lose again.
    (4.0, Action::Ui("game.continue")),
    (4.5, Action::ExpectScreen("")),
    (5.0, Action::Ui("game.menu")),
    (5.1, Action::Ui("ask.leave:game.exit")),
    (5.2, Action::ExpectDialog("Save this game first?")),
    // Don't save goes out and leaves the save as it was.
    (5.3, Action::Ui("dialog.answer:1")),
    (5.9, Action::ExpectScreen("title")),
    (6.0, Action::ExpectSaves(1)),
    // Just saved, so nothing is asked.
    (6.1, Action::Ui("game.continue")),
    (6.6, Action::Ui("game.menu")),
    (6.7, Action::Ui("game.save")),
    (6.9, Action::Ui("ask.leave:game.exit")),
    (7.4, Action::ExpectDialog("")),
    (7.5, Action::ExpectScreen("title")),
    (7.6, Action::ExpectSaves(1)),
    // Tutorials are reachable again, and a new game starts clean, with nothing asked.
    (7.7, Action::Ui("menu.open:tutorials")),
    (7.8, Action::ExpectScreen("tutorials")),
    (7.9, Action::Ui("menu.back")),
    (8.0, Action::Ui("ask.leave:game.new")),
    (8.2, Action::ExpectDialog("")),
    (8.5, Action::ExpectEditor(Some(""))),
    (8.6, Action::Ui("editor.show:sounder")),
    (10.0, Action::Ui("editor.launch")),
    (11.0, Action::ExpectEditor(None)),
    (11.1, Action::ExpectGame(1, "Sounder")),
    (11.2, Action::ExpectSaves(2)),
    // Back from New game leaves the game untouched, after asking.
    (11.5, Action::Ui("game.menu")),
    (11.6, Action::Ui("ask.leave:game.new")),
    (11.7, Action::ExpectDialog("Save this game first?")),
    (11.8, Action::Ui("dialog.answer:1")),
    (12.3, Action::ExpectEditor(Some(""))),
    (12.4, Action::Ui("menu.back")),
    (12.9, Action::ExpectEditor(None)),
    (13.0, Action::ExpectGame(1, "Sounder")),
    // Deleting a save asks, through the dialog, over the screen it was asked on.
    (13.1, Action::Ui("menu.open:load")),
    (13.2, Action::Ui("ask.delete_save:game_1")),
    (13.7, Action::Capture("delete_asked.png")),
    (13.8, Action::ExpectDialog("Delete Game 1?")),
    (13.9, Action::Ui("dialog.answer:1")),
    (14.0, Action::ExpectScreen("load")),
    (14.1, Action::ExpectSaves(2)),
    (14.2, Action::Ui("ask.delete_save:game_1")),
    (14.3, Action::Ui("dialog.answer:0")),
    (14.6, Action::ExpectSaves(1)),
    (14.7, Action::ExpectScreen("load")),
    // Loading from in a game asks; Cancel stays on the list.
    (14.8, Action::Ui("ask.leave:game.load:game_2")),
    (14.9, Action::ExpectDialog("Save this game first?")),
    (15.0, Action::Ui("dialog.answer:2")),
    (15.1, Action::ExpectScreen("load")),
    (15.2, Action::Ui("menu.back")),
    // Quit asks too. Cancelled first, then answered: this ends the run.
    (15.3, Action::Ui("ask.leave:game.quit")),
    (15.4, Action::ExpectDialog("Save this game first?")),
    (15.5, Action::Ui("dialog.answer:2")),
    (15.6, Action::ExpectScreen("pause")),
    (15.7, Action::Ui("ask.leave:game.quit")),
    (15.8, Action::Ui("dialog.answer:1")),
    (17.0, Action::ExpectScreen("the game should have closed")),
];

/// `OSP_CAPTURE_REVERT=1` (with `OSP_USER_DIR=<empty folder>`, for the saves): a game with nothing
/// launched has nothing to revert to. A launch and a flight, then a revert that is cancelled, one to
/// the launch, and one to the hangar with the staging it left with and the pad clear to launch again,
/// over the same save. A second vessel goes back to the first when the hangar is left; with nothing
/// else in the game the hangar goes to the title screen, and a game loaded from a save cannot revert.
const REVERT_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.1, Action::ExpectRevert(None)),
    (1.2, Action::Capture("revert_none.png")),
    (1.3, Action::Ui("game.new")),
    (1.8, Action::ExpectEditor(Some(""))),
    (1.9, Action::Ui("editor.show:sounder")),
    (3.0, Action::Unlock),
    (3.1, Action::Ui("staging.add")),
    (3.3, Action::ExpectSequenceCount(SOUNDER_SEQUENCES + 1)),
    (3.5, Action::Ui("editor.launch")),
    (4.5, Action::ExpectEditor(None)),
    (4.6, Action::ExpectGame(1, "Sounder")),
    (4.7, Action::ExpectSaves(1)),
    (4.8, Action::ExpectRevert(Some("Sounder"))),
    (4.9, Action::Launch),
    (8.0, Action::Capture("revert_flying.png")),
    (8.1, Action::Ui("game.menu")),
    (8.2, Action::ExpectSinceLaunch(2.0, 8.0)),
    (8.6, Action::Capture("revert_menu.png")),
    // Asked first, and Cancel keeps the flight.
    (8.7, Action::Ui("ask.revert:launch")),
    (9.1, Action::Capture("revert_asked.png")),
    (9.2, Action::ExpectDialog("Revert to launch?")),
    (9.3, Action::Ui("dialog.answer:1")),
    (9.4, Action::ExpectDialog("")),
    (9.45, Action::ExpectScreen("pause")),
    (9.5, Action::ExpectSinceLaunch(2.0, 8.0)),
    // Back at the moment it left, flying.
    (9.6, Action::Ui("ask.revert:launch")),
    (9.7, Action::Ui("dialog.answer:0")),
    (10.0, Action::ExpectDialog("")),
    (10.05, Action::ExpectScreen("")),
    (10.1, Action::ExpectSinceLaunch(0.0, 1.0)),
    (10.2, Action::ExpectGame(1, "Sounder")),
    (10.25, Action::ExpectSequenceCount(SOUNDER_SEQUENCES + 1)),
    (11.0, Action::Capture("revert_launch.png")),
    (11.1, Action::Ui("game.menu")),
    (11.2, Action::Ui("ask.revert:hangar")),
    (11.3, Action::ExpectDialog("Revert to hangar?")),
    (11.4, Action::Ui("dialog.answer:0")),
    (12.0, Action::ExpectEditor(Some("sounder"))),
    (12.05, Action::ExpectScreen("")),
    (12.1, Action::ExpectList(false)),
    (12.15, Action::ExpectSequenceCount(SOUNDER_SEQUENCES + 1)),
    (13.0, Action::Capture("revert_hangar.png")),
    // Nothing stands on the pad, and the game stays in the save it was in.
    (13.1, Action::Ui("editor.launch")),
    (14.0, Action::ExpectEditor(None)),
    (14.1, Action::ExpectGame(1, "Sounder")),
    (14.15, Action::ExpectSaves(1)),
    (14.2, Action::ExpectSequenceCount(SOUNDER_SEQUENCES + 1)),
    (14.3, Action::Ui("game.menu")),
    // A second vessel: the first is still there to go back to from the hangar.
    (14.4, Action::Ui("editor.open")),
    (14.9, Action::Ui("editor.show:glider")),
    (16.5, Action::Ui("editor.place:orbit")),
    (16.6, Action::Ui("editor.launch")),
    (17.5, Action::ExpectGame(2, "Kestrel")),
    (17.6, Action::ExpectRevert(Some("Kestrel"))),
    (17.7, Action::Ui("game.menu")),
    (17.8, Action::Ui("ask.revert:hangar")),
    (17.9, Action::Ui("dialog.answer:0")),
    (18.8, Action::ExpectEditor(Some("glider"))),
    (18.9, Action::Ui("menu.back")),
    (19.5, Action::ExpectEditor(None)),
    (19.6, Action::ExpectScreen("pause")),
    (19.7, Action::ExpectGame(1, "Sounder")),
    // Going back to the launch of a new game's first vessel leaves nothing to go back to.
    (19.8, Action::Ui("game.new")),
    (20.3, Action::Ui("editor.show:sounder")),
    (21.5, Action::Ui("editor.launch")),
    (22.3, Action::ExpectSaves(2)),
    (22.4, Action::Ui("game.menu")),
    (22.5, Action::Ui("ask.revert:hangar")),
    (22.6, Action::Ui("dialog.answer:0")),
    (23.3, Action::ExpectEditor(Some("sounder"))),
    (23.4, Action::Ui("menu.back")),
    (24.0, Action::ExpectEditor(None)),
    (24.1, Action::ExpectScreen("title")),
    (24.2, Action::ExpectRevert(None)),
    (24.3, Action::ExpectSaves(2)),
    // Loaded from a save there is no launch to go back to.
    (24.4, Action::Ui("game.continue")),
    (25.0, Action::ExpectGame(1, "Sounder")),
    (25.1, Action::ExpectRevert(None)),
    (25.5, Action::Exit),
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

/// `OSP_CAPTURE_MODS=1`: New game asks what the game will have. A mod clicked off changes nothing until
/// the game starts; a mode takes its mods, and Start goes to the editor with them. Back from there comes
/// to the screen again, and Realistic has the Saturn V.
const MODS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.1, Action::Ui("game.exit")),
    (2.0, Action::ExpectScreen("title")),
    (2.1, Action::Ui("menu.open:new_game")),
    (2.2, Action::ExpectScreen("new_game")),
    (2.3, Action::ExpectData("mods.count == 8")),
    (2.4, Action::ExpectData("mods.custom")),
    (3.0, Action::Capture("new_game_custom.png")),
    (3.1, Action::UiPointAt("mods.toggle:realism")),
    (3.2, Action::UiPress(PointerButton::Primary)),
    (3.3, Action::UiRelease(PointerButton::Primary)),
    (3.9, Action::ExpectData("mods.count == 8")),
    (4.0, Action::UiPointAt("mods.mode:casual")),
    (4.1, Action::UiPress(PointerButton::Primary)),
    (4.2, Action::UiRelease(PointerButton::Primary)),
    (4.3, Action::ExpectData("!mods.custom")),
    (4.8, Action::Capture("new_game_casual.png")),
    (4.9, Action::ExpectData("mods.count == 8")),
    (5.0, Action::UiPointAt("ask.leave:game.new")),
    (5.1, Action::UiPress(PointerButton::Primary)),
    (5.2, Action::UiRelease(PointerButton::Primary)),
    (10.0, Action::ExpectEditor(Some(""))),
    (10.1, Action::ExpectData("mods.count == 1")),
    (10.2, Action::Capture("new_game_casual_editor.png")),
    (10.3, Action::Ui("menu.back")),
    (11.0, Action::ExpectEditor(None)),
    (11.1, Action::ExpectScreen("new_game")),
    (11.2, Action::Ui("mods.mode:realistic")),
    (11.3, Action::ExpectData("!mods.custom")),
    (11.8, Action::Capture("new_game_realistic.png")),
    (11.9, Action::Ui("ask.leave:game.new")),
    (16.0, Action::ExpectData("mods.count == 7")),
    (16.1, Action::Ui("editor.show:saturn_v")),
    (20.0, Action::ExpectEditor(Some("saturn_v"))),
    (20.1, Action::Exit),
];

/// `OSP_CAPTURE_CAREER=1` (with `OSP_USER_DIR=<empty folder>`): New game offers a sandbox or a career.
/// A Casual career's editor offers only the first node's parts and says how many are still locked,
/// and a stock craft with locked parts is not launched, with what is locked said.
const CAREER_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.1, Action::Ui("game.exit")),
    (2.0, Action::ExpectScreen("title")),
    (2.1, Action::Ui("menu.open:new_game")),
    (2.2, Action::ExpectScreen("new_game")),
    (2.3, Action::Ui("mods.mode:casual")),
    (2.4, Action::ExpectData("!menu.career")),
    (2.5, Action::UiPointAt("game.kind:career")),
    (2.6, Action::UiPress(PointerButton::Primary)),
    (2.65, Action::UiRelease(PointerButton::Primary)),
    (2.8, Action::ExpectData("menu.career")),
    (3.2, Action::Capture("career_new_game.png")),
    (3.3, Action::Ui("ask.leave:game.new")),
    (8.0, Action::ExpectEditor(Some(""))),
    (8.1, Action::Ui("editor.new")),
    (8.5, Action::Unlock),
    (8.6, Action::ExpectData("editor.rail == parts")),
    (8.7, Action::ExpectData("editor.locked")),
    (8.8, Action::Ui("editor.group:engines")),
    (10.5, Action::Capture("career_engines.png")),
    (10.6, Action::Ui("editor.group:pods")),
    (12.0, Action::Capture("career_pods.png")),
    (12.1, Action::Ui("editor.show:pathfinder_crew")),
    (15.0, Action::ExpectEditor(Some("pathfinder_crew"))),
    (15.1, Action::Ui("editor.launch")),
    (15.5, Action::ExpectData("editor.notice")),
    (16.5, Action::Capture("career_locked_launch.png")),
    // Tanks as wide as the tree allows: 2 m in a new Casual career.
    (16.6, Action::Ui("editor.new")),
    (17.0, Action::Ui("editor.group:tanks")),
    (17.1, Action::Ui("editor.add:tank")),
    (17.8, Action::ExpectPartCount(1)),
    (17.9, Action::Ui("editor.pick:tank")),
    (18.2, Action::ExpectData("editor.rail == part")),
    (18.3, Action::ExpectField("tank", "diameter", "2")),
    (18.4, Action::Ui("editor.set:diameter:3")),
    (18.9, Action::ExpectField("tank", "diameter", "2")),
    (19.0, Action::ExpectData("editor.part.problem")),
    (19.5, Action::Capture("career_tank_too_wide.png")),
    // The tech tree from the editor bar: a node picked on it, science for it, unlocked, and its
    // parts in the list after Esc.
    (
        19.55,
        Action::ExpectData("editor.locked == 36 parts are still locked in the tech tree"),
    ),
    (19.6, Action::UiPointAt("tree.open")),
    (19.7, Action::UiPress(PointerButton::Primary)),
    (19.75, Action::UiRelease(PointerButton::Primary)),
    (20.6, Action::ExpectData("tree.node.picked")),
    (20.7, Action::Capture("career_tree.png")),
    (20.8, Action::UiPointAt("tree.pick:basic_rocketry")),
    (20.9, Action::UiPress(PointerButton::Primary)),
    (20.95, Action::UiRelease(PointerButton::Primary)),
    (21.3, Action::ExpectData("tree.picked == basic_rocketry")),
    (21.35, Action::ExpectData("!tree.node.can_unlock")),
    (21.4, Action::Ui("debug.science:12")),
    (21.8, Action::ExpectData("tree.node.can_unlock")),
    (22.0, Action::Capture("career_tree_ready.png")),
    (22.1, Action::UiPointAt("tree.unlock")),
    (22.2, Action::UiPress(PointerButton::Primary)),
    (22.25, Action::UiRelease(PointerButton::Primary)),
    (22.7, Action::ExpectData("career.science == 7")),
    (22.8, Action::Capture("career_tree_unlocked.png")),
    (22.9, Action::Ui("game.menu")),
    (23.4, Action::ExpectPartCount(1)),
    (
        23.5,
        Action::ExpectData("editor.locked == 33 parts are still locked in the tech tree"),
    ),
    (23.6, Action::Ui("editor.parts")),
    (23.7, Action::Ui("editor.group:engines")),
    (24.8, Action::Capture("career_engines_after.png")),
    (24.9, Action::Exit),
];

/// `OSP_CAPTURE_SCIENCE=1`: a capsule with a thermometer and a barometer on its side, from the
/// parts list's Science tab, launched; on the pad they collect their readings, landed on Earth, and
/// say so in their windows.
const SCIENCE_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.new")),
    (1.5, Action::Ui("editor.new")),
    (2.0, Action::Unlock),
    (2.1, Action::Ui("editor.group:pods")),
    (2.2, Action::Ui("editor.add:pod:C-1 Capsule")),
    (3.0, Action::ExpectPartCount(1)),
    (3.1, Action::Ui("editor.add:tank")),
    (3.4, Action::EditorPointAtNode("pod", "bottom")),
    (4.2, Action::Press(MouseButton::Left)),
    (4.3, Action::Release(MouseButton::Left)),
    (5.2, Action::ExpectPartCount(2)),
    (5.3, Action::Ui("editor.group:science")),
    (6.2, Action::Capture("science_parts.png")),
    (6.3, Action::Ui("editor.add:instrument:thermometer")),
    (
        6.6,
        Action::EditorPointAtSurface("tank", "barrel", 0.6, 0.7),
    ),
    (7.4, Action::Capture("science_preview.png")),
    (7.5, Action::Press(MouseButton::Left)),
    (7.6, Action::Release(MouseButton::Left)),
    (8.5, Action::ExpectPartCount(3)),
    (8.6, Action::Ui("editor.add:instrument:barometer")),
    (
        8.9,
        Action::EditorPointAtSurface("tank", "barrel", 0.68, 0.7),
    ),
    (9.7, Action::Press(MouseButton::Left)),
    (9.8, Action::Release(MouseButton::Left)),
    (10.7, Action::ExpectPartCount(4)),
    (10.8, Action::ClearPointer),
    (11.6, Action::Capture("science_on_craft.png")),
    (11.7, Action::Ui("editor.launch")),
    (16.0, Action::PartWindow("Thermometer")),
    (17.0, Action::Capture("science_running.png")),
    // On rails, crew aboard: a whole set of ten minutes in a few seconds.
    (17.1, Action::Warp(5)),
    (32.0, Action::Capture("science_done.png")),
    (32.1, Action::Exit),
];

/// A new Realistic career: a new tank is as wide as a V-2 and of an early tank type, and the later
/// types are not offered.
const CAREER_REALISTIC_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.1, Action::Ui("game.exit")),
    (2.0, Action::ExpectScreen("title")),
    (2.1, Action::Ui("menu.open:new_game")),
    (2.2, Action::ExpectScreen("new_game")),
    (2.3, Action::Ui("mods.mode:realistic")),
    (2.5, Action::UiPointAt("game.kind:career")),
    (2.6, Action::UiPress(PointerButton::Primary)),
    (2.65, Action::UiRelease(PointerButton::Primary)),
    (2.8, Action::ExpectData("menu.career")),
    (3.3, Action::Ui("ask.leave:game.new")),
    (12.0, Action::ExpectEditor(Some(""))),
    (12.1, Action::Ui("editor.new")),
    (12.5, Action::Unlock),
    (12.6, Action::Ui("editor.group:tanks")),
    (12.7, Action::Ui("editor.add:tank")),
    (13.5, Action::ExpectPartCount(1)),
    (13.6, Action::Ui("editor.pick:tank")),
    (13.9, Action::ExpectData("editor.rail == part")),
    (14.0, Action::ExpectField("tank", "diameter", "1.7")),
    (
        14.1,
        Action::ExpectField("tank", "tank_type", "Al Gridded Tank"),
    ),
    (14.2, Action::Ui("editor.expand:tank_type")),
    (14.9, Action::Capture("career_realistic_tank_types.png")),
    (15.0, Action::Ui("editor.set:tank_type:Stiffened Al 2219")),
    (
        15.5,
        Action::ExpectField("tank", "tank_type", "Al Gridded Tank"),
    ),
    (15.6, Action::ExpectData("editor.part.problem")),
    (16.1, Action::Capture("career_realistic_locked_type.png")),
    // RP-1's tree: as it opens, at its start, then zoomed out over all of it.
    (16.2, Action::Ui("tree.open")),
    (17.2, Action::ExpectData("tree.node.picked")),
    (17.3, Action::Capture("career_realistic_tree.png")),
    (17.4, Action::Ui("tree.zoom:out")),
    (17.5, Action::Ui("tree.zoom:out")),
    (17.6, Action::Ui("tree.zoom:out")),
    (18.6, Action::Capture("career_realistic_tree_overview.png")),
    (18.7, Action::Ui("tree.close")),
    (19.2, Action::ExpectPartCount(1)),
    (19.3, Action::Exit),
];

/// `OSP_CAPTURE_SAVE_MODS=1` (with `OSP_USER_DIR=<empty folder>`): a save keeps its mods. The Saturn V
/// is launched and saved with Realism on, Realism is turned off, and the save still brings it back
/// with its models. In that game, the editor of a new game has what new games start with, going back
/// from it has the save's again, and launching from it starts a game with none of the save's.
const SAVE_MODS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.1, Action::Ui("game.new")),
    (1.3, Action::Ui("editor.show:saturn_v")),
    (6.0, Action::ExpectEditor(Some("saturn_v"))),
    (6.1, Action::Ui("editor.place:pad")),
    (6.4, Action::Ui("editor.launch")),
    (10.0, Action::ExpectGame(1, "Saturn V")),
    (10.1, Action::ExpectSaves(1)),
    (10.2, Action::Capture("save_mods_launched.png")),
    (10.4, Action::Ui("game.menu")),
    (10.5, Action::Ui("game.exit")),
    (11.5, Action::ExpectScreen("title")),
    (11.6, Action::Ui("menu.open:new_game")),
    (11.7, Action::Ui("mods.toggle:realism")),
    (11.8, Action::Ui("menu.back")),
    (16.0, Action::ExpectData("mods.count == 5")),
    (16.1, Action::Ui("game.load:game_1")),
    (20.0, Action::ExpectData("mods.count == 6")),
    (20.05, Action::ExpectGame(1, "Saturn V")),
    (20.1, Action::Capture("save_mods_loaded.png")),
    (20.2, Action::Ui("game.menu")),
    (20.3, Action::Ui("game.new")),
    (25.0, Action::ExpectData("mods.count == 5")),
    (25.1, Action::Capture("save_mods_new_game_editor.png")),
    (25.2, Action::Ui("menu.back")),
    (30.0, Action::ExpectData("mods.count == 6")),
    (30.05, Action::ExpectScreen("pause")),
    (30.1, Action::Ui("game.new")),
    (34.0, Action::ExpectData("mods.count == 5")),
    (34.1, Action::Ui("editor.show:sounder")),
    (37.0, Action::Ui("editor.place:pad")),
    (37.3, Action::Ui("editor.launch")),
    (41.0, Action::ExpectGame(1, "Sounder")),
    (41.1, Action::ExpectData("mods.count == 5")),
    (41.2, Action::Ui("game.menu")),
    (41.3, Action::Ui("game.exit")),
    (42.5, Action::ExpectData("mods.count == 5")),
    (42.6, Action::Exit),
];

/// `OSP_CAPTURE_BASE_ENGINES=1` (with `OSP_USER_DIR=<empty folder>`): Casual chosen, and a rocket flown on the base game's engines and
/// liquid fuel: cut, they stop at once and open again. Then back to the first screen, which starts
/// its scene from the base game alone.
const BASE_ENGINES_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.1, Action::Ui("game.exit")),
    (2.0, Action::ExpectScreen("title")),
    (2.1, Action::Ui("menu.open:new_game")),
    (2.2, Action::ExpectData("mods.count == 6")),
    (2.3, Action::Ui("mods.mode:casual")),
    (2.8, Action::Capture("base_engines_mods.png")),
    (2.9, Action::Ui("menu.back")),
    (7.0, Action::ExpectData("mods.count == 1")),
    (7.1, Action::Ui("game.new")),
    (7.3, Action::Ui("editor.show:pathfinder")),
    (11.0, Action::ExpectEditor(Some("pathfinder"))),
    (11.1, Action::Ui("editor.place:pad")),
    (11.4, Action::Ui("editor.launch")),
    (15.0, Action::ExpectGame(1, "Pathfinder")),
    (15.1, Action::Launch),
    (15.5, Action::ExpectData("vessel.thrust")),
    (17.0, Action::Capture("base_engines_liftoff.png")),
    (17.05, Action::Unlock),
    (17.1, Action::Ui("vessel.throttle_cut")),
    (17.5, Action::ExpectData("!vessel.thrust")),
    (17.6, Action::Ui("vessel.throttle_full")),
    (18.0, Action::ExpectData("vessel.thrust")),
    (18.5, Action::Capture("base_engines_relit.png")),
    (18.6, Action::Ui("game.menu")),
    (18.7, Action::Ui("game.exit")),
    (20.0, Action::ExpectScreen("title")),
    (20.1, Action::Exit),
];

/// `OSP_CAPTURE_NO_POD=1` (with `OSP_USER_DIR=<empty folder>`): in a Casual game a tank and an engine with
/// no pod are launched, and neither the throttle nor staging does anything.
const NO_POD_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.1, Action::Ui("game.exit")),
    (2.0, Action::ExpectScreen("title")),
    (2.1, Action::Ui("menu.open:new_game")),
    (2.2, Action::Ui("mods.mode:casual")),
    (2.4, Action::Ui("ask.leave:game.new")),
    (7.0, Action::ExpectData("mods.count == 1")),
    (7.1, Action::Ui("editor.new")),
    (7.6, Action::Unlock),
    (7.7, Action::Ui("editor.add:tank")),
    (8.5, Action::ExpectPartCount(1)),
    (8.6, Action::Ui("editor.add:engine:K-0")),
    (9.2, Action::EditorPointAtNode("tank", "bottom")),
    (9.9, Action::Press(MouseButton::Left)),
    (10.0, Action::Release(MouseButton::Left)),
    (10.8, Action::ExpectPartCount(2)),
    (10.9, Action::Ui("editor.launch")),
    (15.0, Action::Capture("no_pod_on_the_pad.png")),
    (15.1, Action::Ui("vessel.throttle_full")),
    (15.2, Action::Ui("vessel.stage")),
    (16.5, Action::ExpectData("!vessel.throttle")),
    (16.55, Action::ExpectData("!vessel.thrust")),
    (16.6, Action::Capture("no_pod_ignores_orders.png")),
    (16.7, Action::Exit),
];

/// `OSP_CAPTURE_EDITOR_DRAG=1`: a tile dragged out of the parts list shows the part at the pointer: the
/// first part of a craft, then a nose cone out in the hangar and on a node, where it is shown where it
/// would go. Dragged off the craft, the cone is at the pointer again.
const EDITOR_DRAG_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.new")),
    (1.5, Action::Ui("editor.new")),
    (2.0, Action::Unlock),
    (2.2, Action::Ui("editor.group:tanks")),
    (3.2, Action::UiPointAt("editor.add:tank")),
    (3.3, Action::UiPress(PointerButton::Primary)),
    (3.31, Action::Press(MouseButton::Left)),
    (3.5, Action::PointerAt(800.0, 450.0)),
    (4.5, Action::ExpectData("editor.floating")),
    (4.6, Action::Capture("drag_first_part.png")),
    (4.7, Action::UiRelease(PointerButton::Primary)),
    (4.71, Action::Release(MouseButton::Left)),
    (6.0, Action::ExpectPartCount(1)),
    (6.05, Action::ExpectData("!editor.floating")),
    (6.1, Action::Ui("editor.group:structure")),
    (8.0, Action::UiPointAt("editor.add:nose_cone")),
    (8.1, Action::UiPress(PointerButton::Primary)),
    (8.11, Action::Press(MouseButton::Left)),
    (8.3, Action::PointerAt(1050.0, 300.0)),
    (9.3, Action::ExpectData("editor.floating")),
    (9.4, Action::Capture("drag_in_the_air.png")),
    (9.5, Action::EditorPointAtNode("tank", "top")),
    (11.5, Action::ExpectData("!editor.floating")),
    (11.6, Action::Capture("drag_on_a_node.png")),
    (11.7, Action::UiRelease(PointerButton::Primary)),
    (11.71, Action::Release(MouseButton::Left)),
    (13.0, Action::ExpectPartCount(2)),
    (13.5, Action::Capture("drag_put_on.png")),
    (13.6, Action::EditorPointAtPart("nose_cone")),
    (14.2, Action::Press(MouseButton::Left)),
    (14.3, Action::PointerBy(0.0, -40.0)),
    (14.5, Action::PointerAt(1050.0, 300.0)),
    (15.7, Action::ExpectData("editor.floating")),
    (15.8, Action::Capture("drag_off_the_craft.png")),
    (15.9, Action::Release(MouseButton::Left)),
    (17.0, Action::ExpectPartCount(1)),
    (17.1, Action::ExpectData("!editor.floating")),
    (17.2, Action::Exit),
];

/// `OSP_CAPTURE_FUEL_LINE=1`: a fuel line put on a tank beside the first, and a click on the first tells
/// it what to feed.
const FUEL_LINE_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.new")),
    (1.5, Action::Ui("editor.new")),
    (2.0, Action::Unlock),
    (2.2, Action::Ui("editor.add:tank")),
    (3.2, Action::ExpectPartCount(1)),
    (3.3, Action::Ui("editor.add:tank")),
    (3.6, Action::EditorPointAtPart("tank")),
    (3.7, Action::PointerBy(55.0, 0.0)),
    (4.6, Action::Press(MouseButton::Left)),
    (4.7, Action::Release(MouseButton::Left)),
    (5.8, Action::ExpectPartCount(2)),
    (5.9, Action::Ui("editor.add:fuel_line")),
    (6.2, Action::EditorPointAtPart("tank_2")),
    (6.3, Action::PointerBy(-70.0, -60.0)),
    (7.2, Action::Press(MouseButton::Left)),
    (7.3, Action::Release(MouseButton::Left)),
    (8.4, Action::ExpectPartCount(3)),
    (8.5, Action::ExpectData("editor.linking")),
    (8.6, Action::Capture("fuel_line_placed.png")),
    (8.7, Action::EditorPointAtPart("tank")),
    (9.5, Action::Press(MouseButton::Left)),
    (9.6, Action::Release(MouseButton::Left)),
    (10.6, Action::Capture("fuel_line_linked.png")),
    (10.7, Action::ExpectData("!editor.linking")),
    (10.9, Action::Exit),
];

/// `OSP_CAPTURE_PROBE=1`: a probe built only from the parts list, a core with a battery on top, a dish
/// on that and a solar panel on its side, put in orbit with a radio.
const PROBE_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.new")),
    (1.5, Action::Ui("editor.new")),
    (2.0, Action::Unlock),
    (2.2, Action::Ui("editor.group:systems")),
    (4.0, Action::Capture("probe_parts.png")),
    (4.1, Action::Ui("editor.add:probe_core")),
    (5.1, Action::ExpectPartCount(1)),
    (5.2, Action::Ui("editor.add:battery")),
    (5.5, Action::EditorPointAtNode("probe_core", "top")),
    (6.3, Action::Press(MouseButton::Left)),
    (6.4, Action::Release(MouseButton::Left)),
    (7.5, Action::ExpectPartCount(2)),
    (7.6, Action::Ui("editor.add:dish")),
    (7.9, Action::EditorPointAtNode("battery", "top")),
    (8.7, Action::Press(MouseButton::Left)),
    (8.8, Action::Release(MouseButton::Left)),
    (9.9, Action::ExpectPartCount(3)),
    (10.0, Action::Ui("editor.add:solar_panel")),
    (10.3, Action::EditorPointAtPart("probe_core")),
    (11.1, Action::Press(MouseButton::Left)),
    (11.2, Action::Release(MouseButton::Left)),
    (12.3, Action::ExpectPartCount(4)),
    (12.8, Action::Capture("probe_built.png")),
    (12.9, Action::Ui("editor.place:orbit")),
    (13.2, Action::Ui("editor.launch")),
    (17.0, Action::ExpectData("vessel.has_radio")),
    (17.1, Action::Capture("probe_in_orbit.png")),
    (17.2, Action::Exit),
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

/// `OSP_SCENARIO=rendezvous OSP_CAPTURE_NODE_INPUT=1`: a burn's Δv typed into its boxes, then
/// flown. A burn placed on the map; its prograde box in the node panel clicked (what it
/// showed in teal), typed into and handed in with Enter; the normal one the same in the
/// Flight plan window; the radial one typed into and put away with Escape, which leaves it and
/// opens no menu. Then the executor flies it: the node stays where it was planned, the path
/// after it stays put and the Δv left goes down, until the burn is done.
const NODE_INPUT_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(true)),
    (0.6, Action::Ui("ui.show:flight_plan")),
    (3.0, Action::PointAt(240.0)),
    (3.5, Action::Press(MouseButton::Left)),
    (3.6, Action::Release(MouseButton::Left)),
    (3.7, Action::ClearPointer),
    (3.8, Action::ExpectBurns(1)),
    (4.2, Action::UiPointAt("plan.set_dv:prograde:$text")),
    (4.4, Action::UiPress(PointerButton::Primary)),
    (4.5, Action::UiRelease(PointerButton::Primary)),
    (5.0, Action::Capture("node_input_clicked.png")),
    (5.1, Action::Type("40.5")),
    (5.6, Action::Capture("node_input_typed.png")),
    (5.7, Action::Type("\n")),
    (6.2, Action::Capture("node_input_set.png")),
    (6.3, Action::ExpectData("burn.prograde == 40.50")),
    (6.4, Action::UiPointAt("plan.set_dv:normal:$text")),
    (6.6, Action::UiPress(PointerButton::Primary)),
    (6.7, Action::UiRelease(PointerButton::Primary)),
    (6.8, Action::Type("-6\n")),
    (7.0, Action::ExpectData("burn.normal == -6")),
    (7.1, Action::UiPointAt("plan.set_dv:radial:$text")),
    (7.3, Action::UiPress(PointerButton::Primary)),
    (7.4, Action::UiRelease(PointerButton::Primary)),
    (7.5, Action::Type("99\x1b")),
    (7.7, Action::ExpectData("burn.radial == 0")),
    (7.8, Action::ExpectScreen("")),
    (8.0, Action::Capture("node_input_planned.png")),
    (8.1, Action::Ui("plan.execute")),
    (8.2, Action::Ui("warp.to:burn")),
    (24.0, Action::Capture("node_input_burn_1.png")),
    (27.0, Action::Capture("node_input_burn_2.png")),
    (30.0, Action::Capture("node_input_burn_3.png")),
    (40.0, Action::Capture("node_input_done.png")),
    (40.1, Action::ExpectBurns(0)),
    (40.5, Action::Exit),
];

/// `OSP_SCENARIO=rendezvous OSP_CAPTURE_RENDEZVOUS=1`: the target windows, the map with
/// the station, closest approaches and plane nodes, then a planned plane match, and an alarm at the
/// closest approach.
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
    // Near enough to click on its parts, so no box in the way of them.
    (5.9, Action::ExpectMarker("Eagle", false)),
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
    (8.1, Action::Ui("target.alarm_approach")),
    (8.2, Action::ExpectData("alarms.count == 1")),
    (8.3, Action::Ui("ui.show:alarm_clock")),
    (8.9, Action::Capture("rendezvous_alarm.png")),
    (9.0, Action::Exit),
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

/// In LEO (`OSP_CAPTURE_TRANSFER=1`): target the Moon, plot transfers, set an alarm for the
/// cheapest one's departure, make its burn and show the trajectory it leads to.
const TRANSFER_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("target.body:Moon")),
    (1.2, Action::Ui("ui.show:transfer_planner")),
    (1.5, Action::Ui("transfer.compute")),
    (5.0, Action::Capture("transfer_plot.png")),
    (5.1, Action::Ui("transfer.alarm")),
    (
        5.15,
        Action::ExpectData("alarms.next_title == Departure for Moon"),
    ),
    (5.2, Action::Ui("transfer.create")),
    (5.4, Action::Ui("ui.hide:transfer_planner")),
    (5.4, Action::Ui("ui.show:flight_plan")),
    (5.6, Action::Ui("plan.aim:100000")),
    (6.2, Action::Ui("plan.circularize:pe")),
    (6.4, Action::MapView(true)),
    (8.0, Action::Capture("transfer_map.png")),
    (8.5, Action::Exit),
];

/// `OSP_SCENARIO=pad OSP_CAPTURE_NIGHT=1`: a launch at night, from the pad to where the cities show,
/// with nothing of the HUD in the way. From the side, from under the nose, and from above with the
/// ground behind.
const NIGHT_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("target.body:Moon")),
    (1.2, Action::Ui("ui.show:transfer_planner")),
    (1.5, Action::Ui("transfer.compute")),
    (2.0, Action::Ui("transfer.alarm_launch:0")),
    (2.05, Action::Ui("alarm.warp:0")),
    (2.5, Action::Ui("ui.hide:transfer_planner")),
    (2.6, Action::Ui("ui.hide_group:flight")),
    (14.0, Action::LookFrom(70.0, 200.0, 12.0)),
    (15.0, Action::Capture("night_pad.png")),
    (15.2, Action::Launch),
    (18.0, Action::LookFrom(70.0, 200.0, 12.0)),
    (19.0, Action::Capture("night_liftoff.png")),
    (26.0, Action::LookFrom(70.0, 40.0, 25.0)),
    (27.0, Action::Capture("night_t12_above.png")),
    (45.0, Action::LookFrom(70.0, 40.0, 25.0)),
    (46.0, Action::Capture("night_t30_above.png")),
    (70.0, Action::LookFrom(40.0, 100.0, 8.0)),
    (71.0, Action::Capture("night_t55_close.png")),
    (72.0, Action::LookFrom(120.0, 220.0, 35.0)),
    (73.0, Action::Capture("night_t57_above.png")),
    (90.0, Action::LookFrom(40.0, 40.0, 8.0)),
    (91.0, Action::Capture("night_t75_close.png")),
    (92.0, Action::LookFrom(120.0, 40.0, 35.0)),
    (93.0, Action::Capture("night_t77_above.png")),
    (110.0, Action::LookFrom(120.0, 40.0, 35.0)),
    (111.0, Action::Capture("night_t95_above.png")),
    (114.0, Action::Exit),
];

/// `OSP_SCENARIO=pad OSP_CAPTURE_ASCENT=1`: the same by day, closer, to see what the air does to the
/// rocket through the fastest part of the climb.
const ASCENT_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("ui.hide_group:flight")),
    (2.0, Action::Launch),
    (21.0, Action::LookFrom(45.0, 40.0, 5.0)),
    (22.0, Action::Capture("ascent_t20.png")),
    (41.0, Action::LookFrom(45.0, 40.0, 5.0)),
    (42.0, Action::Capture("ascent_t40.png")),
    (56.0, Action::LookFrom(45.0, 40.0, 5.0)),
    (57.0, Action::Capture("ascent_t55.png")),
    (66.0, Action::LookFrom(45.0, 40.0, 5.0)),
    (67.0, Action::Capture("ascent_t65.png")),
    (76.0, Action::LookFrom(45.0, 40.0, 5.0)),
    (77.0, Action::Capture("ascent_t75.png")),
    (86.0, Action::LookFrom(45.0, 200.0, 5.0)),
    (87.0, Action::Capture("ascent_t85_far_side.png")),
    (101.0, Action::LookFrom(45.0, 40.0, 5.0)),
    (102.0, Action::Capture("ascent_t100.png")),
    (103.0, Action::Exit),
];

/// `OSP_CAPTURE_FALLING=1`, with `OSP_SCENARIO` a scenario starting fast in the air: what the air does
/// to it, from two sides close to and further off, soon after the start and again once it has had
/// time to turn into the flow.
const FALLING_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (0.6, Action::Ui("ui.hide_group:flight")),
    (2.0, Action::LookFrom(30.0, 90.0, 5.0)),
    (2.8, Action::Capture("falling_close.png")),
    (3.0, Action::LookFrom(80.0, 90.0, 5.0)),
    (3.8, Action::Capture("falling_far.png")),
    (4.0, Action::LookFrom(80.0, 250.0, 15.0)),
    (4.8, Action::Capture("falling_far_other_side.png")),
    (20.0, Action::LookFrom(30.0, 90.0, 5.0)),
    (20.8, Action::Capture("falling_later_close.png")),
    (21.0, Action::LookFrom(30.0, 180.0, 5.0)),
    (21.8, Action::Capture("falling_later_other_side.png")),
    (22.6, Action::Exit),
];

/// `OSP_CAPTURE_POWER=1`: a vessel on batteries alone in the Resources window. On the pad it is on
/// ground power and nothing runs down, however long it waits. Put in orbit, its batteries run down as
/// time goes by, until they run out. Its avionics draw nothing, so it keeps control and only its other
/// loads go.
const POWER_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.new")),
    (1.5, Action::Ui("editor.show:lunar_module")),
    (4.0, Action::Ui("editor.launch")),
    (6.0, Action::Ui("ui.show:resources")),
    (6.5, Action::ExpectData("vessel.has_power")),
    (6.6, Action::ExpectData("vessel.power_supplied")),
    (6.7, Action::ExpectData("!vessel.power_draining")),
    (6.8, Action::ExpectData("vessel.power_fraction == 1")),
    (7.0, Action::Warp(9)),
    (12.0, Action::Capture("power_on_the_pad.png")),
    (12.1, Action::ExpectData("vessel.power_fraction == 1")),
    (12.2, Action::ExpectData("!vessel.power_shedding")),
    (12.3, Action::Warp(0)),
    // The same vessel in orbit, with nothing to keep it up.
    (12.5, Action::Ui("game.menu")),
    (12.6, Action::Ui("editor.open")),
    (13.0, Action::Ui("editor.show:lunar_module")),
    (13.5, Action::Ui("editor.place:orbit")),
    (16.0, Action::Ui("editor.launch")),
    (18.0, Action::ExpectData("!vessel.power_supplied")),
    (18.1, Action::ExpectData("vessel.power_draining")),
    (18.5, Action::Capture("power_start.png")),
    (18.7, Action::Warp(9)),
    (30.0, Action::Capture("power_warped.png")),
    (50.0, Action::ExpectData("vessel.power_shedding")),
    (50.5, Action::Capture("power_gone.png")),
    (51.3, Action::Exit),
];

/// `OSP_CAPTURE_WINDOWS=1` (with `OSP_USER_DIR=<a folder>`, and its `windows.json` if there is to be
/// one): the stage stack is open from the start, whatever that file says, and back after every launch
/// though the empty hangar closes it, a revert to the hangar included. What the player chooses of the
/// other windows is saved.
const WINDOWS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::ExpectOpen("staging", true)),
    (1.1, Action::Ui("ui.show:flight_info")),
    (1.3, Action::ExpectRemembered("flight_info", true)),
    (1.4, Action::Ui("game.menu")),
    (1.5, Action::Ui("game.new")),
    (2.2, Action::ExpectEditor(Some(""))),
    (2.3, Action::ExpectOpen("staging", false)),
    (2.6, Action::Ui("editor.show:sounder")),
    (3.8, Action::ExpectOpen("staging", true)),
    (3.9, Action::Ui("editor.launch")),
    (4.8, Action::ExpectEditor(None)),
    (4.9, Action::ExpectOpen("staging", true)),
    (5.1, Action::Ui("game.menu")),
    (5.2, Action::Ui("game.revert:hangar")),
    (6.0, Action::ExpectEditor(Some("sounder"))),
    (6.1, Action::Ui("editor.launch")),
    (7.0, Action::ExpectEditor(None)),
    (7.1, Action::ExpectOpen("staging", true)),
    (7.6, Action::Capture("windows_relaunched.png")),
    // Closed by the player, it stays closed through the hangar, until the game is next run.
    (7.7, Action::Ui("ui.toggle:staging")),
    (8.0, Action::Ui("game.menu")),
    (8.1, Action::Ui("editor.open")),
    (8.8, Action::Ui("menu.back")),
    (9.4, Action::ExpectEditor(None)),
    (9.5, Action::ExpectOpen("staging", false)),
    (9.6, Action::Exit),
];

/// On the pad (`OSP_SCENARIO=pad OSP_CAPTURE_LAUNCH_TIMES=1`): target the Moon and ask when to launch,
/// set an alarm for the first time, warp to it from the alarm clock and go. Leaving the ground takes the list away, and the plot is then what
/// there is.
const LAUNCH_TIMES_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("target.body:Moon")),
    (1.2, Action::Ui("ui.show:transfer_planner")),
    (1.4, Action::ExpectData("!transfer.on_ground")),
    (1.5, Action::Ui("transfer.compute")),
    (2.5, Action::ExpectData("transfer.on_ground")),
    (2.6, Action::ExpectData("transfer.has_windows")),
    (2.7, Action::ExpectData("transfer.launch_far")),
    (3.0, Action::Capture("launch_times.png")),
    (3.1, Action::Ui("transfer.alarm_launch:0")),
    (3.2, Action::ExpectData("alarms.count == 1")),
    (3.3, Action::Ui("ui.show:alarm_clock")),
    (3.8, Action::Capture("launch_times_alarm.png")),
    (3.9, Action::Ui("alarm.warp:0")),
    (15.0, Action::Capture("launch_times_warped.png")),
    (15.05, Action::ExpectData("!alarms.has")),
    (15.06, Action::ExpectData("time.warp == 1")),
    (15.1, Action::ExpectData("transfer.on_ground")),
    (15.2, Action::Launch),
    (18.0, Action::ExpectData("!transfer.on_ground")),
    (18.5, Action::Capture("launch_times_gone.png")),
    (19.0, Action::Exit),
];

/// `OSP_CAPTURE_QUICKSAVE=1` (with `OSP_USER_DIR=<an empty folder>`): F5 and F9. A quicksave in a
/// game with no save starts the save; after a launch a tap of F9 only says to hold it, and holding it
/// goes back to the pad. A second quicksave after another launch; both under their save on the Load
/// screen, where the save itself was not touched so loading one asks; F9 then takes the newest. One
/// deleted alone, then the save with the other.
const QUICKSAVE_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::ExpectSaves(0)),
    (1.1, Action::Key(KeyCode::F5, true)),
    (1.2, Action::Key(KeyCode::F5, false)),
    (1.4, Action::ExpectSaves(1)),
    (1.5, Action::ExpectQuicksaves(1)),
    (
        1.6,
        Action::ExpectData("saves.flash == Saved as Quicksave 1"),
    ),
    (1.8, Action::Capture("quicksaved.png")),
    // Off the pad, so there is something for a quickload to undo.
    (2.0, Action::Launch),
    (5.0, Action::ExpectData("vessel.engines_running")),
    (5.1, Action::Key(KeyCode::F9, true)),
    (
        5.3,
        Action::ExpectData("saves.flash == Keep holding to load Quicksave 1"),
    ),
    (5.4, Action::Capture("quickload_hold.png")),
    (5.5, Action::Key(KeyCode::F9, false)),
    (6.8, Action::ExpectData("vessel.engines_running")),
    (7.0, Action::Key(KeyCode::F9, true)),
    (8.5, Action::Key(KeyCode::F9, false)),
    (8.6, Action::ExpectData("!vessel.engines_running")),
    (8.7, Action::ExpectData("saves.flash == Loaded Quicksave 1")),
    (8.8, Action::ExpectQuicksaves(1)),
    (9.0, Action::Capture("quickloaded.png")),
    (9.5, Action::Launch),
    (12.0, Action::Ui("game.quicksave")),
    (12.1, Action::ExpectQuicksaves(2)),
    (12.2, Action::ExpectSaves(1)),
    (12.3, Action::Ui("game.menu")),
    (12.4, Action::Ui("menu.open:load")),
    (13.0, Action::Capture("load_quicksaves.png")),
    (13.1, Action::Ui("ask.leave:game.load:game_1/quicksave_1")),
    (13.2, Action::ExpectDialog("Save this game first?")),
    (13.3, Action::Ui("dialog.answer:1")),
    (13.8, Action::ExpectScreen("")),
    (13.9, Action::ExpectData("!vessel.engines_running")),
    (14.0, Action::Ui("game.quickload")),
    (14.5, Action::ExpectData("vessel.engines_running")),
    (
        14.6,
        Action::ExpectData("saves.flash == Loaded Quicksave 2"),
    ),
    (14.7, Action::Ui("game.menu")),
    (14.8, Action::Ui("menu.open:load")),
    (14.9, Action::Ui("ask.delete_save:game_1/quicksave_1")),
    (15.0, Action::ExpectDialog("Delete Quicksave 1?")),
    (15.1, Action::Ui("dialog.answer:0")),
    (15.3, Action::ExpectQuicksaves(1)),
    (15.4, Action::ExpectSaves(1)),
    (15.5, Action::Ui("ask.delete_save:game_1")),
    (15.6, Action::ExpectDialog("Delete Game 1?")),
    (15.9, Action::Capture("delete_with_quicksaves.png")),
    (16.0, Action::Ui("dialog.answer:0")),
    (16.2, Action::ExpectSaves(0)),
    (16.5, Action::Exit),
];

/// `OSP_CAPTURE_SAVE=1` (with `OSP_USER_DIR=<an empty folder>`): the Save screen. New saves, saving
/// over this game's own save at once and over another's after a question, each back on Pause; then
/// the Vessels screen with the one flown marked, and a volume dragged on its slider.
const SAVE_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.1, Action::ExpectSaves(0)),
    (1.2, Action::Ui("menu.open:save")),
    (1.3, Action::ExpectScreen("save")),
    (1.8, Action::Capture("save_none.png")),
    (1.9, Action::UiPointAt("game.save_as")),
    (2.0, Action::UiPress(PointerButton::Primary)),
    (2.1, Action::UiRelease(PointerButton::Primary)),
    (2.3, Action::ExpectScreen("pause")),
    (2.35, Action::ExpectSaves(1)),
    (2.4, Action::ExpectData("menu.save_name == Game 1")),
    (2.8, Action::Capture("save_back_on_pause.png")),
    (2.9, Action::Ui("menu.open:save")),
    (3.0, Action::Ui("game.save_as")),
    (3.1, Action::ExpectSaves(2)),
    (3.15, Action::ExpectData("menu.save_name == Game 2")),
    (3.2, Action::Ui("menu.open:save")),
    (3.8, Action::Capture("save_two.png")),
    // Its own save: no question.
    (3.9, Action::Ui("ask.overwrite:game_2")),
    (4.0, Action::ExpectDialog("")),
    (4.05, Action::ExpectScreen("pause")),
    (4.1, Action::ExpectSaves(2)),
    // Another game's: asked, and Cancel leaves both as they are.
    (4.2, Action::Ui("menu.open:save")),
    (4.3, Action::Ui("ask.overwrite:game_1")),
    (4.4, Action::ExpectDialog("Save over Game 1?")),
    (4.9, Action::Capture("save_over_asked.png")),
    (5.0, Action::Ui("dialog.cancel")),
    (5.1, Action::ExpectScreen("save")),
    (5.15, Action::ExpectData("menu.save_name == Game 2")),
    (5.2, Action::Ui("ask.overwrite:game_1")),
    (5.3, Action::Ui("dialog.answer:0")),
    (5.4, Action::ExpectDialog("")),
    (5.45, Action::ExpectScreen("pause")),
    (5.5, Action::ExpectSaves(2)),
    (5.55, Action::ExpectData("menu.save_name == Game 1")),
    (5.6, Action::Ui("menu.open:save")),
    (6.2, Action::Capture("save_over_done.png")),
    (6.3, Action::Ui("menu.back")),
    (6.4, Action::Ui("menu.open:vessels")),
    (7.0, Action::Capture("save_vessels.png")),
    (7.1, Action::Ui("menu.back")),
    (7.2, Action::Ui("menu.open:settings")),
    (7.3, Action::Ui("ui.tab:settings:sound")),
    (7.4, Action::Ui("audio.set:master:100")),
    (7.5, Action::UiPointAlong("audio.set:master", 0.5)),
    (7.6, Action::UiPress(PointerButton::Primary)),
    (7.7, Action::ExpectData("audio.master == 50")),
    (7.8, Action::UiPointAlong("audio.set:master", 0.3)),
    (7.9, Action::UiPointAlong("audio.set:master", 0.25)),
    (8.0, Action::ExpectData("audio.master == 25")),
    // Past the end, it stops at none.
    (8.1, Action::UiPointAlong("audio.set:master", -0.4)),
    (8.2, Action::ExpectData("audio.master == 0")),
    (8.3, Action::UiPointAlong("audio.set:master", 0.75)),
    (8.4, Action::UiRelease(PointerButton::Primary)),
    (8.5, Action::ExpectData("audio.master == 75")),
    (9.0, Action::Capture("save_sound_sliders.png")),
    (9.1, Action::Ui("audio.set:master:100")),
    (9.3, Action::Exit),
];

/// In orbit (`OSP_SCENARIO=leo OSP_CAPTURE_WARP=1`): the fastest warp, 100 years a second, where no
/// engine starts, by the throttle or a sequence; then at 10x a sequence lights it and warp drops.
const WARP_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(true)),
    (0.6, Action::Unlock),
    (1.0, Action::Warp(14)),
    (1.5, Action::ExpectData("time.warp_label == 100 y/s")),
    (2.0, Action::Capture("warp_fastest.png")),
    (2.1, Action::Ui("vessel.throttle_full")),
    (2.2, Action::ExpectData("vessel.throttle == 0")),
    (2.25, Action::ExpectData("time.warp_label == 100 y/s")),
    (2.3, Action::Ui("vessel.stage")),
    (2.6, Action::Capture("warp_no_engines.png")),
    (2.65, Action::ExpectSequenceCount(3)),
    (2.7, Action::ExpectData("time.warp_label == 100 y/s")),
    (2.8, Action::Warp(0)),
    (2.9, Action::Warp(10)),
    (3.0, Action::ExpectData("time.warp_label == 11.6 d/s")),
    (3.4, Action::Capture("warp_days.png")),
    (3.5, Action::Warp(0)),
    (3.6, Action::Warp(4)),
    (3.7, Action::ExpectData("time.warp == 10")),
    (3.8, Action::Ui("vessel.throttle_full")),
    (3.9, Action::ExpectData("vessel.throttle == 1")),
    (4.0, Action::Ui("vessel.stage")),
    (4.3, Action::ExpectSequenceCount(2)),
    (4.35, Action::ExpectData("time.warp == 1")),
    (4.7, Action::Capture("warp_engines_at_10x.png")),
    (4.8, Action::Ui("vessel.throttle_cut")),
    (5.0, Action::Exit),
];

/// In orbit (`OSP_SCENARIO=leo OSP_CAPTURE_ALARMS=1`): an alarm for a manoeuvre, a timer, and one
/// at a point from the map's menu, in the Alarm clock and under the clocks. The fastest warp stops on
/// the timer, then on the point and on the burn's alarm, short of the burn, whichever comes first.
/// Moving the burn moves its alarm. (`OSP_CAPTURE_LAUNCH_TIMES` warps from the clock.)
const ALARMS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(true)),
    (1.0, Action::Ui("plan.add:ap")),
    (1.1, Action::Ui("plan.select:0")),
    (1.2, Action::Ui("plan.alarm")),
    (1.3, Action::Ui("plan.alarm")),
    (1.4, Action::ExpectData("alarms.count == 1")),
    (1.5, Action::Ui("alarm.add")),
    (1.6, Action::ExpectData("alarms.count == 2")),
    (1.7, Action::Ui("ui.show:alarm_clock")),
    (2.5, Action::Capture("alarms_window.png")),
    (2.6, Action::Ui("plan.orbit:1")),
    (3.4, Action::Capture("alarms_burn_moved.png")),
    (3.5, Action::PointAt(1_500.0)),
    (3.8, Action::Press(MouseButton::Right)),
    (3.9, Action::Release(MouseButton::Right)),
    (4.2, Action::ExpectOpen("map_menu", true)),
    (4.25, Action::Capture("alarms_map_menu.png")),
    (4.3, Action::Ui("map.alarm_here")),
    (4.4, Action::ExpectData("alarms.count == 3")),
    (4.5, Action::ClearPointer),
    (4.6, Action::Warp(9)),
    (7.0, Action::ExpectData("alarms.count == 2")),
    (7.05, Action::ExpectData("time.warp == 1")),
    (7.1, Action::Capture("alarms_rung.png")),
    // The point and the burn, in whichever order they come.
    (7.2, Action::Warp(9)),
    (9.7, Action::ExpectData("alarms.count == 1")),
    (9.75, Action::ExpectData("time.warp == 1")),
    (9.8, Action::Capture("alarms_second.png")),
    (9.9, Action::Warp(9)),
    (12.5, Action::ExpectData("!alarms.has")),
    (12.55, Action::ExpectData("time.warp == 1")),
    (12.6, Action::ExpectBurns(1)),
    (12.7, Action::Capture("alarms_last.png")),
    (13.0, Action::Exit),
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
const STAGED: &str =
    "enginesx4,decouplerx1,enginesx4 | enginesx5 | decouplerx1,enginesx1,decouplerx1 | ";
const SWAPPED: &str =
    "enginesx5 | enginesx4,decouplerx1,enginesx4 | decouplerx1,enginesx1,decouplerx1 | ";
const SPLIT: &str = "enginesx4,decouplerx1,enginesx4 | enginesx1 | enginesx4 | decouplerx1,enginesx1,decouplerx1 | ";
const ADDED: &str =
    " | enginesx4,decouplerx1,enginesx4 | enginesx5 | decouplerx1,enginesx1,decouplerx1 | ";
const REORDERED: &str =
    "enginesx4,enginesx4,decouplerx1 | enginesx5 | decouplerx1,enginesx1,decouplerx1 | ";
// The S-IC's retro motors went with it.
const BY_HAND: &str = "enginesx4 | enginesx5 | decouplerx1,enginesx1,decouplerx1 | ";

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
/// at `OSP_CAPTURE_STAGE_AT=<s>,...`, the flight UI hidden with `OSP_CAPTURE_BARE=1`).
/// Combine with `OSP_CAMERA=<yaw°>,<pitch°>,<distance m>` to frame them.
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
    markers: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            &'static InheritedVisibility,
            &'static MarkerBox,
        ),
    >,
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
            Option<&'static Slider>,
            Option<&'static TextInput>,
        ),
    >,
    keys: MessageWriter<'w, KeyboardInput>,
}

impl UiPointer<'_, '_> {
    /// The middle and size of the shown node that says `what`, physical px.
    fn find(&self, what: &str) -> Option<(Vec2, Vec2)> {
        self.nodes
            .iter()
            .find(
                |(node, _, visible, click, drag, drop, context, slider, input)| {
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
                            || says(context.map(|t| &t.0))
                            || says(slider.map(|s| &s.action))
                            || says(input.map(TextInput::action)))
                },
            )
            .map(|(node, transform, ..)| (transform.translation, node.size()))
    }

    /// The middle of the box shown round what is called `name`, physical px.
    fn find_marker(&self, name: &str) -> Option<Vec2> {
        (self.markers.iter())
            .find(|(node, _, visible, named)| {
                visible.get() && node.size().min_element() > 0.0 && named.0 == name
            })
            .map(|(_, transform, ..)| transform.translation)
    }

    /// Keys typed, as a keyboard sends them: `\n` is Enter and `\x1b` Escape.
    fn type_keys(&mut self, typed: &str) {
        for c in typed.chars() {
            let typed = c.to_string();
            let (key_code, logical_key, text) = match c {
                '\n' => (KeyCode::Enter, Key::Enter, None),
                '\x1b' => (KeyCode::Escape, Key::Escape, None),
                _ => (
                    KeyCode::Unidentified(NativeKeyCode::Unidentified),
                    Key::Character(typed.as_str().into()),
                    Some(typed.as_str()),
                ),
            };
            for state in [ButtonState::Pressed, ButtonState::Released] {
                let pressed = state == ButtonState::Pressed;
                self.keys.write(KeyboardInput {
                    key_code,
                    logical_key: logical_key.clone(),
                    state,
                    text: text.filter(|_| pressed).map(Into::into),
                    repeat: false,
                    window: Entity::PLACEHOLDER,
                });
            }
        }
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
    (menu, dialog, saves, last_launch): (Res<Menu>, Res<Dialog>, Res<Saves>, Res<LastLaunch>),
    mut editor_steps: MessageWriter<EditorStep>,
    shown: Res<LayoutVisibility>,
) {
    let Some(&(at, action)) = script.actions.get(script.step) else {
        return;
    };
    if time.elapsed_secs_f64() < at {
        return;
    }
    script.step += 1;
    // The editor's pointer actions move the UI's pointer too, once the editor has taken them: one
    // mouse.
    script.move_ui_pointer = matches!(
        action,
        Action::EditorPointAtGrip(..)
            | Action::EditorPointAtHandle(..)
            | Action::PointerBy(..)
            | Action::PointerAt(..)
            | Action::EditorPointAtNode(..)
            | Action::EditorPointAtSurface(..)
            | Action::EditorPointAtPart(..)
    );
    match action {
        // What only the editor can look at, which it does as soon as this has run.
        Action::EditorLook(..)
        | Action::ExpectList(..)
        | Action::ExpectEditor(..)
        | Action::EditorPointAtGrip(..)
        | Action::ExpectSize(..)
        | Action::EditorPointAtNode(..)
        | Action::EditorPointAtSurface(..)
        | Action::ExpectPartCount(..)
        | Action::EditorPointAtPart(..)
        | Action::ExpectField(..)
        | Action::ExpectPicked(..)
        | Action::EditorPointAtHandle(..)
        | Action::ExpectHeldBy(..)
        | Action::ExpectAlong(..)
        | Action::ExpectRootAbove(..)
        | Action::ExpectRootPoints(..) => {
            editor_steps.write(EditorStep(action));
        }
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
        Action::PartWindow(name) => {
            let vessel = sim.active_vessel();
            let part = (vessel.propulsion.parts().parts().iter()).find(|p| p.name == name);
            let Some(part) = part else {
                error!("smoke test: the flown vessel has no part called `{name}`");
                exit.write(AppExit::error());
                return;
            };
            let show = format!("ui.show:part_info:{}-{}", vessel.id().0, part.id.0);
            ui.write(UiAction(show));
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
        Action::ExpectDialog(wanted) => {
            if dialog.title() != wanted {
                error!(
                    "smoke test: the dialog asks `{}`, wanted `{wanted}`",
                    dialog.title()
                );
                exit.write(AppExit::error());
            }
        }
        Action::ExpectSaves(wanted) => {
            if saves.list.len() != wanted {
                error!(
                    "smoke test: there are {} saves, wanted {wanted}",
                    saves.list.len()
                );
                exit.write(AppExit::error());
            }
        }
        Action::ExpectQuicksaves(wanted) => {
            let found = saves
                .current_file()
                .map_or(0, |file| saves.quicksave_count(file));
            if found != wanted {
                error!("smoke test: the game's save holds {found} quicksaves, wanted {wanted}");
                exit.write(AppExit::error());
            }
        }
        // As the keyboard sends it, so a press is seen as one by what reads keys this frame.
        Action::Key(key_code, pressed) => {
            ui_pointer.keys.write(KeyboardInput {
                key_code,
                logical_key: Key::Unidentified(NativeKey::Unidentified),
                state: if pressed {
                    ButtonState::Pressed
                } else {
                    ButtonState::Released
                },
                text: None,
                repeat: false,
                window: Entity::PLACEHOLDER,
            });
        }
        Action::ExpectOpen(layout, wanted) => {
            if shown.shown(layout) != wanted {
                error!("smoke test: `{layout}` should be open: {wanted}");
                exit.write(AppExit::error());
            }
        }
        Action::ExpectRemembered(window, wanted) => {
            let text = (crate::windows::windows_file())
                .and_then(|file| std::fs::read_to_string(file).ok());
            let saved: Option<std::collections::BTreeMap<String, bool>> =
                text.and_then(|text| serde_json::from_str(&text).ok());
            let found = saved.and_then(|saved| saved.get(window).copied());
            if found != Some(wanted) {
                error!("smoke test: windows.json says {found:?} of `{window}`, wanted {wanted}");
                exit.write(AppExit::error());
            }
        }
        Action::ExpectData(condition) => {
            if !ui_pointer.data.condition(condition) {
                // What the key on the left of the condition is, to say what was there instead.
                let key =
                    (condition.trim_start_matches('!').split_whitespace().next()).unwrap_or("");
                let found = ui_pointer.data.get(key).map(|v| format!("{v:?}"));
                error!(
                    "smoke test: `{condition}` does not hold ({key} is {})",
                    found.as_deref().unwrap_or("not set")
                );
                exit.write(AppExit::error());
            }
        }
        Action::ExpectRevert(wanted) => {
            if last_launch.name() != wanted {
                error!(
                    "smoke test: can revert to {:?}, wanted {wanted:?}",
                    last_launch.name()
                );
                exit.write(AppExit::error());
            }
        }
        Action::ExpectSinceLaunch(from, to) => {
            let since = last_launch
                .time()
                .map(|t| sim.universe.time().seconds_since(t));
            if !since.is_some_and(|since| (from..=to).contains(&since)) {
                error!("smoke test: {since:?} s since the launch, wanted {from} to {to}");
                exit.write(AppExit::error());
            }
        }
        Action::PointerAt(x, y) => pointer.0 = Some(Vec2::new(x, y)),
        Action::PointerBy(x, y) => pointer.0 = pointer.0.map(|at| at + Vec2::new(x, y)),
        Action::ExpectShown(layout, wanted) => {
            if shown.shown(layout) != wanted {
                error!("smoke test: `{layout}` should be open: {wanted}");
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
            // One mouse: the UI's pointer leaves whatever it was on.
            if let Some(to) = pointer.0 {
                let delta = to - script.ui_pointer;
                script.ui_pointer = to;
                let target = script.target.clone();
                ui_pointer.send(&target, to, PointerAction::Move { delta });
            }
        }
        Action::Type(typed) => ui_pointer.type_keys(typed),
        Action::Press(button) => buttons.press(button),
        Action::Release(button) => buttons.release(button),
        Action::Unlock => locked.0 = false,
        Action::UiPointAt(what) | Action::UiPointAtEdge(what) | Action::UiPointAlong(what, _) => {
            let Some((middle, size)) = ui_pointer.find(what) else {
                panic!("smoke test: nothing on screen is `{what}`");
            };
            let to = match action {
                Action::UiPointAtEdge(_) => middle - Vec2::new(0.5 * size.x - 6.0, 0.0),
                Action::UiPointAlong(_, along) => middle + Vec2::new((along - 0.5) * size.x, 0.0),
                _ => middle,
            };
            let delta = to - script.ui_pointer;
            script.ui_pointer = to;
            let target = script.target.clone();
            ui_pointer.send(&target, to, PointerAction::Move { delta });
        }
        Action::UiPointAtMarker(name) => {
            let Some(to) = ui_pointer.find_marker(name) else {
                panic!("smoke test: no box round `{name}`");
            };
            let delta = to - script.ui_pointer;
            script.ui_pointer = to;
            let target = script.target.clone();
            ui_pointer.send(&target, to, PointerAction::Move { delta });
            if rig.map_view {
                pointer.0 = Some(to);
            }
        }
        Action::ClearPointer => pointer.0 = None,
        Action::LookAtTarget => {
            let vessel = sim.active_vessel();
            let system = &sim.universe.system;
            let target = vessel.target.and_then(Target::vessel);
            let Some(target) = target.and_then(|id| sim.universe.vessel(id)) else {
                panic!("smoke test: no vessel targeted to look at");
            };
            let at = vessel.state_at(system, frame.time).position;
            let there = target.state_at(system, frame.time).position;
            let here = frame.relative(vessel.parent(), at);
            let back = (here - frame.relative(target.parent(), there)).normalize();
            let up = at.normalize();
            let east = system.body(vessel.parent()).pole().cross(up).normalize();
            let north = up.cross(east);
            rig.look_from(back.dot(east).atan2(back.dot(north)), back.dot(up).asin());
        }
        Action::MapOverFlown => {
            let vessel = sim.active_vessel();
            let system = &sim.universe.system;
            let over = vessel.state_at(system, frame.time).position.normalize();
            let pole = system.body(vessel.parent()).pole();
            let east = pole.cross(DVec3::Z).try_normalize().unwrap_or(DVec3::X);
            let north = DVec3::Z.cross(east);
            rig.look_from(
                over.dot(east).atan2(over.dot(north)),
                over.dot(DVec3::Z).asin(),
            );
        }
        Action::LookFrom(off, bearing, height) => {
            rig.look_from(bearing.to_radians(), height.to_radians());
            rig.frame((off - 8.0) / 1.2);
        }
        Action::LookFromSun(off) => {
            let vessel = rig.viewed_vessel(&sim);
            let system = &sim.universe.system;
            let at = vessel.state_at(system, frame.time).position;
            let here = frame.relative(vessel.parent(), at);
            let sun = system.find("Sun").expect("a Sun");
            let back = (frame.body_offset(sun) - here).normalize();
            let up = at.normalize();
            let east = system.body(vessel.parent()).pole().cross(up).normalize();
            let north = up.cross(east);
            let (yaw, pitch) = (back.dot(east).atan2(back.dot(north)), back.dot(up).asin());
            rig.look_from(yaw, pitch);
            rig.frame((off - 8.0) / 1.2);
        }
        Action::ExpectBurns(wanted) => {
            let burns = sim.active_vessel().plan.burns().len();
            if burns != wanted {
                error!("smoke test: {burns} burns are planned, wanted {wanted}");
                exit.write(AppExit::error());
            }
        }
        Action::ExpectTarget(wanted) => {
            let universe = &sim.universe;
            let name = match sim.active_vessel().target {
                None => "",
                Some(Target::Body(id)) => universe.system.body(id).name.as_str(),
                Some(target) => (target.vessel().and_then(|id| universe.vessel(id)))
                    .map_or("", |vessel| vessel.name.as_str()),
            };
            if name != wanted {
                error!("smoke test: the target is `{name}`, wanted `{wanted}`");
                exit.write(AppExit::error());
            }
        }
        Action::ExpectMarker(name, wanted) => {
            if ui_pointer.find_marker(name).is_some() != wanted {
                error!("smoke test: `{name}` should have a box: {wanted}");
                exit.write(AppExit::error());
            }
        }
        Action::ExpectMapView(wanted) => {
            if rig.map_view != wanted {
                error!("smoke test: the map should be showing: {wanted}");
                exit.write(AppExit::error());
            }
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

/// The UI's pointer goes where a step of the script, or of the editor's, put the view's.
fn move_ui_pointer(
    mut script: ResMut<Script>,
    pointer: Res<ScriptedPointer>,
    mut ui_pointer: UiPointer,
) {
    if !std::mem::take(&mut script.move_ui_pointer) {
        return;
    }
    if let Some(to) = pointer.0 {
        let delta = to - script.ui_pointer;
        script.ui_pointer = to;
        let target = script.target.clone();
        ui_pointer.send(&target, to, PointerAction::Move { delta });
    }
}
