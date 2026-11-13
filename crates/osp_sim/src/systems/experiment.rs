//! A scientific instrument's experiment ([`crate::data::ExperimentDef`]). Switched on, it runs while
//! its part is somewhere it works (a situation over a body, in sunlight if it needs it) and it has
//! the power it asks for, and pauses when not. In each place ([`crate::science::subject`]) it
//! collects one whole set over its duration, then rests until it is taken somewhere new. It keeps
//! going while the player flies something else, at any warp: the vessel tells it where it is before
//! each step ([`System::situate`]) and steps no further than to where that changes, or to where it
//! finishes a set ([`System::next_event`]).
//!
//! What it makes goes onto the drives its part is wired to as it makes it ([`super::Drive`]), and it
//! pauses while they have no room. What it has collected of each subject it remembers, so it does
//! not do the same place twice.

use std::{any::Any, collections::BTreeMap};

use serde::{Deserialize, Serialize};

use super::{Held, Power, Reading, System, Writing, Written};
use crate::{
    component::Component,
    data::ExperimentDef,
    environment::Environment,
    parts::Part,
    science::{Place, subject},
    staging::Action,
};

pub const START_EXPERIMENT: Action = Action::new("experiment_start", "Start experiment");
pub const STOP_EXPERIMENT: Action = Action::new("experiment_stop", "Stop experiment");

/// Below the vessel's control and life support: an experiment is the first load shed.
pub const EXPERIMENT_PRIORITY: u8 = 20;

/// A share of a whole set taken as all of it: what is left is not worth a step.
const WHOLE: f64 = 1.0 - 1e-9;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Experiment {
    pub def: ExperimentDef,
    /// Switched on.
    pub on: bool,
    /// Where its part is, as the vessel last said.
    #[serde(default)]
    place: Option<Place>,
    /// Its network gave it the power it asked for.
    #[serde(default)]
    powered: bool,
    /// The share of a whole set collected in each subject so far, by subject.
    #[serde(default)]
    pub collected: BTreeMap<String, f64>,
    /// It collected over the last step.
    #[serde(default)]
    running: bool,
    /// The drives it is wired to have room for what it makes.
    #[serde(default = "yes")]
    room: bool,
    /// What it has made since the drives last took it: its subject, amount (B, or kg of a sample)
    /// and what a whole set of it is worth.
    #[serde(default)]
    made: Option<(String, f64, f64)>,
    #[serde(default)]
    said: Option<String>,
}

fn yes() -> bool {
    true
}

pub(super) fn of(component: &Component, _part: &Part) -> Option<Box<dyn System>> {
    let Component::Experiment { def, on } = component else {
        return None;
    };
    Some(Box::new(Experiment::new(def.clone(), *on)))
}

impl Experiment {
    pub fn new(def: ExperimentDef, on: bool) -> Self {
        Self {
            def,
            on,
            place: None,
            powered: false,
            collected: BTreeMap::new(),
            running: false,
            room: true,
            made: None,
            said: None,
        }
    }

    /// What a whole set comes to: bytes, or kg of a sample.
    pub fn whole(&self) -> f64 {
        self.def.sample.unwrap_or(self.def.size)
    }

    /// The science a whole set where it is now is worth.
    pub fn worth_here(&self) -> f64 {
        self.def.value * self.place.as_ref().map_or(1.0, |p| p.value)
    }

    /// Whether it works at `place`.
    pub fn works_at(&self, place: &Place) -> bool {
        let def = &self.def;
        def.situations.contains(&place.situation)
            && (def.bodies.is_empty() || def.bodies.contains(&place.body))
            && (!def.sunlight || place.sunlit)
    }

    /// What it would collect where it is now, if it works there.
    pub fn subject_here(&self) -> Option<String> {
        let place = self.place.as_ref().filter(|p| self.works_at(p))?;
        Some(subject(&self.def.id, place))
    }

    /// The share of a whole set it has of `subject`.
    pub fn share(&self, subject: &str) -> f64 {
        self.collected.get(subject).copied().unwrap_or(0.0)
    }

    /// It has more to collect where it is: switched on, somewhere it works, not done there.
    fn has_work(&self) -> bool {
        self.on
            && self
                .subject_here()
                .is_some_and(|here| self.share(&here) < WHOLE)
    }

