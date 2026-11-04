//! Craft that are not a stack of bodies: stages with many containers or none, parts that do
//! several things, and stages that hang or stand however they were put on.

use glam::DVec3;

use super::{PartId, PartKind};
use crate::{
    aero::Flow,
    atmosphere::{Atmosphere, AtmosphereModel},
    attitude::mass_properties,
    craft::Craft,
    data::CraftDef,
    propulsion::Propulsion,
    rcs::fire,
    test_data::records,
    vessel::tests::catalog,
};

/// A closed shell, 1 m across and 3 m long, that holds things.
const HOST: &str = r#"{ id: "host", shape: { shell: { bottom_diameter: 1.0,
        sections: [{ length: 3.0 }], cap_bottom: true, cap_top: true } },
    structure: { mass: 120 }, components: [{ container: {} }] }"#;
/// A container of its own, 0.5 m across and 0.6 m long, 8 kg; its id and place follow.
const BOX: &str = r#"shape: { shell: { bottom_diameter: 0.5, sections: [{ length: 0.6 }],
        cap_bottom: true, cap_top: true } },
    structure: { mass: 8 }, components: [{ container: {} }]"#;
/// A ring 0.6 m across and 0.1 m long, 4 kg, that lets go at its top node.
const RING: &str = r#"shape: { ring: { diameter: 0.6, height: 0.1, thickness: 0.3 } },
    structure: { mass: 4 }, components: [{ decoupler: { node: "top" } }]"#;
/// A nitrogen tank with thrusters on it.
const THRUSTER_TANK: &str = r#"shape: { cylinder_tank: { diameter: 0.4, length: 0.8 } },
    components: [
        { storage: { type: "COPV", contents: { Nitrogen: 1 } } },
        { rcs: { id: "Cold Gas Quad" } },
    ]"#;

fn craft(parts: &[String]) -> CraftDef {
    let text = format!(
        r#"{{ id: "free", name: "", parts: [{}] }}"#,
        parts.join(",")
    );
    records(&text).remove(0)
}

fn fly(parts: &[String]) -> Propulsion {
    Propulsion::from_craft(&craft(parts), &catalog()).unwrap()
}

/// What the parts are built to weigh with their propellant, kg.
fn built_mass(parts: &[String]) -> f64 {
    let built = Craft::build(&craft(parts), &catalog()).unwrap();
    built
        .parts
        .iter()
        .map(|part| {
            let propellant: f64 = (part.built.storages())
                .flat_map(|s| &s.contents)
                .map(|c| c.capacity)
                .sum();
            part.built.mass.mass + propellant
        })
        .sum()
}

fn part_named(p: &Propulsion, id: usize) -> PartId {
    p.parts().parts()[id].id
}

#[test]
fn a_stage_holds_any_number_of_containers_and_one_is_its_body() {
    let parts = [
        HOST.to_string(),
        format!(r#"{{ id: "under", on: "host.bottom", by: "top", {BOX} }}"#),
        format!(
            r#"{{ id: "side", on: "host.wall", by: "bottom",
                at: {{ u: 0.25, v: 0.5, standoff: 0.05 }}, {BOX} }}"#
        ),
    ];
    let p = fly(&parts);
    let [section] = p.sections.as_slice() else {
        panic!("one stage, not {}", p.sections.len());
    };
    // The lowest on the axis is its body, and the host stands on it; the one on the side is
    // not its body, whatever it is.
    assert_eq!((section.diameter, section.length), (0.5, 3.6));
    let bodies: Vec<_> = (p.parts().parts().iter())
        .filter(|part| part.body.is_some())
        .map(|part| part.id)
        .collect();
    assert_eq!(bodies, [part_named(&p, 1)]);
    // The others are still where they were put, and weigh.
    assert!((section.dry_mass() - (120.0 + 8.0 + 8.0)).abs() < 1e-9);
    assert!((p.mass() - built_mass(&parts)).abs() < 1e-9);
}

#[test]
fn a_part_with_two_components_does_both_and_weighs_once() {
    let parts = [
        HOST.to_string(),
        format!(
            r#"{{ id: "both", on: "host.wall", by: "bottom",
                at: {{ u: 0.25, v: 0.5, standoff: 0.05 }}, {THRUSTER_TANK} }}"#
        ),
    ];
    let p = fly(&parts);
    assert_eq!((p.tanks.len(), p.rcs.len()), (1, 1));
    assert_eq!(p.tanks[0].part, p.rcs[0].part);
    let dry: f64 = p.sections.iter().map(|s| s.dry_mass()).sum();
    let propellant: f64 = p.tanks.iter().flat_map(|t| &t.amounts).sum();
    assert!((dry + propellant - built_mass(&parts)).abs() < 1e-9);
    assert!((dry - p.parts().dry_mass().mass).abs() < 1e-9);
}

