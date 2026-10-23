//! Scripted smoke test: `OSP_CAPTURE=<dir> cargo run` launches the rocket, saves a few
//! screenshots into `<dir>` and exits. Handy for checking rendering without a human.
//!
//! The camera renders into an offscreen image so captures work even when the window is
//! hidden or occluded (macOS stops presenting occluded windows).

use std::path::PathBuf;

use bevy::{
    math::DVec3,
    prelude::*,
    render::{
        render_resource::TextureFormat,
        view::screenshot::{Screenshot, save_to_disk},
    },
};

use osp_sim::attitude::SasMode;
use osp_ui::UiAction;

use crate::{
    FrameSet,
    camera::{CameraRig, ViewOutput},
    controls::InputLocked,
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
                        .map(|at| (at, Action::Stage)),
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
            } else if std::env::var_os("OSP_CAPTURE_RENDEZVOUS").is_some() {
                RENDEZVOUS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_DEORBIT").is_some() {
                DEORBIT_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_TRANSFER").is_some() {
                TRANSFER_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_MANIFEST").is_some() {
                MANIFEST_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_DEBRIS").is_some() {
                DEBRIS_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_STAGING").is_some() {
                STAGING_SCRIPT.to_vec()
            } else if std::env::var_os("OSP_CAPTURE_GLIDER").is_some() {
                GLIDER_SCRIPT.to_vec()
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
            // `OSP_CAPTURE_HIDE_UI=1`: the vehicle and world only (checking part models).
            if std::env::var_os("OSP_CAPTURE_HIDE_UI").is_some() {
                actions.insert(0, (0.6, Action::Ui("ui.hide_group:flight")));
            }
            app.insert_resource(Script {
                dir: PathBuf::from(dir),
                step: 0,
                target: Handle::default(),
                actions,
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
    Stage,
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
    /// Index into `WARP_RATES`.
    Warp(usize),
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

/// `OSP_SCENARIO=glider OSP_CAPTURE_GLIDER=1`: the glider in flight, then with the stick
/// back and to the right (elevator and ailerons deflected), then banked into a turn.
const GLIDER_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("ui.show:attitude_info")),
    (3.0, Action::Capture("glider.png")),
    (3.2, Action::Steer(DVec3::new(0.6, 0.0, 1.0))),
    (3.7, Action::Capture("glider_controls.png")),
    (3.9, Action::Steer(DVec3::ZERO)),
    (9.0, Action::Capture("glider_turn.png")),
    (9.5, Action::Exit),
];

/// `OSP_CAPTURE_MENU=1`: open the main menu, start the rocket plane on the runway from it,
/// take off and climb out.
const MENU_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (2.5, Action::Capture("menu.png")),
    (2.6, Action::Ui("game.start:runway")),
    (5.0, Action::Capture("runway.png")),
    (5.2, Action::Launch),
    (15.0, Action::Capture("rolling.png")),
    (22.5, Action::Steer(DVec3::new(0.3, 0.0, 0.0))),
    (26.0, Action::Steer(DVec3::ZERO)),
    (30.0, Action::Capture("climb.png")),
    (31.5, Action::Exit),
];

/// `OSP_CAPTURE_GRAPHICS=1`: the main menu's Graphics page with the performance overlay,
/// then the Low preset and back to High.
const GRAPHICS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (1.0, Action::Ui("game.menu")),
    (1.1, Action::Ui("menu.page:graphics")),
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

/// `OSP_SCENARIO=rendezvous OSP_CAPTURE_RENDEZVOUS=1`: the target windows, the map with
/// the station, closest approaches and plane nodes, then a planned plane match.
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

/// In LEO (`OSP_CAPTURE_STAGING=1`): the sequence stack, rearranged (RCS moved in with the
/// fairing, an empty sequence added), then the engine armed and the fairing jettisoned.
const STAGING_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (2.0, Action::Capture("staging_start.png")),
    (2.2, Action::Sequences("staging.drop:item:0:1:1")),
    (2.3, Action::Sequences("staging.add")),
    (3.0, Action::Capture("staging_edited.png")),
    (3.2, Action::Stage),
    (3.4, Action::Stage),
    (5.0, Action::Capture("staging_fairing.png")),
    (5.5, Action::Exit),
];

/// In LEO (`OSP_CAPTURE_DEBRIS=1`): warp into daylight, jettison the fairing and watch the
/// halves swing open, release the payload from the upper stage, then the Universe Manifest
/// and the map with the debris in them.
const DEBRIS_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (0.6, Action::Warp(7)),
    (3.0, Action::Warp(0)),
    (4.0, Action::Stage),
    (4.2, Action::Stage),
    (5.2, Action::Capture("debris_fairing.png")),
    (8.0, Action::Capture("debris_fairing_open.png")),
    (8.2, Action::Stage),
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
const MANIFEST_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (0.6, Action::Warp(7)),
    (3.0, Action::Warp(0)),
    (4.0, Action::Stage),
    (4.2, Action::Stage),
    (4.4, Action::Stage),
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

fn run_script(
    mut commands: Commands,
    time: Res<Time>,
    mut script: ResMut<Script>,
    mut sim: ResMut<Sim>,
    mut rig: ResMut<CameraRig>,
    mut warp: ResMut<TimeWarp>,
    mut ui: MessageWriter<UiAction>,
    mut exit: MessageWriter<AppExit>,
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
        Action::Stage => sim.stage_active(),
        Action::Sequences(action) => {
            let sequences = &mut sim.active_vessel_mut().propulsion.sequences;
            if !crate::controls::edit_sequences(sequences, &UiAction(action.into())) {
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
        Action::MapView(on) => rig.map_view = on,
        Action::Warp(index) => warp.set_index(index, &sim, time.elapsed_secs_f64()),
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
