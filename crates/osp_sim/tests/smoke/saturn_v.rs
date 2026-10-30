//! Apollo 11 on the Saturn V, from the pad at LC-39A to splashdown.
//!
//! The numbers in the checks are the real mission's (Apollo 11 flight journal, the Saturn V
//! flight manual), with room for the game's simpler physics.

use osp_sim::{DVec3, attitude::SasMode, plan::Burn};

use crate::{
    Mission, audit,
    pilot::{Ascent, Profile},
    planning,
    world::{Area, Flight, PhaseSpec, World},
};

pub static MISSION: Mission = Mission {
    id: "saturn_v",
    title: "Saturn V: Apollo 11",
    summary: "Launch from LC-39A, a parking orbit, translunar injection, lunar orbit, the landing, \
              the return and the entry into the Pacific.",
    scenario: "apollo11",
    plan: PLAN,
    fly,
};

const PLAN: &[PhaseSpec] = &[
    PhaseSpec {
        id: "pad",
        title: "On the pad",
        goal: "The stack stands on LC-39A with 2 970 t of propellant and spacecraft; five F-1s \
               light and the hold-downs release.",
    },
    PhaseSpec {
        id: "s_ic",
        title: "S-IC first stage",
        goal: "Five F-1s lift the stack for 2 min 42 s to 67 km and 2.4 km/s; max Q is 35 kPa \
               at 83 s.",
    },
    PhaseSpec {
        id: "s_ii",
        title: "S-II second stage",
        goal: "Five J-2s burn 6 min 24 s to 185 km and 6.8 km/s.",
    },
    PhaseSpec {
        id: "s_ivb_1",
        title: "S-IVB orbit insertion",
        goal: "One J-2 burns 2 min 27 s into a 191 by 183 km orbit at 32.5°.",
    },
    PhaseSpec {
        id: "parking",
        title: "Parking orbit",
        goal: "Two and a half hours and two revolutions of checkout, to the translunar window.",
    },
    PhaseSpec {
        id: "tli",
        title: "Translunar injection",
        goal: "The S-IVB relights on the second revolution for 5 min 47 s: 3.2 km/s towards the Moon.",
    },
    PhaseSpec {
        id: "transposition",
        title: "Separation and transposition",
        goal: "The S-IVB lets go, the CSM turns round, docks with the lunar module in the adapter \
               and pulls it out.",
    },
    PhaseSpec {
        id: "coast_out",
        title: "Translunar coast",
        goal: "Three days to the Moon with a 3 m/s midcourse correction; the Moon's sphere of \
               influence at 61 h.",
    },
    PhaseSpec {
        id: "loi",
        title: "Lunar orbit insertion",
        goal: "The SPS burns 357 s, 890 m/s, behind the Moon into a 313 by 111 km orbit.",
    },
    PhaseSpec {
        id: "lunar_orbit",
        title: "Lunar orbit",
        goal: "Circularised at 100 km; the lunar module's crew checks it out.",
    },
    PhaseSpec {
        id: "descent",
        title: "Powered descent and landing",
        goal: "The lunar module undocks and lands in the Sea of Tranquility, 102 h 45 min.",
    },
    PhaseSpec {
        id: "surface",
        title: "On the surface",
        goal: "21 h 36 min on the Moon: two crew walk for 2 h 31 min; the descent stage stays.",
    },
    PhaseSpec {
        id: "ascent",
        title: "Ascent, rendezvous and docking",
        goal: "The ascent stage lifts off, meets the CSM and docks; the crew moves across.",
    },
    PhaseSpec {
        id: "tei",
        title: "Trans-Earth injection",
        goal: "The SPS burns 150 s, 1 000 m/s, behind the Moon at 135 h 24 min.",
    },
    PhaseSpec {
        id: "coast_home",
        title: "Trans-Earth coast",
        goal: "Two and a half days home with a midcourse correction.",
    },
    PhaseSpec {
        id: "entry",
        title: "Entry and splashdown",
        goal: "The service module is cut loose, the command module enters at 11 km/s (6.5 g peak) \
               and lands on three parachutes in the Pacific.",
    },
];

/// Launch day's targets. Inclination and altitude are the flown ones.
const INCLINATION: f64 = 32.5;
const ORBIT_ALTITUDE: f64 = 185_000.0;
const EARTH_RADIUS: f64 = 6_371_000.0;

fn fly(w: &mut World) -> Flight {
    pad(w)?;
    let mut ascent = Ascent::new(
        w,
        Profile {
            inclination: INCLINATION,
            altitude: ORBIT_ALTITUDE,
            clear: 130.0,
            turn_end: 160_000.0,
            turn_shape: 0.5,
            max_aoa: 6.0,
            vacuum_q: 100.0,
        },
    );
    s_ic(w, &mut ascent)?;
    s_ii(w, &mut ascent)?;
    s_ivb_insertion(w, &mut ascent)?;
    parking(w)?;
    tli(w)?;
    transposition(w)?;
    coast_out(w)?;
    loi(w)?;
    lunar_orbit(w)?;
    landing(w)?;
    tei(w)?;
    coast_home(w)?;
    entry(w)?;
    Ok(())
}