/// Nothing is plumbed to the tank (it stands on a joint that carries no propellant), and yet
/// the thrusters on it draw on it, not on nothing.
#[test]
fn thrusters_on_a_tank_burn_what_it_holds() {
    let parts = [
        HOST.to_string(),
        format!(
            r#"{{ id: "both", on: "host.wall", by: "bottom",
                at: {{ u: 0.25, v: 0.5, standoff: 0.05 }}, {THRUSTER_TANK} }}"#
        ),
    ];
    let mut p = fly(&parts);
    let block = p.rcs[0].part;
    let full = p.reachable(block, "Nitrogen");
    assert!(full > 0.0);
    let (force, _) = fire(&mut p, 1.0, DVec3::ZERO, DVec3::Z, 0.0, 1.0);
    assert!(force.length() > 0.0, "{force}");
    assert!(p.reachable(block, "Nitrogen") < full);
}

/// With no container, the part the joint holds is the stage's body: its nodes give its size,
/// and it leaves when the joint lets go.
#[test]
fn a_ring_on_its_own_is_a_stage_that_lets_go() {
    let parts = [
        HOST.to_string(),
        format!(r#"{{ id: "ring", on: "host.bottom", by: "top", {RING} }}"#),
    ];
    let mut p = fly(&parts);
    assert_eq!(p.sections.len(), 2);
    // In the order they fire: the one hanging under first.
    let ring = &p.sections[0];
    assert_eq!((ring.diameter, ring.length), (0.6, 0.1));
    assert!((ring.dry_mass() - 4.0).abs() < 1e-9);
    assert!(!ring.beside);
    assert_eq!(p.sections[1].name, "host");
    let before = p.parts().base();
    let staged = p.activate_next_sequence().expect("a sequence");
    assert_eq!(staged.released.len(), 1);
    assert!((staged.released[0].mass() - 4.0).abs() < 1e-9);
    assert_eq!(p.sections.len(), 1);
    assert!((p.mass() - 120.0).abs() < 1e-9);
    // The stack's base is now the host's.
    assert!((p.parts().base() - before - 0.1).abs() < 1e-9);
}

/// A small bay on top of a tank does not take the stage up with it: the lowest thing standing
/// on the axis is the body, so the stack still stands on the tank.
#[test]
fn a_stage_stands_on_its_lowest_tank_or_container() {
    let parts = [
        r#"{ id: "tank", shape: { cylinder_tank: { diameter: 1.2, length: 6 } },
            components: [{ storage: { type: "Stiffened Al 2219",
                contents: { Kerosene: 1, LqdOxygen: 2.36 } } }] }"#
            .to_string(),
        format!(r#"{{ id: "bay", on: "tank.top", by: "bottom", {BOX} }}"#),
    ];
    let p = fly(&parts);
    let [section] = p.sections.as_slice() else {
        panic!("one stage, not {}", p.sections.len());
    };
    assert_eq!(section.name, p.parts().parts()[0].name);
    assert!((section.diameter - 1.2).abs() < 1e-9);
    assert!(p.parts().parts()[0].body.is_some());
    assert!(p.parts().parts()[1].body.is_none());
    assert!(p.parts().parts()[1].position.z > 5.9);
    assert!(p.parts().base().abs() < 1e-9);
}

/// A tank with an engine under it has no container at all: the root is the stage's body, and
/// is still a tank.
#[test]
fn a_stage_with_no_container_keeps_what_its_body_does() {
    let parts = [
        r#"{ id: "tank", shape: { cylinder_tank: { diameter: 1.2, length: 6 } },
            components: [{ storage: { type: "Stiffened Al 2219",
                contents: { Kerosene: 1, LqdOxygen: 2.36 } } }] }"#
            .to_string(),
        r#"{ id: "engine", on: "tank.bottom", by: "top", template: "K-1 Booster" }"#.to_string(),
    ];
    let p = fly(&parts);
    let [section] = p.sections.as_slice() else {
        panic!("one stage, not {}", p.sections.len());
    };
    assert_eq!((section.tanks, section.engines), (1, 1));
    assert!((section.diameter - 1.2).abs() < 1e-9);
    assert!(section.length > 6.0 - 1e-9);
    assert!((p.mass() - built_mass(&parts)).abs() < 1e-6);
    assert!(p.reachable(p.engines[0].part, "Kerosene") > 0.0);
}

