//! The technology tree and historical availability.
//!
//! Catalogue records (engines, pods, tank types, batteries, ...) carry the year they first
//! flew and the `tech_nodes` node that unlocks them ([`CatalogInfo`]). A record is
//! available from the start of its own year, or, without one, from its node's year; a node
//! without a year is reached with the latest of its parents. Records with neither are
//! always available. The game's date decides, so a 1957 scenario has the parts of 1957.
//!
//! A science game unlocks parts on one tree instead ([`ScienceTree`]), the one the rules name.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    builder::Allowed,
    component::ComponentDef,
    data::{
        CatalogInfo, CraftDef, PartCatalog, SizeLimitsDef, TechNodeDef, TechTreeDef, TreeLayout,
    },
    shaping::Limits,
    time::SimTime,
};

#[derive(Debug, Clone, Default)]
pub struct TechTree {
    nodes: BTreeMap<String, TechNodeDef>,
}

/// A part a craft uses before it existed.
#[derive(Debug, Clone, PartialEq)]
pub struct Anachronism {
    /// `engine`, `pod` or `tank type`.
    pub kind: &'static str,
    pub part: String,
    /// When it becomes available.
    pub year: i32,
}

impl TechTree {
    pub fn new(nodes: impl IntoIterator<Item = TechNodeDef>) -> Self {
        Self {
            nodes: nodes.into_iter().map(|n| (n.id.clone(), n)).collect(),
        }
    }

    pub fn node(&self, id: &str) -> Option<&TechNodeDef> {
        self.nodes.get(id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &TechNodeDef> {
        self.nodes.values()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Year a node is reached: its own, else the latest of its parents'.
    pub fn node_year(&self, id: &str) -> Option<i32> {
        self.node_year_at(id, 0)
    }

    fn node_year_at(&self, id: &str, depth: usize) -> Option<i32> {
        let node = self.nodes.get(id)?;
        if node.year.is_some() || depth > 64 {
            return node.year;
        }
        node.parents
            .iter()
            .filter_map(|p| self.node_year_at(p, depth + 1))
            .max()
    }

    /// Year a record becomes available, if it has one.
    pub fn year_of(&self, info: &CatalogInfo) -> Option<i32> {
        info.year
            .or_else(|| info.tech.as_deref().and_then(|t| self.node_year(t)))
    }

    pub fn is_available(&self, info: &CatalogInfo, year: i32) -> bool {
        self.year_of(info).is_none_or(|y| y <= year)
    }

    /// Nodes reached by `year`, oldest first.
    pub fn reached(&self, year: i32) -> Vec<&TechNodeDef> {
        let mut reached: Vec<_> = self
            .nodes
            .values()
            .filter(|n| self.node_year(&n.id).is_some_and(|y| y <= year))
            .collect();
        reached.sort_by_key(|n| (self.node_year(&n.id), n.id.clone()));
        reached
    }

    /// The parts of `craft` that were not available yet in `year`. Parts the catalogue
    /// doesn't know are left to the craft loader to report.
    pub fn anachronisms(
        &self,
        catalog: &PartCatalog,
        craft: &CraftDef,
        year: i32,
    ) -> Vec<Anachronism> {
        let mut found: Vec<Anachronism> = Vec::new();
        let mut check = |kind: &'static str, part: &str, info: &CatalogInfo| {
            if let Some(y) = self.year_of(info).filter(|&y| y > year)
                && !found.iter().any(|a| a.part == part && a.kind == kind)
            {
                found.push(Anachronism {
                    kind,
                    part: part.to_owned(),
                    year: y,
                });
            }
        };
        let engine = |id: &str| catalog.engines.iter().find(|e| e.id == id);
        for part in &craft.parts {
            if let Some(id) = &part.template {
                if let Some(e) = engine(id) {
                    check("engine", id, &e.catalog);
                }
                if let Some(p) = catalog.pods.iter().find(|p| &p.id == id) {
                    check("pod", id, &p.catalog);
                }
            }
            for component in &part.components {
                match component {
                    ComponentDef::Engine(r) => {
                        if let Some(e) = engine(&r.id) {
                            check("engine", &r.id, &e.catalog);
                        }
                    }
                    ComponentDef::Storage(storage) => {
                        let id = &storage.tank_type;
                        if let Some(t) = catalog.tank_types.iter().find(|t| &t.id == id) {
                            check("tank type", id, &t.catalog);
                        }
                    }
                    _ => {}
                }
            }
        }
        found
    }
}

/// A tree a science game is played on (a `tech_trees` record): the nodes reached from its roots,
/// the parts-list entries each unlocks, and which of them are shown. A node unlocks the entries it
/// lists (`parts`) and those whose records' `tech` names it (an engine with configurations in
/// several nodes, by the cheapest, a tank type), and may let procedural parts be made bigger
/// (`sizes`). A node that does none of that for the game's parts is not shown and is passed
/// through: what builds on it builds on what it built on. A node costs at least what it builds on
/// ([`Self::cost`]), so the tree never gets cheaper as it goes.
#[derive(Debug, Clone)]
pub struct ScienceTree {
    pub def: TechTreeDef,
    /// Every node reached from the roots.
    nodes: BTreeMap<String, TechNodeDef>,
    /// The entries each node unlocks.
    unlocks: BTreeMap<String, Vec<String>>,
    /// The node that unlocks each entry.
    node_of: BTreeMap<String, String>,
    /// Nodes shown: the roots and those that unlock something.
    shown: BTreeSet<String>,
    /// Each shown node's nearest shown ancestors.
    builds_on: BTreeMap<String, Vec<String>>,
    /// What unlocking each shown node costs ([`Self::cost`]).
    costs: BTreeMap<String, f64>,
    /// Each tank type with the node that unlocks it, if the tree has one, the most advanced first:
    /// the dearest node, then the newest.
    tank_types: Vec<(String, Option<String>)>,
    /// What else each node unlocks, as a player reads it: engine configurations of parts it does
    /// not unlock itself, tank types, batteries, avionics.
    records: BTreeMap<String, Vec<String>>,
    /// What is wrong with it as content: entries in no node or in two, roots that are no node.
    problems: Vec<String>,
}

/// The records a parts-list entry is built from that can name a node: an engine's configurations,
/// a pod.
fn records_of<'a>(entry: &str, catalog: &'a PartCatalog) -> Vec<&'a CatalogInfo> {
    if let Some(id) = entry.strip_prefix("engine:") {
        return (catalog.configurations(id).into_iter())
            .map(|e| &e.catalog)
            .collect();
    }
    if let Some(id) = entry.strip_prefix("pod:") {
        return (catalog.pods.iter())
            .filter(|p| p.id == id)
            .map(|p| &p.catalog)
            .collect();
    }
    Vec::new()
}

