//! A Casual career on from its first orbit towards a crew on the Moon and back (T334, part of
//! T301): the same game as T333's six flights, carried on flight by flight, each for a contract the
//! career has taken on and each craft built from what it has unlocked by then.

use osp_sim::{
    DVec3,
    attitude::SasMode,
    body::BodyId,
    plan::Burn,
    rendezvous::Target,
    systems::Thermal,
    transfer::{LaunchRequest, LaunchWindow},
};

use crate::{
    Mission,
    career::{Game, Hangar, crew, height, recover},
    career_orbit::{
        Books, ORBIT_INCLINATION, WANTS, Want, ascend, close, orbiter, periapsis, ready, summary,
        to_orbit,
    },
    planning,
    world::{Abort, Area, Flight, PhaseSpec, World},
};

pub static CASUAL_MOON: Mission = Mission {
    id: "career_casual_moon",
    title: "Casual career: on from orbit towards the Moon",
    summary: "The Casual career of `career_casual_orbit`, carried on: after its first orbit a crew \
              goes up in the orbiter and comes home, the upper stage bringing the capsule down \
              into the air and the capsule falling through it on its own. Then, with the pad's \
              third level bought, a three-stage rocket sends a crew round the Moon: four K-1s, a \
              K-1 stage, and an A-10 stage for the translunar burn and the corrections home.",
    scenario: "pad",
    mode: Some("casual"),
    plan: PLAN,
    fly,
};

const PLAN: &[PhaseSpec] = &[
    PhaseSpec {
        id: "career",
        title: "A new career",
        goal: "The career starts on the Casual tree and takes on its first contract.",
    },
    PhaseSpec {
        id: "f1",
        title: "Flight 1: first launch",
        goal: "As in career_casual_orbit.",
    },
    PhaseSpec {
        id: "f2",
        title: "Flight 2: 10 km",
        goal: "As in career_casual_orbit.",
    },
    PhaseSpec {
        id: "f3",
        title: "Flight 3: 30 km",
        goal: "As in career_casual_orbit.",
    },
    PhaseSpec {
        id: "f4",
        title: "Flight 4: space",
        goal: "As in career_casual_orbit.",
    },
    PhaseSpec {
        id: "f5",
        title: "Flight 5: a person to space and back",
        goal: "As in career_casual_orbit.",
    },
    PhaseSpec {
        id: "f6",
        title: "Flight 6: orbit",
        goal: "As in career_casual_orbit.",
    },
    PhaseSpec {
        id: "f7",
        title: "Flight 7: a crew in orbit, and home",
        goal: "The orbiter with instruments takes a crew to orbit; at the orbit's highest point \
               the upper stage burns back until the lowest point is in the air, the capsule lets \
               go of it, falls through the air heat side first, and comes down on its parachute \
               with its crew alive.",
    },
    PhaseSpec {
        id: "f8",
        title: "Flight 8: round the Moon",
        goal: "Four K-1s and a K-1 stage put the A-10 stage and the capsule in orbit; the \
               transfer planner's burn sends them to pass the Moon; past it, corrections aim the \
               way home at the entry corridor, and the capsule comes down on its parachute with \
               its crew and what the instruments read on its drive.",
    },
];

/// What the career saves up for after its first orbit: the orbit's, then the pad's third level for
/// a rocket that can go to the Moon.
const MOON_WANTS: &[Want] = &[
    WANTS[0],
    WANTS[1],
    WANTS[2],
    WANTS[3],
    Want::Facility("launch_pad", 3),
];

/// The Moon rocket's tanks as typed in, m: the second stage's (3 m wide), and the first stage's
/// (3 m wide, under four K-1s, which lift it at about 1.4 times its weight).
const SECOND_TANK: f64 = 7.0;
const FIRST_TANK: f64 = 24.0;

/// How far above the Moon the flyby passes, m.
const FLYBY_PERIAPSIS: f64 = 500_000.0;
/// The Earth periapsis the way home is aimed at, m: a C-1 comes in at 8 to 14 g there.
const ENTRY_PERIAPSIS: f64 = 45_000.0;

