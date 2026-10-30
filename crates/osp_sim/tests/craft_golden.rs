//! Every craft weighs and flies as the golden figures in `golden/craft.json` say, recorded
//! by `examples/craft_golden.rs`: after liftoff and after each sequence fires, the stages'
//! masses, delta-v and burn times, the centre of mass and inertia, the outline the air sees
//! and the air's loads in three flows. Rerecord them only when a change is meant to move
//! them.
//!
//! The figures were last recorded when stages could hang beside each other
//! (`docs/vessel-overhaul.md`, step 8): the centre of mass became a point, off the axis
//! where a craft is not balanced round it (a plane's tail fin, mixed engines in a ring), and
//! the inertia is about that point. Nothing else moved. The Titan II GLV was then rebuilt
//! by hand from plain parts (step 7), with Gemini's sections as stages of their own.
//!
//! The air's figures were rerecorded when it came to see the parts' own surfaces (step 9):
//! the outline is now the circles of the cuts' areas, and craft whose stages had drawn
//! tapers, buses and noses their parts do not have moved most.
//!
//! The craft with solid motors were rerecorded when a cast grain stopped weighing on walls
//! of its own (its case is its engine's): its liner lost what the grain's head had
//! thickened it by, most on the big boosters.
//!
//! The Aerobee and the WAC Corporal were rebuilt by hand so they fly as the real ones did:
//! the sustainer lit with its booster over an open joint, its tank above the motor
//! compartment, the instruments in the nose and fins that keep it stable.
//!
//! Then again when solids came to follow their thrust curves (their burn times grew), the
//! V-2, Bumper-WAC, Viking and stock Sounder had their tanks moved up off their bases, the
//! Scout's upper stages got peroxide jets, the Crew Dragon came in at its real size, and
//! the Kestrel, Soyuz and Zenit tanks were built for the g their boosters reach.
//!
//! The glider was given two solid boosters and a wheeled dolly to take off from a runway,
//! and so a sequence for each; the wheels of a stage beside the axis stand out from it.

use std::path::Path;

use osp_sim::{data::CraftDef, propulsion::Propulsion};
use serde_json::Value;

#[path = "golden/record.rs"]
mod record;

/// Relative error allowed, for arithmetic done in another order.
const EXACT: f64 = 1e-9;

#[test]
fn every_craft_matches_its_golden_figures() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let catalog = record::catalog(registry);
    let golden: Value = serde_json::from_str(include_str!("golden/craft.json")).unwrap();
    let golden = golden.as_object().unwrap();
    let mut ids: Vec<String> = registry.records("craft").map(|r| r.id.clone()).collect();
    ids.sort();
    let mut recorded: Vec<&String> = golden.keys().collect();
    recorded.sort();
    assert_eq!(
        ids.iter().collect::<Vec<_>>(),
        recorded,
        "craft without figures"
    );
    let mut failures = Vec::new();
    for id in &ids {
        let def: CraftDef = registry.get("craft", id).unwrap();
        let want = golden[id].as_array().unwrap();
        let about: Vec<f64> = want
            .iter()
            .map(|p| p["center_of_mass"][2].as_f64().unwrap())
            .collect();
        let propulsion = match Propulsion::from_craft(&def, &catalog) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!("{id}: {e}"));
                continue;
            }
        };
        let got = record::record(propulsion, &about);
        compare(id, want, got.as_array().unwrap(), &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn compare(id: &str, want: &[Value], got: &[Value], failures: &mut Vec<String>) {
    if want.len() != got.len() {
        failures.push(format!("{id}: {} phases, not {}", got.len(), want.len()));
        return;
    }
    for (w, g) in want.iter().zip(got) {
        let phase = w["after"].as_str().unwrap();
        if w["after"] != g["after"] {
            failures.push(format!("{id}: `{}` where `{phase}` was", g["after"]));
            continue;
        }
        let (ws, gs) = (
            w["stages"].as_array().unwrap(),
            g["stages"].as_array().unwrap(),
        );
        if ws.len() != gs.len() {
            failures.push(format!(
                "{id}: after `{phase}`: {} stages, not {}",
                gs.len(),
                ws.len()
            ));
            continue;
        }
        let mut exact: Vec<(String, &Value, &Value)> = vec![
            ("mass".into(), &w["mass"], &g["mass"]),
            ("outline".into(), &w["outline"], &g["outline"]),
            (
                "centre of mass".into(),
                &w["center_of_mass"],
                &g["center_of_mass"],
            ),
            ("inertia".into(), &w["inertia"], &g["inertia"]),
        ];
        for (a, b) in ws.iter().zip(gs) {
            if a["name"] != b["name"] {
                failures.push(format!(
                    "{id}: after `{phase}`: stage {} where {} was",
                    b["name"], a["name"]
                ));
            }
            for key in ["dry", "wet", "delta_v", "burn_time"] {
                exact.push((format!("{} {key}", a["name"]), &a[key], &b[key]));
            }
        }
        let (wl, gl) = (
            w["loads"].as_array().unwrap(),
            g["loads"].as_array().unwrap(),
        );
        for (k, (a, b)) in wl.iter().zip(gl).enumerate() {
            exact.push((format!("flow {k} force"), &a["force"], &b["force"]));
            exact.push((format!("flow {k} torque"), &a["torque"], &b["torque"]));
        }
        for (what, a, b) in exact {
            let (a, b) = (numbers(a), numbers(b));
            let scale = a.iter().fold(1e-9f64, |m, x| m.max(x.abs()));
            let off = if a.len() == b.len() {
                a.iter()
                    .zip(&b)
                    .map(|(x, y)| (x - y).abs() / scale)
                    .fold(0.0, f64::max)
            } else {
                f64::INFINITY
            };
            if off > EXACT {
                failures.push(format!("{id}: after `{phase}`: {what} {b:?}, not {a:?}"));
            }
        }
    }
}

/// Every number in `value`, depth first.
fn numbers(value: &Value) -> Vec<f64> {
    match value {
        Value::Number(n) => vec![n.as_f64().unwrap()],
        Value::Array(items) => items.iter().flat_map(numbers).collect(),
        Value::Object(map) => map.values().flat_map(numbers).collect(),
        _ => Vec::new(),
    }
}