/// An engine can be a stage: it has no `bottom` node, so its surfaces say how big it is. It
/// is still an engine, and it leaves with the joint that holds it.
#[test]
fn an_engine_on_its_own_is_a_stage() {
    let parts = [
        r#"{ id: "host", shape: { shell: { bottom_diameter: 1.0, sections: [{ length: 3.0 }],
                cap_bottom: true, cap_top: true } },
            structure: { mass: 120 },
            components: [{ container: {} }, { decoupler: { node: "bottom" } }] }"#
            .to_string(),
        r#"{ id: "engine", on: "host.bottom", by: "top", template: "K-0" }"#.to_string(),
    ];
    let mut p = fly(&parts);
    assert_eq!(p.sections.len(), 2);
    let stage = &p.sections[0];
    assert_eq!(stage.engines, 1);
    assert!(p.parts().parts()[1].body.is_some());
    assert!(p.parts().parts()[1].is(|k| matches!(k, PartKind::Engine(_))));
    assert!((p.mass() - built_mass(&parts)).abs() < 1e-9);
    let engine = p.engines[0].part;
    while let Some(staged) = p.activate_next_sequence() {
        if staged
            .released
            .iter()
            .any(|r| r.parts.get(engine).is_some())
        {
            assert!(p.engines.is_empty());
            return;
        }
    }
    panic!("the engine's stage never left");
}

/// Two stages on the axis, one under the host and one on its top, both hang from it: both are
/// there, and both can go. The one on top goes as one beside it does, as it is not under it.
#[test]
fn two_stages_can_hang_on_one_axis() {
    let parts = [
        HOST.to_string(),
        format!(r#"{{ id: "below", on: "host.bottom", by: "top", {RING} }}"#),
        format!(r#"{{ id: "above", on: "host.top", by: "top", {RING} }}"#),
    ];
    let mut p = fly(&parts);
    assert_eq!(p.sections.len(), 3);
    let hung: Vec<_> = (p.sections.iter()).filter(|s| s.parent.is_some()).collect();
    assert_eq!(hung.len(), 2);
    assert_eq!(hung.iter().filter(|s| s.beside).count(), 1);
    assert!((p.mass() - 128.0).abs() < 1e-9);
    let mut released = 0;
    while let Some(staged) = p.activate_next_sequence() {
        released += staged.released.len();
    }
    assert_eq!(released, 2);
    assert!((p.mass() - 120.0).abs() < 1e-9);
}

/// A stage that stands at an angle is a stage: it weighs where it is, hangs beside the host,
/// and goes.
#[test]
fn a_tilted_stage_hangs_beside_and_lets_go() {
    let parts = [
        HOST.to_string(),
        format!(
            r#"{{ id: "pod", on: "host.wall", by: "side",
                at: {{ u: 0.0, v: 0.5, standoff: 0.05 }},
                shape: {{ shell: {{ bottom_diameter: 0.5, sections: [{{ length: 0.6 }}],
                    cap_bottom: true, cap_top: true }} }},
                structure: {{ mass: 8 }},
                components: [{{ container: {{}} }}, {{ decoupler: {{ node: "side" }} }}],
                nodes: [{{ name: "side", position: [0, 0, 0.3], direction: [1, 0, 1] }}] }}"#
        ),
    ];
    let mut p = fly(&parts);
    assert_eq!(p.sections.len(), 2);
    let pod = p.parts().parts()[1].clone();
    assert!(pod.body.is_some());
    assert!((pod.rotation * DVec3::Z).z < 0.9, "{:?}", pod.rotation);
    assert!(p.sections.iter().any(|s| s.beside));
    let expected = p.parts().mass_of(&pod);
    assert!((expected.mass - 8.0).abs() < 1e-9);
    assert!((p.parts().dry_mass().mass - 128.0).abs() < 1e-9);
    let staged = p.activate_next_sequence().expect("a sequence");
    assert_eq!(staged.released.len(), 1);
    assert!((p.mass() - 120.0).abs() < 1e-9);
}

