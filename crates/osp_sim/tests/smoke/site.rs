//! Where the smoke suites write their pages: `target/smoke/<suite>/` for each, and
//! `target/smoke/index.html` listing every suite that has a `report.json`. Shared by the
//! mission suite (`tests/smoke`) and the build suite (`tests/build`).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

const HOME: &str = include_str!("home.html");

/// Where reports go: `target/smoke`, or under `CARGO_TARGET_DIR` when that is set.
pub fn root() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
        .join("smoke")
}

/// `target/smoke/index.html`: one card per mission with a report.
pub fn home() {
    let mut cards = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root()) {
        for entry in entries.flatten() {
            let Ok(text) = std::fs::read_to_string(entry.path().join("report.json")) else {
                continue;
            };
            let Ok(data) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            cards.push(json!({
                "dir": entry.file_name().to_string_lossy(),
                "mission": data["mission"],
                "counts": data["counts"],
                "generated": data["generated"],
                "stopped": data["stopped"],
                "phases": data["phases"].as_array().map(|p| {
                    p.iter()
                        .map(|p| json!({ "title": p["title"], "outcome": p["outcome"] }))
                        .collect::<Vec<_>>()
                }),
            }));
        }
    }
    cards.sort_by_key(|c| c["mission"]["title"].as_str().unwrap_or("").to_owned());
    let json = serde_json::to_string(&cards).unwrap().replace("</", "<\\/");
    let _ = std::fs::write(
        root().join("index.html"),
        HOME.replace("/*CARDS*/[]", &json),
    );
}

/// UTC time of the run, as `2026-10-30 14:05`.
pub fn generated() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year}-{month:02}-{day:02} {:02}:{:02} UTC",
        rest / 3600,
        rest % 3600 / 60
    )
}