fn pad(w: &mut World) -> Flight {
    w.phase("pad");
    let parts = audit::inspect(w);
    audit::report(w, &parts);
    w.audit = Some(parts);
    let (mass, thrust) = {
        let v = w.vessel();
        let thrust = v
            .propulsion
            .engines
            .iter()
            .filter(|e| e.def.id.starts_with("F-1"))
            .map(|e| e.def.thrust_vac)
            .sum::<f64>();
        (v.flight.mass, thrust)
    };
    w.within(
        Area::Build,
        "Liftoff mass",
        mass / 1e3,
        2_850.0..=3_050.0,
        "t",
        "Apollo 11: 2 938 t",
    );
    w.within(
        Area::Propulsion,
        "Thrust to weight at ignition",
        thrust / (mass * 9.80665),
        1.2..=1.5,
        "",
        "S-IC vacuum thrust over liftoff weight; the real stack was 1.25 at the pad",
    );
    w.vessel_mut().set_throttle(1.0);
    w.stage();
    let lifted = w.fly(30.0, |w| w.liftoff.is_some());
    w.must(
        Area::Propulsion,
        "Lifts off within 30 s of ignition",
        lifted,
        "",
    )?;
    Ok(())
}

fn dry(w: &World) -> bool {
    !w.vessel().propulsion.engines.iter().any(|e| e.running)
}

fn altitude_and_speed(w: &World) -> (f64, f64) {
    let state = w.vessel().state_at(&w.universe.system, w.now());
    (
        state.position.length() - EARTH_RADIUS,
        state.velocity.length(),
    )
}

fn s_ic(w: &mut World, ascent: &mut Ascent) -> Flight {
    w.phase("s_ic");
    let mut since = 0.0;
    let cutoff = w.fly(300.0, |w| {
        ascent.steer(w);
        since += 0.02;
        dry(w) && since > 1.0 && w.t() > 30.0
    });
    w.must(Area::Propulsion, "S-IC burns out", cutoff, "")?;
    let met = w.met();
    let (altitude, speed) = altitude_and_speed(w);
    w.within(
        Area::Propulsion,
        "S-IC burn time",
        met,
        150.0..=175.0,
        "s",
        "Apollo 11: 161.6 s",
    );
    w.within(
        Area::Orbit,
        "Altitude at S-IC cutoff",
        altitude / 1e3,
        55.0..=85.0,
        "km",
        "Apollo 11: 67 km",
    );
    w.within(
        Area::Orbit,
        "Speed at S-IC cutoff",
        speed / 1e3,
        2.2..=3.0,
        "km/s",
        "Apollo 11: 2.76 km/s inertial",
    );
    if let Some((q, at)) = w.peak("s_ic", |s| s.q) {
        w.within(
            Area::Aero,
            "Max Q",
            q / 1e3,
            25.0..=45.0,
            "kPa",
            "Apollo 11: 35 kPa",
        );
        w.within(
            Area::Aero,
            "Max Q comes",
            at,
            60.0..=100.0,
            "s after liftoff",
            "Apollo 11: 83 s",
        );
    }
    if let Some((aoa, _)) = w.peak("s_ic", |s| if s.q > 5_000.0 { s.aoa.abs() } else { 0.0 }) {
        w.within(
            Area::Control,
            "Angle of attack in thick air",
            aoa,
            0.0..=10.0,
            "°",
            "flown at near zero",
        );
    }
    if let Some((load, _)) = w.peak("s_ic", |s| s.load) {
        w.check(
            Area::Structure,
            "The stack holds through max Q and burnout",
            load < 1.0,
            format!("most loaded part at {:.0}% of its strength", load * 100.0),
        );
    }
    if let Some((g, at)) = w.peak("s_ic", |s| s.g) {
        if g <= 4.6 {
            w.within(
                Area::Propulsion,
                "Peak acceleration",
                g,
                3.0..=4.6,
                "g",
                "Apollo 11: 3.9 g",
            );
        } else {
            w.known(
                Area::Staging,
                "No centre engine cutoff: acceleration runs on past 4 g",
                "T026",
                format!("{g:.1} g at {at:.0} s. The real S-IC shut its centre F-1 down at 2 min 15 s to hold 3.9 g (Apollo 11); a sequence can light engines but not cut one."),
            );
        }
    }
    Ok(())
}

fn s_ii(w: &mut World, ascent: &mut Ascent) -> Flight {
    // The S-IC falls away and the S-II's ullage motors light; a second later the J-2s, and
    // 30 s on the spent interstage goes.
    w.phase("s_ii");
    w.stage();
    let mut since = 0.0;
    let mut pressed = 0;
    let cutoff = w.fly(500.0, |w| {
        ascent.steer(w);
        since += 0.02;
        let press = match pressed {
            0 => since > 1.0,
            1 => since > 30.0,
            _ => false,
        };
        if press {
            w.stage();
            pressed += 1;
            since = 0.0;
        }
        pressed == 2 && dry(w) && since > 1.0
    });
    w.must(Area::Propulsion, "S-II burns out", cutoff, "")?;
    let (altitude, speed) = altitude_and_speed(w);
    let failed = w.vessel().log.iter().any(|e| e.message.contains("failed"));
    w.check(
        Area::Propulsion,
        "The J-2s light on the S-II ullage motors",
        !failed,
        "no engine reported a failed start",
    );
    let (_, burn) = w.span("s_ii").unwrap();
    w.within(
        Area::Propulsion,
        "S-II burn time",
        burn,
        340.0..=400.0,
        "s",
        "Apollo 11: 384 s",
    );
    w.within(
        Area::Orbit,
        "Altitude at S-II cutoff",
        altitude / 1e3,
        165.0..=205.0,
        "km",
        "Apollo 11: 185 km",
    );
    w.within(
        Area::Orbit,
        "Speed at S-II cutoff",
        speed / 1e3,
        6.5..=7.2,
        "km/s",
        "Apollo 11: 6.84 km/s inertial",
    );
    Ok(())
}

