//! The first sounding rockets fly as the real ones did, from their own launch site and
//! sequences: the sustainer lights with its booster, firing out through the open joint
//! between them so its propellant never floats, the spent booster falls away, and the
//! sustainer's fins keep it pointing up through the thick air to its apogee.

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

/// Flies `craft` straight up from White Sands with nothing but its sequences: the first at
/// liftoff, the next when the booster burns out. Its apogee, m, and how far it ever leaned
/// off the vertical under thrust, degrees.
fn fly(craft: &str) -> (f64, f64, Vessel) {
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
    let site = sites.iter().find(|s| s.id == "white_sands_lc33").unwrap();
    let def: CraftDef = registry.get("craft", craft).unwrap();
    let mut t = SimTime::J2000;
    let mut v = Vessel::on_launch_site(&def, &catalog, &system, site, t).unwrap();
    let dt = Ticks::from_millis(20);
    let mut events = Vec::new();
    v.set_throttle(1.0);
    v.activate_next_stage(&system, t);
    let (mut apogee, mut lean): (f64, f64) = (0.0, 0.0);
    for _ in 0..(300 * 50) {
        v.step(&system, t, t + dt, &mut events);
        t = t + dt;
        let p = &v.propulsion;
        let booster = p.sections[0].id;
        if p.sections.len() > 1 && !p.engines_of(booster).any(|e| e.running) {
            v.activate_next_stage(&system, t);
        }
        let state = v.state_at(&system, t);
        let up = state.position.normalize();
        apogee = apogee.max(state.position.length() - system.body(v.parent()).radius);
        if v.flight.thrust > 0.0 {
            let nose = v.attitude_at(&system, t) * DVec3::Z;
            lean = lean.max(nose.angle_between(up).to_degrees());
        }
        if state.velocity.dot(up) < 0.0 && apogee > 1000.0 {
            break;
        }
    }
    (apogee, lean, v)
}

fn check(craft: &str, apogee_km: std::ops::Range<f64>) {
    let (apogee, lean, v) = fly(craft);
    let log: Vec<_> = v.log.iter().map(|e| &e.message).collect();
    assert!(!v.is_destroyed(), "{log:?}");
    assert!(
        !log.iter()
            .any(|m| m.contains("failed") || m.contains("torn")),
        "{log:?}"
    );
    assert!(lean < 5.0, "{craft} leaned {lean:.0}°: {log:?}");
    assert!(
        apogee_km.contains(&(apogee / 1e3)),
        "{craft} reached {:.0} km",
        apogee / 1e3
    );
}

#[test]
fn the_aerobee_lights_its_sustainer_with_its_booster() {
    // Aerobees of the 1940s and 50s reached 70 to 130 km.
    check("aerobee", 70.0..140.0);
}

#[test]
fn the_wac_corporal_lights_with_its_tiny_tim() {
    // The first full flight reached about 70 km.
    check("wac_corporal", 50.0..110.0);
}