    /// It would collect if it had power: it has work and room for what it makes.
    fn wants_to_run(&self) -> bool {
        self.has_work() && self.room
    }

    fn needs_power(&self) -> bool {
        self.def.power > 0.0
    }

    fn runs(&self) -> bool {
        self.wants_to_run() && (self.powered || !self.needs_power())
    }

    /// The whole sets it has, and the share of one it has in all, over every subject.
    pub fn sets(&self) -> (usize, f64) {
        let whole = self.collected.values().filter(|s| **s >= WHOLE).count();
        (whole, self.collected.values().sum())
    }

    /// What it is doing, as the player reads it.
    pub fn status(&self) -> String {
        let name = self.def.title();
        let Some(place) = &self.place else {
            return format!("{name}: waiting");
        };
        let here = place.describe();
        if !self.on {
            return format!("{name}: off");
        }
        if !self.works_at(place) {
            return format!("{name}: does not work {here}");
        }
        let share = self.subject_here().map_or(0.0, |s| self.share(&s));
        if share >= WHOLE {
            format!("{name}: done {here}")
        } else if !self.room {
            format!("{name}: no room on the drives")
        } else if self.wants_to_run() && !self.runs() {
            format!("{name}: no power")
        } else {
            format!("{name}: {:.0}% {here}", 100.0 * share)
        }
    }
}