fn s_ivb_insertion(w: &mut World, ascent: &mut Ascent) -> Flight {
    w.phase("s_ivb_1");
    // The S-II falls away and the S-IVB's ullage motors light; a second later its J-2.
    w.stage();
    let mut since = 0.0;
    let mut lit = false;
    let done = w.fly(400.0, |w| {
        ascent.steer(w);
        since += 0.02;
        if !lit && since > 1.0 {
            w.stage();
            lit = true;
        }
        let periapsis = w.vessel().orbit(&w.universe.system, w.now()).periapsis();
        lit && periapsis - EARTH_RADIUS >= ORBIT_ALTITUDE - 3_000.0
    });
    w.vessel_mut().set_throttle(0.0);
    w.vessel_mut().rotation_input = osp_sim::DVec3::ZERO;
    w.must(Area::Orbit, "The S-IVB reaches orbit", done, "")?;
    let met = w.met();
    let (orbit, pole) = {
        let v = w.vessel();
        let system = &w.universe.system;
        (v.orbit(system, w.now()), system.body(v.parent()).pole())
    };
    let peri = orbit.periapsis() - EARTH_RADIUS;
    let apo = orbit.apoapsis().unwrap_or(f64::NAN) - EARTH_RADIUS;
    let (_, burn) = w.span("s_ivb_1").unwrap();
    w.within(
        Area::Propulsion,
        "S-IVB burn time",
        burn,
        110.0..=180.0,
        "s",
        "Apollo 11: 147 s",
    );
    w.within(
        Area::Orbit,
        "Time to orbit",
        met / 60.0,
        10.5..=13.5,
        "min",
        "Apollo 11: 11 min 42 s",
    );
    w.within(
        Area::Orbit,
        "Perigee",
        peri / 1e3,
        175.0..=200.0,
        "km",
        "Apollo 11: 183 km",
    );
    w.within(
        Area::Orbit,
        "Apogee",
        apo / 1e3,
        180.0..=215.0,
        "km",
        "Apollo 11: 191 km",
    );
    w.within(
        Area::Orbit,
        "Inclination",
        orbit.inclination_to(pole).to_degrees(),
        31.5..=33.5,
        "°",
        "Apollo 11: 32.5°",
    );
    Ok(())
}

fn parking(w: &mut World) -> Flight {
    w.phase("parking");
    // Checkout in orbit: RCS on, the attitude held where the burn left it. Turning 139 t on
    // small thrusters costs propellant, so the stack turns once, onto the translunar burn.
    w.vessel_mut().set_rcs(true);
    w.sas(SasMode::Stability);
    w.fly(60.0, |_| false);

    // The translunar window: the transfer planner, from the second revolution on.
    let moon = w.universe.system.find("Moon").unwrap();
    planning::target_body(w, moon);
    let period = w
        .vessel()
        .orbit(&w.universe.system, w.now())
        .period()
        .unwrap();
    let found = planning::transfer_to(w, moon, 1.4 * period, 1.6 * period);
    w.must(
        Area::Orbit,
        "The transfer planner finds a way to the Moon",
        found.is_some(),
        "",
    )?;
    let departure = found.unwrap();
    let wait = departure.transfer.departure.seconds_since(w.now());
    w.within(
        Area::Orbit,
        "Translunar departure Δv from the planner",
        departure.transfer.depart_dv,
        3_000.0..=3_400.0,
        "m/s",
        "Apollo 11: 3 180 m/s",
    );
    w.within(
        Area::Orbit,
        "Time of translunar injection",
        (w.met() + wait) / 3600.0,
        1.5..=3.5,
        "h after liftoff",
        "Apollo 11: 2 h 44 min",
    );
    w.info(
        Area::Orbit,
        "Transfer planner's cheapest departure",
        format!(
            "Leaves in {:.0} min, takes {:.1} h, costs {:.0} m/s to depart and {:.0} m/s to be captured.",
            wait / 60.0,
            departure.transfer.tof / 3600.0,
            departure.transfer.depart_dv,
            departure.transfer.arrive_dv
        ),
    );
    w.vessel_mut().plan.add(departure.burn);
    let reached = planning::aim(w, 0, moon, LUNAR_PERIAPSIS);
    w.within(
        Area::Orbit,
        "Aiming the burn at the Moon",
        reached.unwrap_or(f64::NAN) / 1e3,
        (LUNAR_PERIAPSIS - 30_000.0) / 1e3..=(LUNAR_PERIAPSIS + 30_000.0) / 1e3,
        "km periapsis altitude",
        "Apollo 11: 111 km",
    );
    Ok(())
}

