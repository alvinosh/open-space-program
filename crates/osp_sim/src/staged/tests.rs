//! The stage converter: what it writes weighs, sits and flies as the stages did.

use super::*;
use crate::{
    data::{GearDef, PartCatalog, WingDef},
    mass::Mass,
    propulsion::Propulsion,
    test_data::records,
    vessel::tests::{catalog, staged_pathfinder},
};

/// A rocket plane on a finned booster: a mirrored wing with dihedral, incidence and
/// ailerons, a tail fin on top, mirrored main gear and a steering nose leg.
const PLANE: &str = r#"{
  id: "plane",
  name: "Plane",
  stages: [
    {
      name: "Booster",
      engines: ["K-1 Booster"],
      tanks: [{ type: "Stiffened Al 2219", length: 6, propellants: { Kerosene: 1, LqdOxygen: 2.36 } }],
      fins: { count: 3, root_chord: 1.5, tip_chord: 0.6, span: 0.8, sweep: 0.5, thickness: 0.015,
              material: "Al 2219-T87" },
      structure_mass: 400,
      diameter: 1.4,
      length: 8,
    },
    {
      name: "Glider",
      structure_mass: 600,
      diameter: 1.4,
      length: 6,
      wings: [
        { name: "Wing", position: 3.5, root_chord: 1.8, tip_chord: 0.9, span: 4, sweep: 0.6,
          dihedral: 4, incidence: 1.5, mass: 300, mirror: true,
          control: { axes: ["roll"], chord: 0.3, span: [0.5, 0.95] } },
        { name: "Fin", position: 1.2, angle: 270, root_chord: 1.2, tip_chord: 0.5, span: 1.3,
          sweep: 0.7, mass: 40, control: { axes: ["yaw"] } },
      ],
      gear: [
        { name: "Main", position: 3, angle: 60, length: 1.0, load: 400, mass: 50, mirror: true,
          brakes: true },
        { name: "Nose", position: 5.5, length: 0.9, load: 200, mass: 20, steering: 30 },
      ],
    },
  ],
  sequences: [["engines:Booster"], ["decoupler:Booster"]],
}"#;

fn plane() -> StagedCraftDef {
    records(PLANE).remove(0)
}

#[track_caller]
fn assert_close(a: f64, b: f64) {
    assert!((a - b).abs() <= 1e-9 * b.abs().max(1.0), "{a} vs {b}");
}

#[track_caller]
fn assert_same_mass(a: Mass, b: Mass) {
    assert_close(a.mass, b.mass);
    assert!(
        (a.center - b.center).length() < 1e-9,
        "{} vs {}",
        a.center,
        b.center
    );
    let size = b
        .inertia
        .to_cols_array()
        .iter()
        .fold(1.0f64, |m, x| m.max(x.abs()));
    let off = (a.inertia - b.inertia).to_cols_array();
    assert!(off.iter().all(|x| x.abs() < 1e-9 * size), "{off:?}");
}

#[test]
fn what_the_stages_held_comes_back_from_the_parts() {
    let staged = plane();
    let craft = convert(&staged, &catalog()).unwrap();
    let p = Propulsion::from_craft(&craft, &catalog()).unwrap();
    // The fin set, as it was.
    let fins = &p.sections[0].fins.as_ref().unwrap().def;
    let had = staged.stages[0].fins.as_ref().unwrap();
    assert_eq!(
        (
            fins.count,
            fins.root_chord,
            fins.span,
            fins.sweep,
            fins.thickness
        ),
        (
            had.count,
            had.root_chord,
            had.span,
            had.sweep,
            had.thickness
        )
    );
    // Each wing, the mirrored one gathered back from its two halves.
    let glider = &p.sections[1];
    assert_eq!(glider.wings.len(), 2);
    for (got, had) in glider.wings.iter().zip(&staged.stages[1].wings) {
        assert_wing(got, had);
    }
    // A mirrored leg is two legs, the second across the plane of symmetry.
    let legs: Vec<&GearDef> = glider.gear.iter().collect();
    assert_eq!(legs.len(), 3);
    for (leg, angle, mass) in [
        (legs[0], 60.0, 25.0),
        (legs[1], 120.0, 25.0),
        (legs[2], 90.0, 20.0),
    ] {
        assert_close(leg.angle, angle);
        assert_close(leg.mass, mass);
        assert!(!leg.mirror);
    }
    assert!(legs[0].brakes && legs[2].steering == 30.0);
    // The stage key does what it did.
    assert_eq!(
        craft.sequences.unwrap(),
        [vec!["engines:booster"], vec!["decoupler:booster"]]
    );
}