impl System for Experiment {
    fn kind(&self) -> &'static str {
        "experiment"
    }

    fn parameters(&self) -> Vec<Reading> {
        let mut out = vec![Reading {
            id: "duration",
            label: "A whole set takes",
            value: self.def.duration,
            unit: "s",
        }];
        match self.def.sample {
            Some(kg) => out.push(Reading {
                id: "sample",
                label: "Sample",
                value: kg,
                unit: "kg",
            }),
            None => out.push(Reading {
                id: "size",
                label: "Data",
                value: self.def.size,
                unit: "B",
            }),
        }
        out.push(Reading {
            id: "power",
            label: "Power",
            value: self.def.power,
            unit: "W",
        });
        out
    }

    fn state(&self) -> Vec<Reading> {
        let here = self.subject_here().map_or(0.0, |s| self.share(&s));
        let (whole, all) = self.sets();
        let (id, label, amount, unit) = match self.def.sample {
            Some(kg) => ("collected", "Collected", all * kg, "kg"),
            None => ("collected", "Collected", all * self.def.size, "B"),
        };
        vec![
            Reading {
                id: "here",
                label: "Done here",
                value: 100.0 * here,
                unit: "%",
            },
            Reading {
                id: "worth",
                label: "A set here is worth",
                value: self.worth_here(),
                unit: "science",
            },
            Reading {
                id: "sets",
                label: "Whole sets",
                value: whole as f64,
                unit: "",
            },
            Reading {
                id,
                label,
                value: amount,
                unit,
            },
        ]
    }

    fn step(&mut self, dt: f64) {
        self.running = self.runs() && dt > 0.0;
        if !self.running {
            return;
        }
        let Some(here) = self.subject_here() else {
            return;
        };
        let whole = self.whole();
        let share = self.collected.entry(here.clone()).or_insert(0.0);
        let before = *share;
        *share = (*share + dt / self.def.duration).min(1.0);
        if *share >= WHOLE {
            *share = 1.0;
            let place = self.place.as_ref().map(Place::describe).unwrap_or_default();
            self.said = Some(format!("{}: a whole set {place}", self.def.title()));
        }
        let amount = (*share - before) * whole;
        let worth = self.worth_here();
        match &mut self.made {
            Some((subject, made, _)) if *subject == here => *made += amount,
            _ => self.made = Some((here, amount, worth)),
        }
    }

    fn power(&self, _environment: &Environment) -> Option<Power> {
        self.needs_power().then(|| Power::Load {
            watts: if self.wants_to_run() {
                self.def.power
            } else {
                0.0
            },
            priority: EXPERIMENT_PRIORITY,
        })
    }

    fn powered(&mut self, on: bool) {
        self.powered = on;
    }

    fn uses_light(&self) -> bool {
        self.def.sunlight
    }

    fn situated(&self) -> bool {
        self.on
    }

    fn situate(&mut self, place: &Place) {
        if self.place.as_ref() != Some(place) {
            self.place = Some(place.clone());
        }
    }

    fn writes(&self) -> Option<Writing> {
        Some(Writing {
            sample: self.def.sample.is_some(),
            rate: if self.runs() {
                self.whole() / self.def.duration
            } else {
                0.0
            },
        })
    }

    fn room(&mut self, room: bool) {
        self.room = room;
    }

    fn written(&mut self) -> Option<Written> {
        let (subject, amount, worth) = self.made.take().filter(|(_, amount, _)| *amount > 0.0)?;
        Some(Written {
            subject,
            held: Held {
                amount,
                whole: self.whole(),
                sample: self.def.sample.is_some(),
                worth,
            },
        })
    }

    fn unwritten(&mut self, subject: &str, amount: f64) {
        let whole = self.whole();
        if let Some(share) = self.collected.get_mut(subject) {
            *share = (*share - amount / whole).max(0.0);
            if *share < WHOLE {
                self.said = None;
            }
        }
    }

    fn next_event(&self) -> Option<f64> {
        let here = self.subject_here()?;
        self.runs()
            .then(|| (1.0 - self.share(&here)) * self.def.duration)
    }

    fn announce(&mut self) -> Option<String> {
        self.said.take()
    }

    fn actions(&self) -> Vec<Action> {
        vec![if self.on {
            STOP_EXPERIMENT
        } else {
            START_EXPERIMENT
        }]
    }

    fn act(&mut self, action: Action) -> Option<String> {
        let name = self.def.title().to_owned();
        if action.kind == START_EXPERIMENT.kind && !self.on {
            self.on = true;
            Some(format!("{name} started"))
        } else if action.kind == STOP_EXPERIMENT.kind && self.on {
            self.on = false;
            self.running = false;
            Some(format!("{name} stopped"))
        } else {
            None
        }
    }

    fn boxed(&self) -> Box<dyn System> {
        Box::new(self.clone())
    }

    fn any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        data::{CraftDef, OrbitStartDef},
        science::Situation,
        system::tests::rss,
        test_data::records,
        time::SimTime,
        vessel::{Vessel, tests::catalog},
    };

    /// A probe with an experiment that only works in space low, one that works anywhere in space,
    /// both taking `duration` s and no power, and a drive with room for all they make, on an orbit
    /// over the Earth from `low` to `high` m up.
    fn probe(low: f64, high: f64, duration: f64) -> Vessel {
        let mut catalog = catalog();
        catalog.experiments = records(&format!(
            r#"[
                {{ id: "low", duration: {duration}, size: 100, situations: ["space_low"] }},
                {{ id: "space", duration: {duration}, size: 100, situations: ["space_low", "space_high"] }}
            ]"#
        ));
        let craft: CraftDef = records(
            r#"{ id: "x", name: "", parts: [{ id: "probe",
                shape: { box: { size: [0.5, 0.5, 0.5] } }, structure: { mass: 50 },
                components: [{ experiment: { experiment: "low" } },
                             { experiment: { experiment: "space" } },
                             { drive: { data: 1e9 } }] }] }"#,
        )
        .remove(0);
        let start = OrbitStartDef {
            body: "Earth".into(),
            periapsis_altitude: low,
            apoapsis_altitude: high,
            inclination: 0.0,
            longitude_of_ascending_node: 0.0,
            plane: None,
            argument_of_periapsis: 0.0,
            mean_anomaly: 0.0,
            spin: [0.0; 3],
        };
        Vessel::in_orbit(&craft, &catalog, &rss(), &start, SimTime::J2000).unwrap()
    }

    fn experiment<'a>(vessel: &'a Vessel, id: &str) -> &'a Experiment {
        (vessel.propulsion.systems.all::<Experiment>())
            .map(|(_, e)| e)
            .find(|e| e.def.id == id)
            .unwrap()
    }

    /// Fly `vessel`'s systems for `seconds`, in steps no longer than `longest`, cut where they say,
    /// as time warp cuts them; and the times the steps ended.
    fn fly(vessel: &mut Vessel, seconds: f64, longest: f64) -> Vec<f64> {
        let system = rss();
        let start = SimTime::J2000;
        let end = start.add_seconds(seconds);
        let mut t = start;
        let mut cuts = Vec::new();
        while t < end {
            let horizon = end.min(t.add_seconds(longest.min(1e9)));
            let next = vessel.next_system_event(&system, t, horizon);
            let to = next.map_or(horizon, |n| n.min(horizon));
            vessel.step_systems(&system, t, to.seconds_since(t));
            t = to;
            cuts.push(t.seconds_since(start));
        }
        cuts
    }

    /// In a low orbit it collects a whole set in its time, whatever the warp, and time warp stops
    /// where it finishes.
    #[test]
    fn a_set_takes_its_time_at_any_warp_and_warp_stops_where_it_is_done() {
        for longest in [f64::INFINITY, 7.0] {
            let mut vessel = probe(400e3, 400e3, 600.0);
            fly(&mut vessel, 300.0, longest);
            let here = "low@Earth.space_low";
            let share = experiment(&vessel, "low").share(here);
            assert!((share - 0.5).abs() < 1e-9, "{share} at {longest}");
            let cuts = fly(&mut vessel, 3_000.0, longest);
            assert_eq!(experiment(&vessel, "low").share(here), 1.0);
            if longest.is_infinite() {
                // Stopped at the 300 s it had left, then on to the end.
                assert!((cuts[0] - 300.0).abs() < 1e-6, "{cuts:?}");
            }
            let (whole, _) = experiment(&vessel, "low").sets();
            assert_eq!(whole, 1);
        }
    }

    /// On an orbit that goes from space low up into space high, one that only works low pauses up
    /// there, and one that works in both collects each place on its own; the same at the highest
    /// warp as a few seconds at a time.
    #[test]
    fn it_pauses_where_it_does_not_work_at_any_warp() {
        let mut runs = Vec::new();
        for longest in [f64::INFINITY, 5.0] {
            let mut vessel = probe(300e3, 8_000e3, 1e6);
            let period = vessel.coasting().unwrap().period().unwrap();
            fly(&mut vessel, period, longest);
            let low = experiment(&vessel, "low");
            let space = experiment(&vessel, "space");
            let below = low.share("low@Earth.space_low") * 1e6;
            let above = space.share("space@Earth.space_high") * 1e6;
            assert!(low.collected.len() == 1, "{:?}", low.collected);
            // Low and high make up the orbit.
            assert!(
                (space.share("space@Earth.space_low") * 1e6 - below).abs() < 1e-3,
                "{below} {above}"
            );
            assert!(
                (below + above - period).abs() < 1e-3,
                "{below} + {above} != {period}"
            );
            assert!(
                below > 0.1 * period && above > 0.5 * period,
                "{below} {above}"
            );
            runs.push(below);
        }
        assert!((runs[0] - runs[1]).abs() < 5.0, "{runs:?}");
    }

    /// Switched off or out of where it works it says so, and wants no power.
    #[test]
    fn it_says_what_it_is_doing() {
        let def: ExperimentDef = records(
            r#"{ id: "t", name: "Temperature", duration: 60, size: 10, power: 1,
                 situations: ["landed", "flying_low"], sunlight: true }"#,
        )
        .remove(0);
        let mut e = Experiment::new(def, true);
        let place = |situation, sunlit| Place {
            body: "Earth".into(),
            situation,
            sunlit,
            value: 1.0,
        };
        assert_eq!(e.status(), "Temperature: waiting");
        e.situate(&place(Situation::SpaceLow, true));
        assert_eq!(e.status(), "Temperature: does not work in space near Earth");
        let none = |e: &Experiment| e.power(&Environment::DARK);
        assert_eq!(
            none(&e),
            Some(Power::Load {
                watts: 0.0,
                priority: EXPERIMENT_PRIORITY
            })
        );
        e.situate(&place(Situation::Landed, false));
        assert!(
            !e.works_at(&place(Situation::Landed, false)),
            "it needs light"
        );
        e.situate(&place(Situation::Landed, true));
        assert_eq!(e.status(), "Temperature: no power");
        e.powered(true);
        e.step(30.0);
        assert_eq!(e.status(), "Temperature: 50% landed on Earth");
        assert_eq!(e.next_event(), Some(30.0));
        e.step(30.0);
        assert_eq!(e.status(), "Temperature: done landed on Earth");
        assert_eq!(
            e.announce().as_deref(),
            Some("Temperature: a whole set landed on Earth")
        );
        assert_eq!(e.next_event(), None);
        assert!(e.act(STOP_EXPERIMENT).is_some());
        assert_eq!(e.status(), "Temperature: off");
    }
}