/// Periapsis altitude over the Moon the translunar leg aims for, m.
const LUNAR_PERIAPSIS: f64 = 111_000.0;

fn tli(w: &mut World) -> Flight {
    let (planned, _) = planning::next_burn(w).unwrap();
    // Turn onto the burn ten minutes ahead: the stack is slow to turn on its small thrusters.
    let off = planning::point_at_burn(w, 900.0, 800.0);
    w.within(
        Area::Control,
        "Turns onto the burn direction before translunar injection",
        off.unwrap_or(f64::NAN),
        0.0..=2.0,
        "° off the burn",
        "Apollo: held local horizontal on the S-IVB's APS",
    );
    let rcs = w.vessel().propulsion.rcs_propellant_mass();
    w.info(
        Area::Propulsion,
        "RCS propellant left after turning",
        format!("{rcs:.0} kg"),
    );
    let due = planning::warp_to_burn(w, 2.0 * 86_400.0);
    w.must(
        Area::Control,
        "The burn executor hands over to physics for the burn",
        due,
        "",
    )?;
    w.phase("tli");
    let engine = w
        .vessel()
        .propulsion
        .engines
        .iter()
        .find(|e| e.def.id.contains("S-IVB"))
        .map(|e| e.ignitions_left);
    let done = planning::fly_burn(w, 900.0);
    w.must(Area::Propulsion, "The burn finishes", done, "")?;
    let log: Vec<String> = w.vessel().log.iter().map(|e| e.message.clone()).collect();
    w.check(
        Area::Propulsion,
        "The S-IVB relights for translunar injection",
        !log.iter()
            .any(|m| m.contains("J-2") && (m.contains("failed") || m.contains("no ignitions"))),
        format!("ignitions left before the burn: {engine:?}"),
    );
    let (_, burn) = w.span("tli").unwrap();
    w.within(
        Area::Propulsion,
        "TLI burn time",
        burn,
        300.0..=420.0,
        "s",
        "Apollo 11: 347 s",
    );
    let (altitude, speed) = altitude_and_speed(w);
    w.within(
        Area::Orbit,
        "Speed at TLI cutoff",
        speed / 1e3,
        10.4..=11.2,
        "km/s",
        "Apollo 11: 10.83 km/s",
    );
    let orbit = w.vessel().orbit(&w.universe.system, w.now());
    w.info(Area::Orbit, "After translunar injection", format!(
        "{:.0} km up, {:.2} km/s, apoapsis {:.0} km; planned {:.0} m/s, flown {:.0} m/s of thrust.",
        altitude / 1e3, speed / 1e3, orbit.apoapsis().map_or(f64::INFINITY, |a| a - EARTH_RADIUS) / 1e3,
        planned, w.progress().1));
    w.within(
        Area::Orbit,
        "The new orbit reaches the Moon",
        orbit.apoapsis().unwrap_or(f64::INFINITY) / 1e6,
        360.0..=430.0,
        "thousand km apoapsis radius",
        "the Moon is 384 thousand km away",
    );
    Ok(())
}

fn transposition(w: &mut World) -> Flight {
    w.phase("transposition");
    // The S-IVB lets go of what is above it.
    let before = w.vessel().flight.mass;
    w.stage();
    w.fly(10.0, |_| false);
    let after = w.vessel().flight.mass;
    w.check(
        Area::Staging,
        "The S-IVB separates",
        before - after > 8_000.0,
        format!("{:.1} t lighter", (before - after) / 1e3),
    );
    w.gap(
        Area::Docking,
        "Transposition, docking and extraction",
        "The real CSM separated from the adapter, turned round, docked nose to nose with the lunar module and pulled it out of the S-IVB. The game has no docking: parts cannot join in flight, so a vessel can never carry another that it picked up. The lunar module in this craft is a 15 t drum above the S-IVB, not a spacecraft.",
    );
    // The adapter and the drum go together so that the SPS has a clear nozzle.
    w.stage();
    w.fly(10.0, |_| false);
    let csm = w.vessel().flight.mass;
    w.bridged(
        Area::Docking,
        "The lunar module is thrown away",
        "Sequence 8 drops the adapter and the lunar module drum, and arms the SPS: the flight goes on as a command and service module alone, as Apollo 8 flew.",
    );
    w.within(
        Area::Build,
        "Mass of the CSM",
        csm / 1e3,
        26.0..=33.0,
        "t",
        "Apollo 11: 28.8 t",
    );
    Ok(())
}

fn coast_out(w: &mut World) -> Flight {
    w.phase("coast_out");
    w.vessel_mut().set_rcs(true);
    w.sas(SasMode::Stability);
    let moon = w.universe.system.find("Moon").unwrap();
    // Midcourse corrections: aim at the Moon again, after hours and then a day have shown where
    // the trajectory really goes (Apollo planned up to four).
    for (wait, label) in [
        (6.0 * 3600.0, "first"),
        (22.0 * 3600.0, "second"),
        (24.0 * 3600.0, "third"),
    ] {
        w.warp(wait);
        correct_course(w, moon, label)?;
    }
    let periapsis = planning::predicted_periapsis(w, moon);
    w.check(
        Area::Orbit,
        "The corrections leave a safe pass of the Moon",
        periapsis.is_some_and(|p| (20_000.0..=250_000.0).contains(&p)),
        format!(
            "predicted periapsis {} (aim {:.0} km)",
            periapsis.map_or("none".to_owned(), |p| format!("{:.0} km", p / 1e3)),
            LUNAR_PERIAPSIS / 1e3
        ),
    );
    // On to the Moon's sphere of influence.
    let entered = w.coast(6.0 * 86_400.0, |w| w.vessel().parent() == moon);
    w.must(
        Area::Orbit,
        "The trajectory enters the Moon's sphere of influence",
        entered,
        "",
    )?;
    w.within(
        Area::Orbit,
        "Time of arrival at the Moon's sphere of influence",
        w.met() / 3600.0,
        50.0..=100.0,
        "h after liftoff",
        "Apollo 11: 61 h 40 min",
    );
    Ok(())
}

