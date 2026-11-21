//! The mission smoke tests (`cargo smoke`): each mission of `osp_smoke` flown headless, start to
//! finish, with its report in `target/smoke/<mission>/` (see `docs/smoke-tests.md`).

use osp_smoke::{
    mission, run,
    watch::{Shown, Speed, Watch},
    world::World,
};

#[test]
fn saturn_v() {
    run(mission("saturn_v").unwrap());
}

#[test]
fn lunar_module() {
    run(mission("lunar_module").unwrap());
}

#[test]
fn docking() {
    run(mission("docking").unwrap());
}

#[test]
fn first_orbit() {
    run(mission("first_orbit").unwrap());
}

#[test]
fn saturn_ib() {
    run(mission("saturn_ib").unwrap());
}

#[test]
fn career_casual() {
    run(mission("career_casual").unwrap());
}

#[test]
fn career_realistic() {
    run(mission("career_realistic").unwrap());
}

#[test]
fn career_casual_orbit() {
    run(mission("career_casual_orbit").unwrap());
}

#[test]
fn career_casual_moon() {
    run(mission("career_casual_moon").unwrap());
}

/// A flight that is watched (`--watch`) is handed to whoever shows it and back many times a
/// second, at whatever speed they ask; it must still be the very same flight as the headless one.
#[test]
fn a_watched_flight_is_the_same_flight() {
    let flown = mission("first_orbit").unwrap();
    let mut alone = World::new(flown.scenario, flown.plan);
    (flown.fly)(&mut alone).unwrap();

    // What the game would show before the flight arrives: any universe will do.
    let (system, time) = (alone.universe.system.clone(), alone.universe.time());
    let mut shown = osp_sim::universe::Universe::new(system, Vec::new(), time);
    let mut watch = Watch::start(flown);
    let mut frames = 0;
    let mut speed = Speed::default();
    let summary = loop {
        // Faster and slower as someone watching would ask, so slices cut the flight anywhere.
        speed.physics = [4.0, 16.0, 1.0, 64.0][frames / 200 % 4];
        match watch.show(&mut shown, 1.0 / 60.0, speed) {
            Shown::Loading => std::thread::sleep(std::time::Duration::from_millis(5)),
            Shown::Started | Shown::Flew { .. } => frames += 1,
            Shown::Finished(summary) => break summary,
            Shown::Stopped(why) => panic!("the watched flight stopped: {why}"),
        }
        assert!(frames < 1_000_000, "the watched flight never ended");
    };
    assert!(frames > 100, "it should take many frames: {frames}");
    assert!(summary.contains("0 failed"), "{summary}");

    let at = |universe: &osp_sim::universe::Universe| {
        let v = &universe.vessels[universe.find_vessel(universe.focus.unwrap()).unwrap()];
        (
            universe.time(),
            v.state_at(&universe.system, universe.time()),
        )
    };
    let (time, state) = at(&shown);
    let (alone_time, alone_state) = at(&alone.universe);
    assert_eq!(time, alone_time);
    assert_eq!(state.position, alone_state.position);
    assert_eq!(state.velocity, alone_state.velocity);
}

/// The base game's `controls` tutorial, played headless over the pad scenario the way a player
/// plays it: each step done with the control it names. The two steps the game judges (reading
/// the navball, the map being open) are completed as its buttons and UI data do.
#[test]
fn the_controls_tutorial_plays_through() {
    use osp_sim::{attitude::SasMode, tutorial::TutorialState};

    let mut w = World::new("pad", &[]);
    w.start_tutorial("controls");
    let on = |w: &mut World| {
        w.tutorial()
            .step()
            .map(|s| s.id.clone())
            .unwrap_or_default()
    };
    let tick = |w: &mut World| {
        w.fly(0.2, |_| false);
    };
    assert_eq!(on(&mut w), "throttle");

    // Waiting does not move it on.
    tick(&mut w);
    assert_eq!(on(&mut w), "throttle");
    w.vessel_mut().set_throttle(1.0);
    tick(&mut w);
    assert_eq!(on(&mut w), "stage");
    w.stage();
    tick(&mut w);
    // The navball is for reading: the player presses Continue.
    assert_eq!(on(&mut w), "navball");
    assert!(w.tutorial().step().unwrap().is_manual());
    tick(&mut w);
    assert_eq!(on(&mut w), "navball");
    w.tutorial().complete();
    assert_eq!(on(&mut w), "sas");
    w.sas(SasMode::Stability);
    tick(&mut w);
    assert_eq!(on(&mut w), "climb");
    // The rocket climbs to a kilometre by itself.
    let climbed = w.fly(120.0, |w| {
        w.tutorial().step().is_none_or(|s| s.id != "climb")
    });
    assert!(climbed, "stuck at {} m", w.t());
    // The map: the UI says it is open, and the game completes the step.
    assert_eq!(on(&mut w), "map");
    assert_eq!(w.tutorial().step().unwrap().ui.as_deref(), Some("view.map"));
    w.tutorial().complete();
    assert_eq!(w.tutorial().state(), TutorialState::Finished);
}
