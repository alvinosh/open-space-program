//! A career: a game where parts are unlocked on the tech tree with science, kept in its save. A
//! sandbox has none, and every part. What it has unlocked is a set of the tree's nodes, the roots
//! from the start; what that lets it build is the tree's to say ([`ScienceTree`]). It has funds
//! too, from the tree's starting sum and the rewards of the goals and contracts it meets
//! ([`crate::mission::GoalDef::reward`], [`crate::contract`]), for what is paid with money.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    builder::Allowed,
    component::ComponentDef,
    contract::{ContractDef, Contracts},
    data::{CraftDef, FacilityDef, PartCatalog},
    palette::{self, Entry},
    science::{self, Returned},
    tech::ScienceTree,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Career {
    /// The `tech_trees` record it is played on.
    pub tree: String,
    /// What it has to spend on the tree.
    pub science: f64,
    /// Money, for what is paid with it.
    #[serde(default)]
    pub funds: f64,
    /// The nodes it has unlocked.
    pub unlocked: BTreeSet<String>,
    /// The level of each facility it has upgraded, by id; the others are at their first.
    #[serde(default)]
    pub facilities: BTreeMap<String, usize>,
    /// The contracts it has taken on and done.
    #[serde(default)]
    pub contracts: Contracts,
    /// What experiments' data and samples have reached the ground, brought home or (T274) sent: the
    /// share of a whole set of each subject ([`crate::science::subject`]), more than one if it came
    /// back more than once.
    #[serde(default)]
    pub returned: BTreeMap<String, f64>,
}

impl Career {
    /// A new career on `tree`, with its roots and no science.
    pub fn new(tree: &ScienceTree) -> Self {
        Self {
            tree: tree.def.id.clone(),
            science: 0.0,
            funds: tree.def.funds,
            facilities: BTreeMap::new(),
            contracts: Contracts::default(),
            unlocked: tree.roots(),
            returned: BTreeMap::new(),
        }
    }

    /// Take in `amount` of money.
    pub fn earn(&mut self, amount: f64) {
        if amount.is_finite() && amount > 0.0 {
            self.funds += amount;
        }
    }

    /// Pay `amount` out of its funds, if it has that much.
    pub fn pay(&mut self, amount: f64) -> Result<(), String> {
        if amount > self.funds + 1e-9 {
            return Err(format!(
                "that costs {}, and there is {}",
                money(amount),
                money(self.funds)
            ));
        }
        self.funds = (self.funds - amount).max(0.0);
        Ok(())
    }

