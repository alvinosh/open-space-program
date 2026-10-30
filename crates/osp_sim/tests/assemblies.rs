//! Craft built from assemblies (`mods/realism/assemblies`): placing them gives the craft,
//! part for part, and they fly and stage as the real vehicles did. The Titan II GLV is two
//! assemblies placed together; the radial craft are a core with booster assemblies placed
//! round it.

use std::path::Path;

use osp_sim::{
    data::{AssemblyDef, CraftDef},
    propulsion::Propulsion,
};

#[path = "golden/record.rs"]
#[allow(dead_code)]
mod record;

#[test]
fn the_gemini_titan_is_its_two_assemblies_placed_together() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let assembly = |id: &str| -> AssemblyDef { registry.get("assemblies", id).unwrap() };
    let craft: CraftDef = registry.get("craft", "titan2_gemini").unwrap();

    // The spacecraft is the craft's root; the launch vehicle hangs under it.
    let spacecraft = assembly("gemini_spacecraft");
    let mut built = CraftDef {
        id: craft.id.clone(),
        name: craft.name.clone(),
        parts: spacecraft.parts.clone(),
        sequences: craft.sequences.clone(),
    };
    built
        .place(&assembly("titan2_glv"), "", "equipment.bottom", None, 0.0)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&built.parts).unwrap(),
        serde_json::to_value(&craft.parts).unwrap()
    );

    let catalog = record::catalog(registry);
    let mut p = Propulsion::from_craft(&craft, &catalog).unwrap();
    let names =
        |p: &Propulsion| -> Vec<String> { p.sections.iter().map(|s| s.name.clone()).collect() };
    assert_eq!(
        names(&p),
        [
            "Titan II first stage",
            "Titan II second stage",
            "Equipment section",
            "Retrograde section",
            "Heat shield",
        ]
    );
    // Gemini, about 3.4 t with its OAMS propellant, on 150 t of storable propellant.
    let spacecraft_mass: f64 = p.sections[2..].iter().map(|s| p.section_mass(s.id)).sum();
    assert!(
        (3_100.0..3_400.0).contains(&spacecraft_mass),
        "{spacecraft_mass}"
    );
    // Liftoff, the second stage lighting through the vented interstage, staging,
    // spacecraft separation, the equipment section, retrofire, then the retrograde
    // section: the heat shield and cabin come home alone.
    let left = [5, 5, 4, 3, 2, 2, 1];
    for (k, &n) in left.iter().enumerate() {
        p.activate_next_sequence().unwrap();
        assert_eq!(
            p.sections.len(),
            n,
            "after sequence {}: {:?}",
            k + 1,
            names(&p)
        );
    }
    assert!(p.sections[0].pod.is_some());
}

/// The radial craft, each with the assemblies of the boosters round its core (by the prefix
/// their parts' ids have), and how many sections it has at liftoff and after each sequence.
const RADIAL: [(&str, &[(&str, &str)], &[usize]); 7] = [
    (
        "sputnik_r7",
        &[
            ("r7_strap_on_8k71ps", "blok_b_"),
            ("r7_strap_on_8k71ps", "blok_v_"),
            ("r7_strap_on_8k71ps", "blok_g_"),
            ("r7_strap_on_8k71ps", "blok_d_"),
        ],
        &[6, 6, 2, 2, 1],
    ),
    (
        "vostok",
        &[
            ("r7_strap_on_8k72k", "blok_b_"),
            ("r7_strap_on_8k72k", "blok_v_"),
            ("r7_strap_on_8k72k", "blok_g_"),
            ("r7_strap_on_8k72k", "blok_d_"),
        ],
        &[8, 8, 4, 4, 3, 3, 2, 1],
    ),
    (
        "ariane5_eca",
        &[("eap_p241", "eap_1_"), ("eap_p241", "eap_2_")],
        &[5, 5, 3, 2, 2, 1],
    ),
    (
        "soyuz_11a511",
        &[
            ("r7_strap_on", "blok_b_"),
            ("r7_strap_on", "blok_v_"),
            ("r7_strap_on", "blok_g_"),
            ("r7_strap_on", "blok_d_"),
        ],
        &[7, 7, 3, 3, 2, 2, 1],
    ),
    (
        "titan3e_centaur",
        &[("ua1205", "ua1205_1_"), ("ua1205", "ua1205_2_")],
        &[7, 7, 7, 5, 4, 3, 2, 2, 1],
    ),
    (
        "delta2_7925",
        &[
            ("gem_40_ground", "gem_1_"),
            ("gem_40_ground", "gem_2_"),
            ("gem_40_air", "gem_3_"),
            ("gem_40_ground", "gem_4_"),
            ("gem_40_ground", "gem_5_"),
            ("gem_40_air", "gem_6_"),
            ("gem_40_ground", "gem_7_"),
            ("gem_40_ground", "gem_8_"),
            ("gem_40_air", "gem_9_"),
        ],
        &[13, 13, 7, 4, 3, 2, 2, 1],
    ),
    ("proton_k", &[], &[4, 4, 3, 2, 2, 1]),
];

