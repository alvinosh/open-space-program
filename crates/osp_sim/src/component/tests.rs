//! Parts built from shapes and components, checked against shell theory and against what
//! today's part kinds weigh.

use std::f64::consts::{FRAC_1_SQRT_2, PI};

use glam::DVec3;

use super::*;
use crate::{
    aero::Panel,
    assembly::{Assembly, End, Joint, Mount, Side},
    data::{FinDef, GearDef, ShellDef as StageShellDef, WingDef as WingRecord},
    gear::Leg,
    shell::Shell,
    structure::FinSet,
    test_data::records,
    vessel::tests::catalog as stock,
};

/// The stock records, and a tank type whose walls are sized by their pressure alone: no
/// hydrostatic head, no minimum gauge and nothing on top of the bare wall.
fn catalog() -> PartCatalog {
    let mut catalog = stock();
    catalog.tank_types.push(
        records(
            "{ id: 'Test', material: 'Al 2219-T87', pressure: 3e5, min_gauge: 1e-7, \
             utilization: 1, design_load: 0 }",
        )
        .remove(0),
    );
    catalog
}

const STRENGTH: f64 = 393e6;
const DENSITY: f64 = 2840.0;
const PRESSURE: f64 = 3e5;
const SAFETY: f64 = 1.25;

/// The wall a test tank needs where its stress resultant over its pressure is `per`, m.
fn membrane(per: f64) -> f64 {
    SAFETY * PRESSURE * per / STRENGTH
}

fn def(text: &str) -> PartDef {
    records(text).remove(0)
}

#[track_caller]
fn built(text: &str) -> BuiltPart {
    def(text)
        .build(&catalog())
        .unwrap_or_else(|e| panic!("{e}"))
}

#[track_caller]
fn refused(text: &str) -> String {
    def(text)
        .build(&catalog())
        .expect_err("should be refused")
        .to_string()
}

#[track_caller]
fn piece<'a>(part: &'a BuiltPart, name: &str) -> &'a Piece {
    part.pieces
        .iter()
        .find(|p| p.element == name)
        .unwrap_or_else(|| {
            let names: Vec<_> = part.pieces.iter().map(|p| &p.element).collect();
            panic!("no `{name}` among {names:?}")
        })
}

#[track_caller]
fn assert_close(a: f64, b: f64, tolerance: f64) {
    assert!((a - b).abs() <= tolerance * b.abs().max(1e-9), "{a} vs {b}");
}

#[track_caller]
fn assert_near(a: DVec3, b: DVec3) {
    assert!((a - b).length() < 1e-9, "{a} vs {b}");
}

#[track_caller]
fn assert_same_mass(a: Mass, b: Mass, tolerance: f64) {
    assert_close(a.mass, b.mass, tolerance);
    assert!(
        (a.center - b.center).length() <= tolerance * b.center.length().max(1.0),
        "centre {} vs {}",
        a.center,
        b.center
    );
    let size = b
        .inertia
        .to_cols_array()
        .iter()
        .fold(0.0f64, |m, x| m.max(x.abs()));
    let off = (a.inertia - b.inertia).to_cols_array();
    assert!(
        off.iter().all(|x| x.abs() <= tolerance * size.max(1e-9)),
        "inertia {:?} vs {:?}",
        a.inertia,
        b.inertia
    );
}

fn tank(shape: &str) -> String {
    format!(
        "{{ shape: {shape}, components: [{{ storage: {{ type: 'Test', contents: {{ Kerosene: 1 }} \
         }} }}] }}"
    )
}

