//! Scripted smoke test: `PHYMI_CAPTURE=<dir> cargo run` launches the rocket, saves a few
//! screenshots into `<dir>` and exits. Handy for checking rendering without a human.
//!
//! The camera renders into an offscreen image so captures work even when the window is
//! hidden or occluded (macOS stops presenting occluded windows).

use std::path::PathBuf;

use bevy::{
    camera::RenderTarget,
    prelude::*,
    render::{
        render_resource::TextureFormat,
        view::screenshot::{Screenshot, save_to_disk},
    },
};

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera},
    controls::InputLocked,
    sim::{Sim, TimeWarp},
};

const CAPTURE_SIZE: (u32, u32) = (1600, 900);

pub struct SmokeTestPlugin;

impl Plugin for SmokeTestPlugin {
    fn build(&self, app: &mut App) {
        if let Some(dir) = std::env::var_os("PHYMI_CAPTURE") {
            let in_orbit = std::env::var("PHYMI_SCENARIO").is_ok_and(|s| s != "pad");
            let times: Vec<f64> = std::env::var("PHYMI_CAPTURE_AT")
                .unwrap_or_default()
                .split(',')
                .filter_map(|t| t.trim().parse().ok())
                .collect();
            let actions = if let Some(last) = times.last() {
                // Launch, then stills at the given times (debugging the ascent).
                let mut actions = vec![(0.5, Action::MapView(false)), (3.5, Action::Launch)];
                for (k, &at) in times.iter().enumerate() {
                    let name: &'static str = Box::leak(format!("view{k}.png").into_boxed_str());
                    actions.push((at, Action::Capture(name)));
                }
                actions.push((last + 3.0, Action::Exit));
                actions.sort_by(|a, b| a.0.total_cmp(&b.0));
                actions
            } else if std::env::var_os("PHYMI_CAPTURE_QUICK").is_some() {
                QUICK_SCRIPT.to_vec()
            } else if in_orbit {
                ORBIT_SCRIPT.to_vec()
            } else {
                PAD_SCRIPT.to_vec()
            };
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
    camera: Single<Entity, With<MainCamera>>,
    mut locked: ResMut<InputLocked>,
) {
    locked.0 = true;
    let (width, height) = CAPTURE_SIZE;
    let image = Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
    script.target = images.add(image);
    // UI follows the camera into the image, so captures include the HUD.
    commands
        .entity(*camera)
        .insert((RenderTarget::from(script.target.clone()), IsDefaultUiCamera));
}

#[derive(Clone, Copy)]
enum Action {
    Capture(&'static str),
    Launch,
    MapView(bool),
    /// Index into `WARP_RATES`.
    Warp(usize),
    Exit,
}

/// (real seconds since start, action)
const ORBIT_SCRIPT: &[(f64, Action)] = &[
    (0.5, Action::MapView(false)),
    (3.0, Action::Capture("orbit.png")),
    (3.5, Action::MapView(true)),
    (5.0, Action::Capture("orbit_map.png")),
    (5.5, Action::Warp(9)),
    (15.0, Action::Capture("orbit_warped.png")),
    (16.0, Action::Exit),
];

/// One still once the terrain has loaded (`PHYMI_CAPTURE_QUICK=1`), or with
/// `PHYMI_CAPTURE_AT=<s>,<s>,...` a launch and stills `view<n>.png` at those times. Combine
/// with `PHYMI_CAMERA=<yaw°>,<pitch°>,<distance m>` to frame them.
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
            let vessel = sim.active_vessel_mut();
            vessel.set_throttle(1.0);
            vessel.activate_next_stage(ut);
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