#[test]
fn radial_craft_are_boosters_placed_round_a_core() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let catalog = record::catalog(registry);
    for (id, boosters, left) in RADIAL {
        let craft: CraftDef = registry.get("craft", id).unwrap();
        // The craft less its boosters, with each placed where the craft holds it.
        let mut built = CraftDef {
            parts: craft
                .parts
                .iter()
                .filter(|p| !boosters.iter().any(|(_, prefix)| p.id.starts_with(prefix)))
                .cloned()
                .collect(),
            ..craft.clone()
        };
        for (assembly, prefix) in boosters {
            let assembly: AssemblyDef = registry.get("assemblies", assembly).unwrap();
            let held = craft
                .parts
                .iter()
                .find(|p| p.id == format!("{prefix}body"))
                .unwrap();
            let on = held.on.as_deref().unwrap();
            built
                .place(&assembly, prefix, on, held.at, held.roll)
                .unwrap();
        }
        assert_eq!(
            serde_json::to_value(&built.parts).unwrap(),
            serde_json::to_value(&craft.parts).unwrap(),
            "{id}"
        );

        let mut p = Propulsion::from_craft(&craft, &catalog).unwrap();
        let beside = p.sections.iter().filter(|s| s.beside).count();
        assert_eq!(beside, boosters.len(), "{id}");
        let on_axis = |p: &Propulsion| {
            osp_sim::attitude::mass_properties(p)
                .center_of_mass
                .truncate()
        };
        // The boosters balance round the core (a spacecraft's propellant may not, a little).
        assert!(on_axis(&p).length() < 1e-2, "{id}: {}", on_axis(&p));
        let mut counts = vec![p.sections.len()];
        while p.activate_next_sequence().is_some() {
            counts.push(p.sections.len());
            if p.sections.iter().any(|s| s.beside) {
                assert!(on_axis(&p).length() < 1e-2, "{id}: {}", on_axis(&p));
            }
        }
        assert_eq!(counts, left, "{id}");
    }
}

#[test]
fn the_titan_iiie_core_lights_as_its_solids_burn_out() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let craft: CraftDef = registry.get("craft", "titan3e_centaur").unwrap();
    let mut p = Propulsion::from_craft(&craft, &record::catalog(registry)).unwrap();
    let firing = |p: &Propulsion| -> Vec<String> {
        p.firing()
            .iter()
            .map(|&e| {
                p.engines
                    .iter()
                    .find(|x| x.part == e)
                    .unwrap()
                    .def
                    .id
                    .clone()
            })
            .collect()
    };
    p.activate_next_sequence();
    assert_eq!(firing(&p), ["UA1205", "UA1205"]);
    p.activate_next_sequence();
    assert_eq!(firing(&p).len(), 4);
    p.activate_next_sequence();
    assert_eq!(firing(&p), ["LR87-AJ-11", "LR87-AJ-11"]);
}

