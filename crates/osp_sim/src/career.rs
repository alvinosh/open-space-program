//! A career: a game where parts are unlocked on the tech tree with science, kept in its save. A
//! sandbox has none, and every part. What it has unlocked is a set of the tree's nodes, the roots
//! from the start; what that lets it build is the tree's to say ([`ScienceTree`]).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    builder::Allowed,
    component::ComponentDef,
    data::{CraftDef, PartCatalog},
    palette::{self, Entry},
    tech::ScienceTree,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Career {
    /// The `tech_trees` record it is played on.
    pub tree: String,
    /// What it has to spend on the tree.
    pub science: f64,
    /// The nodes it has unlocked.
    pub unlocked: BTreeSet<String>,
}

impl Career {
    /// A new career on `tree`, with its roots and no science.
    pub fn new(tree: &ScienceTree) -> Self {
        Self {
            tree: tree.def.id.clone(),
            science: 0.0,
            unlocked: tree.roots(),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        data::{TechNodeDef, TechTreeDef},
        test_data::records,
        vessel::tests::catalog,
    };

    /// A two-node tree over the test catalogue: tanks up to 2 m wide from the start, the A-10 and
    /// 4 m for 5 science.
    fn tree(catalog: &PartCatalog) -> ScienceTree {
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
