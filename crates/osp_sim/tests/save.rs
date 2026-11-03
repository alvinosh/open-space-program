//! A saved game carries on exactly as the game would have: load a save and fly on, and every bit
//! of every vessel is what it would have been without saving, for craft of every kind in the
//! states a save can catch them in.

use std::{path::Path, sync::Arc};

use osp_mods::Content;
use osp_sim::{
    DVec3,
    attitude::SasMode,
    data::{BodyDef, CraftDef, LaunchSiteDef, ScenarioDef, StartDef, SystemDef},
    plan::Burn,
    save::{self, Header},
    time::SimTime,
    universe::Universe,
    vessel::Vessel,
};

#[path = "golden/record.rs"]
#[allow(dead_code)]
mod record;

fn content() -> Content {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")))
}

/// The scenario `id`, from the records, in a solar system of its own.
fn scenario(content: &Content, id: &str) -> Universe {
    let bodies = content.registry.all::<BodyDef>("bodies", &mut Vec::new());
    among(content, id, bodies)
}

/// The scenario `id` among these bodies.
fn among(content: &Content, id: &str, bodies: Vec<BodyDef>) -> Universe {
    let registry = &content.registry;
    let system = SystemDef { bodies };
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
    let catalog = record::catalog(registry);
    let scenario: ScenarioDef = registry.get("scenarios", id).unwrap();
    let craft_of = |id: &str| {
        registry
            .get::<CraftDef>("craft", id)
            .map_err(|e| e.to_string())
    };
    let mut universe =
        Universe::new_game(&system, Vec::new(), &sites, &catalog, craft_of, &scenario).unwrap();
    universe
        .colliders
        .set_files(Arc::new(content.file_reader()));
    universe
}

/// The state of a universe, to compare: the same save means the same state, to the bit.
fn bytes(universe: &Universe) -> Vec<u8> {
    let header = Header::new("test", 0, universe);
    save::write(&header, universe).unwrap()
}

/// Save `universe`, load it into another with the same records, and check they are the same and
/// fly on the same for `ticks`.
fn carries_on_the_same(content: &Content, id: &str, universe: &mut Universe, ticks: usize) {
    let saved = bytes(universe);
    let header = save::read_header(&saved).unwrap();
    assert_eq!(header.time, universe.time(), "{id}");
    let mut loaded = scenario(content, id);
    loaded.restore(save::read(&saved).unwrap()).unwrap();
    assert!(
        bytes(&loaded) == saved,
        "{id}: what was loaded is not what was saved"
    );
    for tick in 0..ticks {
        let (a, b) = (universe.step_physics(), loaded.step_physics());
        assert_eq!(a, b, "{id}: tick {tick} after loading, other events");
    }
    assert!(
        bytes(&loaded) == bytes(universe),
        "{id}: flew on differently after loading"
    );
}

#[test]
fn a_rocket_on_the_pad_and_on_its_way_up() {
    let content = content();
    let mut universe = scenario(&content, "pad");
    carries_on_the_same(&content, "pad", &mut universe, 50);
    // Lit, leaving the ground, burning fuel, with SAS holding it up.
    let index = universe
        .vessels
        .iter()
        .position(|v| Some(v.id()) == universe.focus)
        .unwrap();
    universe.vessels[index].set_throttle(1.0);
    universe.activate_next_stage(index);
    let time = universe.time();
    let (vessel, system) = (&mut universe.vessels[index], &universe.system);
    vessel.set_sas(system, time, true);
    for _ in 0..300 {
        universe.step_physics();
    }
    assert!(!universe.vessels[index].is_landed());
    carries_on_the_same(&content, "pad", &mut universe, 300);
}

#[test]
fn stages_that_fell_away_and_fly_on_as_debris() {
    let content = content();
    let mut universe = scenario(&content, "pad");
    let index = universe
        .vessels
        .iter()
        .position(|v| Some(v.id()) == universe.focus)
        .unwrap();
    universe.vessels[index].set_throttle(1.0);
    for _ in 0..4 {
        universe.activate_next_stage(index);
        for _ in 0..200 {
            universe.step_physics();
        }
    }
    assert!(universe.vessels.len() > 1, "nothing fell away to save");
    carries_on_the_same(&content, "pad", &mut universe, 200);
}