/// Flies the first planned burn, a few m/s: by RCS push where small (a pilot's trim, which
/// is exact), with the burn executor otherwise, and writes down what it delivered.
fn fly_correction(w: &mut World, planned: f64) -> Flight {
    if planned <= SMALL_BURN {
        let given = planning::rcs_trim(w, 120.0);
        w.must(
            Area::Control,
            "The RCS trim points and fires",
            given.is_some(),
            "",
        )?;
        let given = given.unwrap();
        w.check(
            Area::Propulsion,
            "An RCS trim delivers the planned Δv",
            (given - planned).abs() < 0.1,
            format!("planned {planned:.2} m/s, gave {given:.2} m/s"),
        );
        return Ok(());
    }
    let off = planning::point_at_burn(w, 600.0, 600.0);
    w.within(
        Area::Control,
        "Turns onto the correction",
        off.unwrap_or(f64::NAN),
        0.0..=2.0,
        "° off the burn",
        "",
    );
    let due = planning::warp_to_burn(w, 86_400.0);
    w.must(
        Area::Control,
        "The burn executor hands over for the correction",
        due,
        "",
    )?;
    let orbit = w.vessel().orbit(&w.universe.system, w.now());
    let done = planning::fly_burn(w, 600.0);
    w.must(Area::Propulsion, "The SPS flies the correction", done, "")?;
    // What the burn really did to the velocity, from the orbits either side of it.
    let (now, after) = (w.now(), w.vessel().orbit(&w.universe.system, w.now()));
    let flown = (after.state_at(now).velocity - orbit.state_at(now).velocity).length();
    if (flown - planned).abs() > 0.3 {
        w.known(
            Area::Control,
            "The burn executor does not deliver the planned Δv",
            "T027",
            format!("Planned {planned:.2} m/s, the velocity changed by {flown:.2} m/s. The 3 s RCS ullage push before ignition gives the stack speed that the executor never counts, so every burn overshoots by about {:.1} m/s: a 55 h coast turns that into {:.0} km at the Moon.", flown - planned, (flown - planned) * 55.0 * 3.6),
        );
    }
    Ok(())
}

/// Burns up to this many m/s are made by RCS push.
const SMALL_BURN: f64 = 6.0;

/// Aims the trajectory at the Moon's `LUNAR_PERIAPSIS` and flies the correction, if it needs
/// one.
fn correct_course(w: &mut World, moon: osp_sim::body::BodyId, label: &str) -> Flight {
    let before = planning::predicted_periapsis(w, moon);
    w.info(
        Area::Orbit,
        &format!("Predicted lunar periapsis before the {label} correction"),
        before.map_or("no encounter".to_owned(), |p| format!("{:.0} km", p / 1e3)),
    );
    let now = w.now();
    let index = w.vessel_mut().plan.add(Burn {
        time: now.add_seconds(3_600.0),
        delta_v: DVec3::ZERO,
    });
    let reached = planning::aim(w, index, moon, LUNAR_PERIAPSIS);
    let dv = w.vessel().plan.burns()[index].delta_v.length();
    w.within(
        Area::Orbit,
        &format!("The {label} midcourse correction"),
        dv,
        0.0..=60.0,
        "m/s",
        "Apollo 11: about 6 m/s",
    );
    w.check(
        Area::Orbit,
        &format!("The {label} correction can be aimed"),
        reached.is_some_and(|r| (r - LUNAR_PERIAPSIS).abs() < 20_000.0),
        format!("aimed to {:.0} km", reached.unwrap_or(f64::NAN) / 1e3),
    );
    if dv < 0.3 {
        w.vessel_mut().plan.clear();
        return Ok(());
    }
    fly_correction(w, dv)?;
    let after = planning::predicted_periapsis(w, moon);
    w.info(
        Area::Orbit,
        &format!("Predicted lunar periapsis after the {label} correction"),
        after.map_or("no encounter".to_owned(), |p| format!("{:.0} km", p / 1e3)),
    );
    Ok(())
}

