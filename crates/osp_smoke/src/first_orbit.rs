//! The first launch: the Pathfinder from the pad to orbit, flown with a player's controls
//! (throttle, the stage key, a pitch program), and the pad scenario's goals ticking off.

use osp_sim::{DVec3, mission::State};

use crate::{
    Mission,
    pilot::{Ascent, Profile},
    world::{Area, Flight, PhaseSpec, World},
};

pub static MISSION: Mission = Mission {
    id: "first_orbit",
    title: "Pathfinder: first launch to orbit",
    summary: "The two-stage Pathfinder lifts off from LC-39A, turns east, drops its booster, \
              lights the upper stage and settles into a circular orbit. The scenario's goals \
              (leave the pad, reach space, reach orbit) are met as it goes.",
    scenario: "pad",
    plan: PLAN,
    fly,
};

const PLAN: &[PhaseSpec] = &[
    PhaseSpec {
        id: "ascent",
        title: "Liftoff and the climb",
        goal: "The booster lifts the stack off the pad and bends it over to the east; it burns \
               out and the upper stage takes over.",
    },
    PhaseSpec {
        id: "orbit",
        title: "Into orbit",
        goal: "The upper stage burns until the orbit's lowest point is clear of the air, and \
               the engine is shut down.",
    },
];

/// Orbit altitude flown for, m, and the periapsis at which the engine is cut.
const ORBIT_ALTITUDE: f64 = 200_000.0;
const CUTOFF_PERIAPSIS: f64 = 195_000.0;
/// Above this the fairing is let go, m.
const FAIRING_ALTITUDE: f64 = 110_000.0;

fn fly(w: &mut World) -> Flight {
    w.phase("ascent");
    w.vessel_mut().set_throttle(1.0);
    w.stage();
    let lifted = w.fly(30.0, |w| w.liftoff.is_some());
    w.must(
        Area::Propulsion,
        "Lifts off within 30 s of ignition",
        lifted,
        "",
    )?;
    let mut ascent = Ascent::new(
        w,
        Profile {
            inclination: 28.6,
            altitude: ORBIT_ALTITUDE,
            clear: 60.0,
            turn_end: 120_000.0,
            turn_shape: 0.5,
            max_aoa: 6.0,
            vacuum_q: 100.0,
        },
    );

    // The booster burns out and the stage key lets it go. The upper stage's propellant floats
    // in free fall, so the throttle stays shut while the RCS pushes the stack forward (RCS on
    // by the stage key, then the forward translation key held); with the propellant settled
    // the throttle goes up and the engine lights.
    let burnt_out = w.fly(300.0, |w| {
        ascent.steer(w);
        !w.vessel().propulsion.engines.iter().any(|e| e.running) && w.t() > 30.0
    });
    w.must(Area::Propulsion, "The booster burns out", burnt_out, "")?;
    w.phase("orbit");
    w.vessel_mut().set_throttle(0.0);
    w.stage();
    w.vessel_mut().translation_input = DVec3::Z;
    w.fly(3.0, |w| {
        ascent.steer(w);
        false
    });
    w.vessel_mut().set_throttle(1.0);
    w.fly(2.0, |w| {
        ascent.steer(w);
        false
    });
    w.vessel_mut().translation_input = DVec3::ZERO;
    let lit = w.vessel().propulsion.engines.iter().any(|e| e.running);
    w.must(
        Area::Propulsion,
        "The upper stage lights on RCS ullage",
        lit,
        "",
    )?;

    // The fairing goes once the air is gone, and the throttle is cut when the orbit clears it.
    let (mut shed, mut cut) = (false, false);
    w.fly(900.0, |w| {
        ascent.steer(w);
        let (altitude, periapsis) = altitude_and_periapsis(w);
        if !shed && altitude > FAIRING_ALTITUDE {
            w.stage();
            shed = true;
        }
        if periapsis >= CUTOFF_PERIAPSIS {
            w.vessel_mut().set_throttle(0.0);
            cut = true;
        }
        cut
    });
    w.must(
        Area::Orbit,
        "The upper stage reaches an orbit clear of the air",
        cut,
        "",
    )?;
    // A moment for the mission to see it.
    w.fly(1.0, |_| false);

    let mission = w
        .universe
        .mission
        .as_ref()
        .expect("the pad scenario has goals");
    let goals: Vec<_> = mission
        .goals()
        .iter()
        .map(|g| (g.def.text.clone(), g.met_at))
        .collect();
    let stats = *mission.stats();
    let state = mission.state().clone();
    for (text, met_at) in goals {
        w.check(
            Area::Orbit,
            &format!("Goal: {text}"),
            met_at.is_some(),
            met_at.map_or("not met".into(), |t| format!("met {t:.0} s in")),
        );
    }
    w.check(
        Area::Orbit,
        "The mission is passed",
        state == State::Passed,
        format!("{state:?}"),
    );
    w.info(
        Area::Orbit,
        "What the results screen shows",
        format!(
            "{:.0} s, {:.0} m/s of delta-v, {:.1} g at most, {:.0} km at highest",
            stats.elapsed,
            stats.delta_v,
            stats.peak_g,
            stats.max_altitude / 1e3
        ),
    );
    Ok(())
}

fn altitude_and_periapsis(w: &World) -> (f64, f64) {
    let (system, now) = (&w.universe.system, w.now());
    let v = w.vessel();
    let radius = system.body(v.parent()).radius;
    (
        v.state_at(system, now).position.length() - radius,
        v.orbit(system, now).periapsis() - radius,
    )
}