#[test]
fn tank_walls_follow_membrane_theory() {
    // A barrel's hoops carry p r; a √2:1 dome carries p r / √2, at its pole, and nothing
    // round its rim.
    let cylinder = built(&tank("{ cylinder_tank: { diameter: 3, length: 8 } }"));
    assert_close(piece(&cylinder, "barrel").thickness, membrane(1.5), 1e-9);
    for dome in ["aft_dome", "forward_dome"] {
        assert_close(
            piece(&cylinder, dome).thickness,
            membrane(1.5 * FRAC_1_SQRT_2),
            1e-6,
        );
    }
    // A sphere carries p r / 2 all over.
    let sphere = built(&tank("{ sphere: { diameter: 3 } }"));
    assert_close(piece(&sphere, "wall").thickness, membrane(0.75), 1e-6);
    // A cone's hoops carry p r / cos α, most at its wide end.
    let cone = built(&tank(
        "{ cone_tank: { length: 6, bottom_diameter: 3, top_diameter: 1, dome: 0.5 } }",
    ));
    let cos = 5.0 / 26f64.sqrt();
    assert_close(piece(&cone, "barrel").thickness, membrane(1.5 / cos), 1e-9);
    // A torus balances at its crowns, so its meridians carry most at the inner equator:
    // p a (2R − a) / (2 (R − a)), with a the tube's radius and R its centre's.
    let torus = built(&tank("{ torus: { diameter: 4, tube_diameter: 1 } }"));
    let (a, big) = (0.5, 1.5);
    let inner = a * (2.0 * big - a) / (2.0 * (big - a));
    assert_close(piece(&torus, "wall").thickness, membrane(inner), 1e-6);
    // Built for its pressure, each yields at the safety factor times it; a torus and a
    // sphere carry no stack.
    for part in [&cylinder, &sphere, &cone, &torus] {
        let storage = part.storages().next().unwrap();
        assert_close(storage.max_pressure, SAFETY * PRESSURE, 1e-6);
        let wall: f64 = part.pieces.iter().map(|p| p.mass.mass).sum();
        assert_close(part.mass.mass, wall, 1e-12);
    }
    assert!(sphere.storages().next().unwrap().max_axial_load.is_none());
    assert!(torus.storages().next().unwrap().max_axial_load.is_none());
    // The barrel's wall weighs its area by thickness and density.
    let barrel = piece(&cylinder, "barrel");
    let area = 2.0 * PI * 1.5 * (8.0 - 2.0 * 1.5 * FRAC_1_SQRT_2);
    assert_close(barrel.mass.mass, area * barrel.thickness * DENSITY, 1e-9);
}

#[test]
fn flat_walls_bend_as_plates() {
    // A flat end is a disc of radius a held at its rim: σ = 1.24 p a² / t².
    let flat = built(&tank(
        "{ cylinder_tank: { diameter: 2, length: 4, dome: 0 } }",
    ));
    let disc = (3.0 * 3.3 / 8.0 * SAFETY * PRESSURE / STRENGTH).sqrt();
    assert_close(piece(&flat, "aft_dome").thickness, disc, 1e-9);
    assert_close(piece(&flat, "barrel").thickness, membrane(1.0), 1e-9);
    // A box tank's skin is panels across its narrower side: σ = 0.75 p b² / t².
    let boxed = built(&tank("{ box: { size: [1, 2, 3], fill: 'cell' } }"));
    let strip = (0.75 * SAFETY * PRESSURE / STRENGTH).sqrt();
    assert_close(piece(&boxed, "box/skin").thickness, strip, 1e-9);
    assert_close(boxed.storages().next().unwrap().volume, 6.0, 1e-9);
    // A sector tank's flat sides are plates too.
    let sector = built(&tank(
        "{ revolved: { span: [0, 90], walls: [{ name: 'wall', curve: { start: [0, 0], \
         segments: [{ line: [1, 0] }, { line: [1, 2] }, { line: [0, 2] }] } }], \
         regions: [{ name: 'tank', fill: 'cell', boundary: { start: [0, 0], \
         segments: [{ line: [1, 0] }, { line: [1, 2] }, { line: [0, 2] }] } }] } }",
    ));
    assert!(piece(&sector, "tank/sides").thickness > 0.0);
}