/// A panel that stands off the stage's axis is no half of a fairing: it is a part of its own
/// that lets go, a stage whose body it is.
#[test]
fn a_panel_off_the_axis_is_a_stage_not_a_fairing() {
    let parts = [
        HOST.to_string(),
        r#"{ id: "panel", on: "host.wall", by: "bottom",
            at: { u: 0.0, v: 0.5, standoff: 0.05 },
            shape: { shell: { bottom_diameter: 1.0, sections: [{ length: 1.0 }], span: [-90, 90] } },
            structure: { mass: 10 }, components: [{ decoupler: { node: "bottom" } }] }"#
            .to_string(),
    ];
    let mut p = fly(&parts);
    assert!(p.sections.iter().all(|s| s.fairing.is_none()));
    assert_eq!(p.sections.len(), 2);
    assert!((p.mass() - 130.0).abs() < 1e-9);
    let staged = p.activate_next_sequence().expect("a sequence");
    assert_eq!(staged.released.len(), 1);
    assert!((p.mass() - 120.0).abs() < 1e-9);
}

/// A common bulkhead makes one part two tanks, each in its own cell: the part weighs once, and
/// what each cell holds is in that cell, the lower one's low and the upper one's high.
#[test]
fn a_common_bulkhead_is_two_tanks_in_their_own_cells() {
    let parts = [r#"{ id: "pair",
        shape: { cylinder_tank: { diameter: 1.2, length: 6, common_bulkhead: 2.0 } },
        components: [
            { storage: { cell: "lower", type: "Stiffened Al 2219", contents: { LqdOxygen: 1 } } },
            { storage: { cell: "upper", type: "Stiffened Al 2219", contents: { Kerosene: 1 } } },
        ] }"#
        .to_string()];
    let mut p = fly(&parts);
    assert_eq!(p.tanks.len(), 2);
    assert_eq!(p.tanks[1].dry_mass, 0.0);
    assert!((p.mass() - built_mass(&parts)).abs() < 1e-9);
    assert_eq!(mass_properties(&p).mass, p.mass());
    assert!(p.tanks[0].cell.is_some() && p.tanks[1].cell.is_some());
    // Lying one above the other.
    assert!(p.tanks[0].base < p.tanks[1].base);
    assert!(p.tanks[0].base + p.tanks[0].length <= p.tanks[1].base + 0.5);
    // Drain the upper one: the centre of mass falls towards the lower.
    let full = mass_properties(&p).center_of_mass.z;
    let upper = p
        .tanks
        .iter_mut()
        .find(|t| t.contents[0].resource == "Kerosene");
    upper.unwrap().amounts[0] = 0.0;
    let drained = mass_properties(&p).center_of_mass.z;
    assert!(drained < full - 0.1, "{drained} of {full}");
}

/// Skirts leave the cell where it was: the contents still sit in it.
#[test]
fn a_tank_with_skirts_settles_in_its_cell() {
    let parts = [r#"{ id: "skirted",
        shape: { cylinder_tank: { diameter: 1.2, length: 6, aft_skirt: 1.0, forward_skirt: 1.0 } },
        components: [{ storage: { type: "Stiffened Al 2219", contents: { Kerosene: 1 } } }] }"#
        .to_string()];
    let p = fly(&parts);
    let [tank] = p.tanks.as_slice() else {
        panic!("one tank, not {}", p.tanks.len());
    };
    assert!(tank.cell.is_some());
    assert!((p.mass() - built_mass(&parts)).abs() < 1e-9);
    let center = mass_properties(&p).center_of_mass;
    assert!(
        center.z > tank.base && center.z < tank.base + tank.length,
        "{center}"
    );
}

/// A cell turned on its side pools low in the vessel, like a plain tank.
#[test]
fn a_cell_on_its_side_pools_low() {
    let parts = [
        HOST.to_string(),
        r#"{ id: "side", on: "host.seat", by: "cant",
            shape: { cylinder_tank: { diameter: 0.4, length: 1.6, aft_skirt: 0.1 } },
            nodes: [{ name: "cant", position: [0, 0, 0.8], direction: [1, 0, 0] }],
            components: [{ storage: { type: "Stiffened Al 2219", contents: { Kerosene: 1 } } }] }"#
            .to_string(),
    ];
    let host = parts[0].replacen(
        "components: [{ container: {} }]",
        r#"components: [{ container: {} }],
           nodes: [{ name: "seat", position: [0, 0, 1.5], direction: [0, 0, 1] }]"#,
        1,
    );
    let parts = [host, parts[1].clone()];
    let mut p = fly(&parts);
    let tank = p.tanks[0].clone();
    assert!(tank.cell.is_some() && tank.max_axial_load.is_none());
    let (position, rotation) = {
        let part = p.parts().get(tank.part).unwrap();
        (part.position, part.rotation)
    };
    assert!((rotation * DVec3::Z).z.abs() < 1e-9);
    // Half full, the liquid's centre lies under the axis of the tank.
    p.tanks[0].amounts[0] = tank.contents[0].capacity / 2.0;
    let half = mass_properties(&p);
    p.tanks[0].amounts[0] = 0.0;
    let empty = mass_properties(&p);
    let liquid = tank.contents[0].capacity / 2.0;
    let at = (half.center_of_mass * half.mass - empty.center_of_mass * empty.mass) / liquid;
    let axis = position.z - p.parts().base();
    assert!(at.z < axis - 0.02, "{} under {axis}", at.z);
}