    /// The contracts of `defs` it is offered now: its tree's ([`crate::contract`]).
    pub fn offers<'a>(&self, defs: &'a [ContractDef]) -> Vec<&'a ContractDef> {
        let ours = |def: &&ContractDef| def.tree.as_ref().is_none_or(|tree| *tree == self.tree);
        self.contracts.offers(defs.iter().filter(ours))
    }

    /// Take on the contract `def`, if it is offered from `defs`, with the launch sites and bodies its
    /// condition names in `sites` and `system`: what to tell the player.
    pub fn accept(
        &mut self,
        def: &ContractDef,
        defs: &[ContractDef],
        sites: &[crate::data::LaunchSiteDef],
        system: &crate::system::SolarSystem,
    ) -> Result<String, String> {
        if !self.offers(defs).iter().any(|offered| offered.id == def.id) {
            return Err(format!("{} is not on offer", def.title));
        }
        self.contracts.take_on(def, sites, system)
    }

    /// The level `facility` is at, from 0.
    pub fn level(&self, facility: &FacilityDef) -> usize {
        let at = self.facilities.get(&facility.id).copied().unwrap_or(0);
        at.min(facility.levels.len().saturating_sub(1))
    }

    /// Upgrade `facility` to its next level, paying its price: what to tell the player.
    pub fn upgrade(&mut self, facility: &FacilityDef) -> Result<String, String> {
        let next = self.level(facility) + 1;
        let Some(level) = facility.levels.get(next) else {
            return Err(format!("{} is as big as it gets", facility.title()));
        };
        self.pay(level.price)?;
        self.facilities.insert(facility.id.clone(), next);
        Ok(format!(
            "{} upgraded to level {} of {}",
            facility.title(),
            next + 1,
            facility.levels.len()
        ))
    }

    /// Why `facilities` at the levels it has cannot launch `craft`, from the pad if `from_pad` (the
    /// launch pad's mass limit holds only there): each limit it is over, by how much. Nothing if it
    /// can.
    pub fn refuses(
        &self,
        facilities: &[FacilityDef],
        craft: &CraftSize,
        from_pad: bool,
    ) -> Vec<String> {
        let mut why = Vec::new();
        for facility in facilities {
            let Some(level) = facility.levels.get(self.level(facility)) else {
                continue;
            };
            let name = facility.title();
            let over =
                |what: &str, value: f64, limit: Option<f64>, unit: &dyn Fn(f64) -> String| {
                    let limit = limit.filter(|limit| value > *limit + 1e-6)?;
                    Some(format!(
                        "{name}: {what} {}, {} over its {}",
                        unit(value),
                        unit(value - limit),
                        unit(limit)
                    ))
                };
            let tonnes = |kg: f64| format!("{:.1} t", kg / 1000.0);
            let metres = |m: f64| format!("{m:.1} m");
            let count = |n: f64| format!("{n:.0}");
            let mass = level.mass.filter(|_| from_pad);
            why.extend(over("liftoff mass", craft.mass, mass, &tonnes));
            why.extend(over("height", craft.height, level.height, &metres));
            why.extend(over("width", craft.width, level.width, &metres));
            let parts = level.parts.map(f64::from);
            why.extend(over("parts", craft.parts as f64, parts, &count));
        }
        why
    }

    /// Count what has reached the ground of each subject, and earn the science it is worth, less
    /// for what has come home of it before ([`science::earned`]): how much.
    pub fn receive(&mut self, returned: &BTreeMap<String, Returned>) -> f64 {
        let mut earned = 0.0;
        for (subject, now) in returned {
            let before = self.returned.entry(subject.clone()).or_insert(0.0);
            earned += science::earned(now.worth, *before, now.sets);
            *before += now.sets;
        }
        self.science += earned;
        earned
    }

    /// Unlock `node`, paying for it with science.
    pub fn unlock(&mut self, node: &str, tree: &ScienceTree) -> Result<(), String> {
        let Some(def) = tree.node(node) else {
            return Err(format!("the tree has no node `{node}`"));
        };
        if self.unlocked.contains(node) {
            return Err(format!("{} is unlocked already", def.title));
        }
        if !tree.can_unlock(node, &self.unlocked) {
            return Err(format!("{} needs what it builds on first", def.title));
        }
        let cost = tree.cost(node);
        if cost > self.science + 1e-9 {
            return Err(format!(
                "{} costs {cost:.0} science, and there is {:.0}",
                def.title, self.science
            ));
        }
        self.science = (self.science - cost).max(0.0);
        self.unlocked.insert(node.to_owned());
        Ok(())
    }

    /// Whether the parts-list entry `entry` can be built.
    pub fn has_entry(&self, tree: &ScienceTree, entry: &str) -> bool {
        tree.has_entry(entry, &self.unlocked)
    }

    /// How big it may build procedural parts, and with which tank types.
    pub fn allowed(&self, tree: &ScienceTree) -> Allowed {
        tree.allowed(&self.unlocked)
    }

    /// The parts of `craft` it cannot build yet, by name: a part all of whose possible entries are
    /// locked, or an engine in a configuration that is; and, with why after the name, a part bigger
    /// than it may build or a tank of a type still locked. A part made by hand, like no entry, is
    /// not held against it for that.
    pub fn locked_parts(
        &self,
        tree: &ScienceTree,
        catalog: &PartCatalog,
        entries: &[Entry],
        craft: &CraftDef,
    ) -> Vec<String> {
        let mut locked = Vec::new();
        let allowed = self.allowed(tree);
        let engine_locked = |id: &str| {
            (catalog.engines.iter().find(|e| e.id == id))
                .is_some_and(|e| !tree.has_record(&e.catalog, &self.unlocked))
        };
        for part in &craft.parts {
            let sources = palette::sources(part, entries, catalog);
            let entry_locked =
                !sources.is_empty() && sources.iter().all(|e| !self.has_entry(tree, &e.id));
            let engines = (part.template.iter())
                .chain(part.components.iter().filter_map(|c| match c {
                    ComponentDef::Engine(r) => Some(&r.id),
                    _ => None,
                }))
                .any(|id| engine_locked(id));
            let name = (part.name.clone())
                .or_else(|| part.template.clone())
                .unwrap_or_else(|| part.id.clone());
            let name = match allowed.refuses(part) {
                _ if entry_locked || engines => name,
                Some(why) => format!("{name} ({why})"),
                None => continue,
            };
            if !locked.contains(&name) {
                locked.push(name);
            }
        }
        locked
    }
}

