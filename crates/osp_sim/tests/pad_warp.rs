//! A craft standing on the pad warps as fast as anything else at rest: its clock moves at the rate
//! asked for, crew aboard or not, and it launches afterwards as it would have.

use std::path::Path;

use osp_sim::{
    data::{BodyDef, CraftDef, LaunchSiteDef, ScenarioDef, StartDef, SystemDef},
    time::SimTime,
    universe::Universe,
};

#[path = "golden/record.rs"]
#[allow(dead_code)]
mod record;

/// A new game with the craft `id` on the Cape's pad, with every mod on (Realistic radiation and
/// life support among them), settled for a few seconds.
fn on_the_pad(id: &str) -> Universe {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let bodies = registry.all::<BodyDef>("bodies", &mut Vec::new());
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
    let catalog = record::catalog(registry);
    let scenario: ScenarioDef = registry.get("scenarios", "leo").unwrap();
    let craft_of = |id: &str| {
        registry
            .get::<CraftDef>("craft", id)
            .map_err(|e| e.to_string())
    };
    let system = SystemDef { bodies };
    let mut universe =
        Universe::new_game(&system, Vec::new(), &sites, &catalog, craft_of, &scenario).unwrap();
    let craft: CraftDef = registry.get("craft", id).unwrap();
    let start: StartDef = serde_json::from_value(serde_json::json!({
        "type": "LaunchSite", "site": "cape_canaveral",
    }))
    .unwrap();
    let time = SimTime::from_seconds(21_600.0);
    universe
        .launch(&sites, &catalog, &craft, &start, None, Some(time))
        .unwrap();
    for _ in 0..200 {
        universe.step_physics();
    }
    universe
}

fn altitude(universe: &Universe) -> f64 {
    let vessel = &universe.vessels[0];
    let body = universe.system.body(vessel.parent());
    let state = vessel.state_at(&universe.system, universe.time());
    state.position.length() - body.radius
}

/// An hour on the pad at a rails warp, in ten-minute jumps, comes to an hour exactly; with a crew
/// aboard too, whose radiation dose under the air would take ages to matter (it used to overflow the
/// clock and hold it still). Then it lifts off.
#[test]
fn a_craft_on_the_pad_warps_an_hour_and_then_lifts_off() {
    for id in ["pathfinder", "pathfinder_crew"] {
        let mut universe = on_the_pad(id);
        assert!(universe.can_use_rails(), "{id}");
        let start = universe.time();
        let ground = altitude(&universe);
        for jump in 1..=6 {
            let outcome = universe.advance_rails(start.add_seconds(600.0 * jump as f64));
            assert!(!outcome.interrupted, "{id}: {:?}", outcome.events);
        }
        let moved = universe.time().seconds_since(start);
        assert!((moved - 3_600.0).abs() < 1e-6, "{id}: {moved} s");
        assert!(
            (altitude(&universe) - ground).abs() < 0.5,
            "{id} stayed put"
        );
        universe.vessels[0].set_throttle(1.0);
        universe.activate_next_stage(0);
        for _ in 0..600 {
            universe.step_physics();
        }
        assert!(
            altitude(&universe) > ground + 20.0,
            "{id} lifted off: {} m",
            altitude(&universe) - ground
        );
    }
}
