//! Seats, and the people in them. Each person has a name, and with a `life_support` on the part
//! they use what the rules' profile says people need, from the tanks it can reach. Going short of
//! something they cannot last without, each goes through stages, short, weak and critical, and
//! dies at the profile's time times their own resilience; given it back, they recover as fast as
//! they declined. With part of what they need, the shortfall is shared. Time warp stops at each
//! stage. Any crew can be lost to acceleration.

use std::{any::Any, collections::BTreeMap};

use serde::{Deserialize, Serialize};

use super::{CREW_PRIORITY, Exchange, FlightNow, PartHeat, Reading, System};
use crate::{
    component::Component,
    data::{NeedDef, fnv},
    parts::Part,
    staging::Action,
};

pub const BOARD: Action = Action::new("board", "Move someone here");

/// Standard gravity, m/s².
const G: f64 = 9.80665;
/// Below this nobody comes to harm however long it lasts, in g.
const G_SAFE: f64 = 5.0;
/// How long a person lasts at `G_REFERENCE`, s; the time goes as the inverse fourth power of g.
const G_REFERENCE: f64 = 15.0;
const G_ENDURANCE: f64 = 30.0;
/// A day, s.
const DAY: f64 = 86_400.0;
/// A share a rounding short of whole is whole: the tanks run dry at a time warp stops at.
const WHOLE: f64 = 1.0 - 1e-9;
/// A stage reached this close is reached, s: warp stops at it, to the rounding.
const ON_TIME: f64 = 1e-6;

/// How a person is doing, from well to nearly dead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Stage {
    Fine,
    Short,
    Weak,
    Critical,
}

impl Stage {
    pub fn word(self) -> &'static str {
        match self {
            Stage::Fine => "fine",
            Stage::Short => "short",
            Stage::Weak => "weak",
            Stage::Critical => "critical",
        }
    }
}

/// Someone aboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub name: String,
    /// How much longer than the profile's times they last: 1 to 1.2, from their name.
    pub resilience: f64,
    /// How long, in effect, they have gone without each need, s, by resource.
    #[serde(default)]
    pub short: BTreeMap<String, f64>,
}

impl Person {
    pub fn new(name: String) -> Self {
        let resilience = 1.0 + 0.2 * (fnv(&name) % 1_000) as f64 / 999.0;
        Self {
            name,
            resilience,
            short: BTreeMap::new(),
        }
    }

    fn short_of(&self, need: &NeedDef) -> f64 {
        self.short.get(&need.resource).copied().unwrap_or(0.0)
    }

    /// Where they are, without `need`, and the times the stages after come at, s.
    fn stages(&self, need: &NeedDef) -> Option<[f64; 3]> {
        let lasts = need.lasts? * self.resilience;
        Some([need.weak * lasts, need.critical * lasts, lasts])
    }

    fn stage_of(&self, need: &NeedDef) -> Stage {
        let short = self.short_of(need);
        match self.stages(need) {
            Some(_) if short <= 0.0 => Stage::Fine,
            Some([weak, critical, _]) => {
                if short + ON_TIME >= critical {
                    Stage::Critical
                } else if short + ON_TIME >= weak {
                    Stage::Weak
                } else {
                    Stage::Short
                }
            }
            None => Stage::Fine,
        }
    }