fn loi(w: &mut World) -> Flight {
    w.phase("loi");
    let moon = w.universe.system.find("Moon").unwrap();
    let radius = w.universe.system.body(moon).radius;
    // Behind the Moon: slow to a 313 by 111 km orbit at periapsis.
    let (orbit, now) = (w.vessel().orbit(&w.universe.system, w.now()), w.now());
    let periapsis = orbit.next_time_at_true_anomaly(0.0, now);
    w.must(
        Area::Orbit,
        "The approach has a periapsis to burn at",
        periapsis.is_some(),
        "",
    )?;
    let time = periapsis.unwrap();
    let state = orbit.state_at(time);
    let (r, fast) = (state.position.length(), state.velocity.length());
    let apoapsis = radius + 313_000.0;
    let slow = (orbit.mu() * (2.0 / r - 2.0 / (r + apoapsis))).sqrt();
    w.info(
        Area::Orbit,
        "Approach to the Moon",
        format!(
            "Periapsis {:.0} km over the surface in {:.1} h at {:.0} m/s; capture needs {:.0} m/s.",
            (r - radius) / 1e3,
            time.seconds_since(now) / 3600.0,
            fast,
            fast - slow
        ),
    );
    w.vessel_mut().plan.clear();
    w.vessel_mut().plan.add(Burn {
        time,
        delta_v: DVec3::new(slow - fast, 0.0, 0.0),
    });
    let off = planning::point_at_burn(w, 600.0, 600.0);
    w.within(
        Area::Control,
        "Turns onto the capture burn",
        off.unwrap_or(f64::NAN),
        0.0..=2.0,
        "° off the burn",
        "",
    );
    let due = planning::warp_to_burn(w, 6.0 * 3600.0);
    w.must(
        Area::Control,
        "The burn executor hands over for the capture burn",
        due,
        "",
    )?;
    let done = planning::fly_burn(w, 900.0);
    w.must(Area::Propulsion, "The SPS flies the capture burn", done, "")?;
    let orbit = w.vessel().orbit(&w.universe.system, w.now());
    w.within(
        Area::Propulsion,
        "LOI burn time",
        w.progress().0,
        180.0..=450.0,
        "s",
        "Apollo 11: 357 s with the lunar module still docked (45 t, not this 30 t)",
    );
    let (peri, apo) = (
        orbit.periapsis() - radius,
        orbit.apoapsis().map_or(f64::INFINITY, |a| a - radius),
    );
    w.within(
        Area::Orbit,
        "Lunar orbit periapsis",
        peri / 1e3,
        20.0..=260.0,
        "km",
        "Apollo 11: 111 km",
    );
    w.within(
        Area::Orbit,
        "Lunar orbit apoapsis",
        apo / 1e3,
        280.0..=350.0,
        "km",
        "Apollo 11: 313 km",
    );
    Ok(())
}

fn lunar_orbit(w: &mut World) -> Flight {
    w.phase("lunar_orbit");
    let moon = w.universe.system.find("Moon").unwrap();
    let radius = w.universe.system.body(moon).radius;
    let orbit = w.vessel().orbit(&w.universe.system, w.now());
    let period = orbit.period().unwrap_or(f64::NAN);
    w.within(
        Area::Orbit,
        "Period of the lunar orbit",
        period / 60.0,
        105.0..=130.0,
        "min",
        "Apollo 11: 119 min in the first orbit",
    );
    // Two revolutions on rails: the orbit must not drift.
    w.coast(2.0 * period, |_| false);
    let after = w.vessel().orbit(&w.universe.system, w.now());
    w.check(
        Area::Orbit,
        "The lunar orbit holds for two revolutions",
        (after.periapsis() - orbit.periapsis()).abs() < 1_000.0 && w.vessel().parent() == moon,
        format!(
            "periapsis {:.1} → {:.1} km",
            (orbit.periapsis() - radius) / 1e3,
            (after.periapsis() - radius) / 1e3
        ),
    );
    Ok(())
}

/// What the real mission did next, and why the game cannot: each phase says what is missing.
fn landing(w: &mut World) -> Flight {
    w.phase("descent");
    w.known(
        Area::Landing,
        "There is no lander to land",
        "T044",
        "The craft has no lunar module: the 15 t drum between the S-IVB and the adapter has no engine, no legs and no crew. The Realism mod has the pieces (an ascent stage with two seats, LMDE and LMAE engines, the legs would be plain parts) but a stage cannot let go from inside another, and nothing can dock.",
    );
    w.gap(
        Area::Landing,
        "Landing on legs",
        "Gear (`GearLegDef`) is wheeled landing gear; there is no leg that crushes, no foot that rests on regolith, and the Moon has no terrain, so it is a smooth ball.",
    );
    w.gap(
        Area::Crew,
        "Crew moving between vessels",
        "A crew seat belongs to one pod. The two who walked on the Moon left the command module for the lunar module through a tunnel, which needs docking.",
    );
    w.phase("surface");
    w.gap(
        Area::Power,
        "Surface stay: power, thermal and comms",
        "21.6 hours on the surface meant a battery budget for the descent and ascent stages, thermal control through the lunar day, and a link to the ground. None of these exist, so the stay is only a number of hours (see Power and Communications).",
    );
    w.phase("ascent");
    w.gap(
        Area::Docking,
        "Ascent, rendezvous and docking",
        "The rendezvous tools exist (targets, closest approach, SAS target mode), but with no lander there is nothing to rendezvous with, and docking is not in the game.",
    );
    // The command module waits in lunar orbit for the surface stay: 21 h of rails.
    let stay = 24.0 * 3600.0;
    w.coast(stay, |_| false);
    w.bridged(
        Area::Landing,
        "The surface stay is waited out in lunar orbit",
        format!(
            "{:.0} h of rails, to keep the trans-Earth timing close to Apollo 11's.",
            stay / 3600.0
        ),
    );
    Ok(())
}