#[test]
fn the_tallest_column_sets_the_design_pressure() {
    // Full of kerosene alone, the column is the whole tank.
    let text = |contents: &str| {
        format!(
            "{{ shape: {{ cylinder_tank: {{ diameter: 2, length: 10 }} }}, components: \
             [{{ storage: {{ type: 'Test', design_load: 3, contents: {contents} }} }}] }}"
        )
    };
    let full = built(&text("{ Kerosene: 1 }"));
    let storage = full.storages().next().unwrap();
    assert_close(storage.contents[0].column, 10.0, 1e-9);
    assert_close(
        storage.design_pressure,
        PRESSURE + 820.0 * 3.0 * crate::constants::G0 * 10.0,
        1e-12,
    );
    // Kerolox: each resource stands as high as its share would at the bottom of the tank,
    // and the oxygen's column, taller and denser, decides.
    let kerolox = built(&text("{ Kerosene: 1, LqdOxygen: 2.36 }"));
    let storage = kerolox.storages().next().unwrap();
    for content in &storage.contents {
        let own = content.capacity / content.density;
        let settled = kerolox.geometry.settle("tank", DVec3::Z, own).unwrap();
        assert_close(settled.moments.measure, own, 1e-9);
        assert_close(settled.level, content.column, 1e-9);
    }
    let oxygen = &storage.contents[1];
    let head = oxygen.density * 3.0 * crate::constants::G0 * oxygen.column;
    assert_close(storage.design_pressure, PRESSURE + head, 1e-12);
    // It drains as it empties.
    let half = storage.pressure_at(3.0 * crate::constants::G0, |_| 0.5);
    assert_close(half, PRESSURE + head / 2.0, 1e-12);
}

#[test]
fn a_common_bulkhead_serves_both_its_cells() {
    let text = |lower: &str, upper: &str| {
        format!(
            "{{ shape: {{ cylinder_tank: {{ diameter: 3, length: 10, common_bulkhead: 4 }} }}, \
             components: [{{ storage: {{ {lower} type: 'Test', contents: {{ LqdOxygen: 1 }} }} }}, \
             {{ storage: {{ {upper} type: 'Test', contents: {{ Kerosene: 1 }} }} }}] }}"
        )
    };
    let parted = built(&text("cell: 'lower',", "cell: 'upper',"));
    let storages: Vec<_> = parted.storages().collect();
    assert_eq!(storages[0].walls, ["aft_dome", "barrel", "bulkhead"]);
    assert_eq!(storages[1].walls, ["barrel", "forward_dome", "bulkhead"]);
    // Each wall is built once, and the two cells hold what the whole tank would.
    let mut names: Vec<_> = parted.pieces.iter().map(|p| p.element.as_str()).collect();
    names.sort();
    assert_eq!(names, ["aft_dome", "barrel", "bulkhead", "forward_dome"]);
    let whole = built(&tank("{ cylinder_tank: { diameter: 3, length: 10 } }"));
    let volume = whole.storages().next().unwrap().volume;
    assert_close(storages[0].volume + storages[1].volume, volume, 1e-9);
    for storage in &storages {
        assert!(storage.max_pressure >= SAFETY * PRESSURE * (1.0 - 1e-9));
    }
    // One cell, one storage; and with two cells, say which.
    assert!(refused(&text("cell: 'lower',", "cell: 'lower',")).contains("two storages"));
    assert!(refused(&text("", "cell: 'upper',")).contains("several cells"));
}

#[test]
fn cells_must_be_walled_in() {
    // A tank whose top has no wall.
    let open = refused(&tank(
        "{ revolved: { walls: [{ name: 'side', curve: { start: [0, 0], \
         segments: [{ line: [1, 0] }, { line: [1, 2] }] } }], \
         regions: [{ name: 'tank', fill: 'cell', boundary: { start: [0, 0], \
         segments: [{ line: [1, 0] }, { line: [1, 2] }, { line: [0, 2] }] } }] } }",
    ));
    assert!(open.contains("open"), "{open}");
    assert!(refused(&tank("{ ring: { diameter: 2, height: 1 } }")).contains("no cell"));
}

