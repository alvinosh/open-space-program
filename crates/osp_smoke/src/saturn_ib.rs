//! Apollo 7 on the Saturn IB, from LC-34 to orbit: the S-IB burns out and falls away, the
//! S-IVB's ullage motors settle its propellant and the J-2 lights on it, all by the craft's own
//! sequences and the stage key.

use crate::{
    Mission,
    pilot::{Ascent, Profile},
    world::{Area, Flight, PhaseSpec, World},
};

pub static MISSION: Mission = Mission {
    id: "saturn_ib",
    title: "Saturn IB: Apollo 7 to orbit",
    summary: "The Saturn IB lifts off from LC-34 with the Apollo CSM, drops its S-IB, lights the \
              S-IVB on its ullage motors and reaches a low orbit, 231 by 285 km on the real day.",
    scenario: "apollo7",
    mode: None,
    plan: PLAN,
    fly,
};

const PLAN: &[PhaseSpec] = &[
    PhaseSpec {
        id: "s_ib",
        title: "S-IB first stage",
        goal: "Eight H-1s lift the stack and bend it over the Atlantic until they burn out.",
    },
    PhaseSpec {
        id: "s_ivb",
        title: "S-IVB to orbit",
        goal: "The S-IB falls away, the ullage motors settle the propellant, and the J-2 burns \
               until the orbit's lowest point is clear of the air.",
    },
];

/// Orbit flown for, and the periapsis at which the engine is cut, m.
const ORBIT_ALTITUDE: f64 = 230_000.0;
const CUTOFF_PERIAPSIS: f64 = 225_000.0;

fn fly(w: &mut World) -> Flight {
    w.phase("s_ib");
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
            inclination: 31.6,
            altitude: ORBIT_ALTITUDE,
            clear: 130.0,
            turn_end: 150_000.0,
            turn_shape: 0.5,
            max_aoa: 6.0,
            vacuum_q: 100.0,
        },
    );
    let burnt_out = w.fly(300.0, |w| {
        ascent.steer(w);
        !w.vessel().propulsion.engines.iter().any(|e| e.running) && w.t() > 30.0
    });
    w.must(Area::Propulsion, "The S-IB burns out", burnt_out, "")?;
    let burn = w.met();
    w.within(
        Area::Propulsion,
        "S-IB burn time",
        burn,
        125.0..=170.0,
        "s",
        "Apollo 7: 141 s",
    );

    w.phase("s_ivb");
    w.stage();
    w.fly(1.0, |w| {
        ascent.steer(w);
        false
    });
    w.stage();
    w.fly(5.0, |w| {
        ascent.steer(w);
        false
    });
    let j2 =
        (w.vessel().propulsion.engines.iter()).any(|e| e.running && e.def.id.starts_with("J-2"));
    let log: Vec<_> = (w.vessel().log.iter())
        .rev()
        .take(4)
        .map(|e| e.message.clone())
        .collect();
    w.must(
        Area::Staging,
        "The J-2 lights on its ullage motors' push",
        j2,
        log.join("; "),
    )?;

    let mut cut = false;
    w.fly(900.0, |w| {
        ascent.steer(w);
        let orbit = w.vessel().orbit(&w.universe.system, w.now());
        let radius = w.universe.system.body(w.vessel().parent()).radius;
        if orbit.periapsis() - radius >= CUTOFF_PERIAPSIS {
            w.vessel_mut().set_throttle(0.0);
            cut = true;
        }
        cut
    });
    w.must(
        Area::Orbit,
        "The S-IVB reaches an orbit clear of the air",
        cut,
        "",
    )?;
    let minutes = w.met() / 60.0;
    w.within(
        Area::Orbit,
        "Time to orbit",
        minutes,
        8.0..=13.0,
        "min",
        "Apollo 7: 10 min 27 s",
    );
    w.fly(1.0, |_| false);
    let held = w.vessel().flight.mass / 1e3;
    w.within(
        Area::Orbit,
        "What reaches orbit",
        held,
        20.0..=60.0,
        "t",
        "the S-IVB and the CSM with what is left in the tanks",
    );
    Ok(())
}
