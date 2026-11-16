//! Playtests (`osp_game::playtest`): the game played as a person plays it, from the title screen,
//! and measured for how easy it is to use and how soon it pays off.
//!
//! `cargo playtest` runs every playtest in the game built for the tests, one after the other, each
//! in an empty user folder (a first game), then writes `target/playtest/report.html` from their
//! reports. `OSP_PLAYTESTS=first_rocket,...` runs only those. A playtest fails when the player
//! cannot do what it sets out to do, unless it is known to get stuck until a task is done (then
//! it fails once it gets through, to have the mark taken out); letdowns (what a player hoped for
//! and did not get) and the figures are in the report, not failures.

use std::{path::PathBuf, process::Command};

use osp_game::playtest::plays::PLAYS;

const PAGE: &str = include_str!("playtest/report.html");

#[test]
fn playtests() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap_or(root);
    let out = root.join("target/playtest");
    let wanted: Option<Vec<String>> = std::env::var("OSP_PLAYTESTS")
        .ok()
        .map(|names| names.split(',').map(|n| n.trim().to_string()).collect());
    let plays: Vec<&str> = (PLAYS.iter())
        .map(|p| p.name)
        .filter(|name| wanted.as_ref().is_none_or(|w| w.iter().any(|n| n == name)))
        .collect();
    assert!(!plays.is_empty(), "no playtest called any of {wanted:?}");

    let mut failed = Vec::new();
    let mut reports = Vec::new();
    for name in plays {
        let user = out.join("users").join(name);
        let _ = std::fs::remove_dir_all(&user);
        std::fs::create_dir_all(&user).expect("a user folder");
        let log = out.join(format!("{name}.log"));
        let status = Command::new(env!("CARGO_BIN_EXE_osp"))
            .current_dir(&root)
            .env("OSP_PLAYTEST", name)
            .env("OSP_PLAYTEST_OUT", &out)
            .env("OSP_USER_DIR", &user)
            .stdout(std::fs::File::create(&log).expect("a log file"))
            .stderr(std::fs::File::create(log.with_extension("err.log")).expect("a log file"))
            .status()
            .expect("the game runs");
        let report = std::fs::read_to_string(out.join(name).join("report.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
        let headline = report.as_ref().map_or_else(
            || {
                format!(
                    "no report (the game exited {status}), see {}",
                    log.display()
                )
            },
            headline,
        );
        let known = PLAYS
            .iter()
            .find(|p| p.name == name)
            .and_then(|p| p.stuck_until);
        let passed = status.success() && report.is_some();
        match (known, passed) {
            (Some(task), false) => {
                println!("playtest {name}: known stuck until {task}: {headline}")
            }
            (Some(task), true) => {
                println!("playtest {name}: gets through now: take out its `stuck_until: {task}`");
                failed.push(name);
            }
            (None, _) => {
                println!("playtest {name}: {headline}");
                if !passed {
                    failed.push(name);
                }
            }
        }
        if let Some(report) = report {
            reports.push(report);
        }
    }

    let data = serde_json::to_string(&reports).expect("the reports as JSON");
    let page = PAGE.replace("/*DATA*/null", &data.replace("</", "<\\/"));
    let path = out.join("report.html");
    std::fs::write(&path, page).expect("the report page");
    println!("playtest report: {}", path.display());
    assert!(failed.is_empty(), "playtests failed: {failed:?}");
}

/// The figures a playtest's report leads with, in a line.
fn headline(report: &serde_json::Value) -> String {
    let s = &report["summary"];
    let failure = match &report["failure"] {
        serde_json::Value::Null => "passed".to_string(),
        f => format!("FAILED at step {}: {}", f["step"], f["reason"]),
    };
    let hopes = report["hopes"].as_array().map_or(0, Vec::len);
    let letdowns =
        (report["hopes"].as_array()).map_or(0, |h| h.iter().filter(|h| h["met"] == false).count());
    format!(
        "{failure}; {} inputs, a person takes about {:.0} s, {} dead, {letdowns} of {hopes} hopes \
         let down",
        s["inputs"],
        s["person_s"].as_f64().unwrap_or(0.0),
        s["dead"].as_array().map_or(0, Vec::len),
    )
}