/// One plate is no set of fins, and neither is an uneven pair: they fly as wings do.
#[test]
fn fins_that_make_no_set_are_wings() {
    const PLATE: &str = r#"shape: { wing: { root_chord: 0.6, tip_chord: 0.3, span: 0.5,
            airfoil: { plate: { thickness: 0.01 } } } },
        structure: { material: "Al 2219-T87" }, components: [{ lifting_surface: {} }]"#;
    let parts = [
        HOST.to_string(),
        format!(
            r#"{{ id: "fin", on: "host.wall", by: "root",
                at: {{ u: 0.0, v: 0.5, standoff: 0.0 }}, {PLATE} }}"#
        ),
    ];
    let p = fly(&parts);
    let section = &p.sections[0];
    assert!(section.fins.is_none());
    assert_eq!(section.wings.len(), 1);
    let thin = section.wings[0].thickness;
    assert!(thin > 0.0 && thin < 0.1, "{thin}");
    assert!((p.mass() - built_mass(&parts)).abs() < 1e-9);
}

/// Camber and twist lift at no angle of attack, the way turning the wing does.
#[test]
fn a_cambered_or_twisted_wing_lifts_with_the_flow_straight() {
    let wing = |airfoil: &str, twist: f64| {
        let text = format!(
            r#"{{ id: "w", shape: {{ wing: {{ root_chord: 1, tip_chord: 1, span: 4, twist: {twist},
                airfoil: {{ naca: {{ thickness: 0.12, {airfoil} }} }} }} }},
              structure: {{ mass: 10 }}, components: [{{ lifting_surface: {{}} }}] }}"#
        );
        let p = fly(&[text]);
        let air = AtmosphereModel::Us1976;
        let atmosphere = Atmosphere {
            model: air,
            height: 140_000.0,
        };
        let sample = atmosphere.sample(0.0);
        let shape = p.shape();
        let flow = Flow {
            velocity: DVec3::Z * 60.0,
            angular_velocity: DVec3::ZERO,
            air: sample,
            plume: false,
            controls: DVec3::ZERO,
        };
        shape.loads_about(&flow, DVec3::ZERO).force
    };
    let plain = wing("", 0.0);
    let cambered = wing("camber: 0.04", 0.0);
    let twisted = wing("", 4.0);
    // Straight into the flow an untwisted, uncambered wing makes only drag along the flow.
    assert!(plain.truncate().length() < 1.0, "{plain}");
    assert!(cambered.truncate().length() > 100.0, "{cambered}");
    assert!(twisted.truncate().length() > 100.0, "{twisted}");
}

/// A panel with a tapering barrel cannot stand round its stage as a fairing does: it is a part
/// of its own that lets go.
#[test]
fn a_tapering_panel_is_no_fairing() {
    let parts = [
        r#"{ id: "host", shape: { shell: { bottom_diameter: 1.0, sections: [{ length: 3.0 }],
                cap_bottom: true, cap_top: true } },
            structure: { mass: 120 }, components: [{ container: {} }],
            nodes: [{ name: "rim", position: [0.5, 0, 0], direction: [0, 0, 1], size: 1.0 }] }"#
            .to_string(),
        r#"{ id: "panel", on: "host.rim",
            shape: { shell: { bottom_diameter: 1.0, span: [-90, 90],
                sections: [{ length: 1.0, top_diameter: 0.5 }] } },
            structure: { mass: 10 }, components: [{ decoupler: { node: "bottom" } }] }"#
            .to_string(),
    ];
    let p = fly(&parts);
    assert!(p.sections.iter().all(|s| s.fairing.is_none()));
    assert_eq!(p.sections.len(), 2);
}
