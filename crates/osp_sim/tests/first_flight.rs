//! A career's first flight brings its science home and earns a first node: in Casual a capsule
//! that comes back on its parachute with what its instruments read, in Realistic a sounding rocket
//! whose telemetry sends it. `FIRST_FLIGHT_PRINT=1` prints the flights and what they earned.

use std::path::Path;

use osp_mods::Content;
use osp_sim::{
    career::Career,
    component::ComponentDef,
    data::{
        BodyDef, CraftDef, GroundStationDef, LaunchSiteDef, ScenarioDef, SystemDef, TechNodeDef,
        TechTreeDef,
    },
    system::SolarSystem,
    tech::{ScienceTree, science_tree},
    time::SimTime,
    universe::Universe,
    vessel::{Vessel, VesselEventKind},
};

#[path = "golden/record.rs"]
#[allow(dead_code)]
mod record;

/// The base game alone (Casual), or with every mod (Realistic).
fn content(realistic: bool) -> Content {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mods = root.join("mods");
    osp_mods::load(
        &root.join("assets/core"),
        realistic.then_some(mods.as_path()),
    )
}

/// The rules' tech tree, and a career on it.
fn career(content: &Content) -> (ScienceTree, Career) {
    let registry = &content.registry;
    let catalog = record::catalog(registry);
    let trees: Vec<TechTreeDef> = registry.all("tech_trees", &mut Vec::new());
    let nodes: Vec<TechNodeDef> = registry.all("tech_nodes", &mut Vec::new());
    let tree = science_tree(&catalog, &trees, &nodes).unwrap().unwrap();
    let career = Career::new(&tree);
    (tree, career)
}

fn components(text: &str) -> Vec<ComponentDef> {
    json5::from_str(text).unwrap()
}

/// The cheapest node the career can unlock next.
fn cheapest(tree: &ScienceTree, career: &Career) -> f64 {
    (tree.ready(&career.unlocked).iter())
        .map(|node| tree.cost(&node.id))
        .fold(f64::INFINITY, f64::min)
}

/// Casual: the suborbital hop's capsule with a thermometer and a barometer, and the parts list's
/// parachute on its nose. Lit, it climbs out of the air; at the top the capsule lets go of the
/// booster and arms its parachute, the last of its default sequences. It comes down softly, is
/// recovered, and what it read earns more than the cheapest node costs.
#[test]
fn a_casual_first_capsule_comes_home_on_its_parachute_and_earns_a_node() {
    let content = content(false);
    let registry = &content.registry;
    let catalog = record::catalog(registry);
    let (tree, career) = career(&content);
    let system = SystemDef {
        bodies: registry.all::<BodyDef>("bodies", &mut Vec::new()),
    };
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
    let scenario: ScenarioDef = registry.get("scenarios", "sounder").unwrap();
    let craft_of = |id: &str| -> Result<CraftDef, String> {
        let mut craft: CraftDef = registry.get("craft", id).map_err(|e| format!("{e:?}"))?;
        let capsule = craft.parts.iter_mut().find(|p| p.id == "pod").unwrap();
        capsule.components.extend(components(
            r#"[{ experiment: { experiment: "temperature" } },
                { experiment: { experiment: "pressure" } }]"#,
        ));
        let chute = osp_sim::palette::entries(&catalog)
            .into_iter()
            .find(|e| e.id == "parachute")
            .unwrap();
        let mut chute = chute.part;
        chute.id = "chute".into();
        chute.on = Some("pod.top".into());
        chute.by = "bottom".into();
        craft.parts.push(chute);
        Ok(craft)
    };
    let mut universe =
        Universe::new_game(&system, Vec::new(), &sites, &catalog, craft_of, &scenario).unwrap();
    universe.set_stations(&registry.all::<GroundStationDef>("ground_stations", &mut Vec::new()));
    universe.data_rates = catalog.rules.data_rates;
    universe.career = Some(career);
    let id = universe.focus.unwrap();
    let index = universe.find_vessel(id).unwrap();
    universe.vessels[index].set_throttle(1.0);
    universe.activate_next_stage(index);

    let mut landed = None;
    let mut apogee: f64 = 0.0;
    let mut staged = false;
    for _ in 0..(1800 * 50) {
        let events = universe.step_physics();
        let index = universe.find_vessel(id).expect("the capsule flies on");
        for event in events.iter().filter(|e| e.vessel == id) {
            match event.event.kind {
                VesselEventKind::Landed { speed } => landed = Some(speed),
                VesselEventKind::Crashed { speed } => panic!("crashed at {speed:.0} m/s"),
                _ => {}
            }
        }
        let time = universe.time();
        let vessel = &universe.vessels[index];
        let state = vessel.state_at(&universe.system, time);
        let body = universe.system.body(vessel.parent());
        apogee = apogee.max(state.position.length() - body.radius);
        let falling = state.velocity.dot(state.position.normalize()) < 0.0;
        let tick = (time.seconds_since(SimTime::J2000) * 50.0).round() as i64;
        if std::env::var_os("FIRST_FLIGHT_PRINT").is_some() && tick % 500 == 0 {
            eprintln!(
                "{:.0} s alt {:.0} m v {:.0} thrust {:.0} sections {} charge {:.0} kJ",
                time.seconds_since(SimTime::J2000),
                state.position.length() - body.radius,
                state.velocity.length(),
                vessel.flight.thrust,
                vessel.propulsion.sections.len(),
                vessel.power_state(&universe.system, time).charge
            );
        }
        if falling && !staged {
            // Let go of the booster, then arm the parachute.
            universe.activate_next_stage(index);
            universe.activate_next_stage(index);
            staged = true;
        }
        if landed.is_some() {
            break;
        }
    }
    let speed = landed.expect("it came down");
    assert!(speed <= 10.0, "landed at {speed:.1} m/s");
    // A minute on the ground: a whole set landed too.
    for _ in 0..(70 * 50) {
        universe.step_physics();
    }
    let sites_now = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
    assert!(universe.recoverable(id, &sites_now));
    let recovered = universe.recover_vessel(id).unwrap();
    let career = universe.career.as_ref().unwrap();
    let first = cheapest(&tree, career);
    if std::env::var_os("FIRST_FLIGHT_PRINT").is_some() {
        eprintln!(
            "{} at {speed:.1} m/s: {:?}",
            recovered.notice(),
            recovered.sets
        );
    }
    assert!(
        recovered.science > first,
        "{} after reaching {:.0} km, the first node costs {first}: {:?}",
        recovered.notice(),
        apogee / 1e3,
        recovered.sets
    );
}

