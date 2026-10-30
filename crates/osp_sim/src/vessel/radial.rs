//! Radial craft: boosters hanging beside a core, built part by part.

use glam::DVec3;

use super::{Vessel, tests::*};
use crate::{
    attitude::{actuate, mass_properties},
    data::CraftDef,
    propulsion::Propulsion,
    staging::Activation,
    system::tests::rss,
    test_data::records,
    time::SimTime,
};

/// A booster on the `side` of the core: `x` is +1 for east, −1 for west; `decoupler` adds
/// to its decoupler's fields.
fn booster(id: &str, side: &str, x: f64, decoupler: &str) -> String {
    format!(
        r#"{{ id: "{id}", on: "core.{side}", by: "side",
      shape: {{ shell: {{ bottom_diameter: 1.2, sections: [{{ length: 8 }}], cap_bottom: true }} }},
      structure: {{ mass: 400 }},
      components: [{{ container: {{}} }}, {{ decoupler: {{ node: "side", {decoupler} }} }}],
      nodes: [{{ name: "side", position: [{}, 0, 2], direction: [{}, 0, 0] }},
              {{ name: "engine", position: [0, 0, 0], direction: [0, 0, -1] }},
              {{ name: "tank", position: [0, 0, 0], direction: [0, 0, 1] }}] }},
    {{ id: "{id}_engine", on: "{id}.engine", by: "top", template: "K-1 Booster" }},
    {{ id: "{id}_tank", on: "{id}.tank", shape: {{ cylinder_tank: {{ diameter: 1.2, length: 6 }} }},
      components: [{{ storage: {{ type: "Stiffened Al 2219", contents: {{ Kerosene: 1, LqdOxygen: 2.36 }} }} }}] }}"#,
        -0.6 * x,
        -x,
    )
}

/// A core with a tank and an engine, and the boosters on `sides` (`east`, `west`), each
/// held 2 m up its side by a decoupler of its own, their bases level with the core's.
fn radial(sides: &[&str]) -> CraftDef {
    radial_with(sides, "")
}

/// [`radial`], its boosters' decouplers with `decoupler` added to their fields.
fn radial_with(sides: &[&str], decoupler: &str) -> CraftDef {
    let boosters: Vec<String> = sides
        .iter()
        .map(|&side| {
            booster(
                side,
                side,
                if side == "east" { 1.0 } else { -1.0 },
                decoupler,
            )
        })
        .collect();
    records::<CraftDef>(&format!(
        r#"{{
  id: "radial",
  name: "Radial",
  parts: [
    {{ id: "core", shape: {{ shell: {{ bottom_diameter: 2, sections: [{{ length: 12 }}], cap_bottom: true }} }},
      structure: {{ mass: 900 }},
      components: [{{ container: {{}} }}],
      nodes: [{{ name: "engine", position: [0, 0, 0], direction: [0, 0, -1] }},
              {{ name: "tank", position: [0, 0, 0], direction: [0, 0, 1] }},
              {{ name: "east", position: [1, 0, 2], direction: [1, 0, 0] }},
              {{ name: "west", position: [-1, 0, 2], direction: [-1, 0, 0] }}] }},
    {{ id: "core_engine", on: "core.engine", by: "top", template: "K-1 Booster" }},
    {{ id: "core_tank", on: "core.tank", shape: {{ cylinder_tank: {{ diameter: 2, length: 10 }} }},
      components: [{{ storage: {{ type: "Stiffened Al 2219", contents: {{ Kerosene: 1, LqdOxygen: 2.36 }} }} }}] }},
    {}
  ],
}}"#,
        boosters.join(",\n    ")
    ))
    .remove(0)
}

