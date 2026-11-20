//! The tech trees a science game is played on, as the game loads them: every part in its mode's
//! parts list is in one node, the tree hangs together, and it costs more the further it goes.

use std::{collections::BTreeSet, path::Path};

use osp_sim::{
    data::{PartCatalog, TechNodeDef, TechTreeDef},
    palette,
    tech::{ScienceTree, science_tree},
};

#[path = "golden/record.rs"]
#[allow(dead_code)]
mod record;

/// The content of a mode: the base game alone (Casual), or with the Realistic game mode's mods.
fn mode(realistic: bool) -> osp_mods::Content {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let core = root.join("assets/core");
    if !realistic {
        return osp_mods::load(&core, None);
    }
    let mods = root.join("mods");
    let base = osp_mods::load(&core, None);
    let mode: serde_json::Value = base.registry.get("game_modes", "realistic").unwrap();
    let ids: Vec<String> = serde_json::from_value(mode["mods"].clone()).unwrap();
    let choices = osp_mods::exactly(&osp_mods::installed(&mods), &ids);
    osp_mods::load_chosen(&core, Some(&mods), &choices)
}

fn tree(realistic: bool) -> (ScienceTree, PartCatalog) {
    let content = mode(realistic);
    let catalog = record::catalog(&content.registry);
    let mut problems = Vec::new();
    let trees: Vec<TechTreeDef> = content.registry.all("tech_trees", &mut problems);
    let nodes: Vec<TechNodeDef> = content.registry.all("tech_nodes", &mut problems);
    assert!(problems.is_empty(), "{problems:?}");
    let tree = science_tree(&catalog, &trees, &nodes)
        .unwrap()
        .expect("a tree");
    (tree, catalog)
}

/// What is wrong with a tree: its own problems, and a node that costs less than one it builds on,
/// by the costs a player is charged or, `as_written`, by its records.
fn check(tree: &ScienceTree, as_written: bool) -> Vec<String> {
    let mut problems = tree.problems().to_vec();
    let cost = |id: &str| {
        if as_written {
            tree.node(id).unwrap().cost
        } else {
            tree.cost(id)
        }
    };
    for node in tree.shown() {
        for parent in tree.builds_on(&node.id) {
            if cost(&node.id) < cost(parent) {
                problems.push(format!(
                    "`{}` ({}) costs less than `{parent}` ({}), which it builds on",
                    node.id,
                    cost(&node.id),
                    cost(parent)
                ));
            }
        }
    }
    problems
}

#[test]
fn the_casual_tree_has_every_casual_part_once_and_rises() {
    let (tree, catalog) = tree(false);
    assert_eq!(tree.def.id, "casual");
    // Written to rise, tier by tier.
    assert_eq!(check(&tree, true), Vec::<String>::new());
    let entries = palette::entries(&catalog);
    assert_eq!(entries.len(), 48);
    assert!(entries.iter().all(|e| tree.node_of(&e.id).is_some()));
    // Seven tiers, a first rocket in the first.
    let start = tree.roots();
    assert_eq!(start, BTreeSet::from(["first_steps".to_owned()]));
    // A sounding rocket's instruments too, and what brings their readings home.
    for part in [
        "parachute",
        "telemetry",
        "pod:C-1 Capsule",
        "tank",
        "engine:K-0",
        "decoupler",
        "instrument:thermometer",
        "instrument:barometer",
    ] {
        assert!(tree.has_entry(part, &start), "{part}");
    }
    assert!(!tree.has_entry("engine:M-2", &start));
}

#[test]
fn the_realistic_tree_has_every_realistic_part_once() {
    let (tree, catalog) = tree(true);
    assert_eq!(tree.def.id, "rp1");
    // RP-1 prices some nodes below what they build on; the game charges them more.
    let problems = check(&tree, false);
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
    let entries = palette::entries(&catalog);
    assert!(entries.iter().all(|e| tree.node_of(&e.id).is_some()));
    // A first sounding rocket can bring its readings home.
    let start = tree.roots();
    for part in ["parachute", "telemetry", "instrument:thermometer"] {
        assert!(tree.has_entry(part, &start), "{part}");
    }
}

