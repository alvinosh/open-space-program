//! The Saturn V's command module on its way home: what the air does to it from 122 km to the sea.

use std::path::Path;
use std::sync::Arc;

use osp_sim::{
    DVec3,
    data::{BodyDef, CraftDef, LaunchSiteDef, ScenarioDef, SystemDef},
    guidance::Guidance,
    systems::{HeatShield, Thermal},
    universe::Universe,
};

#[path = "golden/record.rs"]
#[allow(dead_code)]
mod record;

fn entry() -> Universe {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let bodies = registry.all::<BodyDef>("bodies", &mut Vec::new());
    let system = SystemDef { bodies };
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
    let catalog = record::catalog(registry);
    let scenario: ScenarioDef = registry.get("scenarios", "apollo11_entry").unwrap();
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

/// What came of a flight down.
#[derive(Debug, Default)]
struct Result {
    peak_g: f64,
    peak_flux: f64,
    ablator_used: f64,
    apex_first: bool,
    splashed: Option<f64>,
    burned: bool,
    crew: u32,
    log: Vec<String>,
}

fn fly(universe: &mut Universe, guidance: Guidance, ablator: Option<f64>, print: bool) -> Result {
    let id = universe.focus.unwrap();
    let index = universe.find_vessel(id).unwrap();
    universe.vessels[index].guidance = guidance;
    let _ = ablator;
    let mut result = Result::default();
    for tick in 0..(1800 * 50) {
        universe.step_physics();
        let Some(index) = universe.find_vessel(id) else {
            break;
        };
        let time = universe.time();
        let system = &universe.system;
        let v = &universe.vessels[index];
        let state = v.state_at(system, time);
        let body = system.body(v.parent());
        let altitude = state.position.length() - body.radius;
        let g = v.flight.proper_acceleration / 9.80665;
        result.peak_g = result.peak_g.max(g);
        let nose = v.attitude_state(system, time).nose();
        let air = state.velocity - body.surface_velocity(state.position);
        let speed = air.length();
        let angle = nose.angle_between(-air).to_degrees();
        if print && tick % 500 == 0 {
            let temperature: f64 = (v.propulsion.systems.all::<Thermal>())
                .map(|(_, t)| t.temperature)
                .fold(0.0, f64::max);
            let left: f64 = (v.propulsion.systems.all::<HeatShield>())
                .map(|(_, s)| s.ablator)
                .sum();
            let flux: f64 = (v.propulsion.systems.all::<HeatShield>())
                .map(|(_, s)| s.flux)
                .sum();
            eprintln!(
                "{:5.0}s {:7.1} km {:7.0} m/s {:5.1} g apex-from-retro {:5.1} deg  T {:6.0} K ablator {:5.0} kg flux {:7.0} kW/m2 climb {:7.0}",
                tick as f64 * 0.02,
                altitude / 1e3,
                speed,
                g,
                angle,
                temperature,
                left,
                flux / 1e3,
                state.radial_speed(),
            );
        }
        if altitude < 60_000.0 && speed > 3_000.0 && angle > 120.0 {
            result.apex_first = true;
        }
        for (_, shield) in v.propulsion.systems.all::<HeatShield>() {
            result.peak_flux = result.peak_flux.max(shield.peak_flux);
            result.ablator_used = shield.full - shield.ablator;
        }
        result.crew = v.propulsion.systems.crew_aboard();
        if v.is_destroyed() {
            result.burned = true;
            break;
        }
        if v.is_afloat() {
            let said = v
                .log
                .iter()
                .rev()
                .find(|e| e.message.starts_with("Splashed"));
            result.splashed = said
                .and_then(|e| e.message.split(' ').nth(3)?.parse().ok())
                .or(Some(0.0));
            break;
        }
        if v.is_landed() {
            break;
        }
    }
    if let Some(index) = universe.find_vessel(id) {
        result.log = (universe.vessels[index].log.iter())
            .map(|e| e.message.clone())
            .collect();
    }
    result
}

#[test]
#[ignore = "prints a whole entry, for working on it"]
fn print_an_entry() {
    let mut universe = entry();
    let result = fly(&mut universe, Guidance::Entry, None, true);
    eprintln!("{result:#?}");
}

#[test]
fn the_capsule_flies_home_on_its_lift_and_splashes_down() {
    let mut universe = entry();
    let result = fly(&mut universe, Guidance::Entry, None, false);
    assert!(!result.burned, "{result:#?}");
    assert!(!result.apex_first, "{result:#?}");
    assert!(result.ablator_used > 0.0, "{result:#?}");
    assert!(result.splashed.is_some_and(|s| s <= 20.0), "{result:#?}");
    assert_eq!(result.crew, 3, "{result:#?}");
    let _ = DVec3::ZERO;
}

/// A capsule behind its shield with a fin sticking out into the air past it.
const FINNED: &str = r#"{ id: "finned", name: "Finned", parts: [
    { id: "capsule",
      shape: { shell: { bottom_diameter: 3.9, sections: [{ length: 3, top_diameter: 0.8 }],
          cap_bottom: true, cap_top: true } },
      structure: { mass: 5500 },
      nodes: [{ name: "side", position: [1.2, 0, 1.5], direction: [1, 0, 0], size: 0.5 }],
      components: [{ heat_shield: { ablator: 850 } }, { crew: { seats: 3 } }] },
    { id: "fin", name: "Fin", on: "capsule.side", by: "root",
      shape: { wing: { root_chord: 1.6, tip_chord: 0.7, span: 2.5, sweep: 0.8,
          airfoil: { plate: { thickness: 0.02 } } } },
      structure: { material: "Al 2219-T87" },
      components: [{ lifting_surface: {} }] },
] }"#;

