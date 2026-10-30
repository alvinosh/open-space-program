//! Writes the matrix: `report.json` for machines and `index.html` for people (one
//! self-contained page), in `target/smoke/build/`, and the smoke home page that lists it.
//! The JSON has the shape the home page reads (`mission`, `counts`, `phases`), and the
//! matrix beside it.

use std::{collections::BTreeMap, path::PathBuf};

use serde_json::{Value, json};

use crate::{
    Run, cell,
    checks::{Check, Outcome},
    expect::Expect,
    site::{generated, home, root},
};

const PAGE: &str = include_str!("report.html");

/// How a cell came out, for its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Passes every check, as expected.
    Green,
    /// Fails as its rule expects, until its task lands.
    Red,
    /// Refused by a rule of building, as expected.
    Grey,
    /// Not what its rule expects: fails the test.
    Wrong,
}

fn status(run: &Run) -> Status {
    match (&run.mismatch, run.expect) {
        (Some(_), _) => Status::Wrong,
        (None, Expect::Green) => Status::Green,
        (None, Expect::Grey(_)) => Status::Grey,
        (None, Expect::Red(..)) => Status::Red,
    }
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Green => "green",
            Status::Red => "red",
            Status::Grey => "grey",
            Status::Wrong => "wrong",
        }
    }
}

/// The status of a task id in `.tasks/`, and its title.
fn task_info(id: &str) -> (String, String) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tasks");
    let found = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .find(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(&format!("{id}-"))
        });
    let data = found
        .and_then(|e| std::fs::read_to_string(e.path().join("task.json")).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    match data {
        Some(d) => (
            d["status"].as_str().unwrap_or("?").to_owned(),
            d["title"].as_str().unwrap_or("").to_owned(),
        ),
        None => ("missing".into(), String::new()),
    }
}

/// Tasks that red cells name which are not open: a red cell must wait on work to be done.
pub fn closed_tasks(runs: &[Run]) -> Vec<String> {
    let mut tasks: Vec<&str> = runs
        .iter()
        .filter_map(|r| match r.expect {
            Expect::Red(_, task) => Some(task),
            _ => None,
        })
        .collect();
    tasks.sort();
    tasks.dedup();
    tasks
        .into_iter()
        .filter_map(|id| {
            let (status, _) = task_info(id);
            matches!(status.as_str(), "done" | "dropped" | "missing")
                .then(|| format!("{id} is {status}, and cells still wait on it"))
        })
        .collect()
}

/// Writes the report; returns the page's path.
pub fn write(runs: &[Run]) -> PathBuf {
    let dir = root().join("build");
    std::fs::create_dir_all(&dir).unwrap();
    let count = |s: Status| runs.iter().filter(|r| status(r) == s).count();

    let cells: Vec<Value> = runs
        .iter()
        .map(|run| {
            let (task, checks_failing) = match run.expect {
                Expect::Red(checks, task) => (Some(task), checks.to_vec()),
                _ => (None, Vec::new()),
            };
            json!({
                "row": run.row,
                "column": run.column,
                "status": status(run).name(),
                "task": task,
                "expected": checks_failing,
                "why": run.why,
                "mismatch": run.mismatch,
                "refusal": run.result.refusal,
                "checks": run.result.checks.iter().map(|(check, outcome)| {
                    let (result, detail) = match outcome {
                        Outcome::Pass => ("pass", String::new()),
                        Outcome::Fail(d) => ("fail", d.clone()),
                        Outcome::Skip(d) => ("skip", d.clone()),
                    };
                    json!({ "check": check, "result": result, "detail": detail })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();

    // What each task still has to turn green.
    let mut by_task: BTreeMap<&str, usize> = BTreeMap::new();
    for run in runs {
        if let Expect::Red(_, task) = run.expect {
            *by_task.entry(task).or_default() += 1;
        }
    }
    let tasks: Vec<Value> = by_task
        .iter()
        .map(|(id, cells)| {
            let (status, title) = task_info(id);
            json!({ "id": id, "status": status, "title": title, "cells": cells })
        })
        .collect();

    // One strip item per row for the home page.
    let phases: Vec<Value> = cell::ROWS
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let of_row: Vec<Status> = runs.iter().filter(|x| x.row == r).map(status).collect();
            let outcome = if of_row.contains(&Status::Wrong) {
                "failed"
            } else if of_row.contains(&Status::Red) {
                "partial"
            } else {
                "passed"
            };
            json!({ "title": row.title, "outcome": outcome })
        })
        .collect();

    let data = json!({
        "mission": {
            "id": "build",
            "title": "Build: placement matrix",
            "summary": "Every component the sim has against every way to put a part on a craft: \
                        does it load, is it kept, does it add up, does the air see it, does it \
                        save, does it stand on the pad. A red cell names the task that fixes it.",
            "craft": "a 1 m host with the part placed on it",
        },
        "generated": generated(),
        "stopped": false,
        "counts": {
            "pass": count(Status::Green) + count(Status::Grey),
            "fail": count(Status::Wrong),
            "known": count(Status::Red),
            "gap": 0,
            "bridged": 0,
            "info": 0,
        },
        "phases": phases,
        "rows": cell::ROWS.iter().map(|r| json!({ "id": r.id, "title": r.title })).collect::<Vec<_>>(),
        "columns": cell::COLUMNS.iter().map(|c| json!({ "id": c.id(), "title": c.title() })).collect::<Vec<_>>(),
        "checks": Check::ALL.iter().map(|c| json!({ "id": c, "title": c.title() })).collect::<Vec<_>>(),
        "cells": cells,
        "tasks": tasks,
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