#[test]
fn boosters_hang_beside_their_core_and_burn_with_it() {
    let p = Propulsion::from_craft(&radial(&["east", "west"]), &catalog()).unwrap();
    let names: Vec<&str> = p.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names.len(), 3, "{names:?}");
    let core = &p.sections[2];
    assert!(core.parent.is_none() && !core.beside);
    for booster in &p.sections[..2] {
        assert!(booster.beside && booster.parent == Some(core.id));
        assert!((booster.bottom.truncate().length() - 1.6).abs() < 1e-9);
        assert!(booster.bottom.z.abs() < 1e-9);
    }
    // Lit together, the boosters let go once they have burnt out.
    let [lit, released] = p.sequences.list() else {
        panic!("{:?}", p.sequences.list());
    };
    assert_eq!(lit.len(), 3);
    assert!(lit.iter().all(|a| matches!(a, Activation::Engine(_))));
    assert_eq!(released.len(), 2);
    assert!(
        released
            .iter()
            .all(|a| matches!(a, Activation::Decoupler(_)))
    );
    // Balanced round the axis, and the core's delta-v counts the boosters' burn.
    assert_eq!(
        mass_properties(&p).center_of_mass.truncate(),
        glam::DVec2::ZERO
    );
    let info = p.stage_info();
    assert!(info[..2].iter().all(|i| i.delta_v_vac == 0.0));
    let alone = Propulsion::from_craft(&radial(&[]), &catalog()).unwrap();
    let core_alone = alone.stage_info()[0].delta_v_vac;
    assert!(
        info[2].delta_v_vac > core_alone + 500.0,
        "{info:?} {core_alone}"
    );
}

#[test]
fn an_unbalanced_booster_pushes_off_the_centre_of_mass() {
    let mut p = Propulsion::from_craft(&radial(&["east"]), &catalog()).unwrap();
    let com = mass_properties(&p).center_of_mass;
    assert!(com.x > 0.1 && com.y.abs() < 1e-9, "{com}");
    let ids: Vec<usize> = p.sections.iter().map(|s| s.id).collect();
    for id in ids {
        p.arm(id);
    }
    for engine in &mut p.engines {
        engine.thrust = 1e5;
    }
    // Two equal engines either side of a centre of mass nearer the booster: the core's
    // pushes harder on its long arm, turning the nose towards the booster (about −Y).
    let (force, torque) = actuate(&mut p, com, DVec3::ZERO);
    assert!((force.z - 2e5).abs() < 1e-6 && torque.y < -1e3, "{torque}");
}

#[test]
fn boosters_lift_off_with_the_core_and_fall_away_outward() {
    let system = rss();
    let site = launch_sites().remove(0);
    let t0 = SimTime::J2000;
    let mut v =
        Vessel::on_launch_site(&radial(&["east", "west"]), &catalog(), &system, &site, t0).unwrap();
    let mut t = t0;
    v.set_throttle(1.0);
    v.activate_next_stage(&system, t);
    run(&mut v, &system, &mut t, 8.0);
    let firing: Vec<f64> = v.propulsion.engines.iter().map(|e| e.thrust).collect();
    assert!(
        firing.len() == 3 && firing.iter().all(|&f| f > 0.0),
        "{:?}",
        v.log
    );
    let up = v.state_at(&system, t).position.normalize();
    let nose = v.attitude_at(&system, t) * DVec3::Z;
    assert!(nose.dot(up) > 0.999, "it climbs straight: {nose}");
    let core = v.state_at(&system, t);
    let debris = v.activate_next_stage(&system, t);
    assert_eq!(debris.len(), 2);
    assert_eq!(v.propulsion.sections.len(), 1);
    let attitude = v.attitude_at(&system, t);
    for d in &debris {
        let state = d.state_at(&system, t);
        let out = state.position - core.position;
        let sideways = out.reject_from(attitude * DVec3::Z);
        assert!((sideways.length() - 1.6).abs() < 0.1, "{out}");
        let apart = (state.velocity - v.state_at(&system, t).velocity).dot(sideways.normalize());
        assert!(apart > 0.1, "{apart}");
    }
    run(&mut v, &system, &mut t, 1.0);
    assert!(v.propulsion.any_engine_running());
}

/// Burn the core's engine alone for `seconds`, the boosters' left unlit.
fn burn_core(p: &mut Propulsion, seconds: f64) {
    let core = p.sections.len() - 1;
    p.arm(p.sections[core].id);
    p.set_throttle(1.0);
    for _ in 0..(seconds * 50.0) as usize {
        p.burn(0.0, 0.02, true, &mut Vec::new());
    }
}