#[test]
fn structure_weighs_as_its_material_or_spreads_its_mass() {
    // An interstage: a 3 m aluminium tube, 2 m long and 3 mm thick.
    let shell = "{ shell: { bottom_diameter: 3, sections: [{ length: 2 }] } }";
    let metal = built(&format!(
        "{{ shape: {shell}, structure: {{ material: 'Al 2219-T87', thickness: 0.003 }} }}"
    ));
    let tube = Mass::tube(2.0 * PI * 1.5 * 2.0 * 0.003 * DENSITY, 1.5, 2.0);
    assert_same_mass(metal.mass, tube, 1e-9);
    assert_eq!(piece(&metal, "wall").thickness, 0.003);
    // Given only its mass, it spreads it the same way.
    let given = built(&format!("{{ shape: {shell}, structure: {{ mass: 500 }} }}"));
    assert_same_mass(given.mass, Mass::tube(500.0, 1.5, 2.0), 1e-9);
    // A solid ring weighs its volume, a truss its members; a cell nothing.
    let ring = built(
        "{ shape: { ring: { diameter: 2, height: 0.1, thickness: 0.05 } }, \
         structure: { material: 'Al 2219-T87' } }",
    );
    let annulus = PI * (1.0 - 0.95f64.powi(2)) * 0.1;
    assert_close(ring.mass.mass, annulus * DENSITY, 1e-9);
    let truss = built(
        "{ shape: { truss: { bottom_diameter: 2, length: 1, around: 4, bracing: 'none', \
         rings: false } }, structure: { material: 'Al 2219-T87', member_area: 1e-3 } }",
    );
    assert_close(truss.mass.mass, 4.0 * 1e-3 * DENSITY, 1e-9);
    let empty = built(
        "{ shape: { cylinder_tank: { diameter: 1, length: 2 } }, \
         structure: { material: 'Al 2219-T87', thickness: 0.001 } }",
    );
    assert!(empty.pieces.iter().all(|p| p.element != "tank"));
    // Walls of a material need a thickness.
    let thin = refused(&format!(
        "{{ shape: {shell}, structure: {{ material: 'Al 2219-T87' }} }}"
    ));
    assert!(thin.contains("thickness"), "{thin}");
    // A wing is solid through; its skin is the solid's surface, not a wall as well.
    let wing = built(
        "{ shape: { wing: { root_chord: 2, tip_chord: 1, span: 3 } }, \
         structure: { material: 'Al 2219-T87', solidity: 0.1 } }",
    );
    assert_eq!(wing.pieces.len(), 1);
}

#[test]
fn templates_weigh_what_their_records_say() {
    let catalog = catalog();
    // An engine's mass sits at its gimbal pivot.
    for engine in &catalog.engines {
        let part = PartDef::engine(engine).unwrap().build(&catalog).unwrap();
        assert_same_mass(part.mass, Mass::point(engine.mass, DVec3::ZERO), 1e-12);
        assert!(
            part.assembly_part("e").node("top").is_some(),
            "{}",
            engine.id
        );
        assert!(matches!(&part.components[0], Component::Engine(e) if e.id == engine.id));
    }
    // Its nodes come scaled with its model.
    let mut half = catalog.engines[0].clone();
    half.model_scale = 0.5;
    half.nodes = vec![node("top", [0.0, 0.0, 0.4], [0.0, 0.0, 1.0], 1.0)];
    let part = PartDef::engine(&half).unwrap().build(&catalog).unwrap();
    let top = part.geometry.node("top").unwrap();
    assert_near(top.position, DVec3::Z * 0.2);
    assert_eq!(top.size, 0.5);
    // A pod is a solid frustum to the physics, as today.
    for pod in &catalog.pods {
        let part = PartDef::pod(pod).build(&catalog).unwrap();
        let frustum = Mass::frustum(
            pod.mass,
            pod.diameter / 2.0,
            pod.top_diameter() / 2.0,
            pod.height,
        );
        assert_same_mass(part.mass, frustum, 1e-9);
        assert!(
            part.components
                .iter()
                .any(|c| matches!(c, Component::Crew { seats } if *seats == pod.crew))
        );
        let joined = part.assembly_part("pod");
        assert!(joined.node("bottom").is_some() && joined.node("top").is_some());
    }
    for block in &catalog.rcs {
        let part = PartDef::rcs(block).build(&catalog).unwrap();
        assert_same_mass(part.mass, Mass::point(block.mass, DVec3::ZERO), 1e-12);
        assert_near(part.geometry.node("mount").unwrap().direction, DVec3::NEG_X);
    }
    for wheel in &catalog.reaction_wheels {
        let part = PartDef::reaction_wheel(wheel).build(&catalog).unwrap();
        assert_same_mass(part.mass, Mass::point(wheel.mass, DVec3::ZERO), 1e-12);
    }
}

