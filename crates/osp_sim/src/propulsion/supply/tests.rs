use super::*;
use crate::{
    data::CraftDef,
    systems::{Crew, FuelCell, Process},
    test_data::records,
    vessel::tests::catalog,
};

const RESOURCES: [&str; 3] = ["LqdHydrogen", "LqdOxygen", "Water"];

/// A host part with `body`, and beside it a hydrogen, an oxygen and a water tank, holding `fill`
/// of what each can.
fn plant(body: &str, fill: [f64; 3]) -> Propulsion {
    let text = format!(
        r#"{{ id: "plant", name: "", parts: [
            {{ id: "host", shape: {{ shell: {{ bottom_diameter: 2, sections: [{{ length: 6 }}],
                cap_bottom: true, cap_top: true }} }},
              structure: {{ mass: 500 }},
              components: [{{ container: {{}} }}, {body}],
              nodes: [{{ name: "h2", position: [0, 0, 1], direction: [0, 0, 1] }},
                      {{ name: "o2", position: [0, 0, 3], direction: [0, 0, 1] }},
                      {{ name: "water", position: [0, 0, 5], direction: [0, 0, 1] }}] }},
            {{ id: "hydrogen", on: "host.h2", shape: {{ sphere: {{ diameter: 0.8 }} }},
              components: [{{ storage: {{ type: "Insulated Al-Li",
                contents: {{ LqdHydrogen: 1 }} }} }}] }},
            {{ id: "oxygen", on: "host.o2", shape: {{ sphere: {{ diameter: 0.7 }} }},
              components: [{{ storage: {{ type: "Insulated Al-Li",
                contents: {{ LqdOxygen: 1 }} }} }}] }},
            {{ id: "water", on: "host.water", shape: {{ sphere: {{ diameter: 0.5 }} }},
              components: [{{ storage: {{ type: "Insulated Al-Li",
                contents: {{ Water: 1 }} }} }}] }}
        ] }}"#
    );
    let craft: CraftDef = records(&text).remove(0);
    let mut p = Propulsion::from_craft(&craft, &catalog()).unwrap();
    for tank in &mut p.tanks {
        for (content, amount) in tank.contents.iter().zip(&mut tank.amounts) {
            let k = RESOURCES
                .iter()
                .position(|r| *r == content.resource)
                .unwrap();
            *amount = content.capacity * fill[k];
        }
    }
    p
}

fn held(p: &Propulsion, resource: &str) -> f64 {
    p.tanks.iter().map(|t| t.held(resource)).sum()
}

fn capacity(p: &Propulsion, resource: &str) -> f64 {
    (p.tanks.iter().flat_map(|t| &t.contents))
        .filter(|c| c.resource == resource)
        .map(|c| c.capacity)
        .sum()
}

fn set(p: &mut Propulsion, resource: &str, kg: f64) {
    for tank in &mut p.tanks {
        for (content, amount) in tank.contents.iter().zip(&mut tank.amounts) {
            if content.resource == resource {
                *amount = kg;
            }
        }
    }
}

fn running(p: &Propulsion) -> Vec<f64> {
    p.systems.all::<Process>().map(|(_, s)| s.running).collect()
}

fn total(p: &Propulsion) -> f64 {
    RESOURCES.iter().map(|r| held(p, r)).sum()
}

/// Electrolysis takes 5e-5 kg of water a second: this long takes 1 kg.
const KILOGRAM: f64 = 20_000.0;
const BATTERY: &str = "{ battery: { charge: 1000000 } }";