/// Unlocking a node makes its parts available and opens what builds on it, and only that.
#[test]
fn unlocking_a_node_opens_its_parts_and_what_builds_on_it() {
    let (tree, _) = tree(false);
    let mut unlocked = tree.roots();
    let ready: Vec<&str> = tree
        .ready(&unlocked)
        .iter()
        .map(|n| n.id.as_str())
        .collect();
    assert!(ready.contains(&"basic_rocketry") && ready.contains(&"electrics"));
    assert!(!ready.contains(&"general_rocketry"));
    assert!(!tree.has_entry("engine:A-10", &unlocked));
    unlocked.insert("basic_rocketry".into());
    assert!(tree.has_entry("engine:A-10", &unlocked));
    assert!(tree.can_unlock("general_rocketry", &unlocked));
    // Needing two, one is not enough.
    assert!(!tree.can_unlock("uncrewed_craft", &unlocked));
    unlocked.insert("flight_control".into());
    assert!(!tree.can_unlock("uncrewed_craft", &unlocked));
    unlocked.insert("electrics".into());
    assert!(tree.can_unlock("uncrewed_craft", &unlocked));
}

/// Tanks are as wide as the tree allows: 2 m in a new Casual career, more up the rocketry line;
/// Casual's tank types are not locked.
#[test]
fn casual_tanks_widen_up_the_rocketry_line() {
    let (tree, _) = tree(false);
    let mut unlocked = tree.roots();
    let allowed = tree.allowed(&unlocked);
    assert_eq!(allowed.sizes.diameter, 2.0);
    assert_eq!(allowed.sizes.length, f64::INFINITY);
    assert_eq!(allowed.tank_types, None);
    // A pod is no procedural part: the C-1 is 1.9 m, the C-4 3.7 m, whatever the tanks.
    for (node, wide) in [
        ("basic_rocketry", 2.0),
        ("general_rocketry", 3.0),
        ("advanced_rocketry", 4.0),
        ("heavy_rocketry", 6.0),
        ("very_heavy_rocketry", 10.0),
    ] {
        unlocked.insert(node.into());
        assert_eq!(tree.allowed(&unlocked).sizes.diameter, wide, "{node}");
    }
}

/// A new Realistic career builds V-2 sized tanks of the early types only; the materials nodes widen
/// them and unlock the later types, the base game's among them.
#[test]
fn realistic_tanks_widen_and_get_better_with_materials() {
    let (tree, catalog) = tree(true);
    let mut unlocked = tree.roots();
    let allowed = tree.allowed(&unlocked);
    assert_eq!(allowed.sizes.diameter, 1.7);
    let types = allowed.tank_types.clone().expect("tank types are locked");
    for early in [
        "Steel Fuselage",
        "Al Gridded Tank",
        "Balloon Tank",
        "Solid Grain",
    ] {
        assert!(types.iter().any(|t| t == early), "{early} in {types:?}");
    }
    for later in [
        "Stiffened Al 2219",
        "Isogrid Al-Li",
        "Insulated Al-Li",
        "COPV",
        "Al-Li Stringer Tank",
    ] {
        assert!(!allowed.tank_type(later), "{later} is locked at the start");
    }
    // Every tank type but a motor's grain is in a node of the tree.
    for kind in catalog.tank_types.iter().filter(|t| !t.cast) {
        let node = kind.catalog.tech.as_deref();
        assert!(
            node.is_some_and(|n| tree.node(n).is_some()),
            "{} is in no node of the tree ({node:?})",
            kind.id
        );
    }
    // A new tank from the parts list is brought within: 1.7 m, of the newest early type.
    let mut tank = (palette::entries(&catalog).into_iter())
        .find(|e| e.id == "tank")
        .unwrap()
        .part;
    osp_sim::fitting::fit(&mut tank, &osp_sim::fitting::Against::node(3.0), &catalog);
    allowed.bring_within(&mut tank, &catalog);
    assert_eq!(allowed.refuses(&tank), None);
    for node in [
        "postWarMaterialsScience",
        "earlyMaterialsScience",
        "materialsScienceSatellite",
        "materialsScienceHuman",
    ] {
        unlocked.insert(node.into());
    }
    let allowed = tree.allowed(&unlocked);
    assert_eq!(allowed.sizes.diameter, 5.6);
    assert!(allowed.tank_type("Stiffened Al 2219") && allowed.tank_type("Insulated Al-Li"));
    assert!(!allowed.tank_type("Isogrid Al-Li"));
}

