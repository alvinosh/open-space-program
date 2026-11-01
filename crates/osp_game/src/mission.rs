//! The mission's face in the game: its goals in a window, a notice when one is met, and the
//! results screen when the mission ends. The mission itself is judged in the sim
//! (`osp_sim::mission`, after every tick, so also under time warp and in a headless script);
//! this only shows it.
//!
//! A scenario with goals opens the `mission_goals` window when it starts, and the
//! `mission_results` window opens when the mission is passed or failed. The player can close
//! both and go on flying.

use bevy::prelude::*;
use osp_sim::mission::{Goal, Mission, MissionEvent, State};
use osp_ui::{LayoutVisibility, UiData, UiValue, record};

use crate::{
    FrameSet,
    sim::{Sim, SimRestarted},
};

/// The window listing the goals, and the one shown when the mission ends.
const GOALS: &str = "mission_goals";
const RESULTS: &str = "mission_results";
/// How long the notice for a goal stays up, real seconds.
const NOTICE_DURATION: f64 = 6.0;

pub struct MissionPlugin;

impl Plugin for MissionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, publish_mission.in_set(FrameSet::Render));
    }
}

fn publish_mission(
    mut sim: ResMut<Sim>,
    time: Res<Time>,
    mut restarted: MessageReader<SimRestarted>,
    mut ui: ResMut<LayoutVisibility>,
    mut data: ResMut<UiData>,
    mut notice: Local<Option<(News, f64)>>,
    mut started: Local<bool>,
) {
    let now = time.elapsed_secs_f64();
    let restart = restarted.read().count() > 0;
    if restart || !*started {
        *started = true;
        *notice = None;
        ui.layouts
            .insert(GOALS.into(), sim.universe.mission.is_some());
        ui.layouts.insert(RESULTS.into(), false);
    }
    let Some(mission) = sim.universe.mission.as_mut() else {
        data.set("mission.active", false);
        return;
    };
    for event in mission.take_events() {
        let news = news(&event, mission.goals());
        if news.ends {
            ui.layouts.insert(RESULTS.into(), true);
        }
        *notice = Some((news, now));
    }
    let shown = notice
        .as_ref()
        .filter(|(_, at)| now - at < NOTICE_DURATION)
        .map(|(news, _)| news.clone());
    publish(&mut data, mission, shown.unwrap_or_default());
}

/// What the player is told when something happens in the mission.
#[derive(Debug, Clone, Default, PartialEq)]
struct News {
    text: String,
    /// The mission is over: the results open.
    ends: bool,
    /// Failed, so not in the colour of good news.
    bad: bool,
}

fn news(event: &MissionEvent, goals: &[Goal]) -> News {
    match event {
        MissionEvent::GoalMet(i) => News {
            text: format!("Goal complete: {}", goals[*i].def.text),
            ..News::default()
        },
        MissionEvent::Passed => News {
            text: "Mission passed".into(),
            ends: true,
            bad: false,
        },
        MissionEvent::Failed(reason) => News {
            text: format!("Mission failed: {reason}"),
            ends: true,
            bad: true,
        },
    }
}

/// `mission.*`, see `docs/modding.md`.
fn publish(data: &mut UiData, mission: &Mission, news: News) {
    let stats = mission.stats();
    let current = mission.current();
    let goals: Vec<_> = (mission.goals().iter().enumerate())
        .map(|(i, goal)| {
            record([
                ("index", i.into()),
                ("text", goal.def.text.as_str().into()),
                ("met", goal.met_at.is_some().into()),
                // The one being worked on, in a mission that goes in order.
                (
                    "current",
                    (mission.is_ordered() && current == Some(i)).into(),
                ),
                ("held", held(goal)),
            ])
        })
        .collect();
    let met = mission
        .goals()
        .iter()
        .filter(|g| g.met_at.is_some())
        .count();
    data.set("mission.active", true);
    data.set("mission.goals", goals);
    data.set("mission.count", mission.goals().len());
    data.set("mission.met", met);
    let (state, reason) = match mission.state() {
        State::Running => ("running", ""),
        State::Passed => ("passed", ""),
        State::Failed(reason) => ("failed", reason.as_str()),
    };
    data.set("mission.state", state);
    data.set("mission.passed", state == "passed");
    data.set("mission.failed", state == "failed");
    data.set("mission.reason", reason);
    data.set("mission.notice", if news.bad { "" } else { &news.text });
    data.set("mission.notice_bad", if news.bad { &news.text } else { "" });
    data.set("mission.time", stats.elapsed);
    data.set("mission.delta_v", stats.delta_v);
    data.set("mission.peak_g", stats.peak_g);
    data.set("mission.max_altitude", stats.max_altitude);
}

/// How far a goal that has to be held has got (0..1), empty for one that is not held for a time.
fn held(goal: &Goal) -> UiValue {
    if goal.def.hold > 0.0 && goal.met_at.is_none() && goal.held > 0.0 {
        (goal.held / goal.def.hold).min(1.0).into()
    } else {
        UiValue::Null
    }
}

#[cfg(test)]
mod tests {
    use osp_sim::mission::{Condition, GoalDef};

    use super::*;

    fn goal(text: &str) -> Goal {
        Goal {
            def: GoalDef {
                id: "g".into(),
                text: text.into(),
                when: Condition::Destroyed,
                hold: 0.0,
            },
            met_at: None,
            held: 0.0,
        }
    }

    #[test]
    fn a_goal_met_is_good_news_and_does_not_end_the_mission() {
        let goals = [goal("Leave the pad"), goal("Reach orbit")];
        let news = news(&MissionEvent::GoalMet(1), &goals);
        assert_eq!(news.text, "Goal complete: Reach orbit");
        assert!(!news.ends && !news.bad);
    }

    #[test]
    fn passing_and_failing_end_the_mission_and_only_failing_is_bad_news() {
        let passed = news(&MissionEvent::Passed, &[]);
        assert_eq!(passed.text, "Mission passed");
        assert!(passed.ends && !passed.bad);
        let failed = news(&MissionEvent::Failed("The rocket was lost".into()), &[]);
        assert_eq!(failed.text, "Mission failed: The rocket was lost");
        assert!(failed.ends && failed.bad);
    }

    #[test]
    fn a_goal_held_for_a_time_shows_how_far_it_has_got() {
        let mut goal = goal("Stay up");
        assert_eq!(held(&goal), UiValue::Null);
        goal.def.hold = 10.0;
        assert_eq!(held(&goal), UiValue::Null);
        goal.held = 2.5;
        assert_eq!(held(&goal), UiValue::Number(0.25));
        goal.met_at = Some(30.0);
        assert_eq!(held(&goal), UiValue::Null);
    }
}