#[test]
fn the_higher_priority_gets_a_scarce_kilogram_and_nothing_is_taken_twice() {
    let mut p = plant(
        &format!(
            "{{ process: {{ process: \"electrolysis\", priority: 200 }} }},
             {{ process: {{ process: \"electrolysis\", priority: 100 }} }}, {BATTERY}"
        ),
        [0.0, 0.0, 0.0],
    );
    set(&mut p, "Water", 1.0);
    p.step_systems(&Environment::DARK, KILOGRAM, false);
    assert!(held(&p, "Water") < 1e-9, "{}", held(&p, "Water"));
    assert_eq!(running(&p), [1.0, 0.0]);
    // A kilogram of water, split and kept: no more, no less.
    assert!((total(&p) - 1.0).abs() < 1e-9, "{}", total(&p));
    assert!((held(&p, "LqdOxygen") - 0.8881).abs() < 1e-9);
}

#[test]
fn equals_share_a_scarce_kilogram_in_proportion() {
    let mut p = plant(
        &format!(
            "{{ process: {{ process: \"electrolysis\" }} }},
             {{ process: {{ process: \"electrolysis\", scale: 3 }} }}, {BATTERY}"
        ),
        [0.0, 0.0, 0.0],
    );
    set(&mut p, "Water", 1.0);
    // They want 1 and 3 kg: each gets a quarter of what it wants.
    p.step_systems(&Environment::DARK, KILOGRAM, false);
    let shares = running(&p);
    assert!(shares.iter().all(|s| (s - 0.25).abs() < 1e-9), "{shares:?}");
    assert!((total(&p) - 1.0).abs() < 1e-9);
}

#[test]
fn a_process_with_no_room_for_its_output_slows_to_what_fits() {
    let mut p = plant(
        &format!("{{ process: {{ process: \"electrolysis\" }} }}, {BATTERY}"),
        [0.0, 1.0, 1.0],
    );
    let room = 0.1;
    let full = capacity(&p, "LqdOxygen");
    set(&mut p, "LqdOxygen", full - room);
    let water = held(&p, "Water");
    p.step_systems(&Environment::DARK, KILOGRAM, false);
    let share = room / 0.8881;
    assert!((running(&p)[0] - share).abs() < 1e-9, "{:?}", running(&p));
    assert!((capacity(&p, "LqdOxygen") - held(&p, "LqdOxygen")).abs() < 1e-9);
    assert!((water - held(&p, "Water") - share).abs() < 1e-9);
}

#[test]
fn a_process_without_power_does_not_run_and_one_without_inputs_draws_none() {
    let mut p = plant(
        r#"{ process: { process: "electrolysis" } }"#,
        [0.0, 0.0, 1.0],
    );
    let water = held(&p, "Water");
    p.step_systems(&Environment::DARK, 100.0, false);
    assert_eq!(running(&p), [0.0]);
    assert_eq!(held(&p, "Water"), water);

    let mut p = plant(
        &format!("{{ process: {{ process: \"electrolysis\" }} }}, {BATTERY}"),
        [0.0, 0.0, 0.0],
    );
    let charge = p.systems.power_state(&Environment::DARK, false).charge;
    p.step_systems(&Environment::DARK, 100.0, false);
    assert_eq!(running(&p), [0.0]);
    assert_eq!(
        p.systems.power_state(&Environment::DARK, false).charge,
        charge,
        "nothing to split, so no power asked for"
    );
}

/// A water splitter feeds the fuel cell that powers it, beside a crew who breathe the oxygen: a
/// day in one step leaves the tanks as a day of second-long steps does.
#[test]
fn a_day_in_one_step_is_a_day_of_seconds() {
    let body = r#"{ process: { process: "electrolysis" } }, { fuel_cell: { power: 1500 } },
        { electric_load: { watts: 200 } }, { crew: { seats: 2 } },
        { life_support: { carries: { LiOH: 10, Food: 10 } } }"#;
    let fill = [0.5, 0.5, 0.5];
    let mut long = plant(body, fill);
    let mut short = plant(body, fill);
    long.step_systems(&Environment::DARK, 86_400.0, false);
    for _ in 0..86_400 {
        short.step_systems(&Environment::DARK, 1.0, false);
    }
    for resource in RESOURCES {
        let (a, b) = (held(&long, resource), held(&short, resource));
        assert!((a - b).abs() < 1e-9 * a.max(1.0), "{resource}: {a} and {b}");
    }
    // It all happened: water split, the cell burned, the crew breathed and are well.
    assert!(held(&long, "Water") < 0.5 * capacity(&long, "Water") - 4.0);
    // The splitter at full speed, and the cabin's scrubber at what is breathed out.
    let shares = running(&long);
    assert_eq!(shares[0], 1.0);
    assert!(
        (shares[1] - 2.0 / (2.0 * 1.3889e-5 * 86_400.0)).abs() < 1e-9,
        "{shares:?}"
    );
    let (_, cell) = long.systems.all::<FuelCell>().next().unwrap();
    // The load, the splitter, and the crew's two scrubbers at the 2 kg of CO2 a day they breathe
    // out, of the 2.4 kg they could take.
    let could = 2.0 * 1.3889e-5 * 86_400.0;
    let scrubbers = 2.0 * 20.0 * (2.0 / could);
    assert!(
        (cell.output - 1200.0 - scrubbers).abs() < 1e-6,
        "{}",
        cell.output
    );
    assert_eq!(long.systems.crew_lost(), 0);
    assert!(long.systems.all::<Crew>().next().is_some());
}

/// Time warp stops where a process's input runs dry, and where its output tank fills.
#[test]
fn warp_stops_where_an_input_runs_dry_or_an_output_fills() {
    let mut p = plant(
        &format!("{{ process: {{ process: \"electrolysis\" }} }}, {BATTERY}"),
        [0.0, 0.0, 0.0],
    );
    set(&mut p, "Water", 1.0);
    p.step_systems(&Environment::DARK, 1.0, false);
    let dry = p.next_system_event(&Environment::DARK, false).unwrap();
    assert!((dry - (KILOGRAM - 1.0)).abs() < 1e-6, "{dry}");

    let mut p = plant(
        &format!("{{ process: {{ process: \"electrolysis\" }} }}, {BATTERY}"),
        [0.0, 1.0, 1.0],
    );
    let oxygen = capacity(&p, "LqdOxygen");
    set(&mut p, "LqdOxygen", oxygen - 0.8881);
    p.step_systems(&Environment::DARK, 1.0, false);
    let full = p.next_system_event(&Environment::DARK, false).unwrap();
    assert!((full - (KILOGRAM - 1.0)).abs() < 1e-3, "{full}");
}

/// A process can be stopped and started from its part, and a stopped one does nothing.
#[test]
fn a_stopped_process_does_nothing() {
    let mut p = plant(
        &format!("{{ process: {{ process: \"electrolysis\", on: false }} }}, {BATTERY}"),
        [0.0, 0.0, 1.0],
    );
    let water = held(&p, "Water");
    p.step_systems(&Environment::DARK, 100.0, false);
    assert_eq!(held(&p, "Water"), water);
    let part = p.systems.all::<Process>().next().unwrap().0;
    assert!(p.systems.act(part, crate::systems::START).is_some());
    p.step_systems(&Environment::DARK, 100.0, false);
    assert!(held(&p, "Water") < water);
}
