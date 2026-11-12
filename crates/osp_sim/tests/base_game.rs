//! The base game with no mods on: one mix of liquid fuel and oxidiser, and solid fuel, and every
//! stock craft still built from them. The real propellants are the Realistic Fuels mod's.

use std::{collections::BTreeSet, path::Path};

use glam::DVec3;

use osp_sim::{
    builder, comms,
    data::{
        CraftDef, CraftPartDef, EngineDef, GroundStationDef, RcsDef, ResourceDef, SystemDef,
        TankTypeDef,
    },
    orbit::{Orbit, StateVector},
    palette,
    propulsion::Propulsion,
    sizing::ProceduralDef,
    system::SolarSystem,
    systems::{ANTENNA_OFF, ANTENNA_ON, Antenna, Avionics, Battery},
    time::SimTime,
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
fn there_is_only_liquid_fuel_oxidiser_and_solid_fuel_and_supplies() {
    let content = base_game();
    let mut problems = Vec::new();
    let resources: BTreeSet<String> = (content.registry)
        .all::<ResourceDef>("resources", &mut problems)
        .into_iter()
        .map(|r| r.id)
        .collect();
    // The three propellants, and what people live on.
    assert_eq!(
        resources,
        [FUEL, OXIDISER, SOLID, "Supplies"]
            .map(str::to_owned)
            .into(),
    );

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
fn a_probe_built_from_the_parts_list_has_power_and_a_radio() {
    let content = base_game();
    let catalog = record::catalog(&content.registry);
    let entries = palette::entries(&catalog);
    let part = |entry: &str, id: &str, on: Option<&str>| -> CraftPartDef {
        let mut part = (entries.iter().find(|e| e.id == entry))
            .unwrap_or_else(|| panic!("no `{entry}` in the list"))
            .part
            .clone();
        part.id = id.to_owned();
        part.on = on.map(str::to_owned);
        part.by = "bottom".to_owned();
        part
    };
    let craft = CraftDef {
        id: "probe".into(),
        name: String::new(),
        parts: vec![
            part("probe_core", "core", None),
            part("battery", "battery", Some("core.top")),
            part("dish", "dish", Some("battery.top")),
        ],
        sequences: None,
        triggers: Vec::new(),
        rcs_shut: Vec::new(),
        abort: Vec::new(),
    };
    let probe = Propulsion::from_craft(&craft, &catalog).unwrap();
    let (_, battery) = probe.systems.all::<Battery>().next().expect("a battery");
    assert!(battery.charge > 0.0);
    let (_, dish) = probe.systems.all::<Antenna>().next().expect("a radio");
    assert!(dish.radio.gain > 1.0, "a dish, not a whip");
    // Good for low orbit to a 26 m station with room to spare, and for the Moon too.
    let station = comms::dish_gain(26.0, comms::S_BAND);
    assert!(comms::margin(&dish.radio, station, 50.0, 2e6) > 20.0);
    assert!(comms::margin(&dish.radio, station, 50.0, 3.84e8) > 0.0);
    // In a real orbit, against the real ground stations, a station comes into sight within the
    // day, and the link is there when it does.
    let mut problems = Vec::new();
    let system = SolarSystem::from_def(&SystemDef {
        bodies: content.registry.all("bodies", &mut problems),
    })
    .unwrap();
    let defs = content
        .registry
        .all::<GroundStationDef>("ground_stations", &mut problems);
    let stations: Vec<_> = (defs.iter())
        .filter_map(|def| comms::Station::new(def, &system))
        .collect();
    assert!(stations.len() >= 4 && problems.is_empty(), "{problems:?}");
    let earth = system.find("Earth").unwrap();
    let body = system.body(earth);
    let radius = body.radius + 200_000.0;
    let (lat, speed) = (28.6_f64.to_radians(), (body.gm / radius).sqrt());
    let orbit = Orbit::new(
        body.gm,
        SimTime::J2000,
        StateVector {
            position: DVec3::X * radius,
            velocity: DVec3::new(0.0, lat.cos(), lat.sin()) * speed,
        },
    );
    let after = SimTime::J2000;
    let change = comms::next_link_change(
        &system,
        &stations,
        earth,
        &orbit,
        &dish.radio,
        after,
        after.add_seconds(86_400.0),
    )
    .expect("a station comes round within the day");
    let linked_now = comms::best_link(
        &system,
        &stations,
        earth,
        orbit.state_at(after).position,
        &dish.radio,
        after,
    )
    .is_some();
    let soon = change.add_seconds(5.0);
    let linked_then = comms::best_link(
        &system,
        &stations,
        earth,
        orbit.state_at(soon).position,
        &dish.radio,
        soon,
    )
    .is_some();
    assert_ne!(linked_now, linked_then, "the link changes when it says");
    // Something to command it, and the rule that a pod will do.
    assert!(probe.systems.all::<Avionics>().next().is_some());
    // Every one of them can stand on its own, and what needs a mod is not offered without it.
    for entry in entries
        .iter()
        .filter(|e| e.group == palette::Group::Systems)
    {
        builder::alone(&entry.part, &catalog).unwrap_or_else(|e| panic!("{}: {e}", entry.id));
    }
    assert!(
        entries
            .iter()
            .all(|e| e.id != "fuel_cell" && e.id != "avionics")
    );
    // A bigger dish is drawn bigger.
    let mut big = part("dish", "dish", None);
    if let Some(ProceduralDef::Antenna(antenna)) = &mut big.procedural {
        antenna.diameter = 2.0;
    }
    let scale = |part: &CraftPartDef| {
        let craft = builder::alone(part, &catalog).unwrap();
        let drawn = craft.parts[0]
            .built
            .geometry
            .models()
            .next()
            .map(|(m, ..)| m.scale);
        drawn.expect("a model")
    };
    assert_eq!(scale(&part("dish", "dish", None)), 1.0);
    assert_eq!(scale(&big), 2.0);
}

#[test]
fn antennas_differ_in_reach_and_draw_and_relays_are_marked() {
    let content = base_game();
    let catalog = record::catalog(&content.registry);
    let entries = palette::entries(&catalog);
    let antenna = |entry: &str| -> Antenna {
        let mut part = (entries.iter().find(|e| e.id == entry))
            .unwrap()
            .part
            .clone();
        part.id = "x".to_owned();
        let craft = CraftDef {
            id: "x".into(),
            name: String::new(),
            parts: vec![part],
            sequences: None,
            triggers: Vec::new(),
            rcs_shut: Vec::new(),
            abort: Vec::new(),
        };
        let p = Propulsion::from_craft(&craft, &catalog).unwrap();
        let (_, antenna) = p.systems.all::<Antenna>().next().unwrap();
        antenna.clone()
    };
    let station = comms::dish_gain(26.0, comms::S_BAND);
    let reach = |a: &Antenna| comms::reach(&a.radio, station, 50.0);
    let order = ["antenna", "small_dish", "dish", "large_dish"].map(antenna);
    for pair in order.windows(2) {
        assert!(reach(&pair[1]) > reach(&pair[0]), "bigger reaches further");
        assert!(pair[1].watts() > pair[0].watts(), "and draws more");
        assert!(!pair[1].relay);
    }
    assert!(antenna("relay_dish").relay && antenna("deep_relay").relay);
}

#[test]
fn a_shut_down_antenna_gives_no_radio_and_draws_nothing() {
    let content = base_game();
    let catalog = record::catalog(&content.registry);
    let entries = palette::entries(&catalog);
    let mut part = (entries.iter().find(|e| e.id == "dish"))
        .unwrap()
        .part
        .clone();
    part.id = "x".to_owned();
    let craft = CraftDef {
        id: "x".into(),
        name: String::new(),
        parts: vec![part],
        sequences: None,
        triggers: Vec::new(),
        rcs_shut: Vec::new(),
        abort: Vec::new(),
    };
    let mut p = Propulsion::from_craft(&craft, &catalog).unwrap();
    let (id, before) = p
        .systems
        .all::<Antenna>()
        .next()
        .map(|(id, a)| (id, a.on))
        .unwrap();
    assert!(before);
    assert!(p.systems.act(id, ANTENNA_OFF).is_some());
    let (_, antenna) = p.systems.all::<Antenna>().next().unwrap();
    assert!(antenna.shut && !antenna.on);
    assert!(p.systems.act(id, ANTENNA_OFF).is_none(), "already off");
    assert!(p.systems.act(id, ANTENNA_ON).is_some());
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

/// The base game's life support is the casual one, and the Realistic game mode turns on the real
/// one. The setting comes from the mods that are on, so a save keeps it with its mods.
#[test]
fn life_support_comes_from_the_rules_of_the_mods_on() {
    let casual = record::catalog(&base_game().registry);
    assert_eq!(casual.rules.life_support.as_deref(), Some("casual"));

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mods = root.join("mods");
    let mode: serde_json::Value = (base_game().registry)
        .get("game_modes", "realistic")
        .unwrap();
    let ids: Vec<String> = serde_json::from_value(mode["mods"].clone()).unwrap();
    let choices = osp_mods::exactly(&osp_mods::installed(&mods), &ids);
    let content = osp_mods::load_chosen(&root.join("assets/core"), Some(&mods), &choices);
    let realistic = record::catalog(&content.registry);
    let needs = (realistic.life_support_needs().unwrap()).expect("Realistic has life support");
    let what: Vec<(&str, f64, Option<f64>)> = (needs.iter())
        .map(|need| (need.resource.as_str(), need.daily, need.lasts))
        .collect();
    assert_eq!(
        what,
        [
            ("CabinOxygen", 0.84, Some(300.0)),
            ("Water", 3.0, Some(3.0 * 86_400.0)),
            ("Food", 0.6, Some(30.0 * 86_400.0))
        ]
    );
    // People breathe the cabin's air, which is topped up from liquid oxygen and nitrogen, and
    // every gas in it and what tops it up is a resource.
    let profile = (realistic.life_support_profile().unwrap()).unwrap();
    assert_eq!(profile.supply_of("CabinOxygen"), "LqdOxygen");
    let air = profile.air.as_ref().expect("Realistic cabins have air");
    let gases = (air.gases.iter()).flat_map(|gas| [Some(&gas.resource), gas.from.as_ref()]);
    let named = needs.iter().map(|need| Some(&need.resource));
    for resource in gases.chain(named).flatten() {
        let known = realistic.resources.iter().any(|r| r.id == *resource);
        assert!(known, "{resource} is not a resource");
    }
}

/// In the casual game a crew needs only the one supply, which a capsule carries in its lockers:
/// 14 days of it for its one person.
#[test]
fn in_casual_a_crew_needs_only_supplies() {
    let catalog = record::catalog(&base_game().registry);
    let needs = (catalog.life_support_needs().unwrap()).expect("Casual has life support");
    let what: Vec<&str> = needs.iter().map(|n| n.resource.as_str()).collect();
    assert_eq!(what, ["Supplies"]);
    let craft: CraftDef = json5::from_str(
        r#"{ id: "x", name: "", parts: [{ id: "pod", template: "C-1 Capsule" }] }"#,
    )
    .unwrap();
    let mut p = Propulsion::from_craft(&craft, &catalog).unwrap();
    let pod = p.parts().parts()[0].id;
    assert!((p.held_for(pod, "Supplies") - 14.0).abs() < 1e-9);
    let environment = osp_sim::environment::Environment::DARK;
    p.step_systems(&environment, 10.0 * 86_400.0, false);
    assert!((p.held_for(pod, "Supplies") - 4.0).abs() < 1e-6);
    assert_eq!(p.systems.crew_aboard(), 1);
    // Nothing else is used: no air, no water, no scrubber.
    assert_eq!(p.systems.all::<osp_sim::systems::Process>().count(), 0);
}