#[test]
fn a_fin_burns_off_while_the_shielded_capsule_lives() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let system = SystemDef {
        bodies: registry.all::<BodyDef>("bodies", &mut Vec::new()),
    };
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
    let catalog = record::catalog(registry);
    let mut scenario: ScenarioDef = registry.get("scenarios", "apollo11_entry").unwrap();
    scenario.craft = "finned".into();
    scenario.sequences_done = 0;
    let craft_of = |_: &str| json5::from_str::<CraftDef>(FINNED).map_err(|e| e.to_string());
    let mut universe =
        Universe::new_game(&system, Vec::new(), &sites, &catalog, craft_of, &scenario).unwrap();
    universe
        .colliders
        .set_files(Arc::new(content.file_reader()));
    let id = universe.focus.unwrap();
    let parts = |universe: &Universe| {
        let v = &universe.vessels[universe.find_vessel(id).unwrap()];
        v.propulsion.parts().parts().len()
    };
    assert_eq!(parts(&universe), 2);
    let result = fly(&mut universe, Guidance::Entry, None, false);
    assert!(
        result.log.iter().any(|m| m == "Fin burned off"),
        "{result:#?}"
    );
    assert_eq!(parts(&universe), 1, "{result:#?}");
    assert!(!result.burned, "{result:#?}");
    assert_eq!(result.crew, 3, "{result:#?}");
}

#[test]
#[ignore = "prints what the air does to the capsule at each angle"]
fn print_the_capsule_in_the_air() {
    use osp_sim::{aero::Flow, atmosphere::AtmosphereSample};
    let universe = entry();
    let v = &universe.vessels[universe.find_vessel(universe.focus.unwrap()).unwrap()];
    let shape = v.propulsion.shape();
    let mass = v.mass_properties();
    eprintln!(
        "mass {:.0} com {:?} inertia {:?}",
        mass.mass,
        mass.center_of_mass,
        mass.inertia.diagonal()
    );
    eprintln!("reference area {:.1}", shape.reference_area());
    for (z, r) in shape.outline() {
        eprintln!("  z {z:6.2} r {r:5.2}");
    }
    let air = AtmosphereSample {
        density: 1e-3,
        pressure: 50.0,
        temperature: 250.0,
        speed_of_sound: 320.0,
    };
    for degrees in [
        0.0_f64, 5.0, 10.0, 20.0, 40.0, 60.0, 90.0, 120.0, 150.0, 175.0,
    ] {
        let a = degrees.to_radians();
        // Moving shield first at this angle off it.
        let velocity = DVec3::new(a.sin(), 0.0, -a.cos()) * 10_000.0;
        let flow = Flow {
            velocity,
            angular_velocity: DVec3::ZERO,
            air,
            plume: false,
            controls: DVec3::ZERO,
        };
        let loads = shape.loads_about(&flow, mass.center_of_mass);
        eprintln!(
            "off shield-first {degrees:5.0} deg: force {:?} kN, torque y {:8.1} kN m",
            (loads.force / 1e3)
                .to_array()
                .map(|f| (f * 10.0).round() / 10.0),
            loads.torque.y / 1e3
        );
    }
}
