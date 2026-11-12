//! Seats, and the people in them. Each person has a name, and with a `life_support` on the part
//! they use what the rules' profile says people need, from the tanks it can reach. Going short of
//! something they cannot last without, each goes through stages, short, weak and critical, and
//! dies at the profile's time times their own resilience; given it back, they recover as fast as
//! they declined. With part of what they need, the shortfall is shared. The cabin's air going out
//! of a limit the profile sets (too little pressure, too much CO2) counts the same way, unless they
//! have their suits on, which give them air from the cabin's supply. Under rules with radiation,
//! each takes a dose through the walls of the part they sit in, its mass over its skin, and sickens
//! by stages as it builds up; it does not go away. Time warp stops at each stage. Any crew can be
//! lost to acceleration.

use std::{any::Any, collections::BTreeMap};

use serde::{Deserialize, Serialize};

use super::{Air, CREW_PRIORITY, Exchange, FlightNow, PartHeat, Reading, System};
use crate::{
    component::Component,
    data::{AirLimitDef, NeedDef, RadiationProfileDef, fnv},
    parts::Part,
    radiation::Radiation,
    staging::Action,
};

pub const BOARD: Action = Action::new("board", "Move someone here");
pub const SUITS_ON: Action = Action::new("suits_on", "Suits on");
pub const SUITS_OFF: Action = Action::new("suits_off", "Suits off");

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
/// A dose this close to one, as a share of it, is that dose: warp stops at it, to the rounding.
const REACHED: f64 = 1e-9;
/// What people go short of when it is radiation that sickens them.
const RADIATION: &str = "radiation sickness";

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
    /// The radiation they have taken, Sv.
    #[serde(default)]
    pub dose: f64,
}

impl Person {
    pub fn new(name: String) -> Self {
        let resilience = 1.0 + 0.2 * (fnv(&name) % 1_000) as f64 / 999.0;
        Self {
            name,
            resilience,
            short: BTreeMap::new(),
            dose: 0.0,
        }
    }

    /// The doses at which they are sick, weak, critical, and die, Sv.
    fn doses(&self, profile: &RadiationProfileDef) -> [f64; 4] {
        [profile.sick, profile.weak, profile.critical, profile.dies].map(|d| d * self.resilience)
    }

    /// How their dose has left them, and whether it has killed them.
    fn sickness(&self, profile: &RadiationProfileDef) -> (Stage, bool) {
        let reached = |at: f64| self.dose >= at * (1.0 - REACHED);
        let [sick, weak, critical, dies] = self.doses(profile);
        let stage = if reached(critical) {
            Stage::Critical
        } else if reached(weak) {
            Stage::Weak
        } else if reached(sick) {
            Stage::Short
        } else {
            Stage::Fine
        };
        (stage, reached(dies))
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
    /// What the cabin's air has to be for them to be well.
    #[serde(default)]
    pub limits: Vec<AirLimitDef>,
    /// Which of `limits` the air is out of now.
    #[serde(default)]
    pub outside: Vec<bool>,
    /// Each gas of the cabin's air, and what its regulator tops it up from: a suit breathes that.
    #[serde(default)]
    pub gases: Vec<(String, Option<String>)>,
    /// Their suits are on: the cabin's air is nothing to them.
    #[serde(default)]
    pub suited: bool,
    /// What radiation does to them, under rules with radiation.
    #[serde(default)]
    pub radiation: Option<RadiationProfileDef>,
    /// What their part's walls give them, kg/m²: its mass over its skin.
    #[serde(default)]
    pub shielding: f64,
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
        radiation,
    } = component
    else {
        return None;
    };
    let life_support =
        (part.built.iter().flat_map(|built| &built.components)).find_map(|c| match c {
            Component::LifeSupport(life_support) => Some(life_support),
            _ => None,
        });
    let (needs, dump) = life_support
        .map(|l| (l.needs.clone(), l.dump.clone()))
        .unwrap_or_default();
    let cabin = life_support.and_then(|l| l.cabin.as_ref());
    let limits = cabin.map(|c| c.limits.clone()).unwrap_or_default();
    let gases = (cabin.iter().flat_map(|c| &c.air.gases))
        .map(|gas| (gas.resource.clone(), gas.from.clone()))
        .collect();
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
        outside: vec![false; limits.len()],
        limits,
        gases,
        suited: false,
        radiation: radiation.clone(),
        shielding: shielding(part),
        strain: 0.0,
        peak_g: 0.0,
        said: None,
    };
    crew.name_everyone();
    Some(Box::new(crew))
}