/// Where the deorbit burn brings the orbit's lowest point, m: well into the air.
const DEORBIT_PERIAPSIS: f64 = 30_000.0;

fn fly(w: &mut World) -> Flight {
    let (game, mut books) = to_orbit(w)?;
    books.wants = MOON_WANTS;
    crew_orbit(w, &game, &mut books, "f7")?;
    moon_flyby(w, &game, &mut books, "f8")?;
    summary(w, &game, &books);
    Ok(())
}

/// A crew to orbit in the orbiter, and home: deorbited by the upper stage at the orbit's highest
/// point, the capsule falls through the air by itself and comes down on its parachute.
fn crew_orbit(w: &mut World, game: &Game, books: &mut Books, phase: &str) -> Flight {
    let craft = ready(w, game, books, phase, "Orbiter", |h| orbiter(h, true))?;
    ascend(w, 0, ORBIT_INCLINATION)?;
    // A while in orbit, as far as its highest point.
    let up = w.coast(4.0 * 3600.0, |w| height(w).1 <= 0.0);
    w.must(
        Area::Orbit,
        "It coasts to the orbit's highest point",
        up,
        format!("{:.0} km up", height(w).0 / 1e3),
    )?;
    // Back the way it came, until the lowest point is in the air.
    w.sas(SasMode::Retrograde);
    w.fly(60.0, |_| false);
    w.vessel_mut().set_throttle(1.0);
    w.fly(2.0, |_| false);
    let lowered = w.fly(900.0, |w| {
        periapsis(w) < DEORBIT_PERIAPSIS || !w.vessel().propulsion.engines.iter().any(|e| e.running)
    });
    w.vessel_mut().set_throttle(0.0);
    let low = periapsis(w);
    w.must(
        Area::Orbit,
        "The upper stage brings the lowest point into the air",
        lowered && low < DEORBIT_PERIAPSIS,
        format!("{:.0} km", low / 1e3),
    )?;
    // The capsule lets go of the stage and arms its parachute, and holds its heat side forward.
    let before = w.vessel().propulsion.parts().parts().len();
    w.stage();
    w.stage();
    let after = w.vessel().propulsion.parts().parts().len();
    w.check(
        Area::Staging,
        "The capsule lets go of the upper stage",
        after < before,
        format!("{before} parts before, {after} after"),
    );
    w.sas(SasMode::Retrograde);
    let down = w.coast(4.0 * 3600.0, |w| {
        w.touchdown.is_some() || w.flown().is_none_or(|v| v.is_destroyed())
    });
    let peak = hottest(w);
    w.info(Area::Thermal, "Hottest a part got", peak);
    let whole = w.flown().is_some_and(|v| !v.is_destroyed());
    w.must(
        Area::Thermal,
        "The capsule comes through the air whole",
        down && whole,
        w.flown().map_or("lost".into(), |v| {
            (v.log.iter().rev())
                .find(|e| e.message.contains("hot") || e.message.contains("destroyed"))
                .map_or(String::new(), |e| e.message.clone())
        }),
    )?;
    let (speed, landed) = w.touchdown.unwrap_or_default();
    w.check(
        Area::Recovery,
        "On its parachute, whole",
        landed && speed <= 10.0,
        format!("{speed:.1} m/s"),
    );
    w.fly(10.0, |_| false);
    let (aboard, lost) = crew(w);
    w.check(
        Area::Crew,
        "The crew come home alive",
        aboard > 0 && lost == 0,
        format!("{aboard} aboard, {lost} lost"),
    );
    recover(w, game)?;
    close(w, game, books, &craft);
    Ok(())
}

/// The hottest any part of the vessel flown has been against its limit, as the player reads it.
fn hottest(w: &World) -> String {
    let Some(vessel) = w.flown() else {
        return "the vessel is gone".into();
    };
    let systems = &vessel.propulsion.systems;
    let all = systems.all::<Thermal>();
    let worst = all.max_by(|a, b| (a.1.peak / a.1.limit).total_cmp(&(b.1.peak / b.1.limit)));
    worst.map_or("nothing heated".into(), |(_, t)| {
        format!("{:.0} K of {:.0} K it can take", t.peak, t.limit)
    })
}