/// Realistic: a WAC Corporal from White Sands carrying a thermometer, a barometer and the parts
/// list's telemetry unit. What they read on the way up and down goes home by telemetry to the
/// range's station as it is made, and earns science, though the rocket itself is lost.
#[test]
fn a_realistic_first_sounding_rocket_sends_its_readings_home_by_telemetry() {
    let content = content(true);
    let registry = &content.registry;
    let catalog = record::catalog(registry);
    let (_, career) = career(&content);
    assert!(catalog.rules.data_rates);
    let mut diagnostics = Vec::new();
    let system = SolarSystem::from_def(&SystemDef {
        bodies: registry.all::<BodyDef>("bodies", &mut diagnostics),
    })
    .unwrap();
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut diagnostics);
    let site = sites.iter().find(|s| s.id == "white_sands_lc33").unwrap();
    let mut craft: CraftDef = registry.get("craft", "wac_corporal").unwrap();
    let telemetry = osp_sim::palette::entries(&catalog)
        .into_iter()
        .find(|e| e.id == "telemetry")
        .unwrap();
    let rocket = craft
        .parts
        .iter_mut()
        .find(|p| p.id == "wac_corporal")
        .unwrap();
    rocket.components.extend(components(
        r#"[{ experiment: { experiment: "temperature" } },
            { experiment: { experiment: "pressure" } },
            { drive: { data: 1000 } }]"#,
    ));
    rocket.components.extend(telemetry.part.components);
    let t = SimTime::J2000;
    let mut vessel = Vessel::on_launch_site(&craft, &catalog, &system, site, t).unwrap();
    vessel.propulsion.require_control_under(&catalog.rules);
    let mut universe = Universe::new(system, vec![vessel], t);
    universe.focus = Some(universe.vessels[0].id());
    universe.set_stations(&registry.all::<GroundStationDef>("ground_stations", &mut Vec::new()));
    universe.data_rates = true;
    universe.career = Some(career);
    let id = universe.focus.unwrap();
    universe.update_links();
    let link = universe.vessels[0].link().cloned();
    assert_eq!(
        link.map(|l| l.station).as_deref(),
        Some("white_sands_range")
    );
    universe.vessels[0].set_throttle(1.0);
    universe.activate_next_stage(0);
    for _ in 0..(600 * 50) {
        universe.step_physics();
        let Some(index) = universe.find_vessel(id) else {
            break;
        };
        let vessel = &universe.vessels[index];
        if vessel.is_destroyed() || vessel.is_landed() {
            break;
        }
        let p = &vessel.propulsion;
        let booster = p.sections[0].id;
        if p.sections.len() > 1 && !p.engines_of(booster).any(|e| e.running) {
            universe.activate_next_stage(index);
        }
    }
    let career = universe.career.as_ref().unwrap();
    if std::env::var_os("FIRST_FLIGHT_PRINT").is_some() {
        eprintln!(
            "sent home: {:.2} science: {:?}",
            career.science, career.returned
        );
    }
    assert!(
        career.science > 0.5,
        "{} science: {:?}",
        career.science,
        career.returned
    );
}
