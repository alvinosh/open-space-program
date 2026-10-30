//! The Saturn V flies up through its own sequences: each upper stage lights after the one
//! below it falls away, which only works because solid ullage motors push the stack forward
//! while the liquid engines start.

use std::path::Path;

use osp_sim::{
    DVec3,
    data::{BodyDef, CraftDef, LaunchSiteDef, SystemDef},
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
            // The S-IC runs dry: it falls away and the S-II's ullage motors light.
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
    let Flight { vessel: v, thrust, .. } = fly();
    let log: Vec<_> = v.log.iter().map(|e| &e.message).collect();
    assert!(!v.is_destroyed(), "{log:?}");
    assert!(
        !log.iter().any(|m| m.contains("failed")),
        "an engine failed to light: {log:?}"
    );
    // Every stage's engines gave thrust, the ullage motors that settled them included.
    for id in ["F-1-1.52M", "S-II-Ullage", "J-2-230K", "S-IVB-Ullage", "J-2-230K S-IVB"] {
        assert!(thrust.iter().any(|t| t == id), "{id} never fired: {thrust:?} {log:?}");
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
        !log.iter().any(|m| m.contains("no ignitions left")
            && m.contains("J-2")),
        "{log:?}"
    );
}
