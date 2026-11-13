//! Contracts: goals a career is offered for money (collection `contracts`). A few are offered at a
//! time, those of its tech tree whose `after` are all done, in the records' order, so what is
//! offered grows with what has been done. One taken on is a mission goal ([`crate::mission`]) for whatever vessel is flown;
//! met, it pays its reward into the career's funds, and past its deadline it lapses and can be
//! offered again.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    career::money,
    data::LaunchSiteDef,
    mission::{self, Condition, Goal, GoalDef, Place, Stats, View},
    system::SolarSystem,
    time::SimTime,
    universe::{SimEvent, Universe},
    vessel::VesselId,
};

/// How many contracts are offered at once.
pub const OFFERED: usize = 3;

/// Seconds in a day.
const DAY: f64 = 86_400.0;

/// A contract a career can take on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractDef {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// What meets it, of the vessel being flown.
    pub when: Condition,
    /// It has to hold this long without a break, s.
    #[serde(default)]
    pub hold: f64,
    /// Funds it pays when it is met.
    pub reward: f64,
    /// Days it has to be met in from when it is taken on; no deadline if 0.
    #[serde(default)]
    pub days: f64,
    /// The contracts done before it is offered.
    #[serde(default)]
    pub after: Vec<String>,
    /// Offered only in a career on this tech tree (`tech_trees`); in any if not given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<String>,
}

/// A contract taken on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contract {
    pub def: ContractDef,
    /// When the game first looked at it taken on: a new career takes its first on in the hangar,
    /// before its clock starts.
    pub since: Option<SimTime>,
    goal: Goal,
    /// Where the launch sites its condition names are.
    places: BTreeMap<String, Place>,
}

impl Contract {
    /// When it lapses, if it has a deadline and its clock has started.
    pub fn deadline(&self) -> Option<SimTime> {
        let since = self.since.filter(|_| self.def.days > 0.0)?;
        Some(since.add_seconds(self.def.days * DAY))
    }

    /// Seconds left to meet it at `now`, if it has a deadline: the whole time before its clock
    /// starts.
    pub fn left(&self, now: SimTime) -> Option<f64> {
        if self.def.days <= 0.0 {
            return None;
        }
        Some(match self.deadline() {
            Some(deadline) => deadline.seconds_since(now).max(0.0),
            None => self.def.days * DAY,
        })
    }
}

/// A career's contracts: those taken on and those done.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Contracts {
    pub taken: Vec<Contract>,
    pub done: BTreeSet<String>,
    /// What the vessel being flown has done since it was first flown with contracts taken on.
    #[serde(default)]
    stats: Stats,
    #[serde(default)]
    flown: Option<VesselId>,
    #[serde(default)]
    last: Option<SimTime>,
    /// What to tell the player, once.
    #[serde(skip)]
    news: Vec<String>,
}

