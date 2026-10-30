//! Writes what a mission found: `report.json` for machines, `index.html` for people (one
//! self-contained page: no network, no build step), and `target/smoke/index.html` listing
//! every mission that has been flown.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::{
    Mission,
    site::{generated, home, root},
    world::{Status, World},
};

const PAGE: &str = include_str!("report.html");

/// Four significant figures: telemetry does not need more, and the page is smaller for it.
fn sig(x: f64) -> Value {
    if !x.is_finite() {
        return Value::Null;
    }
    if x == 0.0 {
        return json!(0);
    }
    let digits = 3 - x.abs().log10().floor() as i32;
    let scale = 10f64.powi(digits);
    json!((x * scale).round() / scale)
}

/// The samples as one array per field, thinned where nothing is happening.
fn columns(world: &World, liftoff: f64) -> Value {
    let mut kept = Vec::new();
    let mut last = f64::NEG_INFINITY;
    for s in &world.samples {
        let busy = s.thrust > 0.0 || s.altitude < 150_000.0;
        let gap = if busy { 0.99 } else { 119.0 };
        if s.t - last >= gap {
            kept.push(s);
            last = s.t;
        }
    }
    let rows: Vec<Value> = kept
        .iter()
        .map(|s| {
            let mut row = serde_json::to_value(s).unwrap();
            row["t"] = json!(s.t - liftoff);
            row
        })
        .collect();
    let mut out = serde_json::Map::new();
    if let Some(Value::Object(first)) = rows.first() {
        for key in first.keys() {
            let column: Vec<Value> = rows
                .iter()
                .map(|r| match &r[key] {
                    Value::Number(n) => sig(n.as_f64().unwrap()),
                    other => other.clone(),
                })
                .collect();
            out.insert(key.clone(), Value::Array(column));
        }
    }
    Value::Object(out)
}

/// Writes the report; returns the page's path.
pub fn write(mission: &Mission, world: &World, stopped: bool) -> PathBuf {
    let dir = root().join(mission.id);
    std::fs::create_dir_all(&dir).unwrap();
    let liftoff = world.liftoff.map_or(0.0, |l| l.seconds_since(world.start));
    let count = |s: Status| world.findings.iter().filter(|f| f.status == s).count();
    let shift = |t: f64| sig(t - liftoff);
    let phases: Vec<Value> = world
        .phases
        .iter()
        .map(|p| {
            let mut v = serde_json::to_value(p).unwrap();
            v["started"] = p.started.map_or(Value::Null, shift);
            v["ended"] = p.ended.map_or(Value::Null, shift);
            v
        })
        .collect();
    let events: Vec<Value> = world
        .events
        .iter()
        .map(|e| json!({ "t": shift(e.t), "phase": e.phase, "text": e.text }))
        .collect();
    let data = json!({
        "mission": {
            "id": mission.id,
            "title": mission.title,
            "summary": mission.summary,
            "scenario": mission.scenario,
            "craft": world.craft.name,
        },
        "generated": generated(),
        "stopped": stopped,
        "counts": {
            "pass": count(Status::Pass),
            "fail": count(Status::Fail),
            "known": count(Status::Known),
            "gap": count(Status::Gap),
            "bridged": count(Status::Bridged),
            "info": count(Status::Info),
        },
        "phases": phases,
        "findings": world.findings,
        "events": events,
        "audit": world.audit,
        "samples": columns(world, liftoff),
    });
    let json = serde_json::to_string(&data).unwrap();
    std::fs::write(dir.join("report.json"), &json).unwrap();
    // Inside a <script>, "</" would end it early.
    let page = PAGE.replace("/*DATA*/null", &json.replace("</", "<\\/"));
    let path = dir.join("index.html");
    std::fs::write(&path, page).unwrap();
    home();
    path.canonicalize().unwrap_or(path)
}
