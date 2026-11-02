//! The Saturn V flies up through its own sequences: each upper stage lights after the one
//! below it falls away, which only works because solid ullage motors push the stack forward
//! while the liquid engines start.

use std::path::Path;

use osp_sim::{
    DVec3,
    data::{BodyDef, CraftDef, LaunchSiteDef, SystemDef},
    propulsion::Propulsion,
    system::SolarSystem,
    time::{SimTime, Ticks},
    vessel::Vessel,
};

#[path = "golden/record.rs"]
#[allow(dead_code)]
mod record;

struct Flight {
    system: SolarSystem,
    vessel: Vessel,
    time: SimTime,
    /// The records of the engines that gave thrust at some point.
    thrust: Vec<String>,
}

/// Flies the Saturn V straight up from the Cape, pressing the stage key as a player would:
/// when the stage burning has run dry, or a moment after the last press.
fn fly() -> Flight {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let catalog = record::catalog(registry);
    let mut diagnostics = Vec::new();
    let system = SolarSystem::from_def(&SystemDef {
        bodies: registry.all::<BodyDef>("bodies", &mut diagnostics),
    })
    .unwrap();
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut diagnostics);
    let site = sites.iter().find(|s| s.id == "cape_canaveral").unwrap();
    let def: CraftDef = registry.get("craft", "saturn_v").unwrap();
    let mut t = SimTime::J2000;
    let mut v = Vessel::on_launch_site(&def, &catalog, &system, site, t).unwrap();
    let dt = Ticks::from_millis(20);
    let mut events = Vec::new();
    v.set_throttle(1.0);
    v.set_sas(&system, t, true);
    v.activate_next_stage(&system, t);
    // Sequences pressed so far, and how long ago the last one was.
    let mut pressed = 1;
    let mut since = 0.0;
    let mut thrust: Vec<String> = Vec::new();
    for _ in 0..(900 * 50) {
        v.step(&system, t, t + dt, &mut events);
        t = t + dt;
        since += 0.02;
        let burning = v.propulsion.engines.iter().any(|e| e.running);
        for e in v.propulsion.engines.iter().filter(|e| e.thrust > 0.0) {
            if !thrust.contains(&e.def.id) {
                thrust.push(e.def.id.clone());
            }
        }
        let press = match pressed {
            // The instrument unit cuts the F-1s; then the S-IC falls away and the S-II's ullage
            // motors light.
            1 => !burning && since > 1.0,
            // A second later, while they burn, the S-II's J-2s.
            2 => since > 1.0,
            // The spent interstage goes once the S-II is well under way.
            3 => since > 30.0,
            // The S-II runs dry: it falls away and the S-IVB's ullage motors light.
            4 => !burning && since > 1.0,
            // And a second later, the S-IVB's J-2.
            5 => since > 1.0,
            _ => false,
        };
        if press {
            v.activate_next_stage(&system, t);
            pressed += 1;
            since = 0.0;
        }
        if pressed == 6 && since > 20.0 {
            break;
        }
    }
    Flight {
        system,
        vessel: v,
        time: t,
        thrust,
    }
}

#[test]
fn the_upper_stages_light_after_staging() {
    let Flight {
        vessel: v, thrust, ..
    } = fly();
    let log: Vec<_> = v.log.iter().map(|e| &e.message).collect();
    assert!(!v.is_destroyed(), "{log:?}");
    assert!(
        !log.iter().any(|m| m.contains("failed")),
        "an engine failed to light: {log:?}"
    );
    // Every stage's engines gave thrust, the ullage motors that settled them included.
    for id in [
        "F-1-1.52M",
        "Ullage motor solid motor",
        "J-2-230K",
        "J-2-230K S-IVB",
    ] {
        assert!(
            thrust.iter().any(|t| t == id),
            "{id} never fired: {thrust:?} {log:?}"
        );
    }
    // The S-IVB is the stage burning at the end, with its one J-2.
    let burning: Vec<_> = v
        .propulsion
        .engines
        .iter()
        .filter(|e| e.running && e.thrust > 0.0)
        .collect();
    assert_eq!(burning.len(), 1, "{log:?}");
    assert_eq!(burning[0].def.id, "J-2-230K S-IVB");
}

#[test]
fn the_s_ivb_relights_after_a_coast_on_rcs_ullage() {
    let Flight {
        system,
        vessel: mut v,
        mut time,
        ..
    } = fly();
    let dt = Ticks::from_millis(20);
    let mut run = |v: &mut Vessel, seconds: f64| {
        let mut events = Vec::new();
        for _ in 0..(seconds / 0.02) as usize {
            v.step(&system, time, time + dt, &mut events);
            time = time + dt;
        }
    };
    // Cut off and coast: the propellant floats.
    v.set_throttle(0.0);
    run(&mut v, 30.0);
    assert!(!v.propulsion.any_engine_running());
    // The service module's RCS pushes the stack forward, and the J-2 lights on it.
    v.set_rcs(true);
    v.translation_input = DVec3::Z;
    run(&mut v, 4.0);
    v.set_throttle(1.0);
    run(&mut v, 3.0);
    let log: Vec<_> = v.log.iter().map(|e| &e.message).collect();
    assert!(v.propulsion.any_engine_running(), "{log:?}");
    assert!(
        !log.iter()
            .any(|m| m.contains("no ignitions left") && m.contains("J-2")),
        "{log:?}"
    );
}