#[test]
fn decouplers_let_go_and_containers_hold_things_inside() {
    // A separation ring lets go of whatever stands on it, which keeps the ring below.
    let ring = built(
        "{ shape: { ring: { diameter: 3, height: 0.1 } }, \
         components: [{ decoupler: { node: 'top', impulse: 400 } }] }",
    );
    let joined = ring.assembly_part("separation");
    assert!(joined.releases.contains("top") && !joined.releases.contains("bottom"));
    let top = Mount::Node("top".into());
    assert_eq!(
        joined.carries(&top),
        Carries {
            fluid: false,
            ..Carries::NODE
        }
    );
    assert_eq!(joined.carries(&Mount::Node("bottom".into())), Carries::NODE);
    let crossfeed = built(
        "{ shape: { ring: { diameter: 3, height: 0.1 } }, \
         components: [{ decoupler: { node: 'top', crossfeed: true } }] }",
    );
    assert_eq!(crossfeed.assembly_part("x").carries(&top), Carries::NODE);
    let nowhere = refused(
        "{ shape: { ring: { diameter: 3, height: 0.1 } }, \
         components: [{ decoupler: { node: 'side' } }] }",
    );
    assert!(nowhere.contains("no node `side`"), "{nowhere}");

    // An equipment section: a fuel bottle on the inside of its wall, a battery on its floor.
    let section = |container: &str| {
        built(&format!(
            "{{ shape: {{ shell: {{ bottom_diameter: 3, sections: [{{ length: 1.4, \
             top_diameter: 2.3 }}] }} }}, components: [{container}] }}"
        ))
    };
    let equipment = section(
        "{ container: { nodes: [{ name: 'floor', position: [0, 0, 0.1], \
         direction: [0, 0, 1], size: 1 }] } }",
    );
    let bottle = built(&tank("{ sphere: { diameter: 0.7 } }"));
    let battery = built(
        "{ shape: { box: { size: [0.3, 0.3, 0.2] } }, components: [{ battery: { \
         technology: 'Silver-zinc (Mercury, Vostok)' } }] }",
    );
    let inside = Mount::Surface {
        surface: "wall".into(),
        side: Side::Back,
        u: 0.25,
        v: 0.4,
        standoff: 0.0,
    };
    let end = |part, mount: Mount| End { part, mount };
    let bottom = || Mount::Node("bottom".into());
    let joints = vec![
        Joint::new(end(0, inside.clone()), end(1, bottom())),
        Joint::new(end(0, Mount::Node("floor".into())), end(2, bottom())),
    ];
    let parts = |equipment: &BuiltPart| {
        vec![
            equipment.assembly_part("equipment"),
            bottle.assembly_part("bottle"),
            battery.assembly_part("battery"),
        ]
    };
    let assembly = Assembly::new(parts(&equipment), joints.clone(), 0).unwrap();
    let poses = assembly.poses();
    // The bottle hangs inwards from the wall, the battery stands on the floor.
    let wall = equipment.geometry.surface_point("wall", 0.25, 0.4).unwrap();
    assert_near(
        poses[1].point(DVec3::Z * 0.35),
        wall.position - wall.normal * 0.35,
    );
    assert_near(poses[2].position, DVec3::Z * 0.1);
    // Nothing mounts inside what is no container.
    let plain = section("{ crew: { seats: 1 } }");
    let err = Assembly::new(parts(&plain), joints, 0).unwrap_err();
    assert!(err.contains("not a container"), "{err}");
}

