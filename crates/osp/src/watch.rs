//! `--watch <mission>`: one of the smoke tests' missions (`osp_smoke`, `cargo smoke`) flown in the
//! game, to watch. The mission's own script flies it, on its own thread, and the game shows the
//! universe between its steps (`osp_smoke::watch`): the very flight the test flies, with its report
//! written at the end.
//!
//! The game does not step the universe meanwhile (`Sim::scripted`) and the player's keys do not
//! reach the vessels, but the camera, the map and the windows work. The warp keys set the speed:
//! by default 4x under physics and 10 000x coasting on rails, each press doubling or halving both.
//! Under physics with nothing to see (no engine burning, hardly any air, no vessel near, well above
//! the ground) it goes [`QUIET`] times faster; the stop-warp key goes back to the default speed. Esc
//! pauses it. When the flight is over the player has the controls back.
//!
//! `OSP_WATCH_SHOTS=<dir>` saves a screenshot of the window as each phase begins.

use std::{path::PathBuf, sync::Mutex};

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};
use osp_game::{
    FrameSet,
    sim::{Sim, SimEventMessage, SimRestarted, TimeWarp},
};
use osp_sim::{time::SimTime, vessel::VesselId};
use osp_smoke::{
    MISSIONS, Mission,
    watch::{Shown, Speed, Watch},
};
use osp_ui::UiAction;

/// The mission `--watch <mission>` asks for, if it does. Exits with the list of missions if there
/// is no such mission.
pub fn requested() -> Option<&'static Mission> {
    let mut args = std::env::args().skip(1);
    args.by_ref().find(|a| a == "--watch")?;
    let id = args.next().unwrap_or_default();
    osp_smoke::mission(&id).or_else(|| {
        let ids: Vec<_> = MISSIONS.iter().map(|m| m.id).collect();
        eprintln!("--watch needs a mission: {} (no `{id}`)", ids.join(", "));
        std::process::exit(2);
    })
}

