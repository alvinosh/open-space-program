//! The glider rockets off the runway on a dolly, then lets go of its boosters and wheels.

use glam::DVec3;

use super::{Vessel, tests::*};
use crate::{gear, system::SolarSystem, system::tests::rss, time::SimTime};

/// The glider at the start of the Shuttle Landing Facility's runway.
fn on_runway(system: &SolarSystem) -> (Vessel, SimTime) {
    let site = launch_sites()
        .into_iter()
        .find(|s| s.id == "cape_slf")
        .unwrap();
    let t = SimTime::J2000;
    let v = Vessel::on_runway(&glider(), &catalog(), system, &site, t).unwrap();
    (v, t)
}

/// Fly one tick with the stick holding the nose `pitch` degrees above the horizon.
fn hold(v: &mut Vessel, system: &SolarSystem, t: &mut SimTime, pitch: f64) {
    let (_, _, now) = on_the_ground(v, system, *t);
    let rate = v.attitude_state(system, *t).angular_velocity.x;
    v.rotation_input = DVec3::new(
        (0.08 * (pitch - now) - 2.0 * rate).clamp(-1.0, 1.0),
        0.0,
        0.0,
    );
    run(v, system, t, 0.02);
}

#[test]
fn the_dolly_carries_its_wheels_out_from_the_belly() {
    let system = rss();
    let (v, t) = on_runway(&system);
    // Three wheels, each below the belly, not on the cabin's axis where the dolly's own
    // axis is not.
    let legs = gear::legs(&v.propulsion.sections);
    assert_eq!(legs.len(), 3);
    assert!(legs.iter().all(|l| l.mount.y > 0.5), "{legs:?}");
    // Level on its wheels, the axis a wheel and a belly above the runway.
    let (height, speed, pitch) = on_the_ground(&v, &system, t);
    assert!(v.is_landed() && v.wheels_down);
    assert!(
        (height - 1.41).abs() < 0.05 && speed == 0.0 && pitch.abs() < 0.2,
        "{height} {speed} {pitch}"
    );
}

#[test]
fn the_glider_rockets_off_the_runway_and_lets_go_of_its_boosters_and_wheels() {
    let system = rss();
    let (mut v, mut t) = on_runway(&system);
    run(&mut v, &system, &mut t, 2.0);
    assert!(v.is_landed() && v.wheels_down);
    let dry = v.mass.mass;
    // Throttle up, brakes off, and Space lights both boosters.
    v.set_throttle(1.0);
    v.brakes = false;
    v.activate_next_stage(&system, t);
    let burning = |v: &Vessel| v.flight.thrust > 0.0;
    let (mut fastest, mut lifted, mut roll): (f64, Option<f64>, f64) = (0.0, None, 0.0);
    let mut started = false;
    for _ in 0..(20 * 50) {
        let (_, speed, _) = on_the_ground(&v, &system, t);
        // Level until 38 m/s, then rotate to climb out at 8°.
        hold(
            &mut v,
            &system,
            &mut t,
            if speed > 38.0 { 8.0 } else { 0.0 },
        );
        fastest = fastest.max(speed);
        if v.wheels_down {
            roll += speed * 0.02;
        } else if lifted.is_none() {
            lifted = Some(speed);
        }
        started |= burning(&v);
        if started && !burning(&v) {
            break;
        }
    }
    assert!(started && !burning(&v), "still burning");
    // Eleven seconds of two bottles: off the ground well before the burn is over and
    // through 65 m/s by the end of it.
    let lifted = lifted.expect("never left the ground");
    assert!(
        (35.0..55.0).contains(&lifted) && roll < 300.0,
        "{lifted} {roll}"
    );
    assert!((65.0..78.0).contains(&fastest), "{fastest}");
    assert!(!v.wheels_down && v.log.iter().any(|e| e.message == "Airborne"));
    // The next press lets the empty boosters go, the one after the wheels.
    let boosters = v.activate_next_stage(&system, t);
    assert_eq!(boosters.len(), 2);
    assert_eq!(gear::legs(&v.propulsion.sections).len(), 3);
    let dolly = v.activate_next_stage(&system, t);
    assert_eq!(dolly.len(), 1);
    assert!(gear::legs(&v.propulsion.sections).is_empty());
    let clean = v.mass.mass;
    assert!(
        (500.0..600.0).contains(&clean) && dry - clean > 100.0,
        "{dry} {clean}"
    );
    // It zooms to bleed the speed off, then glides on its own for a minute.
    for _ in 0..(20 * 50) {
        hold(&mut v, &system, &mut t, 6.0);
    }
    v.rotation_input = DVec3::ZERO;
    run(&mut v, &system, &mut t, 40.0);
    let (height, speed, _) = on_the_ground(&v, &system, t);
    assert!(
        !v.is_landed() && height > 20.0 && (25.0..60.0).contains(&speed),
        "{height} {speed}"
    );
    assert!(
        !v.log.iter().any(|e| e.message.starts_with("Crashed")) && !v.is_destroyed(),
        "{:?}",
        v.log
    );
}