#[test]
fn a_battery_fills_its_solid() {
    let battery = built(
        "{ shape: { box: { size: [0.2, 0.3, 0.4] } }, components: [{ battery: { \
         technology: 'Silver-zinc (Mercury, Vostok)' } }] }",
    );
    // 277.8 Wh/L and 96.1 Wh/kg.
    let litres = 0.2 * 0.3 * 0.4 * 1e3;
    let Component::Battery(b) = &battery.components[0] else {
        panic!("a battery");
    };
    assert_close(b.charge, litres * 277.8 * 3.6, 1e-9);
    assert_close(battery.mass.mass, litres * 277.8 / 96.1, 1e-9);
    // Its structure has nothing left to weigh.
    assert_eq!(battery.pieces.len(), 1);
    // A pod's charge weighs nothing of its own: the pod's mass counts it.
    let charged = built(
        "{ shape: { sphere: { diameter: 1 } }, components: [{ battery: { charge: 5000 } }] }",
    );
    assert_eq!(charged.mass.mass, 0.0);
    let both = refused(
        "{ shape: { box: { size: [1, 1, 1] } }, components: [{ battery: { charge: 5, \
         technology: 'Silver-zinc (Mercury, Vostok)' } }] }",
    );
    assert!(both.contains("charge"), "{both}");
}

#[test]
fn a_wing_lays_out_the_panels_the_air_flies() {
    // Today's mirrored wing record, on a hull of no width.
    let record: WingRecord = records(
        "{ name: 'Wing', position: 0, root_chord: 3, tip_chord: 1, span: 5, sweep: 2, \
         dihedral: 6, mass: 400, mirror: true }",
    )
    .remove(0);
    let panels = Panel::of(&record, 0.0, 0.0);
    // As parts: a right wing, and the left one mirrored, each half the mass. The record's
    // span runs along its dihedral; the shape's runs straight out.
    let wing = format!(
        "{{ wing: {{ root_chord: 3, tip_chord: 1, span: {}, sweep: 2, dihedral: 6 }} }}",
        5.0 * 6f64.to_radians().cos()
    );
    for (k, panel) in panels.iter().enumerate() {
        let shape = if k == 0 {
            wing.clone()
        } else {
            format!("{{ mirrored: {wing} }}")
        };
        let part = built(&format!(
            "{{ shape: {shape}, structure: {{ mass: 200 }}, components: [{{ lifting_surface: {{}} }}] }}"
        ));
        assert_close(part.mass.mass, 200.0, 1e-12);
        let Component::LiftingSurface(surface) = &part.components[0] else {
            panic!("a lifting surface");
        };
        let p = surface.panels[0];
        assert_near(p.root, panel.root);
        assert_near(p.tip, panel.tip);
        assert_close(p.root_chord, panel.root_chord, 1e-12);
        assert_close(p.tip_chord, panel.tip_chord, 1e-12);
        assert_near(p.normal, panel.normal);
        assert_near(p.chord, DVec3::NEG_Z);
        assert_close(p.thickness[0], 0.12, 1e-3);
        assert_close(p.area(), panel.area(), 1e-9);
    }
    let lifeless =
        refused("{ shape: { sphere: { diameter: 1 } }, components: [{ lifting_surface: {} }] }");
    assert!(lifeless.contains("loft"), "{lifeless}");
}