#[test]
fn a_vessel_in_orbit_with_a_burn_planned_and_its_executor_engaged() {
    let content = content();
    let mut universe = scenario(&content, "leo");
    let time = universe.time();
    let index = universe
        .vessels
        .iter()
        .position(|v| Some(v.id()) == universe.focus)
        .unwrap();
    universe.vessels[index].plan.add(Burn {
        time: time.add_seconds(120.0),
        delta_v: DVec3::X * 10.0,
    });
    let (vessel, system) = (&mut universe.vessels[index], &universe.system);
    vessel.set_sas(system, time, true);
    vessel.press_sas_mode(system, time, SasMode::Prograde);
    universe.advance_rails(time.add_seconds(30.0));
    carries_on_the_same(&content, "leo", &mut universe, 100);
    assert_eq!(
        universe.vessels[index].plan.burns().len(),
        1,
        "the plan is gone"
    );
    // And with the executor flying it.
    let time = universe.time();
    let (vessel, system) = (&mut universe.vessels[index], &universe.system);
    vessel.engage_burn(system, time);
    for _ in 0..50 {
        universe.step_physics();
    }
    assert!(universe.vessels[index].executor().engaged());
    carries_on_the_same(&content, "leo", &mut universe, 100);
}

/// Every kind of part there is: crew, batteries, antennas, wings, gear, fairings and models.
#[test]
fn every_craft_in_the_game() {
    let content = content();
    let registry = &content.registry;
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
    let catalog = record::catalog(registry);
    let fleet = registry.all::<CraftDef>("craft", &mut Vec::new());
    assert!(fleet.len() >= 40, "only {} craft", fleet.len());
    let start: StartDef = serde_json::from_value(serde_json::json!({
        "type": "Orbit", "body": "Earth", "periapsis_altitude": 300e3, "apoapsis_altitude": 300e3,
    }))
    .unwrap();
    let mut biggest = (0, String::new());
    for craft in &fleet {
        let mut universe = scenario(&content, "leo");
        let launched = universe.launch(&sites, &catalog, craft, &start, None, None);
        launched.unwrap_or_else(|e| panic!("{}: {e}", craft.id));
        biggest = biggest.max((bytes(&universe).len(), craft.id.clone()));
        carries_on_the_same(&content, "leo", &mut universe, 10);
    }
    println!("biggest save: {} ({} kB)", biggest.1, biggest.0 / 1000);
}

#[test]
fn a_rendezvous_with_a_target_and_a_mission_under_way() {
    let content = content();
    let mut universe = scenario(&content, "rendezvous");
    assert!(universe.mission.is_some() || universe.vessels.len() > 1);
    for _ in 0..100 {
        universe.step_physics();
    }
    carries_on_the_same(&content, "rendezvous", &mut universe, 100);
}

#[test]
fn a_game_with_its_new_vessel_launched_into_it() {
    let content = content();
    let mut universe = scenario(&content, "leo");
    let registry = &content.registry;
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
    let catalog = record::catalog(registry);
    let craft: CraftDef = registry.get("craft", "sparrow").unwrap();
    let first = universe.focus.unwrap();
    let before = universe.vessels.len();
    let start: StartDef = serde_json::from_value(serde_json::json!({
        "type": "LaunchSite", "site": "cape_canaveral",
    }))
    .unwrap();
    let id = (universe.launch(&sites, &catalog, &craft, &start, None, None)).unwrap();
    assert_eq!(universe.focus, Some(id));
    assert_eq!(universe.vessels.len(), before + 1);
    assert!(
        universe.vessel(first).is_some(),
        "the first vessel is still there"
    );
    carries_on_the_same(&content, "leo", &mut universe, 100);

    // A new game has nothing else in it, at the time it starts.
    let time = SimTime::from_seconds(21600.0);
    let id = (universe.launch(&sites, &catalog, &craft, &start, None, Some(time))).unwrap();
    assert_eq!(universe.vessels.len(), 1);
    assert_eq!(universe.vessels[0].id(), id);
    assert_eq!(universe.time(), time);
}