/// What the facilities limit of a craft as it stands to launch.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CraftSize {
    /// With what is in its tanks, kg.
    pub mass: f64,
    /// Along its stack (a plane's length), m.
    pub height: f64,
    /// Across, the widest way, m.
    pub width: f64,
    pub parts: usize,
}

/// An amount of money as the player reads it: whole, with commas between the thousands.
pub fn money(amount: f64) -> String {
    let whole = amount.round().max(0.0) as u64;
    let digits = whole.to_string();
    let mut out = String::new();
    for (k, c) in digits.chars().enumerate() {
        if k > 0 && (digits.len() - k) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        data::{TechNodeDef, TechTreeDef},
        test_data::records,
        vessel::tests::catalog,
    };

    /// A two-node tree over the test catalogue: tanks up to 2 m wide from the start, the A-10 and
    /// 4 m for 5 science.
    pub(crate) fn tree(catalog: &PartCatalog) -> ScienceTree {
        let def: TechTreeDef = records(r#"{ id: "t", roots: ["start"] }"#).remove(0);
        let nodes: Vec<TechNodeDef> = records(
            r#"[
                { id: "start", title: "Start", parts: ["tank", "pod:C-1 Capsule"],
                  sizes: { diameter: 2 } },
                { id: "rocketry", title: "Rocketry", cost: 5, parents: ["start"],
                  parts: ["engine:A-10"], sizes: { diameter: 4 } }
            ]"#,
        );
        ScienceTree::new(&def, &nodes, catalog)
    }

    fn craft(text: &str) -> CraftDef {
        records(text).remove(0)
    }

    #[test]
    fn a_new_career_has_its_roots_and_unlocks_with_science() {
        let catalog = catalog();
        let tree = tree(&catalog);
        let mut career = Career::new(&tree);
        assert!(career.has_entry(&tree, "tank"));
        assert!(!career.has_entry(&tree, "engine:A-10"));
        let error = career.unlock("rocketry", &tree).unwrap_err();
        assert!(error.contains("costs 5 science"), "{error}");
        career.science = 7.0;
        career.unlock("rocketry", &tree).unwrap();
        assert!(career.has_entry(&tree, "engine:A-10"));
        assert_eq!(career.science, 2.0);
        assert!(career.unlock("rocketry", &tree).is_err(), "only once");
    }

    /// A career starts with its tree's funds, takes in rewards and pays only what it has.
    #[test]
    fn a_career_spends_only_the_funds_it_has() {
        let catalog = catalog();
        let mut tree = tree(&catalog);
        tree.def.funds = 50_000.0;
        let mut career = Career::new(&tree);
        assert_eq!(career.funds, 50_000.0);
        career.earn(2_500.0);
        career.earn(-100.0);
        assert_eq!(career.funds, 52_500.0);
        let short = career.pay(60_000.0).unwrap_err();
        assert_eq!(short, "that costs 60,000, and there is 52,500");
        career.pay(52_500.0).unwrap();
        assert_eq!(career.funds, 0.0);
        assert_eq!(money(1_234_567.4), "1,234,567");
        assert_eq!(money(999.0), "999");
    }

    /// A pad of 30 t and then 180 t, and a builder of 30 m, 6 m and 40 parts and then 50 m, 10 m and
    /// 100 parts.
    fn facilities() -> Vec<FacilityDef> {
        records(
            r#"[
                { id: "launch_pad", name: "Launch pad", levels: [
                    { mass: 30000 }, { price: 60000, mass: 180000 } ] },
                { id: "builder", name: "Builder", levels: [
                    { height: 30, width: 6, parts: 40 },
                    { price: 40000, height: 50, width: 10, parts: 100 } ] }
            ]"#,
        )
    }

    /// Over the pad's mass and the builder's size, a launch is refused with how much over each it
    /// is; the mass counts only from the pad.
    #[test]
    fn a_craft_over_the_facilities_is_refused_saying_by_how_much() {
        let catalog = catalog();
        let career = Career::new(&tree(&catalog));
        let facilities = facilities();
        let big = CraftSize {
            mass: 32_400.0,
            height: 31.5,
            width: 5.0,
            parts: 45,
        };
        assert_eq!(
            career.refuses(&facilities, &big, true),
            [
                "Launch pad: liftoff mass 32.4 t, 2.4 t over its 30.0 t",
                "Builder: height 31.5 m, 1.5 m over its 30.0 m",
                "Builder: parts 45, 5 over its 40",
            ]
        );
        assert_eq!(career.refuses(&facilities, &big, false).len(), 2);
        let small = CraftSize {
            mass: 20_000.0,
            height: 20.0,
            width: 3.0,
            parts: 12,
        };
        assert!(career.refuses(&facilities, &small, true).is_empty());
    }

    /// An upgrade costs its level's price and raises the limits; there is no level past the last,
    /// and the levels come back from a save.
    #[test]
    fn an_upgrade_costs_its_price_and_raises_the_limits() {
        let catalog = catalog();
        let mut career = Career::new(&tree(&catalog));
        let facilities = facilities();
        let (pad, builder) = (&facilities[0], &facilities[1]);
        let big = CraftSize {
            mass: 100_000.0,
            height: 20.0,
            width: 3.0,
            parts: 12,
        };
        career.funds = 50_000.0;
        let short = career.upgrade(pad).unwrap_err();
        assert_eq!(short, "that costs 60,000, and there is 50,000");
        assert_eq!(career.level(pad), 0);
        career.earn(20_000.0);
        assert_eq!(
            career.upgrade(pad).unwrap(),
            "Launch pad upgraded to level 2 of 2"
        );
        assert_eq!(career.funds, 10_000.0);
        assert!(career.refuses(&facilities, &big, true).is_empty());
        assert!(
            career
                .upgrade(pad)
                .unwrap_err()
                .contains("as big as it gets")
        );
        assert_eq!(career.level(builder), 0);
        let saved = rmp_serde::to_vec_named(&career).unwrap();
        let back: Career = rmp_serde::from_slice(&saved).unwrap();
        assert_eq!(back.level(pad), 1);
        assert_eq!(back, career);
    }

    #[test]
    fn a_craft_with_a_locked_part_says_which() {
        let catalog = catalog();
        let tree = tree(&catalog);
        let entries = palette::entries(&catalog);
        let mut career = Career::new(&tree);
        let rocket = craft(
            r#"{ id: "r", name: "", parts: [
                { id: "pod", template: "C-1 Capsule" },
                { id: "engine", on: "pod.bottom", by: "top", name: "Kick motor", template: "A-10" }
            ] }"#,
        );
        let locked = career.locked_parts(&tree, &catalog, &entries, &rocket);
        assert_eq!(locked, ["Kick motor"]);
        career.science = 5.0;
        career.unlock("rocketry", &tree).unwrap();
        assert!(
            career
                .locked_parts(&tree, &catalog, &entries, &rocket)
                .is_empty()
        );
    }

    #[test]
    fn a_tank_wider_than_the_tree_allows_says_so_until_it_is_unlocked() {
        let catalog = catalog();
        let tree = tree(&catalog);
        // The small tree leaves most parts out, but its sizes are limited from its root.
        assert!(!tree.problems().iter().any(|p| p.contains("limit")));
        let entries = palette::entries(&catalog);
        let mut career = Career::new(&tree);
        assert_eq!(career.allowed(&tree).sizes.diameter, 2.0);
        // Tank types say nothing of this tree: every one may be built.
        assert_eq!(career.allowed(&tree).tank_types, None);
        let tank = |diameter: f64| {
            craft(&format!(
                r#"{{ id: "r", name: "", parts: [
                    {{ id: "tank", name: "Tank", shape: {{ cylinder_tank: {{ diameter: {diameter}, length: 9 }} }},
                      components: [{{ storage: {{ type: "Stiffened Al 2219", contents: {{ LqdOxygen: 1 }} }} }}] }}
                ] }}"#
            ))
        };
        assert!(
            career
                .locked_parts(&tree, &catalog, &entries, &tank(2.0))
                .is_empty()
        );
        let locked = career.locked_parts(&tree, &catalog, &entries, &tank(3.0));
        assert_eq!(locked, ["Tank (diameter 3 m, 2 m so far)"]);
        career.science = 5.0;
        career.unlock("rocketry", &tree).unwrap();
        assert_eq!(career.allowed(&tree).sizes.diameter, 4.0);
        assert!(
            career
                .locked_parts(&tree, &catalog, &entries, &tank(3.0))
                .is_empty()
        );
    }
}
