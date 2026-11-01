//! Tutorials: a guided run through a scenario, one step at a time. A step says what to do,
//! optionally which control to point at, and waits for the player to do it.
//!
//! What a step waits for is a [`Condition`] of the mission machinery (throttle open, an
//! engine running, SAS on, a height reached, a goal of the scenario met), judged by the
//! universe after every tick like a mission is, so a tutorial runs headless. A step that is
//! about the interface (the map is open, the warp is up) names a UI condition (`ui`) that the
//! game checks, and a step with neither is read and moved past with a button; both finish the
//! step with [`Tutorial::complete`]. The words, the highlighted node and the hint are for the
//! game to show.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    data::{LaunchSiteDef, ScenarioDef},
    mission::{Condition, Mission, Stats, View, resolve},
    system::SolarSystem,
    time::SimTime,
    universe::Universe,
};

/// Idle on a step this long, s, before its hint shows, unless the step says otherwise.
const HINT_AFTER: f64 = 20.0;

/// A tutorial record (collection `tutorials`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TutorialDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// A line for the main menu.
    #[serde(default)]
    pub description: String,
    /// The scenario it is played over.
    pub scenario: String,
    pub steps: Vec<StepDef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepDef {
    pub id: String,
    /// What to do. `[action]` is replaced by the key the player has that action on.
    pub text: String,
    /// More help, shown once the player has been on the step for `hint_after` seconds.
    #[serde(default)]
    pub hint: String,
    #[serde(default = "hint_after")]
    pub hint_after: f64,
    /// The id of a UI node (or layout) to outline while the step is current.
    #[serde(default)]
    pub highlight: Option<String>,
    /// The step is done when this holds.
    #[serde(default)]
    pub when: Option<Condition>,
    /// The step is done when this holds in the UI data (`view.map`, `time.warp != 1`); the
    /// game judges it.
    #[serde(default)]
    pub ui: Option<String>,
}

fn hint_after() -> f64 {
    HINT_AFTER
}

impl StepDef {
    /// Neither a condition nor a UI state: the player moves on when they have read it.
    pub fn is_manual(&self) -> bool {
        self.when.is_none() && self.ui.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TutorialState {
    Running,
    /// Every step was done.
    Finished,
    /// The player gave it up.
    Ended,
}

/// What just happened, for the game to react to once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TutorialEvent {
    /// The step at this index was done.
    StepDone(usize),
    Finished,
}

#[derive(Debug, Clone)]
pub struct Tutorial {
    def: TutorialDef,
    current: usize,
    state: TutorialState,
    places: HashMap<String, crate::mission::Place>,
    /// When the current step began, and the time of the last look.
    step_started: SimTime,
    now: SimTime,
    events: Vec<TutorialEvent>,
}

impl Tutorial {
    /// A tutorial over `scenario`, starting at `time`. Steps that name something that is not
    /// there (a body, a site, a vessel, a goal of the scenario) are an error, and so is a step
    /// that waits for both a condition and a UI state.
    pub fn new(
        def: &TutorialDef,
        scenario: &ScenarioDef,
        sites: &[LaunchSiteDef],
        system: &SolarSystem,
        time: SimTime,
    ) -> Result<Self, String> {
        if def.steps.is_empty() {
            return Err(format!("tutorial `{}` has no steps", def.id));
        }
        let mut places = HashMap::new();
        for (i, step) in def.steps.iter().enumerate() {
            let at = format!("tutorial `{}`, step `{}`", def.id, step.id);
            if def.steps[..i].iter().any(|s| s.id == step.id) {
                return Err(format!("{at}: the id is used twice"));
            }
            if step.when.is_some() && step.ui.is_some() {
                return Err(format!("{at}: wait for a `when` or a `ui`, not both"));
            }
            if let Some(when) = &step.when {
                resolve(
                    &at,
                    when,
                    scenario,
                    sites,
                    system,
                    Some(&scenario.goals),
                    &mut places,
                )?;
            }
        }
        Ok(Self {
            def: def.clone(),
            current: 0,
            state: TutorialState::Running,
            places,
            step_started: time,
            now: time,
            events: Vec::new(),
        })
    }

    pub fn def(&self) -> &TutorialDef {
        &self.def
    }

    pub fn state(&self) -> TutorialState {
        self.state
    }

    pub fn is_running(&self) -> bool {
        self.state == TutorialState::Running
    }

    /// Index of the step the player is on (the number of steps when it is finished).
    pub fn index(&self) -> usize {
        self.current
    }

    /// The step the player is on, while the tutorial runs.
    pub fn step(&self) -> Option<&StepDef> {
        self.is_running()
            .then(|| self.def.steps.get(self.current))
            .flatten()
    }

    /// Whether the player has been on the step long enough for its hint.
    pub fn hint_visible(&self) -> bool {
        self.step().is_some_and(|step| {
            !step.hint.is_empty() && self.now.seconds_since(self.step_started) >= step.hint_after
        })
    }