/// The Moon rocket: a C-1 with its parachute on an A-10 stage carrying a thermometer, a barometer
/// and a telemetry unit; under it a K-1 stage; under that four K-1s round the end of a 3 m tank.
fn moon_rocket(h: &mut Hangar) -> Result<(), String> {
    h.root("pod:C-1 Capsule", "capsule")?;
    h.on_node("parachute", "parachute", "capsule", "top", "bottom")?;
    h.on_node("decoupler", "decoupler", "capsule", "bottom", "top")?;
    h.on_node("tank", "third_tank", "decoupler", "bottom", "top")?;
    h.on_node("engine:A-10", "third_engine", "third_tank", "bottom", "top")?;
    let on = [
        ("instrument:thermometer", "thermometer", 0.0),
        ("instrument:barometer", "barometer", 0.25),
        ("telemetry", "telemetry", 0.5),
    ];
    for (entry, id, u) in on {
        h.on_surface(entry, id, "third_tank", "barrel", u, 0.5, 1)?;
    }
    h.on_node("decoupler", "interstage", "third_engine", "bottom", "top")?;
    h.on_node("tank", "second_tank", "interstage", "bottom", "top")?;
    h.resize("second_tank", &[("diameter", 3.0), ("length", SECOND_TANK)])?;
    h.on_node(
        "engine:K-1 Booster",
        "second_engine",
        "second_tank",
        "bottom",
        "top",
    )?;
    h.on_node(
        "decoupler",
        "interstage_2",
        "second_engine",
        "bottom",
        "top",
    )?;
    h.on_node("tank", "first_tank", "interstage_2", "bottom", "top")?;
    h.resize("first_tank", &[("diameter", 3.0), ("length", FIRST_TANK)])?;
    h.under_end(
        "engine:K-1 Booster",
        "first_engine",
        "first_tank",
        "aft_dome",
        0.0,
        0.45,
        4,
    )?;
    h.on_surface("fin", "fin", "first_tank", "barrel", 0.125, 0.12, 4)
}

