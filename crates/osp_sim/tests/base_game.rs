//! The base game with no mods on: one mix of liquid fuel and oxidiser, and solid fuel, and every
//! stock craft still built from them. The real propellants are the Realistic Fuels mod's.

use std::{collections::BTreeSet, path::Path};

use osp_sim::{
    data::{CraftDef, EngineDef, RcsDef, ResourceDef, TankTypeDef},
    propulsion::Propulsion,
};

#[path = "golden/record.rs"]
#[allow(dead_code)]
mod record;

const FUEL: &str = "Liquid Fuel";
const OXIDISER: &str = "Oxidiser";
const SOLID: &str = "Solid Fuel";
/// Oxidiser for each kilogram of fuel: the same in every engine, so any tank suits any engine.
const RATIO: f64 = 2.36;

fn base_game() -> osp_mods::Content {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    osp_mods::load(&root.join("assets/core"), None)
}

fn the_mix(propellants: &std::collections::BTreeMap<String, f64>, what: &str) {
    assert_eq!(propellants.len(), 2, "{what}: {propellants:?}");
    let share = |name: &str| {
        *propellants
            .get(name)
            .unwrap_or_else(|| panic!("{what}: no {name}"))
    };
    assert!(
        (share(OXIDISER) / share(FUEL) - RATIO).abs() < 1e-9,
        "{what}: {propellants:?}"
    );
}

#[test]
fn there_is_only_liquid_fuel_oxidiser_and_solid_fuel() {
    let content = base_game();
    let mut problems = Vec::new();
    let resources: BTreeSet<String> = (content.registry)
        .all::<ResourceDef>("resources", &mut problems)
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(resources, [FUEL, OXIDISER, SOLID].map(str::to_owned).into(),);

    let engines = content.registry.all::<EngineDef>("engines", &mut problems);
    assert!(engines.len() > 10);
    for engine in &engines {
        if engine.propellants.contains_key(SOLID) {
            assert_eq!(engine.propellants.len(), 1, "{}", engine.id);
        } else {
            the_mix(&engine.propellants, &engine.id);
        }
    }
    for rcs in content.registry.all::<RcsDef>("rcs", &mut problems) {
        the_mix(&rcs.propellants, &rcs.id);
    }
    for kind in content
        .registry
        .all::<TankTypeDef>("tank_types", &mut problems)
    {
        for held in kind.propellants.iter().flatten() {
            assert!(resources.contains(held), "{}: {held}", kind.id);
        }
    }
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn any_pod_flies_a_craft_in_the_base_game() {
    let content = base_game();
    assert!(!record::catalog(&content.registry).rules.avionics);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let all = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    assert!(
        record::catalog(&all.registry).rules.avionics,
        "Realistic Avionics turns it on"
    );
}

#[test]
fn every_stock_craft_is_built_from_them() {
    let content = base_game();
    let catalog = record::catalog(&content.registry);
    let mut problems = Vec::new();
    let craft = content.registry.all::<CraftDef>("craft", &mut problems);
    assert!(craft.len() > 10 && problems.is_empty(), "{problems:?}");
    for def in &craft {
        let rocket =
            Propulsion::from_craft(def, &catalog).unwrap_or_else(|e| panic!("{}: {e}", def.id));
        for content in rocket.tanks.iter().flat_map(|tank| &tank.contents) {
            let held = content.resource.as_str();
            assert!(
                [FUEL, OXIDISER, SOLID].contains(&held),
                "{}: {held}",
                def.id
            );
        }
    }
}