/// Periapsis altitude over the Earth the return aims for, m: inside the atmosphere's corridor.
const ENTRY_PERIAPSIS: f64 = 35_000.0;

fn tei(w: &mut World) -> Flight {
    w.phase("tei");
    let earth = w.universe.system.find("Earth").unwrap();
    let moon = w.universe.system.find("Moon").unwrap();
    let now = w.now();
    let parking = w.vessel().orbit(&w.universe.system, now);
    // Leave the Moon backwards along its own motion, so the Earth is where it falls.
    let moon_velocity = w.universe.system.local_state(moon, now).velocity;
    let excess = -moon_velocity.normalize() * 840.0;
    let burn = osp_sim::transfer::ejection_burn(&parking, excess, now.add_seconds(3_600.0));
    w.must(
        Area::Orbit,
        "The planner finds an ejection burn from lunar orbit",
        burn.is_some(),
        "",
    )?;
    w.vessel_mut().plan.clear();
    w.vessel_mut().plan.add(burn.unwrap());
    let reached = planning::aim(w, 0, earth, ENTRY_PERIAPSIS);
    let dv = w.vessel().plan.burns()[0].delta_v.length();
    w.within(
        Area::Orbit,
        "Trans-Earth injection Δv",
        dv,
        800.0..=1_300.0,
        "m/s",
        "Apollo 11: 1 001 m/s",
    );
    w.check(
        Area::Orbit,
        "The return can be aimed at the entry corridor",
        reached.is_some_and(|r| (r - ENTRY_PERIAPSIS).abs() < 30_000.0),
        format!(
            "aimed to a periapsis of {:.0} km over the Earth",
            reached.unwrap_or(f64::NAN) / 1e3
        ),
    );
    let off = planning::point_at_burn(w, 600.0, 600.0);
    w.within(
        Area::Control,
        "Turns onto the return burn",
        off.unwrap_or(f64::NAN),
        0.0..=2.0,
        "° off the burn",
        "",
    );
    let due = planning::warp_to_burn(w, 6.0 * 3600.0);
    w.must(
        Area::Control,
        "The burn executor hands over for the return burn",
        due,
        "",
    )?;
    let done = planning::fly_burn(w, 900.0);
    w.must(Area::Propulsion, "The SPS flies the return burn", done, "")?;
    w.within(
        Area::Propulsion,
        "TEI burn time",
        w.progress().0,
        100.0..=250.0,
        "s",
        "Apollo 11: 150 s",
    );
    let left = w
        .vessel()
        .propulsion
        .sections
        .iter()
        .map(|s| w.vessel().propulsion.propellant_mass(s.id))
        .sum::<f64>();
    w.info(
        Area::Propulsion,
        "Propellant left in the CSM",
        format!("{:.0} kg", left),
    );
    // Out of the Moon's sphere of influence.
    let out = w.coast(3.0 * 86_400.0, |w| w.vessel().parent() == earth);
    w.must(
        Area::Orbit,
        "The burn leaves the Moon's sphere of influence",
        out,
        "",
    )?;
    Ok(())
}

fn coast_home(w: &mut World) -> Flight {
    w.phase("coast_home");
    let earth = w.universe.system.find("Earth").unwrap();
    w.vessel_mut().set_rcs(true);
    w.sas(SasMode::Stability);
    let periapsis = |w: &World| planning::predicted_periapsis(w, earth);
    w.info(
        Area::Orbit,
        "Predicted Earth periapsis leaving the Moon",
        periapsis(w).map_or("no encounter".to_owned(), |p| format!("{:.0} km", p / 1e3)),
    );
    // Corrections to the entry corridor: the first a day out, the last hours before entry.
    for label in ["first", "second", "last"] {
        // A day out, then a few hours before the entry: the nearer, the less the burn's error
        // grows before it arrives.
        let earth_range = match label {
            "first" => 330_000_000.0,
            "second" => 150_000_000.0,
            _ => 40_000_000.0,
        };
        w.coast(4.0 * 86_400.0, |w| {
            let v = w.vessel();
            v.state_at(&w.universe.system, w.now()).position.length() < earth_range
        });
        let now = w.now();
        let before = planning::predicted_periapsis(w, earth);
        let index = w.vessel_mut().plan.add(Burn {
            time: now.add_seconds(3_600.0),
            delta_v: DVec3::ZERO,
        });
        let reached = planning::aim(w, index, earth, ENTRY_PERIAPSIS);
        let dv = w.vessel().plan.burns()[index].delta_v.length();
        w.within(
            Area::Orbit,
            &format!("The {label} return correction"),
            dv,
            0.0..=80.0,
            "m/s",
            "Apollo 11: one of 4.6 m/s",
        );
        w.check(
            Area::Orbit,
            &format!("The {label} return correction can be aimed"),
            reached.is_some_and(|r| (r - ENTRY_PERIAPSIS).abs() < 20_000.0),
            format!("aimed to {:.0} km", reached.unwrap_or(f64::NAN) / 1e3),
        );
        if dv < 0.3 {
            w.vessel_mut().plan.clear();
            continue;
        }
        fly_correction(w, dv)?;
        let after = planning::predicted_periapsis(w, earth);
        let km = |p: Option<f64>| p.map_or("none".to_owned(), |p| format!("{:.0} km", p / 1e3));
        w.info(
            Area::Orbit,
            &format!("Earth periapsis around the {label} return correction"),
            format!(
                "{} before, {} after (aimed {:.0} km), {:.0} thousand km out",
                km(before),
                km(after),
                reached.unwrap_or(f64::NAN) / 1e3,
                w.vessel()
                    .state_at(&w.universe.system, w.now())
                    .position
                    .length()
                    / 1e6
            ),
        );
    }
    let p = periapsis(w);
    w.check(
        Area::Orbit,
        "The corrections leave the entry corridor",
        p.is_some_and(|p| (10_000.0..=60_000.0).contains(&p)),
        format!(
            "predicted periapsis {}",
            p.map_or("none".to_owned(), |p| format!("{:.0} km", p / 1e3))
        ),
    );
    Ok(())
}