fn assert_wing(got: &WingDef, had: &WingDef) {
    assert_eq!((&got.name, got.mirror), (&had.name, had.mirror));
    assert_close(got.angle.rem_euclid(360.0), had.angle.rem_euclid(360.0));
    for (a, b) in [
        (got.position, had.position),
        (got.root_chord, had.root_chord),
        (got.tip_chord, had.tip_chord),
        (got.span, had.span),
        (got.sweep, had.sweep),
        (got.dihedral, had.dihedral),
        (got.incidence, had.incidence),
        (got.thickness, had.thickness),
        (got.mass, had.mass),
    ] {
        assert_close(a, b);
    }
    assert_eq!(got.control, had.control);
}

#[test]
fn the_parts_weigh_and_sit_as_the_stages_did() {
    for staged in [staged_pathfinder(), plane()] {
        let old = PartTree::from_staged(&staged, &catalog()).unwrap();
        let craft = convert(&staged, &catalog()).unwrap();
        let p = Propulsion::from_craft(&craft, &catalog()).unwrap();
        let new = p.parts();
        // Tanks, fins, wings, shells and fairings weigh as the shapes they are now, not as
        // tubes and plates; everything else exactly as before.
        let reshaped = |part: &Part| {
            matches!(
                part.kind,
                PartKind::Tank(_)
                    | PartKind::Fins(_)
                    | PartKind::Wing(_)
                    | PartKind::Mirror
                    | PartKind::Shell(_)
                    | PartKind::FairingHalf { .. }
            )
        };
        let sum = |tree: &PartTree, keep: &dyn Fn(&Part) -> bool| -> Mass {
            tree.parts()
                .iter()
                .filter(|p| keep(p))
                .map(|p| tree.mass_of(p))
                .sum()
        };
        assert_same_mass(sum(new, &|p| !reshaped(p)), sum(&old, &|p| !reshaped(p)));
        assert_close(
            sum(new, &|p| reshaped(p)).mass,
            sum(&old, &|p| reshaped(p)).mass,
        );
    }
}

#[test]
fn converted_craft_read_back_as_written() {
    for staged in [staged_pathfinder(), plane()] {
        let craft = convert(&staged, &catalog()).unwrap();
        let text = serde_json::to_string(&craft).unwrap();
        let back: CraftDef = serde_json::from_str(&text).unwrap();
        assert_eq!(back.parts, craft.parts);
        // Nothing is written that says what it would anyway.
        assert!(
            !text.contains("null") && !text.contains("\"roll\":0"),
            "{text}"
        );
    }
}

#[test]
fn an_unknown_template_is_named() {
    let mut craft = convert(&staged_pathfinder(), &catalog()).unwrap();
    let engine = craft
        .parts
        .iter_mut()
        .find(|p| p.template.is_some())
        .unwrap();
    engine.template = Some("Nonesuch".into());
    let catalog: PartCatalog = catalog();
    let err = Propulsion::from_craft(&craft, &catalog)
        .unwrap_err()
        .to_string();
    assert!(err.contains("Nonesuch"), "{err}");
}

#[test]
fn a_livery_paints_its_stage_from_the_stage_base() {
    use crate::component::{Finish, LayerDef};
    let mut catalog = catalog();
    catalog.liveries = records(
        r#"[{ id: "test", paint: { finish: "metal",
              layers: [{ band: { along: [2, 3], color: [0, 0, 0] } }] } }]"#,
    );
    let mut staged = staged_pathfinder();
    staged.stages[0].appearance.livery = Some("test".into());
    let craft = convert(&staged, &catalog).unwrap();
    let built = crate::craft::Craft::build(&craft, &catalog).unwrap();
    let poses = built.assembly.poses();
    let base = poses[built.find("booster").unwrap()].position.z;
    let mut painted = 0;
    for (i, part) in craft.parts.iter().enumerate() {
        let on_booster = part.id == "booster" || part.id.starts_with("booster_");
        let has = !part.paint.layers.is_empty();
        if !on_booster || part.template.is_some() {
            assert!(!has, "{} is not the booster's to paint", part.id);
            continue;
        }
        if !has {
            continue;
        }
        painted += 1;
        assert_eq!(part.paint.finish, Finish::Metal);
        let rise = poses[i].position.z - base;
        let LayerDef::Band { along, .. } = &part.paint.layers[0] else {
            panic!("{:?}", part.paint.layers);
        };
        assert_close(along[0], 2.0 - rise);
        assert_close(along[1], 3.0 - rise);
    }
    // Its body and its tank.
    assert!(painted >= 2, "{painted}");
    let unknown = {
        staged.stages[0].appearance.livery = Some("nope".into());
        convert(&staged, &catalog).unwrap_err().to_string()
    };
    assert!(unknown.contains("no livery `nope`"), "{unknown}");
}