    /// How they are doing by the worst of `needs`, and what they are short of most.
    pub fn stage<'a>(&self, needs: &'a [NeedDef]) -> (Stage, Option<&'a str>) {
        let worst = needs
            .iter()
            .map(|need| (self.stage_of(need), need))
            .max_by(|a, b| {
                (a.0, self.short_of(a.1))
                    .partial_cmp(&(b.0, self.short_of(b.1)))
                    .expect("no NaN")
            });
        match worst {
            Some((stage, need)) if stage != Stage::Fine => (stage, Some(need.called())),
            _ => (Stage::Fine, None),
        }
    }

    /// What, if anything, they have died for want of.
    fn dead_of<'a>(&self, needs: &'a [NeedDef]) -> Option<&'a NeedDef> {
        needs.iter().find(|need| {
            self.stages(need)
                .is_some_and(|[.., dies]| self.short_of(need) + ON_TIME >= dies)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Crew {
    pub seats: u32,
    /// How many are aboard, alive: as many as `people`.
    pub aboard: u32,
    /// How many have died.
    #[serde(default)]
    pub lost: u32,
    /// Who is aboard. A save from before people had names has none, and they are named when they
    /// are next needed.
    #[serde(default)]
    pub people: Vec<Person>,
    /// What each needs, if the part has life support. A save from before the rules said what
    /// people need has none, and they need nothing.
    #[serde(default)]
    pub needs: Vec<NeedDef>,
    /// What they make that is let go of when there is nowhere to keep it.
    #[serde(default)]
    pub dump: Vec<String>,
    /// How much of what a body can take of acceleration it has taken, 0..1.
    #[serde(default)]
    strain: f64,
    /// The most acceleration they have felt, g.
    #[serde(default)]
    pub peak_g: f64,
    #[serde(default)]
    said: Option<String>,
}

pub(super) fn of(component: &Component, part: &Part) -> Option<Box<dyn System>> {
    let Component::Crew {
        seats,
        aboard,
        names,
    } = component
    else {
        return None;
    };
    let (needs, dump) = (part.built.iter().flat_map(|built| &built.components))
        .find_map(|c| match c {
            Component::LifeSupport(life_support) => {
                Some((life_support.needs.clone(), life_support.dump.clone()))
            }
            _ => None,
        })
        .unwrap_or_default();
    let aboard = aboard.unwrap_or(*seats).min(*seats);
    let mut crew = Crew {
        seats: *seats,
        aboard,
        lost: 0,
        people: names
            .iter()
            .take(aboard as usize)
            .cloned()
            .map(Person::new)
            .collect(),
        needs,
        dump,
        strain: 0.0,
        peak_g: 0.0,
        said: None,
    };
    crew.name_everyone();
    Some(Box::new(crew))
}

impl Crew {
    /// Give a name to anyone aboard who has none: those of a save from before people had them.
    fn name_everyone(&mut self) {
        while self.people.len() < self.aboard as usize {
            let k = self.people.len() + 1;
            self.people.push(Person::new(format!("Crew member {k}")));
        }
    }

    fn say(&mut self, news: String) {
        self.said = Some(match self.said.take() {
            Some(before) => format!("{before}. {news}"),
            None => news,
        });
    }

    fn lose_everyone(&mut self, why: &str) {
        if self.aboard > 0 {
            self.say(format!("Crew lost: {why}"));
            self.lost += self.aboard;
            self.aboard = 0;
            self.people.clear();
        }
    }

    /// What they use of each need now, kg/s, if they use anything.
    fn rates(&self) -> Option<Vec<f64>> {
        (self.aboard > 0 && !self.needs.is_empty()).then(|| {
            (self.needs.iter())
                .map(|need| need.daily * f64::from(self.aboard) / DAY)
                .collect()
        })
    }

    /// How the one doing worst is.
    pub fn worst(&self) -> Stage {
        (self.people.iter())
            .map(|p| p.stage(&self.needs).0)
            .max()
            .unwrap_or(Stage::Fine)
    }
}

impl System for Crew {
    fn kind(&self) -> &'static str {
        "crew"
    }

    fn parameters(&self) -> Vec<Reading> {
        vec![Reading {
            id: "seats",
            label: "Seats",
            value: f64::from(self.seats),
            unit: "",
        }]
    }

    fn state(&self) -> Vec<Reading> {
        vec![
            Reading {
                id: "aboard",
                label: "Aboard",
                value: f64::from(self.aboard),
                unit: "",
            },
            Reading {
                id: "lost",
                label: "Lost",
                value: f64::from(self.lost),
                unit: "",
            },
        ]
    }

    fn consumes(&self) -> bool {
        self.rates().is_some()
    }

    /// Each need is a lot of its own: short of water, they still breathe. What they make of it
    /// goes with it: with nowhere to breathe out to, they cannot breathe.
    fn wants(&self, dt: f64) -> Vec<Exchange> {
        let people = f64::from(self.aboard);
        self.rates().map_or(Vec::new(), |rates| {
            (self.needs.iter().zip(rates))
                .map(|(need, rate)| Exchange {
                    inputs: vec![(need.resource.clone(), rate * dt)],
                    outputs: (need.makes.iter())
                        .map(|(made, daily)| (made.clone(), daily * people / DAY * dt))
                        .collect(),
                    dump: self.dump.clone(),
                    priority: CREW_PRIORITY,
                })
                .collect()
        })
    }

    fn exchanged(&mut self, shares: &[f64], dt: f64) {
        if shares.is_empty() {
            return;
        }
        self.name_everyone();
        let needs = self.needs.clone();
        let before: Vec<Stage> = self.people.iter().map(|p| p.stage(&needs).0).collect();
        for (need, &share) in needs.iter().zip(shares) {
            if need.lasts.is_none() {
                continue;
            }
            for person in &mut self.people {
                let short = person.short.entry(need.resource.clone()).or_insert(0.0);
                *short = if share >= WHOLE {
                    (*short - dt).max(0.0)
                } else {
                    *short + (1.0 - share.max(0.0)) * dt
                };
            }
        }
        let mut news = Vec::new();
        let mut alive = Vec::new();
        for (person, was) in std::mem::take(&mut self.people).into_iter().zip(before) {
            if let Some(need) = person.dead_of(&needs) {
                news.push(format!(
                    "{} has died for want of {}",
                    person.name,
                    need.called()
                ));
                self.lost += 1;
                continue;
            }
            let (now, of) = person.stage(&needs);
            if now != was {
                news.push(match (now, of) {
                    (Stage::Fine, _) | (_, None) => format!("{} has recovered", person.name),
                    (Stage::Short, Some(of)) => format!("{} is short of {of}", person.name),
                    (stage, Some(of)) => {
                        format!("{} is {} from lack of {of}", person.name, stage.word())
                    }
                });
            }
            alive.push(person);
        }
        self.aboard = alive.len() as u32;
        self.people = alive;
        if !news.is_empty() {
            self.say(news.join(". "));
        }
    }

    fn exchanging(&self) -> Vec<Exchange> {
        self.wants(1.0)
    }

    /// The next stage anyone reaches at the shares they get now. Recovering needs no stop: nothing
    /// hangs on it.
    fn next_event_given(&self, shares: &[f64]) -> Option<f64> {
        let mut soonest: Option<f64> = None;
        for (need, &share) in self.needs.iter().zip(shares) {
            if share >= WHOLE {
                continue;
            }
            let rate = 1.0 - share.max(0.0);
            for person in &self.people {
                let short = person.short_of(need);
                let next =
                    (person.stages(need).into_iter().flatten()).find(|t| *t > short + ON_TIME);
                if let Some(at) = next {
                    let wait = (at - short) / rate;
                    soonest = Some(soonest.map_or(wait, |s| s.min(wait)));
                }
            }
        }
        soonest
    }

    fn fly(&mut self, flight: &FlightNow, _heat: &mut PartHeat, dt: f64) {
        let g = flight.acceleration / G;
        self.peak_g = self.peak_g.max(g);
        if g > G_SAFE {
            let lasts = G_ENDURANCE * (G_REFERENCE / g).powi(4);
            self.strain += dt / lasts;
            if self.strain >= 1.0 {
                self.lose_everyone(&format!("{g:.0} g was too much"));
            }
        } else {
            self.strain = (self.strain - dt / G_ENDURANCE).max(0.0);
        }
    }

    fn announce(&mut self) -> Option<String> {
        self.said.take()
    }

    fn actions(&self) -> Vec<Action> {
        vec![BOARD]
    }

    fn take_person(&mut self) -> Option<Person> {
        self.name_everyone();
        let person = self.people.pop()?;
        self.aboard -= 1;
        Some(person)
    }

    fn seat_person(&mut self, person: Person) -> Option<Person> {
        if self.aboard >= self.seats {
            return Some(person);
        }
        self.name_everyone();
        self.people.push(person);
        self.aboard += 1;
        None
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
        data::{CraftDef, OrbitStartDef, PartCatalog, RulesDef},
        system::tests::rss,
        test_data::records,
        time::SimTime,
        vessel::{Vessel, tests::catalog},
    };

    /// A one-seat capsule with life support and nothing aboard to breathe, in low orbit.
    fn capsule(catalog: &PartCatalog) -> Vessel {
        let craft: CraftDef = records(
            r#"{ id: "x", name: "", parts: [{ id: "cabin",
                shape: { shell: { bottom_diameter: 2, sections: [{ length: 2 }],
                    cap_bottom: true, cap_top: true } },
                structure: { mass: 1000 },
                components: [{ container: {} }, { crew: { seats: 1 } }, { life_support: {} }] }] }"#,
        )
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
        Vessel::in_orbit(&craft, catalog, &rss(), &start, SimTime::J2000).unwrap()
    }

    fn has_life_support(vessel: &Vessel) -> bool {
        (vessel.propulsion.parts().parts().iter())
            .flat_map(|part| part.built.iter().flat_map(|built| &built.components))
            .any(|c| matches!(c, Component::LifeSupport(_)))
    }

    #[test]
    fn the_rules_say_what_people_need() {
        let system = rss();
        let t = SimTime::J2000;
        // The test catalogue has the Realistic Life Support mod's rules.
        let mut realistic = capsule(&catalog());
        let casual_catalog = PartCatalog {
            rules: RulesDef {
                life_support: None,
                ..RulesDef::default()
            },
            ..catalog()
        };
        let mut casual = capsule(&casual_catalog);

        assert!(has_life_support(&realistic));
        assert!(!realistic.propulsion.systems.consumers().is_empty());
        // Off, there is nothing of it on the part, so nothing to show or work out each tick.
        assert!(!has_life_support(&casual));
        assert!(casual.propulsion.systems.consumers().is_empty());

        // Five minutes without oxygen, by the profile.
        realistic.step_systems(&system, t, 200.0);
        assert_eq!(realistic.propulsion.systems.crew_lost(), 0, "not yet");
        realistic.step_systems(&system, t.add_seconds(200.0), 200.0);
        assert_eq!(realistic.propulsion.systems.crew_lost(), 1);

        casual.step_systems(&system, t, 86_400.0);
        assert_eq!(casual.propulsion.systems.crew_lost(), 0);
        assert_eq!(casual.propulsion.systems.crew_aboard(), 1);
    }

    #[test]
    fn a_crew_keeps_what_it_needs_through_a_save() {
        let vessel = capsule(&catalog());
        let (_, crew) = vessel.propulsion.systems.all::<Crew>().next().unwrap();
        assert_eq!(crew.needs.len(), 3);
        let saved = rmp_serde::to_vec_named(&crew.boxed()).unwrap();
        let back: Box<dyn System> = rmp_serde::from_slice(&saved).unwrap();
        assert_eq!(back.any().downcast_ref::<Crew>(), Some(crew));
    }

    use crate::{environment::Environment, propulsion::Propulsion, systems::Cabin};

    const THREE_DAYS: f64 = 259_200.0;

    /// A game whose people need only water, and last three days without it.
    fn thirsty() -> PartCatalog {
        let mut catalog = catalog();
        catalog.rules.life_support = Some("thirst".into());
        catalog.life_support = records(
            r#"[{ id: "thirst", needs: [
                { resource: "Water", name: "water", daily: 3, lasts: 259200 }] }]"#,
        );
        catalog
    }

    /// A cabin of three, with a water tank beside it holding `water` of what it can.
    fn cabin(catalog: &PartCatalog, water: f64, crew: &str) -> Propulsion {
        let craft: CraftDef = records(&format!(
            r#"{{ id: "x", name: "", parts: [
                {{ id: "cabin", shape: {{ shell: {{ bottom_diameter: 2, sections: [{{ length: 2 }}],
                    cap_bottom: true, cap_top: true }} }},
                  structure: {{ mass: 1000 }},
                  components: [{{ container: {{}} }}, {{ crew: {crew} }}, {{ life_support: {{}} }}],
                  nodes: [{{ name: "tank", position: [0, 0, 1], direction: [0, 0, 1] }}] }},
                {{ id: "water", on: "cabin.tank", shape: {{ sphere: {{ diameter: 0.5 }} }},
                  components: [{{ storage: {{ type: "Insulated Al-Li",
                    contents: {{ Water: 1 }} }} }}] }}
            ] }}"#
        ))
        .remove(0);
        let mut p = Propulsion::from_craft(&craft, catalog).unwrap();
        fill(&mut p, water);
        p
    }

    fn fill(p: &mut Propulsion, share: f64) {
        for tank in &mut p.tanks {
            for (content, amount) in tank.contents.iter().zip(&mut tank.amounts) {
                *amount = share * content.capacity;
            }
        }
    }

    fn people(p: &Propulsion) -> Vec<Person> {
        p.systems
            .people()
            .map(|(_, person)| person.clone())
            .collect()
    }

    fn stages(p: &Propulsion) -> Vec<Stage> {
        let (_, crew) = p.systems.all::<Crew>().next().unwrap();
        crew.people
            .iter()
            .map(|person| person.stage(&crew.needs).0)
            .collect()
    }

    /// When each person first reached each stage, and when each died, s.
    #[derive(Default)]
    struct Log {
        reached: BTreeMap<(String, Stage), f64>,
        died: Vec<(String, f64)>,
    }

    /// Run `p` for `seconds`, in steps no longer than `longest`, cut where its systems say, as time
    /// warp cuts them.
    fn run(p: &mut Propulsion, seconds: f64, longest: f64) -> Log {
        let environment = Environment::DARK;
        let mut log = Log::default();
        let mut t = 0.0;
        for _ in 0..100_000 {
            if t >= seconds {
                break;
            }
            let before = people(p);
            let next = p
                .next_system_event(&environment, false)
                .unwrap_or(f64::INFINITY);
            let step = next.min(longest).min(seconds - t).max(1e-6);
            p.step_systems(&environment, step, false);
            t += step;
            let after = people(p);
            for person in &before {
                if !after.iter().any(|a| a.name == person.name) {
                    log.died.push((person.name.clone(), t));
                }
            }
            for (person, stage) in after.iter().zip(stages(p)) {
                log.reached.entry((person.name.clone(), stage)).or_insert(t);
            }
        }
        log
    }

    #[test]
    fn without_water_they_weaken_by_stages_and_die_each_at_their_own_time() {
        let catalog = thirsty();
        let crew = cabin(&catalog, 0.0, "{ seats: 3 }");
        let them = people(&crew);
        assert_eq!(them.len(), 3);
        // At the highest warp, and a minute at a time: the same, to the rounding.
        for longest in [f64::INFINITY, 60.0] {
            let mut p = crew.clone();
            let log = run(&mut p, 2.0 * THREE_DAYS, longest);
            for person in &them {
                let lasts = THREE_DAYS * person.resilience;
                for (stage, at) in [(Stage::Weak, 1.0 / 3.0), (Stage::Critical, 2.0 / 3.0)] {
                    let when = log.reached[&(person.name.clone(), stage)];
                    assert!(
                        (when - at * lasts).abs() < 1e-3,
                        "{} {stage:?} at {when}",
                        person.name
                    );
                }
                let died = log
                    .died
                    .iter()
                    .find(|(name, _)| *name == person.name)
                    .unwrap()
                    .1;
                assert!(
                    (died - lasts).abs() < 1e-3,
                    "{} died at {died}, not {lasts}",
                    person.name
                );
            }
            // The least resilient first, and not a moment before their time.
            let first = them
                .iter()
                .map(|p| p.resilience)
                .fold(f64::INFINITY, f64::min);
            assert!((log.died[0].1 - first * THREE_DAYS).abs() < 1e-3);
            assert_eq!(p.systems.crew_lost(), 3);
        }
    }

    #[test]
    fn given_water_back_they_recover() {
        let mut p = cabin(&thirsty(), 0.0, "{ seats: 3 }");
        run(&mut p, 0.5 * THREE_DAYS, f64::INFINITY);
        assert_eq!(stages(&p), [Stage::Weak; 3]);
        fill(&mut p, 1.0);
        // They come back as fast as they went.
        run(&mut p, 0.25 * THREE_DAYS, f64::INFINITY);
        assert!(
            stages(&p).iter().all(|s| *s == Stage::Short),
            "{:?}",
            stages(&p)
        );
        run(&mut p, 0.25 * THREE_DAYS + 1.0, f64::INFINITY);
        assert_eq!(stages(&p), [Stage::Fine; 3]);
        assert_eq!(p.systems.crew_lost(), 0);
    }

    #[test]
    fn with_half_the_water_they_decline_at_half_the_pace() {
        // Three people drink 9 kg a day; give them 4.5 kg a day by filling the tank as they go.
        let mut p = cabin(&thirsty(), 0.0, "{ seats: 3 }");
        let environment = Environment::DARK;
        for _ in 0..3_600 {
            for tank in &mut p.tanks {
                tank.amounts[0] = 4.5 / 86_400.0 * 60.0;
            }
            p.step_systems(&environment, 60.0, false);
        }
        let (_, crew) = p.systems.all::<Crew>().next().unwrap();
        for person in &crew.people {
            assert!((person.short["Water"] - 0.5 * 3_600.0 * 60.0).abs() < 1.0);
        }
    }

    #[test]
    fn people_are_named_from_the_list_or_by_the_craft() {
        let catalog = catalog();
        let p = cabin(&catalog, 1.0, "{ seats: 3 }");
        let names: Vec<String> = people(&p).into_iter().map(|p| p.name).collect();
        let first: Vec<&String> = catalog.crew_names.iter().flat_map(|n| &n.first).collect();
        for name in &names {
            let given = name.split(' ').next().unwrap();
            assert!(first.iter().any(|f| *f == given), "{name}");
        }
        assert_eq!(
            names,
            people(&cabin(&catalog, 1.0, "{ seats: 3 }"))
                .into_iter()
                .map(|p| p.name)
                .collect::<Vec<_>>(),
            "the same seat, the same name"
        );
        let named = cabin(&catalog, 1.0, r#"{ seats: 2, names: ["Ana Lima"] }"#);
        let names: Vec<String> = people(&named).into_iter().map(|p| p.name).collect();
        assert_eq!(names[0], "Ana Lima");
        assert_ne!(names[1], "Ana Lima");
    }

    #[test]
    fn boarding_moves_the_person_and_what_they_lack() {
        let catalog = thirsty();
        let craft: CraftDef = records(
            r#"{ id: "x", name: "", parts: [
                { id: "a", shape: { shell: { bottom_diameter: 2, sections: [{ length: 2 }],
                    cap_bottom: true, cap_top: true } },
                  structure: { mass: 1000 },
                  components: [{ container: {} }, { crew: { seats: 1 } }],
                  nodes: [{ name: "up", position: [0, 0, 2], direction: [0, 0, 1] }] },
                { id: "b", on: "a.up", shape: { shell: { bottom_diameter: 2,
                    sections: [{ length: 2 }], cap_bottom: true, cap_top: true } },
                  structure: { mass: 1000 },
                  components: [{ container: {} }, { crew: { seats: 1, aboard: 0 } }] }
            ] }"#,
        )
        .remove(0);
        let mut p = Propulsion::from_craft(&craft, &catalog).unwrap();
        let before = people(&p);
        assert_eq!(before.len(), 1);
        let to = (p.systems.all::<Crew>())
            .find(|(_, crew)| crew.aboard == 0)
            .unwrap()
            .0;
        assert_eq!(p.systems.board(to), Some(before[0].name.clone()));
        let (part, person) = p.systems.people().next().unwrap();
        assert_eq!((part, person), (to, &before[0]));
        assert_eq!(p.systems.board(to), None, "no room left");
    }

    /// One person in a 2 m³ cabin under the Realistic profile, with what `life_support` carries, a
    /// big battery, `extra` on the cabin, and a water tank holding `water` kg and an empty waste
    /// water tank beside it.
    fn realistic_cabin(life_support: &str, extra: &str, water: f64) -> Propulsion {
        let craft: CraftDef = records(&format!(
            r#"{{ id: "x", name: "", parts: [
                {{ id: "cabin", shape: {{ shell: {{ bottom_diameter: 2, sections: [{{ length: 2 }}],
                    cap_bottom: true, cap_top: true }} }},
                  structure: {{ mass: 1000 }},
                  components: [{{ container: {{}} }}, {{ crew: {{ seats: 1 }} }},
                    {{ life_support: {life_support} }}, {{ battery: {{ charge: 1000000 }} }} {extra}],
                  nodes: [{{ name: "a", position: [0, 0, 0.5], direction: [0, 0, 1] }},
                          {{ name: "b", position: [0, 0, 1.5], direction: [0, 0, 1] }}] }},
                {{ id: "water", on: "cabin.a", shape: {{ sphere: {{ diameter: 0.5 }} }},
                  components: [{{ storage: {{ type: "Insulated Al-Li",
                    contents: {{ Water: 1 }} }} }}] }},
                {{ id: "waste", on: "cabin.b", shape: {{ sphere: {{ diameter: 0.5 }} }},
                  components: [{{ storage: {{ type: "Insulated Al-Li",
                    contents: {{ WasteWater: 1 }} }} }}] }}
            ] }}"#
        ))
        .remove(0);
        let mut p = Propulsion::from_craft(&craft, &catalog()).unwrap();
        for tank in &mut p.tanks {
            for (content, amount) in tank.contents.iter().zip(&mut tank.amounts) {
                *amount = if content.resource == "Water" {
                    water
                } else {
                    0.0
                };
            }
        }
        p
    }

    fn water(p: &Propulsion) -> f64 {
        p.tanks.iter().map(|t| t.held("Water")).sum()
    }

    /// Without LiOH the scrubber cannot run: the cabin's 0.3 kg of CO2 room fills in 0.3 of a
    /// day, and with nowhere to breathe out, they last as long as they would without air.
    #[test]
    fn a_long_flight_without_a_scrubber_is_lost_to_co2() {
        let supplies = "LqdOxygen: 30, Food: 20";
        let mut foul = realistic_cabin(&format!("{{ carries: {{ {supplies} }} }}"), "", 50.0);
        let (_, cabin) = foul.systems.all::<Cabin>().next().unwrap();
        assert_eq!(cabin.air_of("CO2"), Some((0.0, 0.3)));
        let log = run(&mut foul, 2.0 * 86_400.0, f64::INFINITY);
        let full = 0.3 * 86_400.0;
        let person = &people(&realistic_cabin("{}", "", 0.0))[0];
        let (_, died) = log.died[0].clone();
        let expected = full + 300.0 * person.resilience;
        assert!(
            (died - expected).abs() < 1e-3,
            "died at {died}, not {expected}"
        );

        let mut clean = realistic_cabin(
            &format!("{{ carries: {{ {supplies}, LiOH: 15 }} }}"),
            "",
            50.0,
        );
        run(&mut clean, 10.0 * 86_400.0, f64::INFINITY);
        assert_eq!(clean.systems.crew_lost(), 0);
        let (_, cabin) = clean.systems.all::<Cabin>().next().unwrap();
        let (co2, _) = cabin.air_of("CO2").unwrap();
        assert!(co2 < 1e-6, "the scrubber keeps up: {co2} kg");
    }

    /// One person drinks 3 kg a day and makes 2.5 kg of waste water, which the recycler turns back
    /// into 85 % of it: 9 kg of water lasts three days without it, and far longer with it.
    #[test]
    fn recycling_makes_the_water_last_longer() {
        let kit = "{ carries: { LqdOxygen: 30, Food: 20, LiOH: 15 } }";
        let mut without = realistic_cabin(kit, "", 9.0);
        let mut with = realistic_cabin(kit, r#", { process: { process: "water_recovery" } }"#, 9.0);
        run(&mut without, 5.0 * 86_400.0, f64::INFINITY);
        run(&mut with, 5.0 * 86_400.0, f64::INFINITY);
        assert!(water(&without) < 1e-6, "{}", water(&without));
        assert_ne!(stages(&without)[0], Stage::Fine);
        assert!(water(&with) > 4.0, "{}", water(&with));
        assert_eq!(stages(&with)[0], Stage::Fine);
    }

    /// A pod's lockers carry so many days of what the profile says, and the kit it is fitted with
    /// burns, and nothing else is fitted under rules that fit nothing.
    #[test]
    fn lockers_carry_days_of_everything() {
        let p = realistic_cabin("{ days: 10 }", "", 0.0);
        let (_, cabin) = p.systems.all::<Cabin>().next().unwrap();
        let carried = |r: &str| cabin.kept.iter().find(|k| k.resource == r).map(|k| k.held);
        assert!((carried("LqdOxygen").unwrap() - 8.4).abs() < 1e-9);
        assert!((carried("Water").unwrap() - 30.0).abs() < 1e-9);
        assert!((carried("Food").unwrap() - 6.0).abs() < 1e-9);
        // The scrubber's LiOH at its full 1.2 kg of CO2 a day.
        assert!((carried("LiOH").unwrap() - 1.5114e-5 * 86_400.0 * 10.0).abs() < 1e-6);
        assert_eq!(p.systems.all::<crate::systems::Process>().count(), 1);
    }
}