#[test]
fn proton_burns_its_outer_tanks_with_its_core() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let craft: CraftDef = registry.get("craft", "proton_k").unwrap();
    let mut p = Propulsion::from_craft(&craft, &record::catalog(registry)).unwrap();
    let first = p
        .sections
        .iter()
        .find(|s| s.name == "First Stage")
        .unwrap()
        .id;
    // One oxidizer tank in the middle, six fuel tanks round it, all one stage.
    let tanks: Vec<_> = p.tanks_of(first).collect();
    assert_eq!(tanks.len(), 7);
    let off_axis = tanks
        .iter()
        .filter(|t| p.parts().get(t.part).unwrap().position.truncate().length() > 2.0)
        .count();
    assert_eq!(off_axis, 6);
    // Burning, every RD-253 draws on both, and they drain together.
    p.set_throttle(1.0);
    p.activate_next_sequence();
    for _ in 0..60 {
        p.burn(101_325.0, 1.0, true, &mut Vec::new());
    }
    let share = |name: &str| {
        let (mut held, mut capacity) = (0.0, 0.0);
        for t in p.tanks_of(first) {
            for (c, &a) in t.contents.iter().zip(&t.amounts) {
                if c.resource == name {
                    held += a;
                    capacity += c.capacity;
                }
            }
        }
        held / capacity
    };
    let (nto, udmh) = (share("NTO"), share("UDMH"));
    assert!(nto < 0.75 && (nto - udmh).abs() < 0.02, "{nto} {udmh}");
}

#[test]
fn service_modules_carry_their_contents() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let catalog = record::catalog(registry);
    let assembly = |id: &str| -> AssemblyDef { registry.get("assemblies", id).unwrap() };
    let value = |parts: &[osp_sim::data::CraftPartDef]| serde_json::to_value(parts).unwrap();

    // The Saturn V and IB carry the Apollo CSM as it is, at their top.
    let csm = assembly("apollo_csm");
    for id in ["saturn_v", "saturn_ib"] {
        let craft: CraftDef = registry.get("craft", id).unwrap();
        assert_eq!(
            value(&craft.parts[..csm.parts.len()]),
            value(&csm.parts),
            "{id}"
        );
    }
    // The SPS burns its four tanks down together; the RCS quads have their own.
    let craft: CraftDef = registry.get("craft", "saturn_v").unwrap();
    let mut p = Propulsion::from_craft(&craft, &catalog).unwrap();
    while p.sections.len() > 2 {
        p.activate_next_sequence().unwrap();
    }
    let sm = p
        .sections
        .iter()
        .find(|s| s.name == "Service Module")
        .unwrap()
        .id;
    assert_eq!(p.tanks_of(sm).count(), 7);
    assert_eq!(p.rcs_of(sm).count(), 4);
    p.set_throttle(1.0);
    p.arm(sm);
    p.burn(0.0, 1.0, true, &mut Vec::new());
    for _ in 0..120 {
        p.burn(0.0, 1.0, true, &mut Vec::new());
    }
    let shares: Vec<f64> = p
        .tanks_of(sm)
        .filter(|t| {
            p.parts().get(t.part).unwrap().name.contains("sump")
                || p.parts().get(t.part).unwrap().name.contains("storage")
        })
        .map(|t| t.amounts[0] / t.contents[0].capacity)
        .collect();
    assert_eq!(shares.len(), 4);
    assert!(
        shares
            .iter()
            .all(|&s| s < 0.85 && (s - shares[0]).abs() < 0.02),
        "{shares:?}"
    );

    // The Soyuz 7K-OK stands on its adapter, under a fairing fitted round it.
    let craft: CraftDef = registry.get("craft", "soyuz_11a511").unwrap();
    let soyuz = assembly("soyuz_7k_ok");
    let mut built = CraftDef {
        parts: craft
            .parts
            .iter()
            .filter(|p| !soyuz.parts.iter().any(|q| q.id == p.id))
            .cloned()
            .collect(),
        ..craft.clone()
    };
    built
        .place(&soyuz, "", "soyuz_7k_ok.spacecraft", None, 0.0)
        .unwrap();
    let ids = |parts: &[osp_sim::data::CraftPartDef]| -> Vec<String> {
        let mut ids: Vec<String> = parts.iter().map(|p| p.id.clone()).collect();
        ids.sort();
        ids
    };
    assert_eq!(ids(&built.parts), ids(&craft.parts));
    for part in &soyuz.parts {
        let placed = |parts: &[osp_sim::data::CraftPartDef]| {
            serde_json::to_value(parts.iter().find(|p| p.id == part.id).unwrap()).unwrap()
        };
        assert_eq!(placed(&built.parts), placed(&craft.parts), "{}", part.id);
    }
    let mut refitted = craft.clone();
    refitted.fit_fairings(&catalog).unwrap();
    assert_eq!(value(&refitted.parts), value(&craft.parts));
}