impl Contracts {
    /// What is offered from `defs`: up to [`OFFERED`] of those not done or taken on whose `after`
    /// are all done, in their order. [`crate::career::Career::offers`] gives them its tree's.
    pub fn offers<'a>(
        &self,
        defs: impl IntoIterator<Item = &'a ContractDef>,
    ) -> Vec<&'a ContractDef> {
        let taken = |id: &str| self.taken.iter().any(|c| c.def.id == id);
        (defs.into_iter())
            .filter(|def| !self.done.contains(&def.id) && !taken(&def.id))
            .filter(|def| def.after.iter().all(|before| self.done.contains(before)))
            .take(OFFERED)
            .collect()
    }

    /// Take on `def`, with the launch sites and bodies its condition names in `sites` and `system`:
    /// what to tell the player. Whether it is on offer is the career's to say
    /// ([`crate::career::Career::accept`]).
    pub(crate) fn take_on(
        &mut self,
        def: &ContractDef,
        sites: &[LaunchSiteDef],
        system: &SolarSystem,
    ) -> Result<String, String> {
        let mut places = BTreeMap::new();
        mission::resolve(&def.id, &def.when, &[], sites, system, None, &mut places)?;
        self.taken.push(Contract {
            def: def.clone(),
            since: None,
            goal: Goal {
                def: GoalDef {
                    id: def.id.clone(),
                    text: def.title.clone(),
                    when: def.when.clone(),
                    hold: def.hold,
                    reward: def.reward,
                },
                met_at: None,
                held: 0.0,
            },
            places,
        });
        Ok(format!("Taken on: {}", def.title))
    }

    /// Let go of the one taken on with `id`.
    pub fn drop(&mut self, id: &str) -> Option<String> {
        let at = self.taken.iter().position(|c| c.def.id == id)?;
        let contract = self.taken.remove(at);
        Some(format!("Dropped: {}", contract.def.title))
    }

    /// What has happened to them since this was last asked.
    pub fn take_news(&mut self) -> Vec<String> {
        std::mem::take(&mut self.news)
    }

    /// Look at the vessel being flown in `universe` after a tick (`physics`) or a jump on rails, with
    /// the `events` it reported: each contract met is done, and one past its deadline lapses. The
    /// funds the ones met pay.
    pub(crate) fn observe(
        &mut self,
        universe: &Universe,
        events: &[SimEvent],
        physics: bool,
    ) -> f64 {
        let Some(vessel) = universe.focus.and_then(|id| universe.vessel(id)) else {
            return 0.0;
        };
        let now = universe.time();
        if self.flown != Some(vessel.id()) {
            self.stats = Stats::default();
            self.flown = Some(vessel.id());
            self.last = Some(now);
        }
        let dt = self
            .last
            .map_or(0.0, |last| now.seconds_since(last).max(0.0));
        self.last = Some(now);
        mission::record(&mut self.stats, universe, vessel, events, physics, dt);
        let mut earned = 0.0;
        for mut contract in std::mem::take(&mut self.taken) {
            let since = *contract.since.get_or_insert(now);
            let title = contract.def.title.clone();
            if contract.deadline().is_some_and(|deadline| now > deadline) {
                self.news.push(format!("Contract lapsed: {title}"));
                continue;
            }
            let view = View {
                universe,
                vessel,
                stats: &self.stats,
                places: &contract.places,
                mission: None,
            };
            if contract.goal.advance(&view, dt, now.seconds_since(since)) {
                earned += contract.def.reward;
                self.done.insert(contract.def.id.clone());
                let reward = money(contract.def.reward);
                self.news
                    .push(format!("Contract complete: {title} (+{reward} funds)"));
                continue;
            }
            self.taken.push(contract);
        }
        earned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        career::Career, mission::tests::in_orbit, test_data::records, vessel::tests::launch_sites,
    };

    /// Reach orbit, then fly by the Moon; and land at the Cape, in a day.
    fn defs() -> Vec<ContractDef> {
        records(
            r#"[
                { id: "orbit", title: "Reach orbit", reward: 50000, days: 30,
                  when: { type: "Orbit", body: "Earth", periapsis: { min: 150000 } } },
                { id: "moon", title: "Fly by the Moon", reward: 120000, after: ["orbit"],
                  when: { type: "Visit", body: "Moon" } },
                { id: "home", title: "Land at the Cape", reward: 5000, days: 1,
                  when: { type: "All", of: [{ type: "Crew", min: 0 },
                      { type: "Landed", site: "cape_canaveral" }] } },
            ]"#,
        )
    }

    fn career() -> Career {
        Career {
            tree: "t".into(),
            science: 0.0,
            funds: 0.0,
            facilities: Default::default(),
            contracts: Contracts::default(),
            unlocked: Default::default(),
            returned: Default::default(),
        }
    }

    fn offered(universe: &Universe, defs: &[ContractDef]) -> Vec<String> {
        let career = universe.career.as_ref().unwrap();
        career.offers(defs).iter().map(|d| d.id.clone()).collect()
    }

    /// Offered, taken on, met by a vessel in orbit and paid; and then the Moon is offered, which
    /// was not before.
    #[test]
    fn a_contract_to_reach_orbit_is_taken_on_met_and_paid() {
        let defs = defs();
        let mut universe = in_orbit();
        universe.career = Some(career());
        assert_eq!(offered(&universe, &defs), ["orbit", "home"]);
        let system = universe.system.clone();
        let career = universe.career.as_mut().unwrap();
        let said = career.accept(&defs[0], &defs, &launch_sites(), &system);
        assert_eq!(said.as_deref(), Ok("Taken on: Reach orbit"));
        assert!(
            career
                .accept(&defs[1], &defs, &launch_sites(), &system)
                .is_err()
        );
        assert_eq!(offered(&universe, &defs), ["home"]);

        universe.advance_rails(universe.time().add_seconds(60.0));
        let career = universe.career.as_mut().unwrap();
        assert_eq!(career.funds, 50_000.0);
        assert!(career.contracts.done.contains("orbit"));
        assert!(career.contracts.taken.is_empty());
        assert_eq!(
            career.contracts.take_news(),
            ["Contract complete: Reach orbit (+50,000 funds)"]
        );
        assert_eq!(offered(&universe, &defs), ["moon", "home"]);
        // Paid once.
        universe.advance_rails(universe.time().add_seconds(60.0));
        assert_eq!(universe.career.as_ref().unwrap().funds, 50_000.0);
    }

    /// One not met by its deadline lapses, pays nothing and is offered again; taken on, it comes
    /// back from a save.
    #[test]
    fn a_contract_past_its_deadline_lapses() {
        let defs = defs();
        let mut universe = in_orbit();
        universe.career = Some(career());
        let system = universe.system.clone();
        let career = universe.career.as_mut().unwrap();
        career
            .accept(&defs[2], &defs, &launch_sites(), &system)
            .unwrap();
        let saved = rmp_serde::to_vec_named(&universe.career).unwrap();
        let back: Option<Career> = rmp_serde::from_slice(&saved).unwrap();
        assert_eq!(
            back.as_ref().map(|c| &c.contracts),
            universe.career.as_ref().map(|c| &c.contracts)
        );

        // Its day starts when the game first looks at it, then runs.
        let left = |u: &Universe| u.career.as_ref().unwrap().contracts.taken[0].left(u.time());
        universe.advance_rails(universe.time().add_seconds(60.0));
        assert_eq!(left(&universe), Some(86_400.0));
        universe.advance_rails(universe.time().add_seconds(60.0));
        assert!(left(&universe).is_some_and(|s| (s - 86_340.0).abs() < 1.0));
        universe.advance_rails(universe.time().add_seconds(86_400.0));
        let career = universe.career.as_mut().unwrap();
        assert!(career.contracts.taken.is_empty());
        assert_eq!(
            career.contracts.take_news(),
            ["Contract lapsed: Land at the Cape"]
        );
        assert_eq!(career.funds, 0.0);
        assert!(offered(&universe, &defs).contains(&"home".to_owned()));
    }
}
