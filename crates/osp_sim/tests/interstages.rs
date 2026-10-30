//! Interstages as plain parts: an engine fires through an open one (a truss, a vented
//! interstage) while the stage under it is still on, as the Soyuz's Blok I and the Titan
//! II's second stage do; a closed one holds it until the stage under it falls away, and a
//! ring left on after that (the Saturn V's S-IC/S-II interstage, let go at both planes)
//! lets it fire through, falling away a sequence later.

use std::path::Path;

use osp_sim::{data::CraftDef, propulsion::Propulsion};

#[path = "golden/record.rs"]
#[allow(dead_code)]
mod record;

fn craft(id: &str) -> Propulsion {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let def: CraftDef = registry.get("craft", id).unwrap();
    Propulsion::from_craft(&def, &record::catalog(registry)).unwrap()
}

/// The records of the engines that would burn with the throttle open.
fn firing(p: &Propulsion) -> Vec<String> {
    let mut out: Vec<String> = (p.firing().iter())
        .map(|&e| {
            p.engines
                .iter()
                .find(|x| x.part == e)
                .unwrap()
                .def
                .id
                .clone()
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

fn names(p: &Propulsion) -> Vec<&str> {
    p.sections.iter().map(|s| s.name.as_str()).collect()
}

#[test]
fn open_interstages_are_found_by_their_shape() {
    for (id, part) in [
        ("soyuz_11a511", "Interstage truss"),
        ("vostok", "Interstage truss"),
        ("titan2_gemini", "Titan II interstage"),
        ("saturn_v", ""),
    ] {
        let p = craft(id);
        let open: Vec<&str> = (p.parts().parts().iter())
            .filter(|q| q.open)
            .map(|q| q.name.as_str())
            .collect();
        let want: &[&str] = if part.is_empty() { &[] } else { &[part] };
        assert_eq!(open, want, "{id}");
    }
}

#[test]
fn the_blok_i_lights_through_its_truss() {
    let mut p = craft("soyuz_11a511");
    p.activate_next_sequence();
    p.activate_next_sequence();
    assert_eq!(firing(&p), ["RD-108-8D727"]);
    // Lit with the core still burning under it, then the core lets go.
    p.activate_next_sequence();
    assert_eq!(firing(&p).len(), 2, "{:?}", firing(&p));
    assert!(names(&p).contains(&"Core (Blok A)"));
    p.activate_next_sequence();
    assert_eq!(firing(&p), ["RD-0110"]);
}

#[test]
fn the_titan_ii_fires_in_the_hole() {
    let mut p = craft("titan2_gemini");
    p.activate_next_sequence();
    p.activate_next_sequence();
    assert_eq!(firing(&p), ["LR87-AJ-7", "LR91-AJ-7"]);
    assert!(names(&p).contains(&"Titan II first stage"));
    p.activate_next_sequence();
    assert_eq!(firing(&p), ["LR91-AJ-7"]);
}

#[test]
fn the_s_ii_fires_through_the_ring_the_s_ic_leaves() {
    let mut p = craft("saturn_v");
    p.activate_next_sequence();
    // Armed under a closed interstage with the S-IC on, the S-II would wait.
    let s_ii = p.sections.iter().find(|s| s.name == "S-II").unwrap().id;
    p.arm(s_ii);
    assert_eq!(firing(&p), ["F-1-1.52M"]);
    // The S-IC falls away, the ring stays: the S-II fires through it, with the ullage motors
    // that sit in the ring.
    p.activate_next_sequence();
    assert!(names(&p).contains(&"S-IC/S-II interstage"));
    assert_eq!(firing(&p), ["J-2-230K", "S-II-Ullage"]);
    // The S-II's own sequence, then the ring goes with its motors.
    p.activate_next_sequence();
    assert_eq!(firing(&p), ["J-2-230K", "S-II-Ullage"]);
    p.activate_next_sequence();
    assert!(!names(&p).contains(&"S-IC/S-II interstage"));
    assert_eq!(firing(&p), ["J-2-230K"]);
}

#[test]
fn the_atlas_sustainer_fires_through_its_booster_skirt() {
    let mut p = craft("mercury_atlas");
    // Booster engines and sustainer light together; the skirt is open down the middle.
    p.activate_next_sequence();
    assert_eq!(firing(&p), ["LR105-NA-5", "LR89-NA-5"]);
    // All three draw on the sustainer's one tank, across the skirt's crossfeed.
    p.set_throttle(1.0);
    let full = p.propellant_mass(
        p.sections
            .iter()
            .find(|s| s.name == "Sustainer")
            .unwrap()
            .id,
    );
    for _ in 0..60 {
        p.burn(101_325.0, 1.0, true, &mut Vec::new());
    }
    let sustainer = p
        .sections
        .iter()
        .find(|s| s.name == "Sustainer")
        .unwrap()
        .id;
    let used = full - p.propellant_mass(sustainer);
    let flow: f64 = p.engines.iter().map(|e| e.max_mass_flow()).sum();
    assert!((used / (60.0 * flow) - 1.0).abs() < 0.05, "{used} {flow}");
    // The skirt drops; the sustainer burns on.
    p.activate_next_sequence();
    assert_eq!(firing(&p), ["LR105-NA-5"]);
}
