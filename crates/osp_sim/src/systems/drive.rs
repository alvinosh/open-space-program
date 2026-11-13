//! A drive ([`crate::component::DriveDef`]): where a vessel keeps what its experiments make, by
//! subject ([`crate::science::subject`]), until it reaches the ground. Data is bytes and can be sent
//! (T274); a sample is kg of material and has to be brought home. Experiments write to the drives on
//! the parts they are wired to ([`super::Systems`] pours what they make into them, the biggest
//! first), and pause when there is no room left. What a drive holds goes with its part when the
//! vessel comes apart.

use std::{any::Any, collections::BTreeMap};

use serde::{Deserialize, Serialize};

use super::{Reading, System};
use crate::{component::Component, component::DriveDef, parts::Part, staging::Action};

/// Bring the data and samples on the other drives this one is wired to here, as much as fits.
pub const TAKE_DATA: Action = Action::new("drive_take", "Move data here");
pub const DELETE_DATA: Action = Action::new("drive_delete", "Delete data");
pub const DUMP_SAMPLES: Action = Action::new("drive_dump", "Dump samples");

/// What a drive holds of one subject.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Held {
    /// Bytes, or kg of a sample.
    pub amount: f64,
    /// What a whole set of it comes to, in the same unit.
    pub whole: f64,
    pub sample: bool,
    /// The science a whole set of it is worth ([`crate::science`]).
    #[serde(default)]
    pub worth: f64,
}

