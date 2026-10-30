//! Mission smoke tests: whole flights, start to finish, flown the way a player flies them.
//!
//! Each mission is a script of phases (see `saturn_v.rs`). It loads the real mods, starts a
//! scenario through the game's own code and then only uses what a player has: the throttle,
//! the stage key, SAS, RCS, burns planned on the map. Nothing is placed or patched in. Where
//! the sim cannot do what the real mission did, the script says so in the report instead of
//! hiding it, and flies on if it can.
//!
//! ```sh
//! cargo smoke               # every mission
//! cargo smoke saturn_v      # one
//! ```
//!
//! The report lands in `target/smoke/<mission>/index.html` (see `docs/smoke-tests.md`).

mod audit;
mod pilot;
mod planning;
mod report;
mod saturn_v;
mod site;
mod world;

#[path = "../golden/record.rs"]
#[allow(dead_code)]
mod record;

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

fn run(mission: &Mission) {
    let mut world = World::new(mission.scenario, mission.plan);
    let flight = (mission.fly)(&mut world);
    world.finish();
    let path = report::write(mission, &world, flight.is_err());
    let count = |s: Status| world.findings.iter().filter(|f| f.status == s).count();
    eprintln!(
        "\n{}: {} passed, {} failed, {} known, {} gaps, {} bridged{}\nreport: {}\n",
        mission.title,
        count(Status::Pass),
        count(Status::Fail),
        count(Status::Known),
        count(Status::Gap),
        count(Status::Bridged),
        if flight.is_err() {
            ", stopped early"
        } else {
            ""
        },
        path.display(),
    );
    if std::env::var_os("OSP_SMOKE_OPEN").is_some() {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let _ = std::process::Command::new(opener).arg(&path).status();
    }
    let failed: Vec<_> = world
        .failures()
        .iter()
        .map(|f| format!("[{}] {}: {}", f.area.name(), f.title, f.detail))
        .collect();
    assert!(
        failed.is_empty(),
        "{} failed:\n{}",
        mission.title,
        failed.join("\n")
    );
}

#[test]
fn saturn_v() {
    run(&saturn_v::MISSION);
}