fn entry(w: &mut World) -> Flight {
    w.phase("entry");
    let earth = w.universe.system.find("Earth").unwrap();
    // Down to an hour before the atmosphere, then heat shield first.
    let near = w.coast(4.0 * 86_400.0, |w| {
        let v = w.vessel();
        let system = &w.universe.system;
        let r = v.state_at(system, w.now()).position.length();
        let speed = v.state_at(system, w.now()).velocity.length();
        r - 6_371_000.0 < 200_000.0 + speed * 600.0
    });
    w.must(
        Area::Orbit,
        "The trajectory comes down to the Earth",
        near,
        "",
    )?;
    w.sas(SasMode::Retrograde);
    w.fly(300.0, |_| false);
    w.sas(SasMode::Stability);
    w.fly(30.0, |_| false);
    let cm_mass = w.vessel().flight.mass;
    w.stage();
    w.fly(5.0, |_| false);
    w.within(
        Area::Staging,
        "The service module is cut loose",
        (cm_mass - w.vessel().flight.mass) / 1e3,
        4.0..=30.0,
        "t lighter",
        "Apollo 11: the SM weighed 24.5 t with propellant; this one has burned most of it",
    );
    let mass = w.vessel().flight.mass;
    w.within(
        Area::Build,
        "Mass of the command module",
        mass / 1e3,
        4.0..=7.0,
        "t",
        "Apollo 11: 5.6 t at entry",
    );
    // The rest is the atmosphere's.
    let down = w.fly(3_600.0, |w| {
        let v = w.vessel();
        v.is_landed() || v.is_destroyed()
    });
    w.check(
        Area::Thermal,
        "The command module reaches the ground",
        down,
        "",
    );
    let landing = w
        .vessel()
        .log
        .iter()
        .rev()
        .find(|e| e.message.starts_with("Landed") || e.message.starts_with("Crashed"))
        .map(|e| e.message.clone());
    w.known(
        Area::Recovery,
        "No parachutes, no splashdown",
        "T048",
        format!("{}. The real command module opened two drogues at 7 km and three mains at 3 km to land at 9 m/s in the Pacific; the sim has no parachute part and no ocean (the sea is the terrain's zero), so a capsule can only hit the ground.", landing.as_deref().unwrap_or("no touchdown recorded")),
    );
    w.gap(
        Area::Thermal,
        "Entry heating",
        "Nothing heats the command module: there is no temperature on a part, no heat flux from the shock layer, no ablator that chars (the pod's heat shield is a flat shape) and so nothing that burns up. A 25 g entry and a 6 g entry both survive, and a shield-first entry is no safer than a side-on one except through the drag.",
    );
    w.gap(
        Area::Crew,
        "Crew and life support",
        "The pod seats the crew as a number: no oxygen, water, food or carbon dioxide, no cabin pressure, and no g tolerance (an entry at 26 g would kill them).",
    );
    // Which way up the capsule fell: its heat shield (the base, away from the nose) should lead.
    let thick: Vec<_> = w
        .samples
        .iter()
        .filter(|s| {
            s.t >= w
                .phases
                .iter()
                .find(|p| p.id == "entry")
                .and_then(|p| p.started)
                .unwrap_or(0.0)
                && s.q > 2_000.0
        })
        .collect();
    let flipped = thick.iter().filter(|s| s.aoa.abs() < 90.0).count() * 2 > thick.len();
    if let Some((g, at)) = w.peak("entry", |s| s.g) {
        if g <= 10.0 && !flipped {
            w.within(
                Area::Thermal,
                "Peak deceleration in entry",
                g,
                3.0..=10.0,
                "g",
                "Apollo 11: 6.5 g",
            );
        } else {
            w.known(
                Area::Aero,
                "The entry is ballistic and the capsule does not hold its heat shield forward",
                "T047",
                format!(
                    "Peak {g:.0} g at {at:.0} s (Apollo 11: 6.5 g), with the capsule {} through the thick air. The real command module rode with its centre of mass off the axis, so it trimmed at 157° and the crew rolled it to steer the lift; the sim's pod is a uniform frustum with no RCS of its own, so it tumbles, settles apex first, and has no lift to steer: a ballistic entry from the Moon is 30 g or more.",
                    if flipped { "apex first" } else { "heat shield first" }
                ),
            );
        }
    }
    let _ = earth;
    Ok(())
}
