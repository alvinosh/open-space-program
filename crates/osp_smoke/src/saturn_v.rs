//! Apollo 11 on the Saturn V, from the pad at LC-39A to splashdown.
//!
//! The numbers in the checks are the real mission's (Apollo 11 flight journal, the Saturn V
//! flight manual), with room for the game's simpler physics.

use osp_sim::{
    DVec3,
    attitude::SasMode,
    guidance::Guidance,
    orbit::{Orbit, StateVector},
    parts::PartId,
    plan::Burn,
    rendezvous::Target,
    staging::Activation,
    systems::{BOARD, Battery, Crew, HeatShield, Parachute, Thermal},
    time::SimTime,
    transfer::lambert,
    vessel::VesselId,
};

use crate::{
    Mission, audit,
    docking::{Hands, approach},
    pilot::{Ascent, Profile},
    planning,
    world::{Abort, Area, Flight, PhaseSpec, World},
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
            turn_end: 150_000.0,
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

/// When the real S-IC shut its centre F-1 down, s after ignition.
const CENTRE_CUTOFF: f64 = 135.0;

fn s_ic(w: &mut World, ascent: &mut Ascent) -> Flight {
    w.phase("s_ic");
    let mut since = 0.0;
    // The instrument unit cuts the centre engine at 2 min 15 s, as the real one did.
    let centre = w.fly(CENTRE_CUTOFF + 5.0, |w| {
        ascent.steer(w);
        w.met() >= CENTRE_CUTOFF + 0.1
    });
    w.must(
        Area::Propulsion,
        "Flies to the centre engine's cutoff",
        centre,
        "",
    )?;
    let running = (w.vessel().propulsion.engines.iter())
        .filter(|e| e.running)
        .count();
    w.within(
        Area::Avionics,
        "F-1s running after the instrument unit's cutoff",
        running as f64,
        4.0..=4.0,
        "",
        "Apollo 11: the centre F-1 shut down at 2 min 15 s",
    );
    // And the outboard ones when the sensors in the tanks say so, with some left in them.
    let cutoff = w.fly(300.0, |w| {
        ascent.steer(w);
        since += 0.02;
        dry(w) && since > 1.0 && w.t() > 30.0
    });
    w.must(Area::Propulsion, "S-IC burns out", cutoff, "")?;
    let residual = w.vessel().propulsion.sections[0].id;
    let left = w.vessel().propulsion.propellant_mass(residual);
    w.within(
        Area::Avionics,
        "Propellant the instrument unit leaves in the S-IC",
        left / 1e3,
        0.0..=30.0,
        "t",
        "cut off on the low-level sensors",
    );
    let unit = (w.vessel().propulsion.parts().parts().iter())
        .find(|p| p.name == "Instrument Unit")
        .and_then(|p| p.built.as_ref())
        .map_or(0.0, |built| built.mass.mass);
    w.within(
        Area::Avionics,
        "Mass of the instrument unit, from what it controls",
        unit,
        1800.0..=2300.0,
        "kg",
        "the real one, about 2 t",
    );
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
    if let Some((g, _)) = w.peak("s_ic", |s| s.g) {
        w.within(
            Area::Propulsion,
            "Peak acceleration",
            g,
            3.0..=4.0,
            "g",
            "Apollo 11: 3.9 g",
        );
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
    let left = (w.vessel().propulsion.engines.iter())
        .find(|e| e.def.id.contains("S-IVB"))
        .and_then(|e| e.ignitions_left);
    w.check(
        Area::Propulsion,
        "The S-IVB has used both its starts",
        left == Some(0),
        format!("{left:?} left"),
    );
    let service_rcs: f64 = (w.vessel().propulsion.tanks.iter())
        .filter(|t| {
            (w.vessel().propulsion.parts().get(t.part))
                .is_some_and(|part| part.name == "RCS propellant")
        })
        .map(|t| t.held("NTO") + t.held("Aerozine50"))
        .sum();
    w.check(
        Area::Propulsion,
        "Its ullage came from its own thrusters, the spacecraft's still full",
        service_rcs > 400.0,
        format!("{service_rcs:.0} kg in the service module's tanks"),
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

/// How far the CSM backs off before it turns round, m: its own length and some room to swing.
const TURN_ROOM: f64 = 26.0;

fn transposition(w: &mut World) -> Flight {
    w.phase("transposition");
    // The adapter's panels open and the CSM lets go, the lunar module staying on the S-IVB.
    w.stage();
    w.fly(3.0, |_| false);
    let csm = w.vessel().propulsion.mass();
    w.within(
        Area::Build,
        "Mass of the CSM",
        csm / 1e3,
        26.0..=33.0,
        "t",
        "Apollo 11: 28.8 t",
    );
    let panels = (w.universe.vessels.iter())
        .filter(|v| v.id() != w.focus && v.name.starts_with("Fairing"))
        .count();
    w.check(
        Area::Staging,
        "The adapter's four panels come away",
        panels == 4,
        format!("{panels} of them"),
    );
    let carrier: Option<VesselId> = (w.universe.vessels.iter())
        .find(|v| v.id() != w.focus && v.propulsion.parts().has_seats())
        .map(|v| v.id());
    w.must(
        Area::Staging,
        "The lunar module stays on the S-IVB, a vessel and not debris",
        carrier.is_some(),
        "",
    )?;
    w.with_vessel(|v, system, now| {
        v.target = carrier.map(Target::Vessel);
        v.set_rcs(true);
        v.set_sas_mode(system, now, SasMode::Stability);
    });
    w.must(
        Area::Docking,
        "The CSM's probe has the lunar module's drogue to go for",
        approach(w).is_some(),
        "",
    )?;
    // The push of letting go carries it away, and the keys stop it once there is room to turn.
    let away = w.fly(300.0, |w| {
        approach(w).is_some_and(|a| a.alignment.distance() > TURN_ROOM)
    });
    w.must(
        Area::Docking,
        "The CSM moves off far enough to turn round",
        away,
        "",
    )?;
    w.with_vessel(|v, system, now| v.set_sas_mode(system, now, SasMode::TargetPort));
    let turned = w.fly(900.0, |w| {
        approach(w).is_some_and(|a| a.alignment.angle < 1f64.to_radians())
    });
    let angle = approach(w).map_or(180.0, |a| a.alignment.angle.to_degrees());
    w.must(
        Area::Control,
        "SAS turns the probe round to face the drogue",
        turned,
        format!("{angle:.1} degrees off"),
    )?;
    let rcs_before = w.vessel().propulsion.rcs_propellant_mass();
    let mut hands = Hands::new();
    let mut last = None;
    let joined = w.fly(3_600.0, |w| {
        if w.vessel().propulsion.joined_parts().next().is_some() {
            return true;
        }
        let Some(seen) = approach(w) else {
            return false;
        };
        hands.fly(w, &seen);
        last = Some(seen.alignment);
        false
    });
    w.with_vessel(|v, system, now| {
        v.translation_input = DVec3::ZERO;
        v.set_sas_mode(system, now, SasMode::Stability);
    });
    w.must(
        Area::Docking,
        "The CSM docks with the lunar module in the adapter",
        joined,
        last.map_or(String::new(), |a| {
            format!(
                "still {:.2} m off, {:.2} m/s: {}",
                a.distance(),
                a.closing,
                a.problem().map_or("no problem", |p| p.label())
            )
        }),
    )?;
    w.within(
        Area::Docking,
        "RCS propellant for the turn and the docking",
        rcs_before - w.vessel().propulsion.rcs_propellant_mass(),
        0.0..=60.0,
        "kg",
        "the service module carried 550 kg for the whole flight",
    );
    // What holds the lunar module in the adapter lets go, by a player's click on the part: docking
    // turned the joint round, so it is the ring on the adapter's floor that hangs from the lander.
    let propulsion = &w.vessel().propulsion;
    let tree = propulsion.parts();
    let extract = (tree.parts().iter())
        .filter(|part| !propulsion.carried(part.stage))
        .filter(|part| {
            tree.parent(part.id)
                .is_some_and(|holder| propulsion.carried(holder.stage))
        })
        .flat_map(|part| propulsion.part_actions(part.id, false))
        .find(|activation| activation.is_decoupler());
    w.must(
        Area::Docking,
        "The lunar module can be let go of from the docked stack",
        extract.is_some(),
        "",
    )?;
    let docked = w.vessel().propulsion.mass();
    let extracted = extract.is_some_and(|activation| w.universe.perform(w.focus, activation));
    w.fly(30.0, |_| false);
    let stack = w.vessel().propulsion.mass();
    w.check(
        Area::Docking,
        "The S-IVB stays behind",
        extracted && docked - stack > 10_000.0,
        format!("{:.1} t lighter", (docked - stack) / 1e3),
    );
    w.within(
        Area::Build,
        "Mass of the CSM and lunar module",
        stack / 1e3,
        42.0..=52.0,
        "t",
        "Apollo 11: 43.9 t",
    );
    w.check(
        Area::Propulsion,
        "Nothing of the lunar module fires in the stack",
        (w.vessel().propulsion.engines.iter()).all(|e| e.thrust == 0.0),
        "",
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

/// Flies the first planned burn, a few m/s, with the burn executor (the RCS for a small one),
/// and writes down what it delivered.
fn fly_correction(w: &mut World, planned: f64) -> Flight {
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
    // Under the engine, the service module's RCS holds the nose only so well against a thrust line
    // a little off the centre of mass, so a burn ends a little off: 1% of it, or 0.1 m/s.
    let limit = (0.01 * planned).max(0.1);
    w.within(
        Area::Control,
        "The burn executor delivers the planned Δv",
        flown - planned,
        -limit..=limit,
        "m/s over",
        "",
    );
    Ok(())
}

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
        "Apollo 11: 357 s with the lunar module docked",
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
    circularize(w)?;
    let moon = w.universe.system.find("Moon").unwrap();
    let radius = w.universe.system.body(moon).radius;
    let orbit = w.vessel().orbit(&w.universe.system, w.now());
    let period = orbit.period().unwrap_or(f64::NAN);
    w.within(
        Area::Orbit,
        "Period of the lunar orbit",
        period / 60.0,
        100.0..=130.0,
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

/// The link along the flight so far: it comes and goes as the stations turn under the vessel,
/// goes behind the Moon, and is 1.3 s late from there.
fn signal(w: &mut World) {
    let (mut losses, mut behind, mut acquired) = (0, 0, 0);
    let mut delay: f64 = 0.0;
    let mut last = None;
    for sample in &w.samples {
        let linked = sample.link_margin.is_some();
        if let Some(was) = last {
            if was && !linked {
                losses += 1;
                behind += usize::from(sample.body == "Moon");
            }
            if !was && linked {
                acquired += 1;
            }
        }
        last = Some(linked);
        if sample.body == "Moon" {
            delay = delay.max(sample.link_delay.unwrap_or(0.0));
        }
    }
    w.check(
        Area::Comms,
        "The signal is acquired and lost along the flight",
        acquired > 0 && losses > 0,
        format!("{acquired} acquisitions, {losses} losses by the end of lunar orbit"),
    );
    w.check(
        Area::Comms,
        "The signal is lost behind the Moon",
        behind > 0,
        format!("{behind} losses in the Moon's sphere of influence"),
    );
    w.within(
        Area::Comms,
        "Signal delay at the Moon",
        delay,
        1.2..=1.45,
        "s",
        "about 1.3 s each way",
    );
}

/// The world's eyes and hands go to vessel `id`.
fn switch_to(w: &mut World, id: VesselId) {
    w.focus = id;
    w.universe.focus = Some(id);
}

/// The part with seats for this many, on the vessel flown.
fn crew_part(w: &World, seats: u32) -> Option<PartId> {
    (w.vessel().propulsion.systems.all::<Crew>())
        .find(|(_, crew)| crew.seats == seats)
        .map(|(part, _)| part)
}

/// Who is aboard the part with seats for this many, on the vessel flown.
fn aboard(w: &World, seats: u32) -> u32 {
    (w.vessel().propulsion.systems.all::<Crew>())
        .find(|(_, crew)| crew.seats == seats)
        .map_or(0, |(_, crew)| crew.aboard)
}

/// Have `count` people move to the seats for `seats`, through the docked tunnel.
fn move_crew(w: &mut World, seats: u32, count: u32) -> bool {
    let Some(part) = crew_part(w, seats) else {
        return false;
    };
    (0..count).all(|_| w.universe.perform(w.focus, Activation::new(part, BOARD)))
}

/// Fly the first planned burn from here: turn onto it, wait for it, burn. False if any failed.
fn fly_the_burn(w: &mut World) -> bool {
    let turned = planning::point_at_burn(w, 300.0, 600.0).is_some();
    let due = planning::warp_to_burn(w, 2.0 * 86_400.0);
    turned && due && planning::fly_burn(w, 900.0)
}

/// Down to a circle at the low point of the orbit: at the low point, against the way it is going.
fn circularize(w: &mut World) -> Flight {
    let (system, now) = (&w.universe.system, w.now());
    let v = w.vessel();
    let orbit = v.orbit(system, now);
    let mu = system.body(v.parent()).gm;
    let time = orbit.next_time_at_true_anomaly(0.0, now).unwrap_or(now);
    let state = orbit.state_at(time);
    let circular = (mu / state.position.length()).sqrt();
    let burn = Burn {
        time,
        delta_v: DVec3::new(circular - state.velocity.length(), 0.0, 0.0),
    };
    w.vessel_mut().plan.clear();
    w.vessel_mut().plan.add(burn);
    let done = fly_the_burn(w);
    w.must(Area::Propulsion, "The SPS circularises the orbit", done, "")?;
    let radius = w.universe.system.body(w.vessel().parent()).radius;
    let orbit = w.vessel().orbit(&w.universe.system, w.now());
    w.within(
        Area::Orbit,
        "The lunar orbit is nearly round",
        orbit.apoapsis().map_or(f64::INFINITY, |a| a - radius) / 1e3,
        60.0..=140.0,
        "km apoapsis",
        "Apollo 11: 122 km",
    );
    Ok(())
}

/// The best way to meet `target` from the orbit flown: when to burn, and how long it takes. It aims
/// `behind` metres back along the target's way.
fn best_intercept(w: &World, target: VesselId, behind: f64) -> Option<(SimTime, f64, DVec3)> {
    let (system, now) = (&w.universe.system, w.now());
    let v = w.vessel();
    let body = system.body(v.parent());
    let mine = v.orbit(system, now);
    let theirs = w.universe.vessel(target)?.orbit(system, now);
    let period = theirs.period()?;
    let mut best: Option<(f64, SimTime, f64, DVec3)> = None;
    let mut wait = 120.0;
    while wait < 2.0 * period {
        let t1 = now.add_seconds(wait);
        let from = mine.state_at(t1);
        let normal = from.position.cross(from.velocity);
        let mut tof = 0.25 * period;
        while tof < 0.75 * period {
            let t2 = t1.add_seconds(tof);
            let to = theirs.state_at(t2);
            let aim = to.position - to.velocity.normalize() * behind;
            if let Some((leave, arrive)) = lambert(body.gm, from.position, aim, tof, normal) {
                let low = Orbit::new(body.gm, t1, StateVector::new(from.position, leave));
                if low.periapsis() - body.radius > 8_000.0 {
                    let cost = (leave - from.velocity).length() + (to.velocity - arrive).length();
                    if best.as_ref().is_none_or(|b| cost < b.0) {
                        best = Some((cost, t1, tof, leave - from.velocity));
                    }
                }
            }
            tof += 120.0;
        }
        wait += 120.0;
    }
    best.map(|(_, t1, tof, dv)| (t1, tof, dv))
}

/// A planned burn of this much, inertial axes, at `time`.
fn plan_inertial(w: &mut World, time: SimTime, dv: DVec3) -> bool {
    let (system, now) = (&w.universe.system, w.now());
    let state = w.vessel().orbit(system, now).state_at(time);
    let Some(burn) = Burn::from_inertial(time, &state, dv) else {
        return false;
    };
    w.vessel_mut().plan.clear();
    w.vessel_mut().plan.add(burn);
    true
}

/// How long the real crew stayed on the surface, s.
const SURFACE_STAY: f64 = 77_760.0;
/// How high the lander's orbit is at its low point after it leaves the circle, m.
const LOW_POINT: f64 = 15_000.0;
/// How high the ascent flies for, and the low point of the orbit that will do, m.
const ASCENT_HEIGHT: f64 = 18_000.0;
const ASCENT_PERIAPSIS: f64 = 12_000.0;
/// How far behind the command module the lander aims to come out, m.
const STAND_OFF: f64 = 400.0;

/// The lunar module's part of the mission: down, a day on the surface, up, and back to the command
/// module, which it docks with. Ends flying the command module.
fn landing(w: &mut World) -> Flight {
    signal(w);
    w.phase("descent");
    let csm = w.focus;
    // Two of the crew go through the tunnel.
    let moved = move_crew(w, 2, 2);
    w.check(
        Area::Crew,
        "Two of the crew move to the lunar module through the tunnel",
        moved && aboard(w, 2) == 2 && aboard(w, 3) == 1,
        format!(
            "{} in the lander, {} in the command module",
            aboard(w, 2),
            aboard(w, 3)
        ),
    );
    let port = w.vessel().propulsion.joined_parts().next();
    w.must(
        Area::Docking,
        "The lunar module is still docked in lunar orbit",
        port.is_some(),
        "",
    )?;
    let undocked = port.is_some_and(|port| w.universe.perform(w.focus, Activation::undock(port)));
    w.fly(60.0, |_| false);
    let lander = (w.universe.vessels.iter())
        .find(|v| v.id() != csm && v.propulsion.parts().has_seats())
        .map(|v| (v.id(), v.name.clone(), v.is_debris(), v.propulsion.mass()));
    let (lander, name, debris, mass) = lander.unwrap_or((csm, String::new(), true, 0.0));
    w.check(
        Area::Docking,
        "The lunar module undocks and flies as a vessel of its own",
        undocked && !debris && name == "Lunar Module",
        name.clone(),
    );
    w.within(
        Area::Build,
        "Mass of the lunar module",
        mass / 1e3,
        14.0..=18.0,
        "t",
        "Apollo 11: 15.1 t",
    );
    switch_to(w, lander);
    w.with_vessel(|v, system, now| {
        v.set_rcs(true);
        v.set_sas(system, now, true);
        v.set_sas_mode(system, now, SasMode::Stability);
    });
    // Its first sequence: the descent engine armed, its thrusters on and its legs unfolded.
    let folded = osp_sim::gear::legs(&w.vessel().propulsion).len();
    w.stage();
    let legs = osp_sim::gear::legs(&w.vessel().propulsion).len();
    w.check(
        Area::Landing,
        "The legs unfold",
        folded == 0 && legs == 4,
        format!("{folded} legs standing before, {legs} after"),
    );

    // Down to a low point of 15 km on the far side of the Moon from here.
    let (system, now) = (&w.universe.system, w.now());
    let body = system.body(w.vessel().parent());
    let radius = body.radius;
    let mu = body.gm;
    let time = now.add_seconds(300.0);
    let state = w.vessel().orbit(system, now).state_at(time);
    let r = state.position.length();
    let a = (r + radius + LOW_POINT) / 2.0;
    let slower = (mu * (2.0 / r - 1.0 / a)).sqrt();
    let burn = Burn {
        time,
        delta_v: DVec3::new(slower - state.velocity.length(), 0.0, 0.0),
    };
    w.vessel_mut().plan.clear();
    w.vessel_mut().plan.add(burn);
    let done = fly_the_burn(w);
    w.must(Area::Propulsion, "It burns down to the low point", done, "")?;
    let orbit = w.vessel().orbit(&w.universe.system, w.now());
    w.within(
        Area::Orbit,
        "The low point of the way down",
        (orbit.periapsis() - radius) / 1e3,
        5.0..=40.0,
        "km",
        "Apollo 11: 15 km",
    );

    // Coast to it, the engine pointing the way it will fire, and let the tool fly it down.
    let start = orbit
        .next_time_at_true_anomaly(0.0, w.now())
        .unwrap_or(w.now());
    let lead = start.seconds_since(w.now()) - 240.0;
    w.coast(lead.max(0.0), |_| false);
    w.sas(SasMode::Retrograde);
    let braking = w.fly(600.0, |w| {
        let (system, now) = (&w.universe.system, w.now());
        let v = w.vessel();
        v.state_at(system, now).position.length() - radius < LOW_POINT + 1_500.0
    });
    w.must(Area::Orbit, "It comes down to the low point", braking, "")?;
    w.vessel_mut().guidance = Guidance::Land;
    let touched = w.fly(1_800.0, |w| {
        let v = w.vessel();
        v.wheels_down() || v.is_landed() || v.is_destroyed()
    });
    let log: Vec<String> = w.vessel().log.iter().map(|e| e.message.clone()).collect();
    let crashed = log.iter().find(|m| m.starts_with("Crashed")).cloned();
    w.must(
        Area::Landing,
        "The descent tool lands it",
        touched && crashed.is_none() && !w.vessel().is_destroyed(),
        crashed.unwrap_or_default(),
    )?;
    let sink = log
        .iter()
        .find_map(|m| m.strip_prefix("Touched down at "))
        .and_then(|m| m.split(' ').next())
        .and_then(|m| m.parse::<f64>().ok())
        .unwrap_or(f64::INFINITY);
    w.within(
        Area::Landing,
        "It touches down softly",
        sink,
        0.0..=4.0,
        "m/s",
        "Eagle: 0.5 m/s",
    );
    let stopped = w.fly(60.0, |w| w.vessel().is_landed());
    w.must(Area::Landing, "It comes to rest on its legs", stopped, "")?;
    let (system, now) = (&w.universe.system, w.now());
    let v = w.vessel();
    let tilt = (v.attitude_state(system, now).nose())
        .angle_between(v.state_at(system, now).position.normalize())
        .to_degrees();
    w.within(
        Area::Landing,
        "It stands upright",
        tilt,
        0.0..=15.0,
        "degrees",
        "Eagle: 4.5 degrees",
    );

    w.phase("surface");
    w.coast(SURFACE_STAY, |_| false);
    let p = &w.vessel().propulsion;
    let held: f64 = p.systems.all::<Battery>().map(|(_, b)| b.charge).sum();
    let full: f64 = p.systems.all::<Battery>().map(|(_, b)| b.capacity).sum();
    let brownout = p.systems.brownout();
    w.check(
        Area::Power,
        "The batteries last the stay on the surface",
        !brownout && held > 0.0,
        format!(
            "{:.0}% of {:.0} MJ left",
            100.0 * held / full.max(1.0),
            full / 1e3
        ),
    );
    w.check(
        Area::Crew,
        "The crew live through the stay on their own supplies",
        aboard(w, 2) == 2,
        format!("{} aboard", aboard(w, 2)),
    );
    let margin = w.vessel().link().map(|link| link.margin);
    w.info(
        Area::Comms,
        "The link from the surface",
        margin.map_or("no station in sight just now".to_owned(), |margin| {
            format!("{margin:.1} dB of margin")
        }),
    );

    // Lift off when the command module has just gone over, in its plane.
    let csm_orbit = |w: &World| {
        w.universe
            .vessel(csm)
            .map(|v| v.orbit(&w.universe.system, w.now()))
    };
    for _ in 0..600 {
        let (system, now) = (&w.universe.system, w.now());
        let here = w.vessel().state_at(system, now).position.normalize();
        let Some(orbit) = csm_orbit(w) else { break };
        let there = orbit.state_at(now);
        let ahead = (there.position.normalize() - here).dot(there.velocity);
        if here.dot(there.position.normalize()) > 20f64.to_radians().cos() && ahead > 0.0 {
            break;
        }
        w.warp(30.0);
    }
    w.phase("ascent");
    let normal = csm_orbit(w).map_or(DVec3::Z, |orbit| {
        let state = orbit.state_at(w.now());
        state.position.cross(state.velocity).normalize()
    });
    w.vessel_mut().set_throttle(1.0);
    w.stage();
    let aps = (w.vessel().propulsion.engines.iter())
        .find(|e| e.def.id == "LMAE")
        .map_or(0.0, |e| e.def.thrust_vac);
    w.must(
        Area::Staging,
        "The ascent engine is what is left",
        aps > 0.0,
        "",
    )?;
    let mut pilot = crate::pilot::Pilot::new();
    let lifted = w.fly(20.0, |w| {
        let (system, now) = (&w.universe.system, w.now());
        let up = w.vessel().state_at(system, now).position.normalize();
        pilot.point(w, up);
        w.vessel()
            .state_at(&w.universe.system, w.now())
            .radial_speed()
            > 1.0
    });
    w.must(Area::Landing, "It lifts off the descent stage", lifted, "")?;
    let in_orbit = w.fly(900.0, |w| {
        let (system, now) = (&w.universe.system, w.now());
        let v = w.vessel();
        let body = system.body(v.parent());
        let state = v.state_at(system, now);
        let r = state.position.length();
        let up = state.position / r;
        let climb = state.radial_speed();
        let across = (state.velocity - up * climb).length();
        let height = r - body.radius;
        let periapsis = v.orbit(system, now).periapsis() - body.radius;
        if periapsis >= ASCENT_PERIAPSIS {
            w.vessel_mut().set_throttle(0.0);
            return true;
        }
        let push = aps / v.flight.mass.max(1.0);
        let weight = body.gm / (r * r) - across * across / r;
        let aim = if height < 2_000.0 {
            up
        } else {
            let circular = (body.gm / r).sqrt();
            let left = (circular - across).max(0.0) / push;
            let climb_to = ((ASCENT_HEIGHT - height) / left.max(20.0)).clamp(0.0, 60.0)
                * (left / 60.0).min(1.0);
            let most = 0.95 * push;
            let lift = (weight + (climb_to - climb) / 10.0).clamp(-0.3 * most, most);
            up * lift + normal.cross(up) * (push * push - lift * lift).sqrt()
        };
        pilot.point(w, aim);
        false
    });
    w.with_vessel(|v, _, _| v.rotation_input = DVec3::ZERO);
    w.must(
        Area::Orbit,
        "The ascent stage reaches lunar orbit",
        in_orbit,
        "",
    )?;
    let descent_left = w
        .universe
        .vessels
        .iter()
        .any(|v| v.is_debris() && v.is_landed());
    w.check(
        Area::Staging,
        "The descent stage stays on the surface",
        descent_left,
        "",
    );

    // After the command module: a burn to meet it, a look halfway, and the speeds matched.
    let way = best_intercept(w, csm, STAND_OFF);
    w.must(
        Area::Orbit,
        "A way to meet the command module can be found",
        way.is_some(),
        "",
    )?;
    let (t1, tof, dv) = way.ok_or(Abort)?;
    w.info(
        Area::Orbit,
        "The way up to the command module",
        format!(
            "{:.0} m/s in {:.0} s, then {:.0} min on the way",
            dv.length(),
            t1.seconds_since(w.now()),
            tof / 60.0
        ),
    );
    let planned = plan_inertial(w, t1, dv);
    let flown = planned && fly_the_burn(w);
    w.must(
        Area::Propulsion,
        "It burns to meet the command module",
        flown,
        "",
    )?;
    let arrive = t1.add_seconds(tof);
    // Halfway, a correction for what the first burn missed by.
    let halfway = w.now().add_seconds(arrive.seconds_since(w.now()) / 2.0);
    w.coast(halfway.seconds_since(w.now()), |_| false);
    let (system, now) = (&w.universe.system, w.now());
    let mu = system.body(w.vessel().parent()).gm;
    let burn_at = now.add_seconds(120.0);
    let state = w.vessel().orbit(system, now).state_at(burn_at);
    let target = csm_orbit(w).map(|o| o.state_at(arrive));
    if let Some(target) = target {
        let aim = target.position - target.velocity.normalize() * STAND_OFF;
        let normal = state.position.cross(state.velocity);
        let time_left = arrive.seconds_since(burn_at);
        if let Some((leave, _)) = lambert(mu, state.position, aim, time_left, normal) {
            let fix = leave - state.velocity;
            if fix.length() > 0.2 {
                let fixed = plan_inertial(w, burn_at, fix) && fly_the_burn(w);
                w.check(
                    Area::Orbit,
                    "It corrects its way halfway",
                    fixed,
                    format!("{:.1} m/s", fix.length()),
                );
            }
        }
    }
    // The speeds matched where it arrives.
    let (system, now) = (&w.universe.system, w.now());
    let mine = w.vessel().orbit(system, now).state_at(arrive);
    let theirs = csm_orbit(w).map(|o| o.state_at(arrive));
    if let Some(theirs) = theirs {
        let matched = plan_inertial(w, arrive, theirs.velocity - mine.velocity) && fly_the_burn(w);
        w.must(
            Area::Propulsion,
            "It matches the command module's speed",
            matched,
            "",
        )?;
    }
    let relative = |w: &World| {
        let (system, now) = (&w.universe.system, w.now());
        let there = w.universe.vessel(csm).map(|v| v.state_at(system, now));
        let here = w.vessel().state_at(system, now);
        there.map_or(DVec3::ZERO, |there| there.velocity - here.velocity)
    };
    // What the burn left over, on the thrusters.
    for _ in 0..3 {
        let left = relative(w);
        if left.length() < 0.3 {
            break;
        }
        let soon = w.now().add_seconds(60.0);
        if plan_inertial(w, soon, left) {
            fly_the_burn(w);
        }
    }
    let left = relative(w).length();
    w.within(
        Area::Orbit,
        "How much speed is left to match",
        left,
        0.0..=2.0,
        "m/s",
        "",
    );
    let (system, now) = (&w.universe.system, w.now());
    let gap = w.universe.vessel(csm).map_or(f64::INFINITY, |v| {
        v.state_at(system, now)
            .position
            .distance(w.vessel().state_at(system, now).position)
    });
    w.within(
        Area::Orbit,
        "How far off the command module it comes out",
        gap / 1e3,
        0.0..=30.0,
        "km",
        "Apollo 11: station-keeping at about 15 m",
    );

    // The lander holds still with its drogue towards the command module, which docks with it.
    w.with_vessel(|v, system, now| {
        v.target = Some(Target::Vessel(csm));
        v.set_rcs(true);
        v.set_sas(system, now, true);
        v.set_sas_mode(system, now, SasMode::Target);
    });
    switch_to(w, csm);
    w.with_vessel(|v, system, now| {
        v.target = Some(Target::Vessel(lander));
        v.set_rcs(true);
        v.set_sas(system, now, true);
        v.set_sas_mode(system, now, SasMode::TargetPort);
    });
    w.must(
        Area::Docking,
        "The probe has the ascent stage's drogue to go for",
        approach(w).is_some(),
        "",
    )?;
    let turned = w.fly(900.0, |w| {
        approach(w).is_some_and(|a| a.alignment.angle < 2f64.to_radians())
    });
    w.check(
        Area::Control,
        "The command module turns to face it",
        turned,
        "",
    );
    let mut hands = Hands::new();
    let mut holding = false;
    let mut last = None;
    let joined = w.fly(3_600.0, |w| {
        if w.vessel().propulsion.joined_parts().next().is_some() {
            return true;
        }
        let Some(seen) = approach(w) else {
            return false;
        };
        // Close in, the lander stops following and holds still to be docked with.
        if !holding && seen.alignment.distance() < 150.0 {
            holding = true;
            let (system, now) = (&w.universe.system, w.now());
            let time = now;
            if let Some(index) = w.universe.find_vessel(lander) {
                let system = system.clone();
                w.universe.vessels[index].set_sas_mode(&system, time, SasMode::Stability);
            }
        }
        hands.fly(w, &seen);
        last = Some(seen.alignment);
        false
    });
    if let Some(a) = last.filter(|_| !joined) {
        w.info(
            Area::Docking,
            "Where the docking stopped",
            format!(
                "gap {:.1} m, off axis {:.2} m, closing {:.2} m/s, {:.1} degrees off, {:.2} deg/s, drifting {:.2} m/s: {}",
                a.gap,
                a.off_axis.length(),
                a.closing,
                a.angle.to_degrees(),
                a.turning.to_degrees(),
                a.drift.length(),
                a.problem().map_or("no problem", |p| p.label())
            ),
        );
    }
    w.with_vessel(|v, system, now| {
        v.translation_input = DVec3::ZERO;
        v.set_sas_mode(system, now, SasMode::Stability);
    });
    w.must(
        Area::Docking,
        "The ascent stage and the command module dock in lunar orbit",
        joined,
        "",
    )?;
    let back = move_crew(w, 3, 2);
    w.check(
        Area::Crew,
        "The crew come back through the tunnel",
        back && aboard(w, 3) == 3,
        format!("{} in the command module", aboard(w, 3)),
    );
    let port = w.vessel().propulsion.joined_parts().next();
    let gone = port.is_some_and(|port| w.universe.perform(w.focus, Activation::undock(port)));
    w.check(Area::Staging, "The ascent stage is let go", gone, "");
    w.fly(30.0, |_| false);
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

/// The power budget, just before the service module goes with the fuel cells: what they burned
/// and what is left to burn. Only the command module's batteries are left after this.
fn power_budget(w: &mut World) {
    use osp_sim::systems::FuelCell;

    let p = &w.vessel().propulsion;
    let burned: f64 = p.systems.all::<FuelCell>().map(|(_, c)| c.hydrogen).sum();
    let oxygen: f64 = p.systems.all::<FuelCell>().map(|(_, c)| c.oxygen).sum();
    let left: f64 = p.tanks.iter().map(|t| t.held("LqdHydrogen")).sum();
    let cells = p.systems.all::<FuelCell>().count();
    let brownout = p.systems.brownout();
    let days = w.met() / 86_400.0;
    w.within(
        Area::Power,
        "Fuel cells at work to the end of the service module",
        cells as f64,
        3.0..=3.0,
        "",
        "Apollo 11: three, 1.4 kW each",
    );
    w.within(
        Area::Power,
        "Hydrogen the fuel cells burned",
        burned,
        5.0..=25.0,
        "kg",
        "of two tanks' 25 kg, on about 2 kW",
    );
    w.check(
        Area::Power,
        "The cryogenic tanks last the mission",
        left > 0.0 && !brownout,
        format!(
            "{left:.1} kg of hydrogen left after {days:.1} days; {oxygen:.0} kg of oxygen burned"
        ),
    );
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
    power_budget(w);
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
    // The rest is the thrusters' roll and the air's, and then the parachutes'.
    w.vessel_mut().guidance = Guidance::Entry;
    let down = w.fly(3_600.0, |w| {
        let v = w.vessel();
        v.is_landed() || v.is_afloat() || v.is_destroyed()
    });
    w.must(
        Area::Thermal,
        "The command module comes through the entry whole",
        down && !w.vessel().is_destroyed(),
        w.vessel()
            .log
            .last()
            .map_or(String::new(), |e| e.message.clone()),
    )?;
    let p = &w.vessel().propulsion;
    let (used, peak_flux) = (p.systems.all::<HeatShield>())
        .next()
        .map_or((0.0, 0.0), |(_, s)| (s.full - s.ablator, s.peak_flux));
    let hottest: f64 = (p.systems.all::<Thermal>())
        .map(|(_, t)| t.peak)
        .fold(0.0, f64::max);
    w.check(
        Area::Thermal,
        "The heat shield takes the entry and is not burned through",
        used > 0.0 && used < 850.0,
        format!(
            "{used:.0} kg of 850 kg of ablator burned, at up to {:.1} MW/m²",
            peak_flux / 1e6
        ),
    );
    w.info(
        Area::Thermal,
        "The hottest any part of the capsule got",
        format!("{hottest:.0} K"),
    );
    if let Some((g, _)) = w.peak("entry", |s| s.g) {
        w.within(
            Area::Thermal,
            "Peak deceleration in entry",
            g,
            2.0..=10.0,
            "g",
            "Apollo 11: 6.5 g",
        );
    }
    let opened = (w.vessel().log.iter())
        .filter(|e| e.message.ends_with("parachute opening"))
        .count();
    w.within(
        Area::Recovery,
        "The parachutes open",
        opened as f64,
        5.0..=5.0,
        "of them",
        "Apollo 11: two drogues at 7 km and three mains at 3 km",
    );
    let torn = (w.vessel().log.iter()).any(|e| e.message.contains("torn"));
    w.check(Area::Recovery, "None of them tears", !torn, "");
    let afloat = w.vessel().is_afloat();
    let splash = (w.vessel().log.iter().rev())
        .find_map(|e| e.message.strip_prefix("Splashed down at "))
        .and_then(|m| m.split(' ').next())
        .and_then(|m| m.parse::<f64>().ok())
        .unwrap_or(f64::INFINITY);
    w.check(
        Area::Recovery,
        "It comes down on the sea and floats",
        afloat,
        "",
    );
    w.within(
        Area::Recovery,
        "Speed at splashdown",
        splash,
        0.0..=15.0,
        "m/s",
        "Apollo 11: 9 m/s",
    );
    // The batteries carried the command module from the service module's going to the sea.
    let p = &w.vessel().propulsion;
    let held: f64 = p.systems.all::<Battery>().map(|(_, b)| b.charge).sum();
    let full: f64 = p.systems.all::<Battery>().map(|(_, b)| b.capacity).sum();
    let margin = if full > 0.0 { held / full } else { 0.0 };
    let brownout = p.systems.brownout();
    w.check(
        Area::Power,
        "The batteries see the command module down",
        margin > 0.05 && !brownout,
        format!("{:.0}% of {full:.0} kJ left", margin * 100.0),
    );
    let crew = aboard(w, 3);
    let lost: u32 = w.vessel().propulsion.systems.crew_lost();
    w.check(
        Area::Crew,
        "All three of the crew come home alive",
        crew == 3 && lost == 0,
        format!("{crew} aboard, {lost} lost"),
    );
    let tanks = |w: &World, name: &str| -> f64 {
        w.vessel()
            .propulsion
            .tanks
            .iter()
            .map(|t| t.held(name))
            .sum()
    };
    let (oxygen, water, food) = (tanks(w, "LqdOxygen"), tanks(w, "Water"), tanks(w, "Food"));
    w.check(
        Area::Crew,
        "The supplies last the mission",
        oxygen > 0.0 && water > 0.0 && food > 0.0,
        format!("{oxygen:.0} kg of oxygen, {water:.0} kg of water, {food:.0} kg of food left"),
    );
    let dose = (w.vessel().propulsion.systems.people())
        .map(|(_, person)| person.dose)
        .fold(0.0, f64::max);
    w.info(
        Area::Crew,
        "The most radiation anyone took",
        format!(
            "{:.1} mSv (Apollo 11: 1.8 mGy absorbed, a few mSv)",
            dose * 1e3
        ),
    );
    let chutes = w.vessel().propulsion.systems.all::<Parachute>().count();
    w.info(
        Area::Recovery,
        "Parachutes on the capsule",
        format!("{chutes}"),
    );
    let _ = earth;
    Ok(())
}