impl ScienceTree {
    /// The tree `def` over `nodes`, for the parts of `catalog`.
    pub fn new(def: &TechTreeDef, nodes: &[TechNodeDef], catalog: &PartCatalog) -> Self {
        let mut problems = Vec::new();
        let by_id: BTreeMap<&str, &TechNodeDef> =
            nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for node in nodes {
            for parent in &node.parents {
                children.entry(parent.as_str()).or_default().push(&node.id);
            }
        }
        // Everything reached from the roots.
        let mut members: BTreeMap<String, TechNodeDef> = BTreeMap::new();
        let mut todo: Vec<&str> = Vec::new();
        for root in &def.roots {
            if by_id.contains_key(root.as_str()) {
                todo.push(root);
            } else {
                problems.push(format!("the root `{root}` is no node"));
            }
        }
        while let Some(id) = todo.pop() {
            if members.contains_key(id) {
                continue;
            }
            members.insert(id.to_owned(), by_id[id].clone());
            todo.extend(children.get(id).into_iter().flatten());
        }
        fn member<'a>(
            members: &BTreeMap<String, TechNodeDef>,
            tech: &'a Option<String>,
        ) -> Option<&'a String> {
            tech.as_ref().filter(|t| members.contains_key(*t))
        }
        // The node of each entry: the one that lists it, or the cheapest its records name.
        let mut unlocks: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut node_of = BTreeMap::new();
        for entry in crate::palette::entries(catalog) {
            let listing: Vec<&String> = (members.values())
                .filter(|n| n.parts.contains(&entry.id))
                .map(|n| &n.id)
                .collect();
            if listing.len() > 1 {
                problems.push(format!(
                    "`{}` is in more than one node: {listing:?}",
                    entry.id
                ));
            }
            let by_record = (records_of(&entry.id, catalog).into_iter())
                .filter_map(|info| member(&members, &info.tech))
                .min_by(|a, b| members[*a].cost.total_cmp(&members[*b].cost));
            match listing.first().copied().or(by_record) {
                Some(node) => {
                    unlocks
                        .entry(node.clone())
                        .or_default()
                        .push(entry.id.clone());
                    node_of.insert(entry.id, node.clone());
                }
                None => problems.push(format!("`{}` is in no node", entry.id)),
            }
        }
        let known: BTreeSet<String> = (crate::palette::entries(catalog).into_iter())
            .map(|e| e.id)
            .collect();
        for node in members.values() {
            for part in node.parts.iter().filter(|p| !known.contains(*p)) {
                problems.push(format!("`{}` lists `{part}`, which is no part", node.id));
            }
        }
        // Shown: the roots, the nodes of entries, and those any record names (an engine's other
        // configurations, tank types, batteries, avionics).
        let infos = (catalog.engines.iter().map(|e| &e.catalog))
            .chain(catalog.tank_types.iter().map(|t| &t.catalog))
            .chain(catalog.pods.iter().map(|p| &p.catalog))
            .chain(catalog.batteries.iter().map(|b| &b.catalog))
            .chain(catalog.avionics.iter().map(|a| &a.catalog));
        let mut shown: BTreeSet<String> = (def.roots.iter())
            .filter(|r| members.contains_key(*r))
            .cloned()
            .collect();
        shown.extend(unlocks.keys().cloned());
        shown.extend(
            infos
                .filter_map(|info| member(&members, &info.tech))
                .cloned(),
        );
        shown.extend(
            (members.values())
                .filter(|n| n.sizes.is_some())
                .map(|n| n.id.clone()),
        );
        // Sizes are limited from the start, if they are limited at all.
        for (what, limits) in [
            (
                "diameter",
                (|s: &SizeLimitsDef| s.diameter) as fn(&SizeLimitsDef) -> _,
            ),
            ("length", |s: &SizeLimitsDef| s.length),
        ] {
            let given = |n: &TechNodeDef| n.sizes.as_ref().and_then(limits).is_some();
            if members.values().any(given)
                && !(def.roots.iter()).any(|r| members.get(r).is_some_and(given))
            {
                problems.push(format!(
                    "nodes limit the {what} of procedural parts, but no root says what it is to start"
                ));
            }
        }
        // What each shown node builds on, through the nodes passed over.
        let mut builds_on = BTreeMap::new();
        for id in &shown {
            let mut found: BTreeSet<String> = BTreeSet::new();
            let mut up: Vec<&String> = members[id].parents.iter().collect();
            let mut seen = BTreeSet::new();
            while let Some(parent) = up.pop() {
                if !seen.insert(parent.clone()) || !members.contains_key(parent) {
                    continue;
                }
                if shown.contains(parent) {
                    found.insert(parent.clone());
                } else {
                    up.extend(members[parent].parents.iter());
                }
            }
            builds_on.insert(id.clone(), found.into_iter().collect::<Vec<_>>());
        }
        // Leave out what another of its parents already builds on: the line goes through it.
        let ancestors = |id: &str, builds_on: &BTreeMap<String, Vec<String>>| {
            let mut all = BTreeSet::new();
            let mut up: Vec<String> = builds_on.get(id).cloned().unwrap_or_default();
            while let Some(p) = up.pop() {
                if all.insert(p.clone()) {
                    up.extend(builds_on.get(&p).cloned().unwrap_or_default());
                }
            }
            all
        };
        let reduced: BTreeMap<String, Vec<String>> = (builds_on.iter())
            .map(|(id, parents)| {
                let keep = (parents.iter())
                    .filter(|p| {
                        !parents
                            .iter()
                            .any(|q| q != *p && ancestors(q, &builds_on).contains(*p))
                    })
                    .cloned()
                    .collect();
                (id.clone(), keep)
            })
            .collect();
        // What each costs: its own, or the dearest it builds on; nothing for a root.
        let mut costs: BTreeMap<String, f64> = BTreeMap::new();
        fn cost_of(
            id: &str,
            def: &TechTreeDef,
            members: &BTreeMap<String, TechNodeDef>,
            builds_on: &BTreeMap<String, Vec<String>>,
            costs: &mut BTreeMap<String, f64>,
        ) -> f64 {
            if let Some(cost) = costs.get(id) {
                return *cost;
            }
            let cost = if def.roots.iter().any(|r| r == id) {
                0.0
            } else {
                let own = members.get(id).map_or(0.0, |n| n.cost);
                (builds_on.get(id).into_iter().flatten())
                    .map(|p| cost_of(p, def, members, builds_on, costs))
                    .fold(own, f64::max)
            };
            costs.insert(id.to_owned(), cost);
            cost
        }
        for id in &shown {
            cost_of(id, def, &members, &reduced, &mut costs);
        }
        let mut tank_types: Vec<(usize, &crate::data::TankTypeDef, Option<String>)> =
            (catalog.tank_types.iter().enumerate())
                .map(|(at, t)| (at, t, member(&members, &t.catalog.tech).cloned()))
                .collect();
        let cost = |node: &Option<String>| node.as_ref().map_or(0.0, |n| costs[n]);
        tank_types.sort_by(|(i, a, an), (j, b, bn)| {
            (cost(bn).total_cmp(&cost(an)))
                .then(b.catalog.year.cmp(&a.catalog.year))
                .then(i.cmp(j))
        });
        let tank_types = (tank_types.into_iter())
            .map(|(_, t, node)| (t.id.clone(), node))
            .collect();
        let mut records: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut add = |info: &CatalogInfo, label: String| {
            if let Some(node) = member(&members, &info.tech) {
                records.entry(node.clone()).or_default().push(label);
            }
        };
        for engine in &catalog.engines {
            let entry = format!("engine:{}", engine.id);
            let own = (engine.catalog.tech.as_ref())
                .is_some_and(|node| unlocks.get(node).is_some_and(|u| u.contains(&entry)));
            if !own {
                add(&engine.catalog, format!("{} configuration", engine.id));
            }
        }
        for kind in &catalog.tank_types {
            add(&kind.catalog, format!("{} tanks", kind.id));
        }
        for battery in &catalog.batteries {
            add(&battery.catalog, format!("{} batteries", battery.id));
        }
        for avionics in &catalog.avionics {
            add(&avionics.catalog, format!("{} avionics", avionics.id));
        }
        Self {
            def: def.clone(),
            nodes: members,
            unlocks,
            node_of,
            shown,
            builds_on: reduced,
            costs,
            tank_types,
            records,
            problems,
        }
    }

    /// What else `id` unlocks besides its parts-list entries ([`Self::unlocks`]): engine
    /// configurations, tank types, batteries, avionics, as a player reads them.
    pub fn records(&self, id: &str) -> &[String] {
        self.records.get(id).map_or(&[], Vec::as_slice)
    }

    /// What a game that has unlocked `unlocked` may build: procedural parts as big as the biggest
    /// those nodes allow (the smallest in the tree, if none of them says), and the tank types they
    /// unlock, the most advanced first. Anything, where the tree says nothing of sizes or of tank
    /// types.
    pub fn allowed(&self, unlocked: &BTreeSet<String>) -> Allowed {
        let limit = |get: fn(&SizeLimitsDef) -> Option<f64>| {
            let given: Vec<(bool, f64)> = (self.nodes.values())
                .filter(|n| self.shown.contains(&n.id))
                .filter_map(|n| Some((unlocked.contains(&n.id), get(n.sizes.as_ref()?)?)))
                .collect();
            let reached = (given.iter()).filter(|(have, _)| *have).map(|(_, v)| *v);
            let least = given.iter().map(|(_, v)| *v).fold(f64::INFINITY, f64::min);
            reached.reduce(f64::max).unwrap_or(least)
        };
        let named = self.tank_types.iter().any(|(_, node)| node.is_some());
        Allowed {
            sizes: Limits {
                diameter: limit(|s| s.diameter),
                length: limit(|s| s.length),
            },
            tank_types: named.then(|| {
                (self.tank_types.iter())
                    .filter(|(_, node)| node.as_ref().is_none_or(|n| unlocked.contains(n)))
                    .map(|(id, _)| id.clone())
                    .collect()
            }),
        }
    }

    /// What is wrong with it as content.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// The nodes shown, the cheapest first.
    pub fn shown(&self) -> Vec<&TechNodeDef> {
        let mut shown: Vec<&TechNodeDef> = self.shown.iter().map(|id| &self.nodes[id]).collect();
        shown.sort_by(|a, b| (self.cost(&a.id).total_cmp(&self.cost(&b.id))).then(a.id.cmp(&b.id)));
        shown
    }

    /// What unlocking `id` costs, science: its own cost, or that of the dearest node it builds on
    /// if that is more. Nothing for a root.
    pub fn cost(&self, id: &str) -> f64 {
        self.costs.get(id).copied().unwrap_or(0.0)
    }

    pub fn node(&self, id: &str) -> Option<&TechNodeDef> {
        self.nodes.get(id).filter(|_| self.shown.contains(id))
    }

    /// The shown nodes `id` builds on.
    pub fn builds_on(&self, id: &str) -> &[String] {
        self.builds_on.get(id).map_or(&[], Vec::as_slice)
    }

    /// The shown nodes that build on `id`.
    pub fn leads_to(&self, id: &str) -> Vec<&TechNodeDef> {
        (self.shown().into_iter())
            .filter(|n| self.builds_on(&n.id).iter().any(|p| p == id))
            .collect()
    }

    /// What heads each column of [`Self::layout`], by column: the year, in a tree laid out by
    /// years; nothing in one laid out by positions.
    pub fn column_labels(&self) -> Vec<(f64, String)> {
        if self.def.layout != TreeLayout::Years {
            return Vec::new();
        }
        let years: BTreeSet<i32> = self.years().into_values().collect();
        (years.into_iter().enumerate())
            .map(|(column, year)| (column as f64, year.to_string()))
            .collect()
    }

    /// The parts-list entries `id` unlocks.
    pub fn unlocks(&self, id: &str) -> &[String] {
        self.unlocks.get(id).map_or(&[], Vec::as_slice)
    }

    /// The node that unlocks the parts-list entry `entry`, if the tree has it.
    pub fn node_of(&self, entry: &str) -> Option<&str> {
        self.node_of.get(entry).map(String::as_str)
    }

    /// Where each shown node goes on a screen, [column, row], by the tree's [`TreeLayout`]. In
    /// years, a node with no year has that of the latest it builds on, and one with no category is
    /// in a band of its own at the top; nodes of the same year and category stack.
    pub fn layout(&self) -> BTreeMap<String, [f64; 2]> {
        match self.def.layout {
            TreeLayout::Positions => (self.shown.iter())
                .map(|id| (id.clone(), self.nodes[id].position.unwrap_or_default()))
                .collect(),
            TreeLayout::Years => self.by_years(),
        }
    }

    /// The year each shown node is drawn at: its own, but no earlier than what it builds on, and
    /// that one's with none of its own. Cheapest first, so parents come before children.
    fn years(&self) -> BTreeMap<String, i32> {
        let mut years: BTreeMap<String, i32> = BTreeMap::new();
        for node in self.shown() {
            let inherited = (self.builds_on(&node.id).iter())
                .filter_map(|p| years.get(p).copied())
                .max();
            let year = node.year.or(inherited).unwrap_or_default();
            years.insert(node.id.clone(), year.max(inherited.unwrap_or(i32::MIN)));
        }
        years
    }

    fn by_years(&self) -> BTreeMap<String, [f64; 2]> {
        let years = self.years();
        let columns: BTreeSet<i32> = years.values().copied().collect();
        let column = |year: i32| columns.iter().position(|y| *y == year).unwrap_or(0) as f64;
        // Bands in the order each category first appears, with what has none on top.
        let mut bands: Vec<Option<String>> = vec![None];
        let mut by_year: Vec<&TechNodeDef> = self.shown();
        by_year.sort_by_key(|n| (years[&n.id], n.id.clone()));
        for node in &by_year {
            if !bands.contains(&node.category) {
                bands.push(node.category.clone());
            }
        }
        // Each band as deep as its deepest stack.
        let mut stacks: BTreeMap<(Option<String>, i32), usize> = BTreeMap::new();
        let mut place: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for node in &by_year {
            let band = bands.iter().position(|b| *b == node.category).unwrap_or(0);
            let depth = stacks
                .entry((node.category.clone(), years[&node.id]))
                .or_default();
            place.insert(node.id.clone(), (band, *depth));
            *depth += 1;
        }
        let mut deepest = vec![1_usize; bands.len()];
        for ((category, _), depth) in &stacks {
            let band = bands.iter().position(|b| b == category).unwrap_or(0);
            deepest[band] = deepest[band].max(*depth);
        }
        let mut top = vec![0.0; bands.len()];
        for band in 1..bands.len() {
            top[band] = top[band - 1] + deepest[band - 1] as f64;
        }
        (place.into_iter())
            .map(|(id, (band, depth))| {
                let at = [column(years[&id]), top[band] + depth as f64];
                (id, at)
            })
            .collect()
    }

    /// What a science game starts with.
    pub fn roots(&self) -> BTreeSet<String> {
        (self.def.roots.iter())
            .filter(|r| self.shown.contains(*r))
            .cloned()
            .collect()
    }

    /// Whether `id` can be unlocked next: it is not yet, and what it builds on is (any one of it,
    /// for a node that needs only one).
    pub fn can_unlock(&self, id: &str, unlocked: &BTreeSet<String>) -> bool {
        let Some(node) = self.node(id) else {
            return false;
        };
        let parents = self.builds_on(id);
        let have = |p: &String| unlocked.contains(p);
        !unlocked.contains(id)
            && (parents.is_empty()
                || if node.any_parent {
                    parents.iter().any(have)
                } else {
                    parents.iter().all(have)
                })
    }

    /// The nodes that can be unlocked next, the cheapest first.
    pub fn ready(&self, unlocked: &BTreeSet<String>) -> Vec<&TechNodeDef> {
        (self.shown().into_iter())
            .filter(|n| self.can_unlock(&n.id, unlocked))
            .collect()
    }

    /// Whether the parts-list entry `entry` can be used with `unlocked`: its node is, or the tree
    /// does not have it.
    pub fn has_entry(&self, entry: &str, unlocked: &BTreeSet<String>) -> bool {
        self.node_of(entry)
            .is_none_or(|node| unlocked.contains(node))
    }

    /// Whether a record (an engine configuration, a tank type) can be used with `unlocked`: its
    /// node is, or it names none of the tree's.
    pub fn has_record(&self, info: &CatalogInfo, unlocked: &BTreeSet<String>) -> bool {
        (info.tech.as_ref())
            .filter(|t| self.nodes.contains_key(*t))
            .is_none_or(|t| unlocked.contains(t))
    }
}