/// Watches `mission`, if there is one.
pub struct WatchPlugin(pub Option<&'static Mission>);

impl Plugin for WatchPlugin {
    fn build(&self, app: &mut App) {
        let Some(mission) = self.0 else {
            return;
        };
        info!("watching {}: {}", mission.id, mission.title);
        app.world_mut().resource_mut::<Sim>().scripted = true;
        app.insert_resource(Watching {
            mission,
            watch: Some(Mutex::new(Watch::start(mission))),
            level: 0,
            phase: None,
            kept: None,
            shown_at: None,
            shots: std::env::var_os("OSP_WATCH_SHOTS").map(PathBuf::from),
            phases_seen: 0,
        })
        .add_systems(Update, watch.in_set(FrameSet::Simulate));
    }
}

/// How much faster the flight goes under physics while nothing happens: drifting towards another
/// vessel, falling through thin air, hanging under parachutes.
const QUIET: f64 = 4.0;
/// Something happens when the air pushes harder than this, Pa...
const QUIET_AIR: f64 = 500.0;
/// ...or another vessel is nearer than this, m...
const QUIET_NEAR: f64 = 200.0;
/// ...or the ground is, m above the body's mean radius.
const QUIET_HIGH: f64 = 1_000.0;

/// Presses of the warp keys from the default speed, either way.
const SLOWEST: i32 = -2;
const FASTEST: i32 = 5;

#[derive(Resource)]
struct Watching {
    mission: &'static Mission,
    /// None once the flight is over.
    watch: Option<Mutex<Watch>>,
    /// Doublings of the default speed (halvings below zero).
    level: i32,
    /// The phase the flight was last in.
    phase: Option<String>,
    /// What the game may change in the universe while it shows it, as the script left it: put back
    /// before it flies on.
    kept: Option<(Option<VesselId>, bool)>,
    /// The universe's clock when it was last shown. If the game shows another time, it has started
    /// another universe (a save loaded, a revert): the flight is not in it, and the watch ends.
    shown_at: Option<SimTime>,
    /// Where a screenshot goes as each phase begins (`OSP_WATCH_SHOTS`).
    shots: Option<PathBuf>,
    phases_seen: usize,
}

impl Watching {
    fn speed(&self) -> Speed {
        let base = Speed::default();
        let factor = 2f64.powi(self.level);
        Speed {
            physics: base.physics * factor,
            rails: base.rails * factor,
        }
    }

    /// The flight is over or cannot go on: the player has the game back.
    fn end(&mut self, sim: &mut Sim, warp: &mut TimeWarp) {
        self.watch = None;
        sim.scripted = false;
        sim.restarted();
        *warp = TimeWarp::default();
    }
}

fn watch(
    mut commands: Commands,
    time: Res<Time>,
    mut watching: ResMut<Watching>,
    mut sim: ResMut<Sim>,
    mut warp: ResMut<TimeWarp>,
    mut actions: MessageReader<UiAction>,
    mut events: MessageWriter<SimEventMessage>,
    mut restarted: MessageWriter<SimRestarted>,
) {
    let now = time.elapsed_secs_f64();
    for action in actions.read() {
        if watching.watch.is_none() {
            continue;
        }
        let level = watching.level;
        watching.level = match action.name() {
            "warp.increase" => (level + 1).min(FASTEST),
            "warp.decrease" => (level - 1).max(SLOWEST),
            "warp.stop" => 0,
            _ => continue,
        };
        let speed = watching.speed();
        let notice = format!(
            "Watching at {}x under physics, {}x on rails",
            speed.physics, speed.rails
        );
        warp.notice = Some((notice, now));
    }
    if watching.watch.is_none() || sim.paused {
        return;
    }
    if watching
        .shown_at
        .is_some_and(|at| at != sim.universe.time())
    {
        warn!("another universe was started: stopped watching");
        warp.notice = Some(("Stopped watching".into(), now));
        watching.end(&mut sim, &mut warp);
        return;
    }
    if let Some((focus, links)) = watching.kept {
        sim.universe.focus = focus;
        sim.universe.warp_stops_at_links = links;
    }

    let mut speed = watching.speed();
    if quiet(&sim) {
        speed.physics *= QUIET;
    }
    let real = time.delta_secs_f64().min(0.1);
    let watch = watching.watch.as_mut().unwrap().get_mut().unwrap();
    match watch.show(&mut sim.universe, real, speed) {
        Shown::Loading => {}
        Shown::Started => {
            let title = watching.mission.title;
            info!("{title}: the flight has begun");
            warp.notice = Some((format!("Watching {title}"), now));
            sim.restarted();
            restarted.write(SimRestarted);
        }
        Shown::Flew {
            events: happened,
            phase,
            rails,
        } => {
            events.write_batch(happened.into_iter().map(SimEventMessage));
            if phase.is_some() && phase != watching.phase {
                let title = phase.clone().unwrap_or_default();
                info!("{}: {title}", watching.mission.id);
                watching.phases_seen += 1;
                if let Some(dir) = &watching.shots {
                    let _ = std::fs::create_dir_all(dir);
                    let name = format!("{:02}_{}.png", watching.phases_seen, file_name(&title));
                    commands
                        .spawn(Screenshot::primary_window())
                        .observe(save_to_disk(dir.join(name)));
                }
                warp.notice = Some((title, now));
                watching.phase = phase;
            }
            warp.show_rate(if rails { speed.rails } else { speed.physics });
        }
        Shown::Finished(summary) => {
            info!("{summary}");
            watching.end(&mut sim, &mut warp);
            warp.notice = Some(("The flight is over: yours to fly".into(), now));
        }
        Shown::Stopped(why) => {
            warn!("stopped watching: {why}");
            watching.end(&mut sim, &mut warp);
            warp.notice = Some((format!("Stopped watching: {why}"), now));
        }
    }
    sim.render_time = sim.universe.time();
    watching.shown_at = Some(sim.universe.time());
    watching.kept = Some((sim.universe.focus, sim.universe.warp_stops_at_links));
}

/// Whether the vessel being watched is under physics with nothing to see: see [`QUIET`].
fn quiet(sim: &Sim) -> bool {
    let universe = &sim.universe;
    let (system, now) = (&universe.system, universe.time());
    let vessel = sim.active_vessel();
    let here = vessel.state_at(system, now).position;
    let high = here.length() - system.body(vessel.parent()).radius > QUIET_HIGH;
    let alone = (universe.vessels.iter())
        .filter(|v| v.id() != vessel.id() && v.parent() == vessel.parent())
        .all(|v| v.state_at(system, now).position.distance(here) > QUIET_NEAR);
    let flight = &vessel.flight;
    flight.thrust <= 0.0 && flight.dynamic_pressure < QUIET_AIR && high && alone
}

/// `title` as part of a file's name.
fn file_name(title: &str) -> String {
    (title.chars())
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}