    /// What just happened since this was last asked.
    pub fn take_events(&mut self) -> Vec<TutorialEvent> {
        std::mem::take(&mut self.events)
    }

    /// Look at the universe as it is now: the step is done if what it waits for holds.
    pub fn observe(&mut self, universe: &Universe, mission: Option<&Mission>) {
        self.now = universe.time();
        let Some(when) = self.step().and_then(|s| s.when.as_ref()) else {
            return;
        };
        let Some(vessel) = universe.focus.and_then(|id| universe.vessel(id)) else {
            return;
        };
        let stats = mission.map_or_else(Stats::default, |m| *m.stats());
        let view = View {
            universe,
            vessel,
            stats: &stats,
            places: &self.places,
            mission,
        };
        if view.holds(when) {
            self.complete();
        }
    }

    /// The current step is done (the player did it, read it, or skipped it).
    pub fn complete(&mut self) {
        if !self.is_running() {
            return;
        }
        self.events.push(TutorialEvent::StepDone(self.current));
        self.current += 1;
        self.step_started = self.now;
        if self.current >= self.def.steps.len() {
            self.state = TutorialState::Finished;
            self.events.push(TutorialEvent::Finished);
        }
    }

    /// The player gives the tutorial up.
    pub fn end(&mut self) {
        if self.is_running() {
            self.state = TutorialState::Ended;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        mission::tests::{in_orbit, mission, on_the_pad, scenario},
        test_data::records,
        vessel::tests::launch_sites,
    };

    /// A tutorial in the syntax of the records, over a scenario with these goals.
    fn tutorial(universe: &Universe, goals: &str, steps: &str) -> Tutorial {
        let def: TutorialDef = records(&format!(
            r#"{{ id: "t", name: "T", scenario: "pad", steps: [{steps}] }}"#
        ))
        .remove(0);
        Tutorial::new(
            &def,
            &scenario(goals),
            &launch_sites(),
            &universe.system,
            universe.time(),
        )
        .unwrap()
    }

    fn seconds(universe: &mut Universe, seconds: f64) {
        for _ in 0..(seconds * 50.0).round() as usize {
            universe.step_physics();
        }
    }

    const THREE_STEPS: &str = r#"
        { id: "throttle", text: "Press [vessel.throttle_full] to open the throttle.",
          hint: "Z opens it", hint_after: 5, highlight: "navball",
          when: { type: "Throttle", min: 0.9 } },
        { id: "stage", text: "Press [vessel.stage] to light the engines.",
          when: { type: "Engines", min: 1 } },
        // The pad stands 35 m above sea level.
        { id: "climb", text: "Climb to 80 m.", when: { type: "Altitude", min: 80 } },
    "#;

    #[test]
    fn a_tutorial_of_three_steps_advances_as_the_player_acts() {
        let mut universe = on_the_pad();
        universe.tutorial = Some(tutorial(&universe, "", THREE_STEPS));
        let at = |u: &Universe| u.tutorial.as_ref().unwrap().index();

        // Nothing happens until the player does something.
        seconds(&mut universe, 2.0);
        assert_eq!(at(&universe), 0);
        let step = universe.tutorial.as_ref().unwrap().step().unwrap().clone();
        assert_eq!(step.id, "throttle");
        assert_eq!(step.highlight.as_deref(), Some("navball"));

        // The throttle opens: the next step.
        universe.vessels[0].set_throttle(1.0);
        seconds(&mut universe, 0.1);
        assert_eq!(at(&universe), 1);
        // The throttle is open, but the engines are not lit.
        seconds(&mut universe, 1.0);
        assert_eq!(at(&universe), 1);

        // The stage key: the engines light, and the rocket climbs away.
        universe.activate_next_stage(0);
        seconds(&mut universe, 0.2);
        assert_eq!(at(&universe), 2);
        seconds(&mut universe, 10.0);
        let tutorial = universe.tutorial.as_mut().unwrap();
        assert_eq!(tutorial.state(), TutorialState::Finished);
        assert!(tutorial.step().is_none());
        assert_eq!(
            tutorial.take_events(),
            [
                TutorialEvent::StepDone(0),
                TutorialEvent::StepDone(1),
                TutorialEvent::StepDone(2),
                TutorialEvent::Finished
            ]
        );
    }

    #[test]
    fn a_hint_shows_after_the_player_has_been_idle_on_a_step() {
        let mut universe = on_the_pad();
        universe.tutorial = Some(tutorial(&universe, "", THREE_STEPS));
        let hint = |u: &Universe| u.tutorial.as_ref().unwrap().hint_visible();
        seconds(&mut universe, 4.0);
        assert!(!hint(&universe));
        seconds(&mut universe, 1.5);
        assert!(hint(&universe));
        // On to a step: the clock starts over, and that step has no hint at all.
        universe.vessels[0].set_throttle(1.0);
        seconds(&mut universe, 0.1);
        assert!(!hint(&universe));
        seconds(&mut universe, 30.0);
        assert!(!hint(&universe));
    }

    #[test]
    fn a_step_with_nothing_to_wait_for_or_a_ui_state_is_done_by_the_game() {
        let mut universe = in_orbit();
        let steps = r#"
            { id: "read", text: "This is the navball." },
            { id: "map", text: "Open the map.", ui: "view.map" },
            { id: "last", text: "Done.", when: { type: "Altitude", min: 1 } },
        "#;
        universe.tutorial = Some(tutorial(&universe, "", steps));
        assert!(
            universe
                .tutorial
                .as_ref()
                .unwrap()
                .step()
                .unwrap()
                .is_manual()
        );
        // Time passes and the world is as it is, but the player has not read it.
        seconds(&mut universe, 5.0);
        let tutorial = universe.tutorial.as_mut().unwrap();
        assert_eq!(tutorial.index(), 0);
        tutorial.complete();
        // The UI step does not look at the sim.
        assert_eq!(tutorial.step().unwrap().ui.as_deref(), Some("view.map"));
        seconds(&mut universe, 5.0);
        let tutorial = universe.tutorial.as_mut().unwrap();
        assert_eq!(tutorial.index(), 1);
        tutorial.complete();
        // The last one is already true, and is done on the next tick.
        seconds(&mut universe, 0.1);
        assert_eq!(
            universe.tutorial.as_ref().unwrap().state(),
            TutorialState::Finished
        );
    }

    #[test]
    fn giving_up_ends_the_tutorial_and_nothing_moves_it_on() {
        let mut universe = on_the_pad();
        universe.tutorial = Some(tutorial(&universe, "", THREE_STEPS));
        universe.tutorial.as_mut().unwrap().end();
        universe.vessels[0].set_throttle(1.0);
        seconds(&mut universe, 1.0);
        let tutorial = universe.tutorial.as_mut().unwrap();
        assert_eq!(tutorial.state(), TutorialState::Ended);
        assert_eq!(tutorial.index(), 0);
        assert!(tutorial.step().is_none() && !tutorial.hint_visible());
        tutorial.complete();
        assert!(tutorial.take_events().is_empty());
    }

    #[test]
    fn a_step_can_wait_for_a_goal_of_the_mission() {
        let mut universe = on_the_pad();
        let goals = r#"goals: [{ id: "up", text: "Climb", when: { type: "Altitude", min: 80 } }]"#;
        universe.mission = Some(mission(goals, &universe));
        let steps = r#"{ id: "up", text: "Climb.", when: { type: "Goal", id: "up" } }"#;
        universe.tutorial = Some(tutorial(&universe, goals, steps));
        universe.vessels[0].set_throttle(1.0);
        universe.activate_next_stage(0);
        seconds(&mut universe, 3.0);
        assert!(universe.tutorial.as_ref().unwrap().is_running());
        seconds(&mut universe, 10.0);
        assert_eq!(
            universe.tutorial.as_ref().unwrap().state(),
            TutorialState::Finished
        );
    }

    #[test]
    fn a_tutorial_that_names_something_that_is_not_there_does_not_start() {
        let universe = in_orbit();
        let sites = launch_sites();
        let goals =
            r#"goals: [{ id: "up", text: "Climb", when: { type: "Elapsed", seconds: 5 } }]"#;
        let error = |goals: &str, steps: &str| {
            let def: TutorialDef = records(&format!(
                r#"{{ id: "t", scenario: "pad", steps: [{steps}] }}"#
            ))
            .remove(0);
            Tutorial::new(
                &def,
                &scenario(goals),
                &sites,
                &universe.system,
                universe.time(),
            )
            .unwrap_err()
        };
        let step = |when: &str| format!(r#"{{ id: "s", text: "S", when: {when} }}"#);
        assert!(error("", "").contains("no steps"));
        assert!(error(goals, &step("{ type: 'Goal', id: 'down' }")).contains("down"));
        assert!(error("", &step("{ type: 'Goal', id: 'up' }")).contains("up"));
        assert!(error("", &step("{ type: 'Orbit', body: 'Vulcan' }")).contains("Vulcan"));
        assert!(error("", &step("{ type: 'Sas', mode: 'sideways' }")).contains("sideways"));
        assert!(
            error(
                "",
                r#"{ id: "s", text: "S", when: { type: "Destroyed" }, ui: "view.map" }"#
            )
            .contains("not both")
        );
        let twice = format!("{0}, {0}", step("{ type: 'Destroyed' }"));
        assert!(error("", &twice).contains("twice"));
    }

    #[test]
    fn a_step_record_has_defaults() {
        let def: TutorialDef = records(
            r#"{ id: "t", scenario: "pad", steps: [{ id: "a", text: "A", ui: "view.map" }] }"#,
        )
        .remove(0);
        let step = &def.steps[0];
        assert_eq!(step.hint_after, 20.0);
        assert!(step.hint.is_empty() && step.highlight.is_none() && step.when.is_none());
        assert!(!step.is_manual());
        assert_eq!(def.name, "");
    }
}