/// What a node leads to is what builds on it; a tree laid out by years heads every column with its
/// year, oldest on the left.
#[test]
fn nodes_say_what_they_lead_to_and_year_columns_have_their_years() {
    let (casual, _) = tree(false);
    let next: Vec<&str> = (casual.leads_to("first_steps").into_iter())
        .map(|n| n.id.as_str())
        .collect();
    for id in ["basic_rocketry", "aerodynamics", "electrics"] {
        assert!(next.contains(&id), "{id} in {next:?}");
    }
    assert!(!next.contains(&"general_rocketry"));
    assert!(casual.column_labels().is_empty());
    let (realistic, _) = tree(true);
    let labels = realistic.column_labels();
    let years: Vec<i32> = labels.iter().map(|(_, y)| y.parse().unwrap()).collect();
    assert!(years.windows(2).all(|w| w[0] < w[1]), "{years:?}");
    let layout = realistic.layout();
    for (id, [column, _]) in &layout {
        assert!(
            labels.iter().any(|(c, _)| c == column),
            "{id} is in column {column}, which has no year"
        );
    }
    assert_eq!(labels.len() as f64 - 1.0, labels.last().unwrap().0);
}

/// Each mode's facilities: a launch pad and a builder whose levels cost more and allow more, the
/// first pad taking the mode's first rockets and not the next ones (liftoff masses from
/// `golden/craft.json`: the Sounder 27.2 t, the Talon 137.9 t; the V-2 12.8 t, the Mercury-Redstone
/// 28.9 t).
#[test]
fn facilities_grow_level_by_level_and_start_with_the_first_rockets() {
    use osp_sim::data::FacilityDef;
    for (realistic, fits, too_big) in [(false, 27.2e3, 137.9e3), (true, 12.8e3, 28.9e3)] {
        let content = mode(realistic);
        let mut problems = Vec::new();
        let facilities: Vec<FacilityDef> = content.registry.all("facilities", &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
        let ids: Vec<&str> = facilities.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["launch_pad", "builder"], "{realistic}");
        for facility in &facilities {
            assert!(facility.levels.len() >= 3, "{}", facility.id);
            assert_eq!(facility.levels[0].price, 0.0);
            for pair in facility.levels.windows(2) {
                let (a, b) = (&pair[0], &pair[1]);
                assert!(b.price > a.price, "{} {realistic}", facility.id);
                let grows = |x: Option<f64>, y: Option<f64>| match (x, y) {
                    (Some(x), Some(y)) => y > x,
                    (Some(_), None) => true,
                    (None, None) => true,
                    (None, Some(_)) => false,
                };
                assert!(
                    grows(a.mass, b.mass) && grows(a.height, b.height),
                    "{}",
                    facility.id
                );
                assert!(grows(a.width, b.width), "{}", facility.id);
                assert!(grows(a.parts.map(f64::from), b.parts.map(f64::from)));
            }
        }
        let pad = facilities.iter().find(|f| f.id == "launch_pad").unwrap();
        let first = pad.levels[0].mass.unwrap();
        assert!(fits <= first && first < too_big, "{realistic}: {first}");
    }
}

/// Each mode's contracts: a new career is offered only what needs nothing done first, what is
/// offered grows as contracts are done until every one of its tree's has been offered, and each
/// can be taken on (its condition names bodies and sites there are).
#[test]
fn contracts_are_offered_in_turn_until_all_have_been() {
    use osp_sim::{
        career::Career,
        contract::ContractDef,
        data::{BodyDef, LaunchSiteDef, SystemDef},
        system::SolarSystem,
    };
    for realistic in [false, true] {
        let content = mode(realistic);
        let registry = &content.registry;
        let mut problems = Vec::new();
        let defs: Vec<ContractDef> = registry.all("contracts", &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
        let system = SolarSystem::from_def(&SystemDef {
            bodies: registry.all::<BodyDef>("bodies", &mut problems),
        })
        .unwrap();
        let sites: Vec<LaunchSiteDef> = registry.all("launch_sites", &mut problems);
        let (tree, _) = tree(realistic);
        let mut career = Career::new(&tree);
        let ours: Vec<&ContractDef> = (defs.iter())
            .filter(|d| d.tree.as_deref() == Some(tree.def.id.as_str()))
            .collect();
        assert!(ours.len() >= 10, "{realistic}: {}", ours.len());
        let first = career.offers(&defs);
        assert!(!first.is_empty() && first.iter().all(|d| d.after.is_empty()));
        let mut seen = std::collections::BTreeSet::new();
        while let Some(def) = career.offers(&defs).first().map(|d| (*d).clone()) {
            assert_eq!(def.tree.as_deref(), Some(tree.def.id.as_str()));
            seen.insert(def.id.clone());
            career.accept(&def, &defs, &sites, &system).unwrap();
            career.contracts.taken.clear();
            career.contracts.done.insert(def.id.clone());
        }
        assert_eq!(
            seen.len(),
            ours.len(),
            "{realistic}: every contract is offered in turn"
        );
    }
}
