//! Mission smoke tests: whole flights, start to finish, flown the way a player flies them.
//!
//! Each mission is a script of phases (see `saturn_v.rs`). It loads the real mods, starts a
//! scenario through the game's own code and then only uses what a player has: the throttle,
//! the stage key, SAS, RCS, burns planned on the map. Nothing is placed or patched in. Where
//! the sim cannot do what the real mission did, the script says so in the report instead of
//! hiding it, and flies on if it can.
//!
//! ```sh
//! cargo smoke               # every mission (tests/smoke.rs)
//! cargo smoke saturn_v      # one
//! cargo run -- --watch saturn_v   # the same flight in the game, to watch (see `watch`)
//! ```
//!
//! The report lands in `target/smoke/<mission>/index.html` (see `docs/smoke-tests.md`).

mod audit;
mod docking;
mod first_orbit;
mod lunar_module;
mod pilot;
mod planning;
mod report;
mod saturn_ib;
mod saturn_v;
pub mod site;
pub mod watch;
pub mod world;

#[path = "../../osp_sim/tests/golden/record.rs"]
#[allow(dead_code)]
mod record;

use std::path::PathBuf;

use world::{Flight, PhaseSpec, Status, World};

/// A flight to run and write up.
pub struct Mission {
    pub id: &'static str,
    pub title: &'static str,
    /// What the real mission did, in a few sentences.
    pub summary: &'static str,
    /// The scenario (in the mods) the flight starts from.
    pub scenario: &'static str,
    pub plan: &'static [PhaseSpec],
    pub fly: fn(&mut World) -> Flight,
}

/// Every mission there is, in the order `cargo smoke` lists them.
pub const MISSIONS: &[&Mission] = &[
    &saturn_v::MISSION,
    &lunar_module::MISSION,
    &docking::MISSION,
    &first_orbit::MISSION,
    &saturn_ib::MISSION,
];

/// The mission called `id`.
pub fn mission(id: &str) -> Option<&'static Mission> {
    MISSIONS.iter().copied().find(|m| m.id == id)
}

/// What a flight came to, once written up.
pub struct WriteUp {
    /// One line: the counts of each result, and whether it stopped early.
    pub summary: String,
    /// The report's page.
    pub page: PathBuf,
    /// The checks that failed, one line each.
    pub failed: Vec<String>,
}

/// Closes the books on `mission`, flown in `world`, and writes its report.
pub fn write_up(mission: &Mission, world: &mut World, stopped_early: bool) -> WriteUp {
    world.finish();
    let page = report::write(mission, world, stopped_early);
    let count = |s: Status| world.findings.iter().filter(|f| f.status == s).count();
    let summary = format!(
        "{}: {} passed, {} failed, {} known, {} gaps, {} bridged{}",
        mission.title,
        count(Status::Pass),
        count(Status::Fail),
        count(Status::Known),
        count(Status::Gap),
        count(Status::Bridged),
        if stopped_early { ", stopped early" } else { "" },
    );
    let failed = world
        .failures()
        .iter()
        .map(|f| format!("[{}] {}: {}", f.area.name(), f.title, f.detail))
        .collect();
    WriteUp {
        summary,
        page,
        failed,
    }
}

/// Flies `mission` headless, writes its report and fails if a check did.
pub fn run(mission: &Mission) {
    let mut world = World::new(mission.scenario, mission.plan);
    let flight = (mission.fly)(&mut world);
    let done = write_up(mission, &mut world, flight.is_err());
    eprintln!("\n{}\nreport: {}\n", done.summary, done.page.display());
    if std::env::var_os("OSP_SMOKE_OPEN").is_some() {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let _ = std::process::Command::new(opener).arg(&done.page).status();
    }
    assert!(
        done.failed.is_empty(),
        "{} failed:\n{}",
        mission.title,
        done.failed.join("\n")
    );
}