/// The instrument unit steers the stack; without it the CM's computer is too small for the mass,
/// and there is nothing to hold it on course.
#[test]
fn the_saturn_v_cannot_steer_without_its_instrument_unit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let catalog = record::catalog(registry);
    let mut diagnostics = Vec::new();
    let system = SolarSystem::from_def(&SystemDef {
        bodies: registry.all::<BodyDef>("bodies", &mut diagnostics),
    })
    .unwrap();
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut diagnostics);
    let site = sites.iter().find(|s| s.id == "cape_canaveral").unwrap();
    let def: CraftDef = registry.get("craft", "saturn_v").unwrap();
    let mut t = SimTime::J2000;
    let start = |def: &CraftDef| Vessel::on_launch_site(def, &catalog, &system, site, t).unwrap();

    let whole = start(&def);
    assert!(whole.controlled(&system));

    // The same stack without the unit: the S-IVB hangs straight off the ring above it.
    let mut cut = def.clone();
    cut.parts.retain(|p| p.id != "instrument_unit");
    let s_ivb = cut.parts.iter_mut().find(|p| p.id == "s_ivb").unwrap();
    s_ivb.on = Some("s_ivb_decoupler.bottom".into());
    cut.triggers.clear();
    let mut v = start(&cut);
    assert!(!v.controlled(&system));
    v.set_throttle(1.0);
    v.set_sas(&system, t, true);
    v.activate_next_stage(&system, t);
    let dt = Ticks::from_millis(20);
    let mut events = Vec::new();
    for _ in 0..(60 * 50) {
        v.rotation_input = DVec3::new(1.0, 0.0, 0.0);
        v.step(&system, t, t + dt, &mut events);
        t = t + dt;
    }
    assert!(!v.sas().enabled());
    assert!(
        (v.propulsion.engines.iter()).all(|e| e.deflection.length() == 0.0),
        "an engine is swinging with no unit to swing it"
    );
    let log: Vec<_> = v.log.iter().map(|e| &e.message).collect();
    assert!(log.iter().any(|m| m.contains("No avionics")), "{log:?}");
}

/// The S-IC's retro motors light with the joint letting go, and burn as it falls away: they are
/// solids, and care nothing for the throttle it was left with.
#[test]
fn the_retro_motors_pull_the_spent_stage_away() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let catalog = record::catalog(registry);
    let def: CraftDef = registry.get("craft", "saturn_v").unwrap();
    let mut p = Propulsion::from_craft(&def, &catalog).unwrap();
    p.set_throttle(1.0);
    p.activate_next_sequence().unwrap();
    let staged = p.activate_next_sequence().unwrap();
    let spent = staged
        .released
        .iter()
        .find(|r| r.sections.iter().any(|s| s.name == "S-IC"))
        .expect("the S-IC fell away")
        .clone();
    let mut debris = Propulsion::from_released(spent);
    let thrust = debris.burn(0.0, 0.1, true, &mut Vec::new());
    let retros: Vec<_> = (debris.engines.iter())
        .filter(|e| e.is_separation_motor() && e.running)
        .collect();
    assert_eq!(retros.len(), 4);
    assert!(thrust > 0.0, "{thrust}");
    // Pushing the way the stage has to go to get clear: back down the stack.
    for retro in retros {
        assert!(retro.direction.z < -0.99, "{}", retro.direction);
    }
    // And they are done in about two thirds of a second.
    for _ in 0..30 {
        debris.burn(0.0, 0.1, true, &mut Vec::new());
    }
    assert!(
        !debris
            .engines
            .iter()
            .any(|e| e.is_separation_motor() && e.running)
    );
}

/// A ring sized for too little breaks when the stack pulls on it.
#[test]
fn a_joint_too_weak_for_what_it_carries_breaks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let catalog = record::catalog(registry);
    let mut diagnostics = Vec::new();
    let system = SolarSystem::from_def(&SystemDef {
        bodies: registry.all::<BodyDef>("bodies", &mut diagnostics),
    })
    .unwrap();
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut diagnostics);
    let site = sites.iter().find(|s| s.id == "cape_canaveral").unwrap();
    let craft = |load: f64| -> CraftDef {
        json5::from_str(&format!(
            r#"{{ id: "weak", name: "", parts: [
                {{ id: "heavy", shape: {{ shell: {{ bottom_diameter: 2, sections: [{{ length: 3 }}],
                    cap_bottom: true, cap_top: true }} }},
                  structure: {{ mass: 20000 }}, components: [{{ container: {{}} }}] }},
                {{ id: "ring", on: "heavy.bottom", by: "top",
                  procedural: {{ decoupler: {{ diameter: 2, load: {load} }} }} }},
                {{ id: "base", on: "ring.bottom", by: "top",
                  shape: {{ shell: {{ bottom_diameter: 2, sections: [{{ length: 3 }}],
                    cap_bottom: true, cap_top: true }} }},
                  structure: {{ mass: 500 }}, components: [{{ container: {{}} }}] }}
            ] }}"#
        ))
        .unwrap()
    };
    let hold = |load: f64| {
        let mut t = SimTime::J2000;
        let mut v = Vessel::on_launch_site(&craft(load), &catalog, &system, site, t).unwrap();
        let dt = Ticks::from_millis(20);
        let mut events = Vec::new();
        for _ in 0..50 {
            v.step(&system, t, t + dt, &mut events);
            t = t + dt;
        }
        v
    };
    // 20 t on the pad is about 200 kN: well within 1 MN, far past what 1 kN's wall holds even at
    // its thinnest (a 2 m ring's 2 mm wall holds 3.8 MN, so this one's sized by a light gauge).
    let strong = hold(1e6);
    assert!(!strong.is_destroyed());
    assert!(strong.flight.structure_load < 1.0);
    let joint = osp_sim::structure::loads(
        &strong.propulsion,
        &osp_sim::structure::Conditions {
            acceleration: 1000.0,
            landed: true,
            ..Default::default()
        },
    );
    assert!(
        joint
            .iter()
            .any(|l| l.part == osp_sim::structure::Part::Joint && l.fraction > 1.0)
    );
}
