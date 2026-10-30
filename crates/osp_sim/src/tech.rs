//! The technology tree and historical availability.
//!
//! Catalogue records (engines, pods, tank types, batteries, ...) carry the year they first
//! flew and the `tech_nodes` node that unlocks them ([`CatalogInfo`]). A record is
//! available from the start of its own year, or, without one, from its node's year; a node
//! without a year is reached with the latest of its parents. Records with neither are
//! always available. The game's date decides, so a 1957 scenario has the parts of 1957.

use std::collections::BTreeMap;

use crate::{
    component::ComponentDef,
    data::{CatalogInfo, CraftDef, PartCatalog, TechNodeDef},
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