#[test]
fn the_edited_sequences_go_with_the_launch() {
    let content = content();
    let mut universe = scenario(&content, "leo");
    let registry = &content.registry;
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
    let catalog = record::catalog(registry);
    let craft: CraftDef = registry.get("craft", "sparrow").unwrap();
    let time = universe.time();
    let display = Vessel::on_display(&craft, &catalog, &universe.system, time);
    let mut sequences = display.unwrap().propulsion.sequences;
    sequences.insert_empty(0);
    let start: StartDef = serde_json::from_value(serde_json::json!({
        "type": "LaunchSite", "site": "cape_canaveral",
    }))
    .unwrap();
    let id = universe.launch(
        &sites,
        &catalog,
        &craft,
        &start,
        Some(sequences.clone()),
        None,
    );
    let vessel = universe.vessel(id.unwrap()).unwrap();
    assert_eq!(vessel.propulsion.sequences, sequences);
}

#[test]
fn a_save_says_what_it_is_without_being_read() {
    let content = content();
    let universe = scenario(&content, "leo");
    let saved = bytes(&universe);
    let line = saved.iter().position(|&b| b == b'\n').unwrap();
    let header = save::read_header(&saved[..line]).unwrap();
    assert_eq!(header.format, save::FORMAT);
    assert_eq!(header.vessels, 1);
    assert_eq!(header.flying, universe.vessels[0].name);
}

#[test]
fn what_cannot_be_read_is_refused_and_changes_nothing() {
    let content = content();
    let universe = scenario(&content, "leo");
    assert!(save::read(b"not a save\nat all").is_err());

    // Another version of the game.
    let mut header = Header::new("old", 0, &universe);
    header.format = save::FORMAT + 1;
    let newer = save::write(&header, &universe).unwrap();
    assert!(save::read(&newer).unwrap_err().contains("another version"));

    // Cut short.
    let whole = bytes(&universe);
    assert!(save::read(&whole[..whole.len() / 2]).is_err());

    // Among other bodies than these: a mod took one away.
    let mut bodies = content.registry.all::<BodyDef>("bodies", &mut Vec::new());
    bodies.pop();
    let mut other = among(&content, "leo", bodies);
    let before = bytes(&other);
    let refused = other.restore(save::read(&whole).unwrap());
    assert!(refused.unwrap_err().contains("other bodies"));
    assert!(bytes(&other) == before);
}

#[test]
fn an_orbit_can_start_in_daylight_whatever_the_time() {
    let content = content();
    let registry = &content.registry;
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
    let catalog = record::catalog(registry);
    let craft: CraftDef = registry.get("craft", "satellite").unwrap();
    let mut universe = scenario(&content, "leo");
    let earth = universe.system.find("Earth").unwrap();
    for hours in [0.0, 5.0, 11.0, 17.0, 23.0, 24.0 * 100.0] {
        let time = SimTime::from_seconds(hours * 3600.0);
        let anomaly = Vessel::sunlit_anomaly(&universe.system, "Earth", 28.6, time).unwrap();
        let start: StartDef = serde_json::from_value(serde_json::json!({
            "type": "Orbit", "body": "Earth", "periapsis_altitude": 200e3,
            "apoapsis_altitude": 200e3, "inclination": 28.6, "mean_anomaly": anomaly,
        }))
        .unwrap();
        let launched = universe.launch(&sites, &catalog, &craft, &start, None, Some(time));
        let vessel = universe.vessel(launched.unwrap()).unwrap();
        let root = universe.system.root();
        let sun = universe.system.relative_state(root, earth, time).position;
        let here = vessel.state_at(&universe.system, time).position;
        // The sun is well up: the orbit's plane is at most 28.6° + 23.4° from it.
        let up = here.normalize().dot(sun.normalize());
        assert!(up > 0.6, "at {hours} h the sun is {up} of the way up");
    }
}