/// The tree the rules of `catalog` name, over `nodes` and `trees`: none if they name none, an error
/// if they name one nobody wrote.
pub fn science_tree(
    catalog: &PartCatalog,
    trees: &[TechTreeDef],
    nodes: &[TechNodeDef],
) -> Result<Option<ScienceTree>, String> {
    let Some(id) = &catalog.rules.tech_tree else {
        return Ok(None);
    };
    let def = (trees.iter().find(|t| t.id == *id))
        .ok_or_else(|| format!("the rules' tech tree `{id}` has no record in `tech_trees`"))?;
    Ok(Some(ScienceTree::new(def, nodes, catalog)))
}

/// Calendar date (year, month, day, UT) of a time in seconds since J2000 (2000-01-01
/// 12:00). Ignores leap seconds, like the rest of the game's clock.
pub fn civil_date(t: SimTime) -> (i32, u32, u32) {
    let since_midnight = t.as_seconds() + 43_200.0;
    let days = since_midnight.div_euclid(86_400.0) as i64 + 10_957; // days since 1970-01-01
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

pub fn calendar_year(t: SimTime) -> i32 {
    civil_date(t).0
}

/// Seconds since J2000 of midnight UT on a calendar date (Hinnant's days_from_civil).
pub fn j2000_seconds(year: i32, month: u32, day: u32) -> f64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_1970 = era * 146_097 + doe - 719_468;
    (days_since_1970 - 10_957) as f64 * 86_400.0 - 43_200.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, year: Option<i32>, parents: &[&str]) -> TechNodeDef {
        TechNodeDef {
            id: id.into(),
            title: String::new(),
            description: String::new(),
            year,
            year_end: None,
            category: None,
            cost: 0.0,
            parents: parents.iter().map(|p| p.to_string()).collect(),
            any_parent: false,
            position: None,
            parts: Vec::new(),
            sizes: None,
        }
    }

    #[test]
    fn dates_round_trip() {
        for (y, m, d) in [
            (1957, 10, 4),
            (1969, 7, 16),
            (2000, 1, 1),
            (1942, 10, 3),
            (2026, 2, 28),
        ] {
            let t = SimTime::from_seconds(j2000_seconds(y, m, d) + 3_600.0);
            assert_eq!(civil_date(t), (y, m, d));
        }
        assert_eq!(j2000_seconds(2000, 1, 1), -43_200.0);
    }

    #[test]
    fn records_unlock_with_their_year_or_their_node() {
        let tree = TechTree::new([
            node("start", Some(1942), &[]),
            node("orbital1958", Some(1958), &["start"]),
            node("undated", None, &["orbital1958"]),
        ]);
        let own_year = CatalogInfo {
            year: Some(1961),
            tech: Some("orbital1958".into()),
            ..Default::default()
        };
        assert!(!tree.is_available(&own_year, 1960));
        assert!(tree.is_available(&own_year, 1961));
        let by_node = CatalogInfo {
            tech: Some("undated".into()),
            ..Default::default()
        };
        assert_eq!(tree.year_of(&by_node), Some(1958));
        assert!(tree.is_available(&CatalogInfo::default(), 1900));
        assert_eq!(tree.reached(1950).len(), 1);
    }
}