#[test]
fn todays_part_kinds_weigh_the_same_as_components() {
    let catalog = catalog();
    // A stage's hull is a solid cylinder of its structure.
    let hull = built(
        "{ shape: { ring: { diameter: 3, height: 12, thickness: 1.5 } }, structure: { mass: 2500 } }",
    );
    assert_same_mass(hull.mass, Mass::cylinder(2500.0, 1.5, 12.0), 1e-9);
    // Fins: each a plate wing of solid metal, bevelled as the fin set's are.
    let fins: FinDef = records(
        "{ count: 4, root_chord: 2, tip_chord: 1, span: 1, sweep: 0.8, thickness: 0.01, \
         material: 'Al 2219-T87' }",
    )
    .remove(0);
    let set = FinSet::new(&fins, &catalog).unwrap();
    let fin = built(
        "{ shape: { wing: { root_chord: 2, tip_chord: 1, span: 1, sweep: 0.8, \
         airfoil: { plate: { thickness: 0.01 } } } }, structure: { material: 'Al 2219-T87' }, \
         components: [{ lifting_surface: {} }] }",
    );
    assert_close(4.0 * fin.mass.mass, set.mass, 1e-9);
    // A shell spread its mass over its wall as an outline of 24 points; the geometry
    // integrates the curve itself.
    let nose: StageShellDef =
        records("{ length: 4, top_diameter: 0, profile: 'ogive', mass: 300 }").remove(0);
    let old = Shell::new(&nose, 0.0, 1.5).unwrap();
    let outline: Vec<_> = old
        .profile(24)
        .iter()
        .map(|&(z, r)| (z - old.base, r))
        .collect();
    let new = built(
        "{ shape: { shell: { bottom_diameter: 3, sections: [{ length: 4, top_diameter: 0, \
         profile: 'ogive' }] } }, structure: { mass: 300 } }",
    );
    assert_same_mass(new.mass, Mass::revolved_shell(300.0, &outline), 2e-3);
    // A fairing half is a shell over half a turn that lets go of its base. Its nose's mass
    // was spread along its barrel; here it sits on the nose.
    let half = built(
        "{ shape: { shell: { bottom_diameter: 3, sections: [{ length: 6 }], span: [-90, 90] } }, \
         structure: { mass: 400 }, components: [{ decoupler: { node: 'bottom' } }] }",
    );
    assert_same_mass(half.mass, Mass::half_tube(400.0, 1.5, 6.0), 1e-9);
    assert!(half.assembly_part("half").releases.contains("bottom"));
    let nosed = built(
        "{ shape: { shell: { bottom_diameter: 3, sections: [{ length: 6 }, { length: 3, \
         top_diameter: 0, profile: 'ogive' }], span: [-90, 90] } }, structure: { mass: 400 } }",
    );
    assert_close(nosed.mass.mass, 400.0, 1e-12);
    // A stage's decoupler is a ring that weighs nothing and lets go of what stands on it.
    let ring = built(
        "{ shape: { ring: { diameter: 3, height: 0.05 } }, \
         components: [{ decoupler: { node: 'top' } }] }",
    );
    assert_eq!(ring.mass.mass, 0.0);
    // A gear leg's mass sits halfway down its strut.
    let leg: GearDef = records("{ position: 1, length: 1.2, load: 2000, mass: 80 }").remove(0);
    let old = Leg::of(&leg, 0.0, 0.0)[0];
    let new = built(
        "{ shape: { beam: { length: 1.2, section: { circle: { diameter: 0.1 } } } }, \
         components: [{ gear: { length: 1.2, load: 2000, mass: 80 } }] }",
    );
    assert_same_mass(
        new.mass,
        Mass::point(old.mass, DVec3::NEG_Z * old.length / 2.0),
        1e-12,
    );
    // A stage's tank is a storage part: the tank and the part weigh the same.
    let def: crate::data::TankDef = records(
        "{ type: 'Isogrid Al-Li', length: 9.6, propellants: { Kerosene: 1, LqdOxygen: 2.36 } }",
    )
    .remove(0);
    let tank = crate::structure::Tank::new(&def, 3.7, 0.0, &catalog).unwrap();
    let part = built(
        "{ shape: { cylinder_tank: { diameter: 3.7, length: 9.6 } }, components: [{ storage: \
         { type: 'Isogrid Al-Li', contents: { Kerosene: 1, LqdOxygen: 2.36 } } }] }",
    );
    assert_eq!(part.mass.mass, tank.dry_mass);
}