/// What a part's walls give whoever is inside, kg/m²: what it weighs, dry, over the area of its
/// skin (that of its stand-in shape if it is drawn from a model).
fn shielding(part: &Part) -> f64 {
    let Some(built) = &part.built else {
        return 0.0;
    };
    let geometry = &built.geometry;
    let area = Some(geometry.skin_area(false))
        .filter(|a| *a > 0.0)
        .unwrap_or_else(|| geometry.skin_area(true));
    if area > 0.0 {
        built.mass.mass / area
    } else {
        0.0
    }
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
            .map(|p| self.stage_of(p).0)
            .max()
            .unwrap_or(Stage::Fine)
    }

    /// Everything they can go short of: what they use, then the cabin's air limits.
    pub fn conditions(&self) -> Vec<NeedDef> {
        (self.needs.iter().cloned())
            .chain(self.limits.iter().map(AirLimitDef::need))
            .collect()
    }

    /// How `person` is doing by the worst of everything, and what they are short of most.
    pub fn stage_of(&self, person: &Person) -> (Stage, Option<String>) {
        let conditions = self.conditions();
        let (stage, of) = person.stage(&conditions);
        let sick = (self.radiation.as_ref()).map_or(Stage::Fine, |p| person.sickness(p).0);
        if sick > stage {
            return (sick, Some(RADIATION.to_owned()));
        }
        (stage, of.map(str::to_owned))
    }

    /// The share of each kind of radiation that gets to them under `column` kg/m² of air.
    pub fn through(&self, column: f64) -> Option<Radiation> {
        let profile = self.radiation.as_ref()?;
        Some(Radiation::through(
            self.shielding + column,
            &profile.shielding,
        ))
    }

    /// The share they have of each of the cabin's air limits: none out of one, unless suited.
    fn limit_shares(&self) -> impl Iterator<Item = f64> + '_ {
        (0..self.limits.len()).map(|k| {
            let out = self.outside.get(k).copied().unwrap_or(false);
            if out && !self.suited { 0.0 } else { 1.0 }
        })
    }

    /// What a need for `resource` is taken from: in a suit, a gas of the cabin's air comes from
    /// what tops it up.
    fn source(&self, resource: &str) -> String {
        let from = (self.gases.iter())
            .find(|(gas, _)| gas == resource)
            .and_then(|(_, from)| from.clone());
        match from {
            Some(from) if self.suited => from,
            _ => resource.to_owned(),
        }
    }

    fn is_air(&self, resource: &str) -> bool {
        self.gases.iter().any(|(gas, _)| gas == resource)
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
    /// goes with it: with nowhere to breathe out to, they cannot breathe. In suits, they breathe
    /// what tops the cabin's air up, and what they breathe out goes into their suits.
    fn wants(&self, dt: f64) -> Vec<Exchange> {
        let people = f64::from(self.aboard);
        let mut dump = self.dump.clone();
        if self.suited {
            dump.extend(self.gases.iter().map(|(gas, _)| gas.clone()));
        }
        self.rates().map_or(Vec::new(), |rates| {
            (self.needs.iter().zip(rates))
                .map(|(need, rate)| Exchange {
                    inputs: vec![(self.source(&need.resource), rate * dt)],
                    outputs: (need.makes.iter())
                        .filter(|(made, _)| !(self.suited && self.is_air(made)))
                        .map(|(made, daily)| (made.clone(), daily * people / DAY * dt))
                        .collect(),
                    dump: dump.clone(),
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
        let needs = self.conditions();
        let shares: Vec<f64> = (shares.iter().copied())
            .take(self.needs.len())
            .chain(self.limit_shares())
            .collect();
        let before: Vec<Stage> = self.people.iter().map(|p| p.stage(&needs).0).collect();
        for (need, &share) in needs.iter().zip(&shares) {
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
        let shares: Vec<f64> = (shares.iter().copied())
            .take(self.needs.len())
            .chain(self.limit_shares())
            .collect();
        for (need, &share) in self.conditions().iter().zip(&shares) {
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

    fn exposed(&self) -> bool {
        self.radiation.is_some() && !self.people.is_empty()
    }

    fn irradiate(&mut self, exposure: &Radiation, column: f64) {
        let (Some(profile), Some(through)) = (self.radiation.clone(), self.through(column)) else {
            return;
        };
        let dose = exposure.weighted(&through);
        if dose <= 0.0 {
            return;
        }
        let mut news = Vec::new();
        let mut alive = Vec::new();
        for mut person in std::mem::take(&mut self.people) {
            let (was, _) = person.sickness(&profile);
            person.dose += dose;
            let (now, dead) = person.sickness(&profile);
            if dead {
                news.push(format!("{} has died of {RADIATION}", person.name));
                self.lost += 1;
                continue;
            }
            if now != was {
                news.push(match now {
                    Stage::Short => format!("{} is sick from radiation", person.name),
                    stage => format!("{} is {} from {RADIATION}", person.name, stage.word()),
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

    /// The least dose anyone aboard still has to take to reach their next stage.
    fn dose_until(&self, column: f64) -> Option<(Radiation, f64)> {
        let profile = self.radiation.as_ref()?;
        let through = self.through(column)?;
        let left = (self.people.iter())
            .filter_map(|person| {
                (person.doses(profile).into_iter())
                    .find(|at| person.dose < at * (1.0 - REACHED))
                    .map(|at| at - person.dose)
            })
            .reduce(f64::min)?;
        Some((through, left))
    }

    fn breathe(&mut self, air: Option<&Air>) {
        self.outside = (self.limits.iter())
            .map(|limit| air.is_some_and(|air| limit.broken_by(air.pressure, &air.partial)))
            .collect();
    }

    fn actions(&self) -> Vec<Action> {
        match (self.limits.is_empty(), self.suited) {
            (true, _) => vec![BOARD],
            (false, false) => vec![BOARD, SUITS_ON],
            (false, true) => vec![BOARD, SUITS_OFF],
        }
    }

    fn act(&mut self, action: Action) -> Option<String> {
        if action.kind == SUITS_ON.kind && !self.suited && !self.limits.is_empty() {
            self.suited = true;
            Some("Suits on".to_owned())
        } else if action.kind == SUITS_OFF.kind && self.suited {
            self.suited = false;
            Some("Suits off".to_owned())
        } else {
            None
        }
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
    use std::collections::BTreeMap;

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

        // With nothing to top its air up or take the CO2 out, the cabin's own air lasts hours,
        // not a day.
        realistic.step_systems(&system, t, 3_600.0);
        assert_eq!(realistic.propulsion.systems.crew_lost(), 0, "not yet");
        realistic.step_systems(&system, t.add_seconds(3_600.0), 86_400.0);
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

    use crate::{
        environment::Environment,
        propulsion::Propulsion,
        systems::{Cabin, FILL, LET_DOWN},
    };

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
            .map(|person| crew.stage_of(person).0)
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

    /// The kg of a gas of `molar_mass` at `pascals` in the test cabin: 2 m³ at 294 K.
    fn kg_at(pascals: f64, molar_mass: f64) -> f64 {
        pascals * 2.0 * molar_mass / (8.314_462_618 * 294.0)
    }

    /// Without LiOH the scrubber cannot run: one person breathes out a kilogram of CO2 a day into
    /// the cabin. Past 2 kPa of it they go short of clean air, and are weak a third of the half day
    /// they last later. At 8 kPa no more goes in: with nowhere to breathe out, they last as long as
    /// they would without air. The same at the highest warp and a minute at a time.
    #[test]
    fn breathing_without_a_scrubber_fouls_the_air_until_the_crew_weakens_and_dies() {
        let supplies = "LqdOxygen: 30, Food: 20";
        let foul = realistic_cabin(&format!("{{ carries: {{ {supplies} }} }}"), "", 50.0);
        let (_, cabin) = foul.systems.all::<Cabin>().next().unwrap();
        let most = kg_at(8_000.0, 0.044);
        assert_eq!(cabin.air_of("CO2"), Some((0.0, most)));
        let person = people(&foul)[0].clone();
        let per_second = 1.0 / 86_400.0;
        let foul_from = kg_at(2_000.0, 0.044) / per_second;
        let weak = foul_from + 43_200.0 / 3.0 * person.resilience;
        let full = most / per_second;
        assert!(weak < full, "they weaken before the air is full");
        let died = full + 300.0 * person.resilience;
        for longest in [f64::INFINITY, 60.0] {
            let mut p = foul.clone();
            let log = run(&mut p, 2.0 * 86_400.0, longest);
            if longest.is_infinite() {
                // Warp stops where the air fouls, and then not until they are weak.
                let stop = log.reached[&(person.name.clone(), Stage::Fine)];
                assert!((stop - foul_from).abs() < 1e-3, "first stop at {stop}");
            }
            let when = log.reached[&(person.name.clone(), Stage::Weak)];
            assert!((when - weak).abs() < 1e-3, "weak at {when}, not {weak}");
            let (_, at) = log.died[0].clone();
            assert!((at - died).abs() < 1e-3, "died at {at}, not {died}");
        }

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

    fn cabin_of(p: &Propulsion) -> &Cabin {
        p.systems.all::<Cabin>().next().unwrap().1
    }

    fn cabin_part(p: &Propulsion) -> crate::parts::PartId {
        p.systems.all::<Cabin>().next().unwrap().0
    }

    fn locker(p: &Propulsion, resource: &str) -> f64 {
        let kept = cabin_of(p).kept.iter().find(|k| k.resource == resource);
        kept.map_or(0.0, |k| k.held)
    }

    /// A hole of a square centimetre in a cabin with nothing to top its air up lets the air out at
    /// the rate choked flow through it gives, by e^-kt, and the crew is lost a minute and a half
    /// after the pressure falls below the 6.3 kPa people can live at: the same at the highest warp
    /// and a second at a time.
    #[test]
    fn a_leak_lets_the_air_out_at_the_rate_its_hole_gives_and_the_crew_is_lost() {
        let mut fresh = realistic_cabin("{ carries: { Food: 20, LiOH: 15 } }", "", 50.0);
        let part = cabin_part(&fresh);
        fresh.systems.breach(part, 1e-4);
        let k = cabin_of(&fresh).leak_rate();
        // 0.8 of a nozzle's choked flow at the speed of sound in air at 294 K, out of 2 m³.
        assert!((1.0 / k - 125.7).abs() < 0.05, "{} s", 1.0 / k);
        let p0 = cabin_of(&fresh).pressure();
        assert!((p0 - 101_000.0).abs() < 1e-6, "{p0} Pa");

        let mut p = fresh.clone();
        run(&mut p, 60.0, f64::INFINITY);
        let expected = p0 * (-k * 60.0).exp();
        let now = cabin_of(&p).pressure();
        assert!(
            (now - expected).abs() < 1e-3 * expected,
            "{now}, not {expected} Pa"
        );

        let person = people(&fresh)[0].clone();
        let below = (p0 / 6_300.0).ln() / k;
        let died = below + 90.0 * person.resilience;
        let mut when = Vec::new();
        for longest in [f64::INFINITY, 1.0] {
            let mut p = fresh.clone();
            let log = run(&mut p, 600.0, longest);
            assert_eq!(p.systems.crew_lost(), 1);
            let (_, at) = log.died[0].clone();
            // Breathing takes a little of the oxygen besides.
            assert!((at - died).abs() < 2.0, "died at {at}, not {died}");
            when.push(at);
        }
        assert!((when[0] - when[1]).abs() < 1e-3, "{when:?}");

        // The hole goes with the cabin into a save.
        let (_, cabin) = fresh.systems.all::<Cabin>().next().unwrap();
        let saved = rmp_serde::to_vec_named(&cabin.boxed()).unwrap();
        let back: Box<dyn System> = rmp_serde::from_slice(&saved).unwrap();
        assert_eq!(back.any().downcast_ref::<Cabin>(), Some(cabin));
    }

    /// A hole of a square millimetre is held: the regulator tops the air up from the lockers as
    /// fast as it goes, so the pressure stays where it is and the nitrogen in the lockers goes at
    /// the rate the hole gives, exactly, at any warp. Once that is gone the nitrogen in the air
    /// goes too, but the oxygen is still kept at 21 kPa, which people live at.
    #[test]
    fn a_small_leak_is_held_while_what_tops_the_air_up_lasts() {
        let kit = "{ carries: { LqdOxygen: 30, Nitrogen: 2, Food: 20, LiOH: 15 } }";
        let mut fresh = realistic_cabin(kit, "", 50.0);
        let part = cabin_part(&fresh);
        fresh.systems.breach(part, 1e-6);
        let k = cabin_of(&fresh).leak_rate();
        let nitrogen = kg_at(80_000.0, 0.028);
        let p0 = cabin_of(&fresh).pressure();
        for longest in [f64::INFINITY, 60.0] {
            let mut p = fresh.clone();
            run(&mut p, 3_600.0, longest);
            let now = cabin_of(&p).pressure();
            assert!((now - p0).abs() < 1e-6 * p0, "{now} Pa");
            let used = 2.0 - locker(&p, "Nitrogen");
            let expected = k * nitrogen * 3_600.0;
            assert!((used - expected).abs() < 1e-9, "{used}, not {expected} kg");
        }
        let gone = 2.0 / (k * nitrogen);
        let mut p = fresh.clone();
        run(&mut p, gone + 20.0 / k, f64::INFINITY);
        assert!(locker(&p, "Nitrogen") < 1e-9);
        let cabin = cabin_of(&p);
        assert!(cabin.partial_of("nitrogen").unwrap() < 1.0);
        let oxygen = cabin.partial_of("oxygen").unwrap();
        assert!((oxygen - 21_000.0).abs() < 1e-6 * 21_000.0, "{oxygen} Pa");
        assert_eq!(p.systems.crew_lost(), 0);
        assert_eq!(stages(&p), [Stage::Fine]);
    }

    /// A one-seat capsule weighing `mass` kg (2 m across and 2 m long, 18.8 m² of skin) in a
    /// circular orbit `altitude` m over the Earth, with nobody needing anything but radiation
    /// counting.
    fn exposed_capsule(altitude: f64, mass: f64) -> Vessel {
        let craft: CraftDef = records(&format!(
            r#"{{ id: "x", name: "", parts: [{{ id: "cabin",
                shape: {{ shell: {{ bottom_diameter: 2, sections: [{{ length: 2 }}],
                    cap_bottom: true, cap_top: true }} }},
                structure: {{ mass: {mass} }},
                components: [{{ container: {{}} }}, {{ crew: {{ seats: 1 }} }}] }}] }}"#
        ))
        .remove(0);
        let start = OrbitStartDef {
            body: "Earth".into(),
            periapsis_altitude: altitude,
            apoapsis_altitude: altitude,
            inclination: 0.0,
            longitude_of_ascending_node: 0.0,
            plane: None,
            argument_of_periapsis: 0.0,
            mean_anomaly: 0.0,
            spin: [0.0; 3],
        };
        Vessel::in_orbit(&craft, &catalog(), &rss(), &start, SimTime::J2000).unwrap()
    }

    fn dose(vessel: &Vessel) -> f64 {
        let (_, crew) = vessel.propulsion.systems.all::<Crew>().next().unwrap();
        crew.people.first().map_or(0.0, |p| p.dose)
    }

    /// Fly `vessel`'s systems for `days`, in steps no longer than `longest` seconds, cut where they
    /// say, as time warp cuts them: when its one person first reached each stage, s.
    fn fly(vessel: &mut Vessel, days: f64, longest: f64) -> BTreeMap<Stage, f64> {
        let system = rss();
        let start = SimTime::J2000;
        let end = start.add_seconds(days * 86_400.0);
        let mut reached = BTreeMap::new();
        let mut t = start;
        while t < end {
            let horizon = end.min(t.add_seconds(longest.min(1e9)));
            let next = vessel.next_system_event(&system, t, horizon);
            let to = next.map_or(horizon, |n| n.min(horizon));
            vessel.step_systems(&system, t, to.seconds_since(t));
            t = to;
            let (_, crew) = vessel.propulsion.systems.all::<Crew>().next().unwrap();
            if let Some(person) = crew.people.first() {
                reached
                    .entry(crew.stage_of(person).0)
                    .or_insert(t.seconds_since(start));
            }
        }
        reached
    }

    /// A day in the heart of the inner belt is far worse than a day in low orbit, and a capsule
    /// with heavier walls lets less of it through.
    #[test]
    fn a_capsule_in_the_inner_belt_takes_far_more_than_one_in_low_orbit() {
        let mut low = exposed_capsule(400e3, 1_000.0);
        let mut belt = exposed_capsule(3_500e3, 1_000.0);
        let mut heavy = exposed_capsule(3_500e3, 4_000.0);
        for vessel in [&mut low, &mut belt, &mut heavy] {
            fly(vessel, 1.0, f64::INFINITY);
        }
        let (_, crew) = low.propulsion.systems.all::<Crew>().next().unwrap();
        assert!(
            (crew.shielding - 1_000.0 / 18.85).abs() < 0.5,
            "{}",
            crew.shielding
        );
        // Cosmic rays only, much of them kept out, a little cut by the walls.
        assert!(
            dose(&low) > 0.2e-3 && dose(&low) < 0.4e-3,
            "{} Sv",
            dose(&low)
        );
        assert!(
            dose(&belt) > 100.0 * dose(&low),
            "{} {}",
            dose(&belt),
            dose(&low)
        );
        assert!(
            dose(&heavy) < 0.01 * dose(&belt),
            "{} {}",
            dose(&heavy),
            dose(&belt)
        );
    }

    /// Parked in the inner belt, a person takes about a tenth of a sievert a day: sick at 1 Sv and
    /// weak at 2, times their resilience, and time warp stops there, at the highest warp as an hour
    /// at a time.
    #[test]
    fn radiation_sickens_by_stages_at_its_doses_at_any_warp() {
        let fresh = exposed_capsule(3_500e3, 1_000.0);
        let person = people(&fresh.propulsion)[0].clone();
        let mut when = Vec::new();
        for longest in [f64::INFINITY, 3_600.0] {
            let mut vessel = fresh.clone();
            let reached = fly(&mut vessel, 25.0, longest);
            let sick = reached[&Stage::Short];
            let weak = reached[&Stage::Weak];
            assert!(
                sick > 5.0 * 86_400.0 && weak < 25.0 * 86_400.0,
                "{reached:?}"
            );
            when.push((sick, weak));
            // Stopped at the dose, then on until weak.
            let mut again = fresh.clone();
            fly(&mut again, sick / 86_400.0, f64::INFINITY);
            let at = dose(&again);
            assert!((at - person.resilience).abs() < 1e-6 * at, "{at} Sv");
            assert_eq!(vessel.propulsion.systems.crew_lost(), 0);
        }
        let ((a, b), (c, d)) = (when[0], when[1]);
        assert!((a - c).abs() < 5.0 && (b - d).abs() < 5.0, "{when:?}");
    }

    /// Let down, the cabin is empty in a couple of minutes. With their suits on, the crew breathe
    /// the oxygen the air is topped up from and are well; filled again, it is back at sea-level
    /// pressure in the minute the regulator takes, and they take their suits off. Without suits
    /// they are lost.
    #[test]
    fn a_cabin_let_down_and_filled_again_with_the_crew_in_suits() {
        let kit = "{ carries: { LqdOxygen: 30, Nitrogen: 5, Food: 20, LiOH: 15 } }";
        let fresh = realistic_cabin(kit, "", 50.0);
        let part = cabin_part(&fresh);
        let p0 = cabin_of(&fresh).pressure();

        let mut p = fresh.clone();
        assert_eq!(p.systems.act(part, SUITS_ON).as_deref(), Some("Suits on"));
        assert!(p.systems.act(part, LET_DOWN).is_some());
        run(&mut p, 3_600.0, f64::INFINITY);
        assert!(
            cabin_of(&p).pressure() < 1.0,
            "{} Pa",
            cabin_of(&p).pressure()
        );
        assert_eq!(p.systems.crew_lost(), 0);
        assert_eq!(stages(&p), [Stage::Fine]);
        // An hour of one person's breathing came out of the lockers' oxygen.
        let breathed = 30.0 - locker(&p, "LqdOxygen");
        assert!((breathed - 0.84 / 24.0).abs() < 1e-9, "{breathed} kg");

        assert!(p.systems.act(part, FILL).is_some());
        run(&mut p, 120.0, f64::INFINITY);
        let now = cabin_of(&p).pressure();
        assert!((now - p0).abs() < 1e-6 * p0, "{now} Pa");
        assert!(p.systems.act(part, SUITS_OFF).is_some());
        run(&mut p, 3_600.0, f64::INFINITY);
        assert_eq!(stages(&p), [Stage::Fine]);

        let mut q = fresh.clone();
        q.systems.act(part, LET_DOWN);
        let log = run(&mut q, 600.0, f64::INFINITY);
        assert_eq!(q.systems.crew_lost(), 1);
        assert!(log.died[0].1 < 300.0, "died at {}", log.died[0].1);
    }
}