impl Held {
    /// The share of a whole set it is.
    pub fn sets(&self) -> f64 {
        if self.whole > 0.0 {
            self.amount / self.whole
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Drive {
    /// The room it has.
    pub room: DriveDef,
    /// What it holds, by subject.
    #[serde(default)]
    pub held: BTreeMap<String, Held>,
}

pub(super) fn of(component: &Component, _part: &Part) -> Option<Box<dyn System>> {
    let Component::Drive(room) = component else {
        return None;
    };
    Some(Box::new(Drive::new(*room)))
}

/// Nothing left, of a store this size: what rounding leaves.
pub(super) fn negligible(amount: f64, room: f64) -> bool {
    amount <= 1e-9 * room.max(1e-9)
}

impl Drive {
    pub fn new(room: DriveDef) -> Self {
        Self {
            room,
            held: BTreeMap::new(),
        }
    }

    /// What it holds of data (B) or samples (kg).
    pub fn holds(&self, sample: bool) -> f64 {
        // Folded from 0, not summed: a sum of nothing is -0, which a window shows as such.
        (self.held.values())
            .filter(|h| h.sample == sample)
            .fold(0.0, |all, h| all + h.amount)
    }

    /// The room it has for data (B) or samples (kg).
    pub fn capacity(&self, sample: bool) -> f64 {
        if sample {
            self.room.samples
        } else {
            self.room.data
        }
    }

    /// The room it has left for data or samples.
    pub fn free(&self, sample: bool) -> f64 {
        (self.capacity(sample) - self.holds(sample)).max(0.0)
    }

    /// Keep `held` of `subject`, as much of it as there is room for; what did not fit.
    pub fn put(&mut self, subject: &str, held: Held) -> f64 {
        let fits = held.amount.min(self.free(held.sample));
        if fits > 0.0 {
            let kept = self.held.entry(subject.to_owned()).or_insert(Held {
                amount: 0.0,
                ..held
            });
            kept.amount += fits;
        }
        held.amount - fits
    }

    /// Take out up to `amount` of `subject`: what it gave.
    pub fn take(&mut self, subject: &str, amount: f64) -> f64 {
        let Some(held) = self.held.get_mut(subject) else {
            return 0.0;
        };
        let taken = amount.min(held.amount);
        held.amount -= taken;
        if negligible(held.amount, held.whole) {
            self.held.remove(subject);
        }
        taken
    }

    /// What it holds that can be sent: its data, never a sample.
    pub fn sendable(&self) -> impl Iterator<Item = (&str, &Held)> {
        (self.held.iter())
            .filter(|(_, h)| !h.sample)
            .map(|(s, h)| (s.as_str(), h))
    }

    /// Throw away its data, or its samples.
    pub fn clear(&mut self, sample: bool) {
        self.held.retain(|_, h| h.sample != sample);
    }
}

/// kB, as the window shows a drive's data.
const KB: f64 = 1e3;

impl System for Drive {
    fn kind(&self) -> &'static str {
        "drive"
    }

    fn parameters(&self) -> Vec<Reading> {
        let mut out = Vec::new();
        if self.room.data > 0.0 {
            out.push(Reading {
                id: "data_room",
                label: "Room for data",
                value: self.room.data / KB,
                unit: "kB",
            });
        }
        if self.room.samples > 0.0 {
            out.push(Reading {
                id: "sample_room",
                label: "Room for samples",
                value: self.room.samples,
                unit: "kg",
            });
        }
        out
    }

    fn state(&self) -> Vec<Reading> {
        let mut out = Vec::new();
        if self.room.data > 0.0 {
            out.push(Reading {
                id: "data",
                label: "Data here",
                value: self.holds(false) / KB,
                unit: "kB",
            });
        }
        if self.room.samples > 0.0 {
            out.push(Reading {
                id: "samples",
                label: "Samples here",
                value: self.holds(true),
                unit: "kg",
            });
        }
        out.push(Reading {
            id: "subjects",
            label: "Subjects",
            value: self.held.len() as f64,
            unit: "",
        });
        out
    }

    fn drive(&self) -> Option<&Drive> {
        Some(self)
    }

    fn drive_mut(&mut self) -> Option<&mut Drive> {
        Some(self)
    }

    fn actions(&self) -> Vec<Action> {
        let mut out = Vec::new();
        if self.free(false) > 0.0 || self.free(true) > 0.0 {
            out.push(TAKE_DATA);
        }
        if self.holds(false) > 0.0 {
            out.push(DELETE_DATA);
        }
        if self.holds(true) > 0.0 {
            out.push(DUMP_SAMPLES);
        }
        out
    }

    fn act(&mut self, action: Action) -> Option<String> {
        let sample = if action.kind == DELETE_DATA.kind {
            false
        } else if action.kind == DUMP_SAMPLES.kind {
            true
        } else {
            return None;
        };
        let had = self.holds(sample);
        self.clear(sample);
        Some(if sample {
            format!("Dumped {had:.2} kg of samples")
        } else {
            format!("Deleted {:.1} kB of data", had / KB)
        })
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
        career::Career,
        data::{CraftDef, OrbitStartDef},
        system::tests::rss,
        systems::Experiment,
        test_data::records,
        time::SimTime,
        universe::Universe,
        vessel::{Vessel, tests::catalog},
    };

    /// A probe in a 400 km orbit with `parts` (each a list of components, the first part the root)
    /// and the experiments `measure` (100 B in 600 s, worth 4) and `scoop` (a 0.5 kg sample in 600
    /// s), both working in space low and taking no power.
    fn probe(parts: &[&str]) -> Vessel {
        let mut catalog = catalog();
        catalog.experiments = records(
            r#"[
                { id: "measure", duration: 600, size: 100, value: 4, situations: ["space_low"] },
                { id: "scoop", duration: 600, sample: 0.5, situations: ["space_low"] }
            ]"#,
        );
        let parts: Vec<String> = (parts.iter().enumerate())
            .map(|(k, components)| {
                let on = if k == 0 {
                    String::new()
                } else {
                    format!(r#"on: "p{}.top", by: "bottom","#, k - 1)
                };
                format!(
                    r#"{{ id: "p{k}", {on} shape: {{ shell: {{ bottom_diameter: 0.5,
                        sections: [{{ length: 0.5 }}], cap_bottom: true, cap_top: true }} }},
                      structure: {{ mass: 50 }}, components: [{components}] }}"#
                )
            })
            .collect();
        let craft: CraftDef = records(&format!(
            r#"{{ id: "x", name: "Probe", parts: [{}] }}"#,
            parts.join(",")
        ))
        .remove(0);
        let start = OrbitStartDef {
            body: "Earth".into(),
            periapsis_altitude: 400e3,
            apoapsis_altitude: 400e3,
            inclination: 0.0,
            longitude_of_ascending_node: 0.0,
            plane: None,
            argument_of_periapsis: 0.0,
            mean_anomaly: 0.0,
            spin: [0.0; 3],
        };
        Vessel::in_orbit(&craft, &catalog, &rss(), &start, SimTime::J2000).unwrap()
    }

    const MEASURE: &str = r#"{ experiment: { experiment: "measure" } }"#;
    const SCOOP: &str = r#"{ experiment: { experiment: "scoop" } }"#;
    const HERE: &str = "measure@Earth.space_low";

    fn experiment<'a>(vessel: &'a Vessel, id: &str) -> &'a Experiment {
        (vessel.propulsion.systems.all::<Experiment>())
            .map(|(_, e)| e)
            .find(|e| e.def.id == id)
            .unwrap()
    }

    /// The drives, root part first.
    fn drives(vessel: &Vessel) -> Vec<&Drive> {
        vessel.propulsion.systems.drives().map(|(_, d)| d).collect()
    }

    /// Fly `vessel`'s systems `seconds` on from `*t`, in steps no longer than `longest`, cut where
    /// they say, as time warp cuts them.
    fn fly(vessel: &mut Vessel, t: &mut SimTime, seconds: f64, longest: f64) {
        let system = rss();
        let end = t.add_seconds(seconds);
        while *t < end {
            let horizon = end.min(t.add_seconds(longest.min(1e9)));
            let next = vessel.next_system_event(&system, *t, horizon);
            let to = next.map_or(horizon, |n| n.min(horizon));
            vessel.step_systems(&system, *t, to.seconds_since(*t));
            *t = to;
        }
    }

    /// With room for 30 B of its 100 it collects 30 and pauses, at any warp, and says why; deleting
    /// the data lets it go on to 60, and moving it onto a bigger drive lets it finish.
    #[test]
    fn a_full_drive_pauses_an_experiment_and_freeing_room_starts_it_again() {
        for longest in [f64::INFINITY, 7.0] {
            let small = format!(r#"{MEASURE}, {{ drive: {{ data: 30 }} }}"#);
            let mut vessel = probe(&[&small, r#"{ drive: { data: 1 } }"#]);
            let mut t = SimTime::J2000;
            fly(&mut vessel, &mut t, 500.0, longest);
            let share = experiment(&vessel, "measure").share(HERE);
            // The one byte drive is a smaller drive wired to it: that fills last.
            assert!((share - 0.31).abs() < 1e-9, "{share} at {longest}");
            assert_eq!(
                experiment(&vessel, "measure").status(),
                "measure: no room on the drives"
            );
            let held: f64 = drives(&vessel).iter().map(|d| d.holds(false)).sum();
            assert!((held - 31.0).abs() < 1e-6, "{held}");

            let (root, _) = vessel.propulsion.systems.drives().next().unwrap();
            assert_eq!(
                vessel.propulsion.systems.act(root, DELETE_DATA).as_deref(),
                Some("Deleted 0.0 kB of data")
            );
            fly(&mut vessel, &mut t, 500.0, longest);
            let share = experiment(&vessel, "measure").share(HERE);
            assert!((share - 0.61).abs() < 1e-9, "{share} at {longest}");
            // What it collected went on the drives, and is the same as it says.
            let held: f64 = drives(&vessel).iter().map(|d| d.holds(false)).sum();
            assert!((held - 31.0).abs() < 1e-6, "{held}");
        }
    }

    /// A probe core's big drive takes the data before an instrument's own small one, and moving
    /// what the small one holds onto it frees the small one.
    #[test]
    fn data_goes_to_the_biggest_drive_and_moves_between_them() {
        let instrument = format!(r#"{MEASURE}, {{ drive: {{ data: 100 }} }}"#);
        let mut vessel = probe(&[r#"{ drive: { data: 1000 } }"#, &instrument]);
        let mut t = SimTime::J2000;
        fly(&mut vessel, &mut t, 300.0, f64::INFINITY);
        let [core, own] = drives(&vessel)[..] else {
            panic!("two drives")
        };
        assert!((core.holds(false) - 50.0).abs() < 1e-6);
        assert_eq!(own.holds(false), 0.0);

        // Put something on the small one by hand, then bring it over.
        let parts: Vec<_> = vessel.propulsion.systems.drives().map(|(p, _)| p).collect();
        let (core, own) = (parts[0], parts[1]);
        let systems = &mut vessel.propulsion.systems;
        let small = (systems.on_mut(own).iter_mut()).find_map(|s| s.drive_mut());
        let left = small.unwrap().put(
            "other@Earth.landed",
            Held {
                amount: 80.0,
                whole: 100.0,
                sample: false,
                worth: 0.0,
            },
        );
        assert_eq!(left, 0.0);
        let said = systems.act(core, TAKE_DATA);
        assert_eq!(said.as_deref(), Some("Moved 0.1 kB of data here"));
        let held = systems.held();
        assert!((held["other@Earth.landed"].sets - 0.8).abs() < 1e-9);
        assert!((held[HERE].sets - 0.5).abs() < 1e-9);
        let [core, own] = drives(&vessel)[..] else {
            panic!("two drives")
        };
        assert!((core.holds(false) - 130.0).abs() < 1e-6);
        assert_eq!(own.holds(false), 0.0);
    }

    /// A sample is kept by mass and is never offered to be sent; brought home, the career counts it
    /// with the data, and the data earns what a set of it is worth in space near the Earth.
    #[test]
    fn a_sample_cannot_be_sent_and_counts_when_it_is_recovered() {
        let pod = format!(r#"{MEASURE}, {SCOOP}, {{ drive: {{ data: 1000, samples: 2 }} }}"#);
        let mut vessel = probe(&[&pod]);
        let mut t = SimTime::J2000;
        fly(&mut vessel, &mut t, 900.0, f64::INFINITY);
        let [drive] = drives(&vessel)[..] else {
            panic!("one drive")
        };
        assert!((drive.holds(true) - 0.5).abs() < 1e-9);
        assert!((drive.holds(false) - 100.0).abs() < 1e-6);
        let sendable: Vec<&str> = drive.sendable().map(|(s, _)| s).collect();
        assert_eq!(sendable, [HERE]);
        // An empty drive holds nothing, not less than nothing (a window shows "-0").
        assert!(Drive::new(drive.room).holds(true).is_sign_positive());

        let mut universe = Universe::new(rss(), vec![vessel], t);
        universe.career = Some(Career {
            tree: "t".into(),
            science: 0.0,
            funds: 0.0,
            facilities: Default::default(),
            contracts: Default::default(),
            unlocked: Default::default(),
            returned: Default::default(),
        });
        let id = universe.vessels[0].id();
        let recovered = universe.recover_vessel(id).unwrap();
        let worth = 4.0
            * rss()
                .body(rss().find("Earth").unwrap())
                .science
                .values
                .space_low;
        assert_eq!(
            recovered.notice(),
            format!(
                "Recovered Probe with 0.1 kB of data and 0.50 kg of samples: {worth:.1} science"
            )
        );
        assert!(universe.vessel(id).is_none());
        let career = universe.career.as_ref().unwrap();
        assert!((career.science - worth).abs() < 1e-9, "{}", career.science);
        let returned = &career.returned;
        assert!((returned["scoop@Earth.space_low"] - 1.0).abs() < 1e-9);
        assert!((returned[HERE] - 1.0).abs() < 1e-9);
    }

    /// What drives hold, and that an experiment is waiting for room, come back from a save.
    #[test]
    fn drives_and_samples_come_back_from_a_save() {
        let pod = format!(r#"{MEASURE}, {SCOOP}, {{ drive: {{ data: 40, samples: 2 }} }}"#);
        let mut vessel = probe(&[&pod]);
        let mut t = SimTime::J2000;
        fly(&mut vessel, &mut t, 900.0, f64::INFINITY);
        let systems = &vessel.propulsion.systems;
        let saved = rmp_serde::to_vec_named(systems).unwrap();
        let back: crate::systems::Systems = rmp_serde::from_slice(&saved).unwrap();
        assert_eq!(back.held(), systems.held());
        let [drive] = back.drives().map(|(_, d)| d).collect::<Vec<_>>()[..] else {
            panic!("one drive")
        };
        assert!((drive.holds(true) - 0.5).abs() < 1e-9);
        assert!((drive.holds(false) - 40.0).abs() < 1e-6);
        let measure = back
            .all::<Experiment>()
            .find(|(_, e)| e.def.id == "measure");
        assert_eq!(
            measure.unwrap().1.status(),
            "measure: no room on the drives"
        );
    }
}