/// A crew round the Moon and home: to orbit, the transfer planner's burn to the Moon, the flyby,
/// corrections aimed at the entry corridor, the capsule's own way down.
fn moon_flyby(w: &mut World, game: &Game, books: &mut Books, phase: &str) -> Flight {
    let craft = ready(w, game, books, phase, "Moon Flyer", moon_rocket)?;
    let system = &w.universe.system;
    let (moon, earth) = (system.find("Moon").unwrap(), system.find("Earth").unwrap());
    // On the pad: the Moon targeted, the planner's first launch time waited for.
    planning::target_body(w, moon);
    let window = launch_window(w, moon);
    w.must(
        Area::Orbit,
        "The planner gives a launch time for the Moon",
        window.is_some(),
        "",
    )?;
    let window = window.expect("checked");
    let wait = window.time.seconds_since(w.now());
    w.info(
        Area::Orbit,
        "The launch time",
        format!(
            "in {:.1} h, heading {:.0} degrees, into an orbit inclined {:.1} degrees",
            wait / 3600.0,
            window.heading.to_degrees(),
            window.inclination.to_degrees()
        ),
    );
    let waited = w.warp(wait);
    w.must(Area::Orbit, "It waits on the pad for it", waited, "")?;
    ascend(w, 1, window.inclination.to_degrees())?;
    // What is left of the K-1 stage goes: the A-10 stage flies the rest, lit and idle.
    let k1 =
        |w: &World| (w.vessel().propulsion.engines.iter()).any(|e| e.def.id.starts_with("K-1"));
    if k1(w) {
        let mass = w.vessel().flight.mass;
        w.stage();
        w.fly(2.0, |_| false);
        w.info(
            Area::Staging,
            "The K-1 stage is let go in orbit",
            format!(
                "{:.1} t before, {:.1} t after",
                mass / 1e3,
                w.vessel().flight.mass / 1e3
            ),
        );
    }
    let period = (w.vessel().orbit(&w.universe.system, w.now()).period()).unwrap_or(5400.0);
    let found = planning::transfer_to(w, moon, 0.5 * period, 1.5 * period);
    w.must(
        Area::Orbit,
        "The transfer planner finds a way to the Moon",
        found.is_some(),
        "",
    )?;
    let departure = found.expect("checked");
    w.info(
        Area::Orbit,
        "The transfer planner's departure",
        format!(
            "{:.0} m/s, {:.1} days to the Moon",
            departure.transfer.depart_dv,
            departure.transfer.tof / 86_400.0
        ),
    );
    w.vessel_mut().plan.add(departure.burn);
    let reached = planning::aim(w, 0, moon, FLYBY_PERIAPSIS);
    w.check(
        Area::Orbit,
        "The burn is aimed past the Moon",
        reached.is_some_and(|r| (r - FLYBY_PERIAPSIS).abs() < 100_000.0),
        format!("{:.0} km over it", reached.unwrap_or(f64::NAN) / 1e3),
    );
    planning::point_at_burn(w, 600.0, 600.0);
    let due = planning::warp_to_burn(w, 2.0 * 86_400.0);
    w.must(
        Area::Control,
        "The burn executor hands over to physics for the burn",
        due,
        "",
    )?;
    let done = planning::fly_burn(w, 1500.0);
    let aborted =
        (w.vessel().log.iter().rev().take(20)).any(|e| e.message.starts_with("Burn aborted"));
    w.must(
        Area::Propulsion,
        "The translunar burn is flown to the end",
        done && !aborted && w.vessel().plan.is_empty(),
        if aborted { "aborted" } else { "" },
    )?;
    // The long burn on the small stage is no kick: a correction an hour on puts it back on
    // its way past the Moon.
    aim_and_fly(w, moon, FLYBY_PERIAPSIS, "midcourse correction")?;
    let over = planning::predicted_periapsis(w, moon);
    w.must(
        Area::Orbit,
        "It is on its way past the Moon",
        over.is_some(),
        format!("{:.0} km over it", over.unwrap_or(f64::NAN) / 1e3),
    )?;
    let there = w.coast(8.0 * 86_400.0, |w| w.vessel().parent() == moon);
    w.must(
        Area::Orbit,
        "It reaches the Moon's sphere of influence",
        there,
        "",
    )?;
    // Home from here, while a little speed turns the way out a long way: aimed at the entry
    // corridor, as the way home is the next leg round the Earth.
    aim_and_fly(w, earth, ENTRY_PERIAPSIS, "burn for home")?;
    let back = w.coast(8.0 * 86_400.0, |w| w.vessel().parent() == earth);
    w.must(Area::Orbit, "It leaves the Moon behind", back, "")?;
    let home = planning::predicted_periapsis(w, earth);
    w.info(
        Area::Orbit,
        "Earth periapsis leaving the Moon",
        home.map_or("none".into(), |p| format!("{:.0} km", p / 1e3)),
    );
    for (label, range) in [("first", 250e6), ("last", 60e6)] {
        correct(w, earth, label, range)?;
    }
    let aimed = planning::predicted_periapsis(w, earth);
    w.must(
        Area::Orbit,
        "The way home is in the entry corridor",
        aimed.is_some_and(|p| (35_000.0..=60_000.0).contains(&p)),
        format!("{:.0} km", aimed.unwrap_or(f64::NAN) / 1e3),
    )?;
    entry(w, earth)?;
    recover(w, game)?;
    close(w, game, books, &craft);
    Ok(())
}

/// A correction on the way home, `label`led, once within `range` m of the Earth: aimed at the
/// entry corridor and flown, if it is worth flying.
fn correct(w: &mut World, earth: BodyId, label: &str, range: f64) -> Flight {
    w.coast(6.0 * 86_400.0, |w| {
        let v = w.vessel();
        v.state_at(&w.universe.system, w.now()).position.length() < range
    });
    aim_and_fly(
        w,
        earth,
        ENTRY_PERIAPSIS,
        &format!("{label} correction home"),
    )
}