#[test]
fn propellant_crosses_a_decoupler_only_with_its_crossfeed_on() {
    let left = |p: &Propulsion, k: usize| p.propellant_mass(p.sections[k].id);
    let mut apart = Propulsion::from_craft(&radial(&["east", "west"]), &catalog()).unwrap();
    let full = (left(&apart, 0), left(&apart, 2));
    burn_core(&mut apart, 20.0);
    assert_eq!(left(&apart, 0), full.0);
    assert!(left(&apart, 2) < full.1);

    // With crossfeed the core drains every tank it reaches, keeping them equally full.
    let mut fed = Propulsion::from_craft(
        &radial_with(&["east", "west"], "crossfeed: true"),
        &catalog(),
    )
    .unwrap();
    burn_core(&mut fed, 20.0);
    let share = |p: &Propulsion, k: usize| left(p, k) / p.propellant_capacity(p.sections[k].id);
    assert!(left(&fed, 0) < full.0);
    assert!((share(&fed, 0) - share(&fed, 2)).abs() < 1e-9);
    // The same mass burned either way.
    let burned = |p: &Propulsion| {
        p.sections
            .iter()
            .map(|s| p.propellant_capacity(s.id) - p.propellant_mass(s.id))
            .sum::<f64>()
    };
    assert!((burned(&fed) - burned(&apart)).abs() < 1e-6 * burned(&apart));
    // Once the boosters let go, the core has its own tank again.
    let core_before = left(&fed, 2);
    fed.activate_next_sequence();
    fed.activate_next_sequence();
    assert_eq!(fed.sections.len(), 1);
    burn_core(&mut fed, 1.0);
    assert!(left(&fed, 0) < core_before);
}

#[test]
fn a_decoupler_with_an_impulse_parts_its_sides_by_it() {
    let system = rss();
    let site = launch_sites().remove(0);
    let t0 = SimTime::J2000;
    let craft = radial_with(&["east", "west"], "impulse: 3000");
    let mut v = Vessel::on_launch_site(&craft, &catalog(), &system, &site, t0).unwrap();
    let mut t = t0;
    v.set_throttle(1.0);
    v.activate_next_stage(&system, t);
    run(&mut v, &system, &mut t, 5.0);
    let masses: Vec<f64> = v
        .propulsion
        .sections
        .iter()
        .map(|s| v.propulsion.section_mass(s.id))
        .collect();
    let core = v.state_at(&system, t);
    let debris = v.activate_next_stage(&system, t);
    let attitude = v.attitude_at(&system, t);
    let kept = v.state_at(&system, t).velocity - core.velocity;
    for (d, &mass) in debris.iter().zip(&masses) {
        let out = (d.state_at(&system, t).position - core.position)
            .reject_from(attitude * DVec3::Z)
            .normalize();
        let apart = (d.state_at(&system, t).velocity - v.state_at(&system, t).velocity).dot(out);
        // Each booster gets its decoupler's impulse; the core feels both, which cancel.
        assert!(
            (apart - 3000.0 / mass).abs() < 1e-3,
            "{apart} {}",
            3000.0 / mass
        );
    }
    assert!(kept.length() < 1e-3, "{kept}");
}

#[test]
fn a_booster_pushes_on_its_core_through_its_joint() {
    use crate::structure::{Conditions, joint_loads};
    let mut p = Propulsion::from_craft(&radial(&["east", "west"]), &catalog()).unwrap();
    // On the pad everything stands on the ground: no joint carries anything.
    let pad = Conditions {
        acceleration: 9.81,
        landed: true,
        ..Default::default()
    };
    assert!(joint_loads(&p, &pad).iter().all(|f| f.abs() < 1e-6));
    // In flight each booster pushes with its thrust less what it takes to carry itself.
    let thrust = 2e5;
    for engine in &mut p.engines {
        engine.thrust = thrust;
    }
    let total = p.mass();
    let a = 3.0 * thrust / total;
    let flight = Conditions {
        acceleration: a,
        ..Default::default()
    };
    let loads = joint_loads(&p, &flight);
    for k in 0..2 {
        let expected = thrust - p.section_mass(p.sections[k].id) * a;
        assert!((loads[k] - expected).abs() < 1e-6 * thrust, "{loads:?}");
    }
    assert_eq!(loads[2], 0.0);
}
