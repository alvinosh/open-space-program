//! The Alarm clock window: the alarms set, soonest first, to warp to or delete, and a new one some
//! time from now. Time warp at any rate stops as each one rings (see [`osp_sim::alarm`] and
//! `crate::sim`).
//!
//! Alarms are also set where the times are: a launch time or a departure in the transfer planner,
//! a manoeuvre's panel or menu (the alarm follows the burn), a point in the map's menu, and the
//! closest approach in the Target window. [`set_alarm`] is what they call.
//!
//! | Action | Effect |
//! | --- | --- |
//! | `alarm.warp:<id>` | Warp to that alarm |
//! | `alarm.delete:<id>` | Take it off the clock |
//! | `alarm.step:<+1|-1>` | A longer or shorter wait for a new alarm |
//! | `alarm.add` | Set a new alarm that long from now |

use bevy::prelude::*;
use osp_sim::{
    alarm::{AlarmId, AlarmTime},
    vessel::VesselId,
};
use osp_ui::{UiAction, UiData, record};

use crate::{
    FrameSet,
    sim::{Sim, TimeWarp},
};

/// The waits a new alarm can be set for, s, and how they read.
const WAITS: [(f64, &str); 10] = [
    (60.0, "1 minute"),
    (300.0, "5 minutes"),
    (600.0, "10 minutes"),
    (1_800.0, "30 minutes"),
    (3_600.0, "1 hour"),
    (6.0 * 3_600.0, "6 hours"),
    (86_400.0, "1 day"),
    (7.0 * 86_400.0, "1 week"),
    (30.0 * 86_400.0, "30 days"),
    (365.25 * 86_400.0, "1 year"),
];
/// The wait a new alarm starts at: ten minutes.
const FIRST_WAIT: usize = 2;

pub struct AlarmClockPlugin;

impl Plugin for AlarmClockPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(NewAlarm { wait: FIRST_WAIT })
            .add_systems(Update, alarm_actions.in_set(FrameSet::Input))
            .add_systems(Update, publish.in_set(FrameSet::Render));
    }
}

/// The wait the window sets a new alarm for, as an index into [`WAITS`].
#[derive(Resource)]
struct NewAlarm {
    wait: usize,
}

/// Set an alarm from wherever its time was found, and say whether it was set.
pub fn set_alarm(
    sim: &mut Sim,
    warp: &mut TimeWarp,
    real: f64,
    title: String,
    vessel: Option<VesselId>,
    when: AlarmTime,
) {
    let notice = match sim.universe.set_alarm(title.clone(), vessel, when) {
        Ok(_) => format!("Alarm set: {title}"),
        Err(why) => why.to_string(),
    };
    warp.notice = Some((notice, real));
}

fn alarm_actions(
    mut actions: MessageReader<UiAction>,
    mut sim: ResMut<Sim>,
    mut warp: ResMut<TimeWarp>,
    mut new: ResMut<NewAlarm>,
    time: Res<Time>,
) {
    let real = time.elapsed_secs_f64();
    for action in actions.read() {
        let id = || action.arg().and_then(|id| id.parse().ok()).map(AlarmId);
        match action.name() {
            "alarm.warp" => {
                let alarm = id().and_then(|id| sim.universe.alarms.get(id));
                let at = alarm.and_then(|a| Some((sim.universe.alarm_time(a)?, a.title.clone())));
                match at {
                    Some((time, title)) => warp.warp_to(time, &title, &sim, real),
                    None => warn!("no alarm in `{}`", action.0),
                }
            }
            "alarm.delete" => {
                if let Some(id) = id() {
                    sim.universe.alarms.remove(id);
                }
            }
            "alarm.step" => {
                let longer = action.arg() != Some("-1");
                new.wait = if longer {
                    (new.wait + 1).min(WAITS.len() - 1)
                } else {
                    new.wait.saturating_sub(1)
                };
            }
            "alarm.add" => {
                let (wait, name) = WAITS[new.wait];
                let at = sim.universe.time().add_seconds(wait);
                let title = format!("Timer ({name})");
                set_alarm(&mut sim, &mut warp, real, title, None, AlarmTime::At(at));
            }
            _ => {}
        }
    }
}

fn publish(sim: Res<Sim>, new: Res<NewAlarm>, mut data: ResMut<UiData>) {
    let now = sim.render_time;
    let universe = &sim.universe;
    let active = sim.active_id();
    let alarms = universe.alarms_by_time();
    let list: Vec<_> = (alarms.iter())
        .map(|(time, alarm)| {
            let other = (alarm.vessel)
                .filter(|&id| id != active)
                .and_then(|id| universe.vessel(id));
            record([
                ("id", alarm.id.0.into()),
                ("title", alarm.title.as_str().into()),
                ("in", time.seconds_since(now).into()),
                ("other", other.is_some().into()),
                ("vessel", other.map_or("", |v| v.name.as_str()).into()),
            ])
        })
        .collect();
    data.set("alarms.has", !list.is_empty());
    data.set("alarms.count", list.len());
    data.set("alarms.list", list);
    let next = alarms.first();
    data.set(
        "alarms.next_in",
        next.map(|(time, _)| time.seconds_since(now)),
    );
    data.set(
        "alarms.next_title",
        next.map_or(String::new(), |(_, alarm)| alarm.title.clone()),
    );
    data.set("alarms.new_in", WAITS[new.wait].1);
}