/// A correction an hour ahead, aimed to pass `body` at `altitude`, and flown if it is worth
/// flying: `label` names it.
fn aim_and_fly(w: &mut World, body: BodyId, altitude: f64, label: &str) -> Flight {
    let now = w.now();
    let index = w.vessel_mut().plan.add(Burn {
        time: now.add_seconds(3_600.0),
        delta_v: DVec3::ZERO,
    });
    let reached = planning::aim(w, index, body, altitude);
    let dv = w.vessel().plan.burns()[index].delta_v.length();
    w.check(
        Area::Orbit,
        &format!("The {label} can be aimed"),
        reached.is_some_and(|r| (r - altitude).abs() < 20_000.0),
        format!(
            "{dv:.0} m/s, aimed to {:.0} km",
            reached.unwrap_or(f64::NAN) / 1e3
        ),
    );
    if dv < 0.3 {
        w.vessel_mut().plan.clear();
        return Ok(());
    }
    planning::point_at_burn(w, 600.0, 600.0);
    let due = planning::warp_to_burn(w, 86_400.0);
    let flown = due && planning::fly_burn(w, 600.0);
    w.must(Area::Control, &format!("The {label} is flown"), flown, "")
}

/// From the way home to the ground: near the Earth the capsule lets go of its stage and arms its
/// parachute, holds its heat side forward through the air, and comes down with its crew.
fn entry(w: &mut World, earth: BodyId) -> Result<(), Abort> {
    w.coast(4.0 * 86_400.0, |w| {
        let v = w.vessel();
        v.parent() == earth && height(w).0 < 2_000_000.0
    });
    let before = w.vessel().propulsion.parts().parts().len();
    w.stage();
    w.stage();
    let after = w.vessel().propulsion.parts().parts().len();
    w.check(
        Area::Staging,
        "The capsule lets go of its stage",
        after < before,
        format!("{before} parts before, {after} after"),
    );
    w.sas(SasMode::Retrograde);
    let down = w.coast(86_400.0, |w| {
        w.touchdown.is_some() || w.flown().is_none_or(|v| v.is_destroyed())
    });
    w.info(Area::Thermal, "Hottest a part got", hottest(w));
    let phase = w.phase_id().unwrap_or_default().to_owned();
    let peak = w.peak(&phase, |s| s.g).map_or(0.0, |(g, _)| g);
    w.info(Area::Crew, "The most the crew felt", format!("{peak:.1} g"));
    let whole = w.flown().is_some_and(|v| !v.is_destroyed());
    w.must(
        Area::Thermal,
        "The capsule comes through the air whole",
        down && whole,
        "",
    )?;
    let (speed, landed) = w.touchdown.unwrap_or_default();
    w.check(
        Area::Recovery,
        "On its parachute, whole",
        landed && speed <= 10.0,
        format!("{speed:.1} m/s"),
    );
    w.fly(10.0, |_| false);
    let (aboard, lost) = crew(w);
    w.check(
        Area::Crew,
        "The crew come home alive",
        aboard > 0 && lost == 0,
        format!("{aboard} aboard, {lost} lost"),
    );
    Ok(())
}

/// The first launch time the transfer planner gives for `target` from where the vessel flown
/// stands, as its launch times show on the pad.
fn launch_window(w: &World, target: BodyId) -> Option<LaunchWindow> {
    let (system, now) = (&w.universe.system, w.now());
    let vessel = w.vessel();
    let frame = vessel.parent();
    let (_, arrival) = w.universe.target_orbit(Target::Body(target), now)?;
    let body = system.body(frame);
    let site = body.orientation(now).inverse() * vessel.state_at(system, now).position;
    let request = LaunchRequest::new(body, frame, site, arrival)?;
    request.windows(system, now, 1).into_iter().next()
}
