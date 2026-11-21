//! A Casual career from its first launch to its first orbit (T333, part of T301), in one game. Each
//! flight is flown for the contract the career has taken on; between flights it takes on what it is
//! offered, unlocks what the next craft needs with its science and upgrades what it needs with its
//! money, as a player saving up would: the next thing on its list as soon as it can pay, and nothing
//! after it until then. Each craft is built in the builder from what is unlocked by then and within
//! the facilities, and the books say per flight what it was made of, what it earned and what was
//! bought after it, and when.

use std::collections::BTreeSet;

use osp_sim::{
    DVec3,
    career::money,
    data::{CraftDef, FacilityDef},
    palette,
    tech::civil_date,
    time::SimTime,
};

use crate::{
    Mission,
    career::{
        Game, Hangar, Typed, build, capsule_hop, first_rocket, height, launch, new_career, recover,
    },
    pilot::{Ascent, Profile},
    world::{Abort, Area, Flight, PhaseSpec, World},
};

pub static CASUAL_ORBIT: Mission = Mission {
    id: "career_casual_orbit",
    title: "Casual career: first launch to the first orbit",
    summary: "A new Casual career flies for each contract it is offered in turn: a first launch, \
              10 km, 30 km, space, a person to space and back, and orbit. Its first two rockets are \
              the parts list's capsule on a solid booster; then a booster typed bigger for space; \
              then, with Basic and General Rocketry unlocked and the pad and the builder upgraded \
              from what the contracts paid, three K-1s under one tank and an A-10 upper stage take \
              the capsule to orbit. The books give what each flight earned and what it bought.",
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
        goal: "The parts list's first rocket, a capsule on a solid booster, leaves the pad.",
    },
    PhaseSpec {
        id: "f2",
        title: "Flight 2: 10 km",
        goal: "The same rocket again, above the weather.",
    },
    PhaseSpec {
        id: "f3",
        title: "Flight 3: 30 km",
        goal: "A bigger booster, typed in, takes the capsule higher than aircraft fly, and on to \
               space.",
    },
    PhaseSpec {
        id: "f4",
        title: "Flight 4: space",
        goal: "The same, past the Karman line, for the contract it is now on.",
    },
    PhaseSpec {
        id: "f5",
        title: "Flight 5: a person to space and back",
        goal: "The same, with its crew home safe at under 15 g.",
    },
    PhaseSpec {
        id: "f6",
        title: "Flight 6: orbit",
        goal: "Three K-1s under one tank and an A-10 upper stage put the capsule in an orbit clear \
               of the air.",
    },
];

/// What the career buys between flights, in this order, each as soon as it can pay for it: the
/// upper stage's engine, the pad an orbital rocket needs, the booster engine and the width for it,
/// the builder's height for it.
pub(crate) const WANTS: &[Want] = &[
    Want::Node("basic_rocketry"),
    Want::Facility("launch_pad", 2),
    Want::Node("general_rocketry"),
    Want::Facility("builder", 2),
];

/// The booster typed in for space: wider and longer than the parts list makes it, and harder.
const SPACE_BOOSTER: Typed = Typed {
    diameter: 1.9,
    length: 2.4,
    thrust: 300.0,
};

/// The orbiter's first stage tank, as long as typed in, m: three K-1s lift it at about 1.4 times
/// its weight.
const LOWER_TANK: f64 = 18.0;

/// The inclination flown to from the Cape when nothing else asks for one, degrees: due east.
pub(crate) const ORBIT_INCLINATION: f64 = 28.6;

/// The orbit flown to, m: a circle, cut off once the lowest point clears the contract's.
const ORBIT_ALTITUDE: f64 = 180_000.0;
const CUTOFF_PERIAPSIS: f64 = 165_000.0;

/// Something the career saves up for: a node of the tree, or a facility at a level (from 1).
#[derive(Debug, Clone, Copy)]
pub(crate) enum Want {
    Node(&'static str),
    Facility(&'static str, usize),
}

/// The career's books: where science and money stood when the flight under way began.
pub(crate) struct Books {
    science: f64,
    funds: f64,
    flights: usize,
    started: SimTime,
    /// What it saves up for, in order.
    pub wants: &'static [Want],
}

fn fly(w: &mut World) -> Flight {
    let (game, books) = to_orbit(w)?;
    summary(w, &game, &books);
    Ok(())
}

/// A new Casual career flown from its first launch to its first orbit (T333): the six flights.
pub(crate) fn to_orbit(w: &mut World) -> Result<(Game, Books), Abort> {
    let game = new_career(w)?;
    let mut books = Books {
        science: game.career(w).science,
        funds: game.career(w).funds,
        flights: 0,
        started: w.now(),
        wants: WANTS,
    };
    hop(w, &game, &mut books, "f1", "First Rocket", None)?;
    hop(w, &game, &mut books, "f2", "First Rocket", None)?;
    let bigger = Some(SPACE_BOOSTER);
    hop(w, &game, &mut books, "f3", "Sky Rocket", bigger)?;
    hop(w, &game, &mut books, "f4", "Sky Rocket", bigger)?;
    hop(w, &game, &mut books, "f5", "Sky Rocket", bigger)?;
    orbit(w, &game, &mut books, "f6")?;
    Ok((game, books))
}

/// Where the career stands, in all.
pub(crate) fn summary(w: &mut World, game: &Game, books: &Books) {
    let days = w.now().seconds_since(books.started) / 86_400.0;
    let career = game.career(w);
    let detail = format!(
        "{} flights in {days:.1} days of game time; {:.1} science and {} in hand, {} nodes \
         unlocked",
        books.flights,
        career.science,
        money(career.funds),
        career.unlocked.len()
    );
    w.info(Area::Career, "The career so far", detail);
}

/// A flight of a capsule on a solid booster, the parts list's or `typed`, up and down and
/// recovered, in phase `phase`; then the books.
fn hop(
    w: &mut World,
    game: &Game,
    books: &mut Books,
    phase: &str,
    name: &str,
    typed: Option<Typed>,
) -> Flight {
    let craft = ready(w, game, books, phase, name, |h| first_rocket(h, typed))?;
    capsule_hop(w)?;
    recover(w, game)?;
    close(w, game, books, &craft);
    Ok(())
}

/// A capsule to orbit on three K-1s under one tank and an A-10 upper stage, in phase `phase`; it
/// stays there. Then the books.
fn orbit(w: &mut World, game: &Game, books: &mut Books, phase: &str) -> Flight {
    let craft = ready(w, game, books, phase, "Orbiter", |h| orbiter(h, false))?;
    ascend(w, 0, ORBIT_INCLINATION)?;
    close(w, game, books, &craft);
    Ok(())
}

/// The orbiter: a C-1 with its parachute on an A-10 stage, on a 3 m tank with three K-1s round its
/// end; with a thermometer, a barometer and a telemetry unit on the upper stage if `instruments`.
pub(crate) fn orbiter(h: &mut Hangar, instruments: bool) -> Result<(), String> {
    h.root("pod:C-1 Capsule", "capsule")?;
    h.on_node("parachute", "parachute", "capsule", "top", "bottom")?;
    h.on_node("decoupler", "decoupler", "capsule", "bottom", "top")?;
    h.on_node("tank", "upper_tank", "decoupler", "bottom", "top")?;
    h.on_node("engine:A-10", "upper_engine", "upper_tank", "bottom", "top")?;
    h.on_node("decoupler", "interstage", "upper_engine", "bottom", "top")?;
    h.on_node("tank", "lower_tank", "interstage", "bottom", "top")?;
    // Typed in: the parts list sizes a tank for an engine under it on its node, not for a cluster
    // round its end (T337).
    let sizes = [("diameter", 3.0), ("length", LOWER_TANK)];
    h.resize("lower_tank", &sizes)?;
    h.under_end(
        "engine:K-1 Booster",
        "booster_engine",
        "lower_tank",
        "aft_dome",
        0.0,
        0.45,
        3,
    )?;
    h.on_surface("fin", "fin", "lower_tank", "barrel", 0.125, 0.12, 4)?;
    if instruments {
        let on = [
            ("instrument:thermometer", "thermometer", 0.0),
            ("instrument:barometer", "barometer", 0.25),
            ("telemetry", "telemetry", 0.5),
        ];
        for (entry, id, u) in on {
            h.on_surface(entry, id, "upper_tank", "barrel", u, 0.5, 1)?;
        }
    }
    Ok(())
}

/// The orbiter on the pad flown to an orbit clear of the air: the cluster, then the upper stage
/// until the lowest point clears the contract's.
pub(crate) fn ascend(w: &mut World, spare: u32, inclination: f64) -> Flight {
    let mut ascent = Ascent::new(
        w,
        Profile {
            inclination,
            altitude: ORBIT_ALTITUDE,
            clear: 60.0,
            turn_end: 120_000.0,
            turn_shape: 0.5,
            max_aoa: 6.0,
            vacuum_q: 100.0,
        },
    );
    w.vessel_mut().set_throttle(1.0);
    w.stage();
    let lifted = w.fly(10.0, |w| w.liftoff.is_some());
    w.must(
        Area::Propulsion,
        "The cluster lifts it off the pad",
        lifted,
        "",
    )?;
    let burnt = w.fly(400.0, |w| {
        ascent.steer(w);
        !w.vessel().propulsion.engines.iter().any(|e| e.running) && w.met() > 10.0
    });
    w.must(Area::Propulsion, "The first stage burns out", burnt, "")?;
    let (altitude, _) = height(w);
    w.info(
        Area::Propulsion,
        "First stage gone",
        format!("{:.0} km up at {:.0} m/s", altitude / 1e3, speed(w)),
    );
    // The next sequence drops it and lights the upper stage, in one.
    w.stage();
    w.fly(2.0, |w| {
        ascent.steer(w);
        false
    });
    let lit = w.vessel().propulsion.engines.iter().any(|e| e.running);
    w.must(Area::Propulsion, "The upper stage lights", lit, "")?;
    // Up to `spare` more stages, each lit by the next sequence as the one before burns out.
    let (mut cut, mut spare, mut staged) = (false, spare, w.now());
    let out = w.fly(1800.0, |w| {
        ascent.steer(w);
        if periapsis(w) >= CUTOFF_PERIAPSIS {
            w.vessel_mut().set_throttle(0.0);
            cut = true;
        }
        let burning = w.vessel().propulsion.engines.iter().any(|e| e.running);
        if !cut && !burning && spare > 0 && w.now().seconds_since(staged) > 3.0 {
            w.stage();
            (spare, staged) = (spare - 1, w.now());
            return false;
        }
        cut || (!burning && w.now().seconds_since(staged) > 3.0)
    });
    // The climb's hands off the controls, for SAS to fly by.
    w.vessel_mut().rotation_input = DVec3::ZERO;
    let (apo, peri) = (apoapsis(w), periapsis(w));
    w.must(
        Area::Orbit,
        "The upper stage puts it in an orbit clear of the air",
        out && cut,
        format!("{:.0} by {:.0} km", peri / 1e3, apo / 1e3),
    )?;
    w.fly(2.0, |_| false);
    Ok(())
}

/// The start of a flight in phase `phase`: what is offered taken on, the craft `name` built with
/// `parts` and put on the pad.
pub(crate) fn ready(
    w: &mut World,
    game: &Game,
    books: &mut Books,
    phase: &str,
    name: &str,
    parts: impl FnOnce(&mut Hangar) -> Result<(), String>,
) -> Result<CraftDef, Abort> {
    w.phase(phase);
    take_offers(w, game);
    let career = game.career(w);
    books.science = career.science;
    books.funds = career.funds;
    let craft = build(w, game, name, parts)?;
    launch(w, game, craft.clone(), books.flights == 0)?;
    books.flights += 1;
    Ok(craft)
}

/// Every contract offered is taken on.
fn take_offers(w: &mut World, game: &Game) {
    let mut career = game.career(w).clone();
    let offered: Vec<_> = career
        .offers(&game.contracts)
        .into_iter()
        .cloned()
        .collect();
    for def in &offered {
        let taken = career.accept(def, &game.contracts, &game.sites, &w.universe.system);
        let detail = match &taken {
            Ok(_) => format!("pays {}", money(def.reward)),
            Err(e) => e.clone(),
        };
        w.check(
            Area::Career,
            &format!("Taken on: {}", def.title),
            taken.is_ok(),
            detail,
        );
    }
    w.universe.career = Some(career);
}

/// The end of a flight of `craft`: what it was made of and what it earned, then what is bought
/// with it, and the day.
pub(crate) fn close(w: &mut World, game: &Game, books: &Books, craft: &CraftDef) {
    let career = game.career(w).clone();
    // Each contract taken on before the flight and met in it.
    let done: Vec<&str> = (game.contracts.iter())
        .filter(|def| career.contracts.done.contains(&def.id))
        .filter(|def| !career.contracts.taken.iter().any(|c| c.def.id == def.id))
        .map(|def| def.title.as_str())
        .collect();
    let taken: Vec<&str> = (career.contracts.taken.iter())
        .map(|c| c.def.title.as_str())
        .collect();
    let entries = palette::entries(&w.catalog);
    let mut nodes = BTreeSet::new();
    for part in &craft.parts {
        for entry in palette::sources(part, &entries, &w.catalog) {
            if let Some(node) = (game.tree.node_of(&entry.id)).and_then(|n| game.tree.node(n)) {
                nodes.insert(node.title.clone());
            }
        }
    }
    let science = career.science - books.science;
    let funds = career.funds - books.funds;
    w.check(
        Area::Career,
        "It meets a contract",
        funds > 0.0,
        format!("{} paid; done so far: {}", money(funds), done.join(", ")),
    );
    let bought = invest(w, game, books.wants);
    let (year, month, day) = civil_date(w.now());
    let detail = format!(
        "{} ({} parts, from {}): {science:.1} science and {} earned; then {}; {year}-{month:02}-{day:02}; \
         still taken on: {}",
        craft.name,
        craft.parts.len(),
        nodes.into_iter().collect::<Vec<_>>().join(", "),
        money(funds),
        if bought.is_empty() {
            "nothing bought".to_owned()
        } else {
            bought.join(", ")
        },
        if taken.is_empty() {
            "none".to_owned()
        } else {
            taken.join(", ")
        },
    );
    w.info(Area::Career, "The books", detail);
}

/// Buys what `wants` lists, in order, as far as the career can pay; stops at the first it cannot
/// yet. What was bought.
fn invest(w: &mut World, game: &Game, wants: &[Want]) -> Vec<String> {
    let mut career = game.career(w).clone();
    let mut bought = Vec::new();
    for want in wants {
        match *want {
            Want::Node(id) => {
                if career.unlocked.contains(id) {
                    continue;
                }
                let Some(node) = game.tree.node(id) else {
                    w.check(Area::Science, &format!("The tree has {id}"), false, "");
                    break;
                };
                let title = node.title.clone();
                let cost = game.tree.cost(id);
                if career.unlock(id, &game.tree).is_err() {
                    break;
                }
                bought.push(format!("{title} for {cost:.0} science"));
            }
            Want::Facility(id, level) => {
                let Some(facility) = game.facilities.iter().find(|f| f.id == id) else {
                    w.check(Area::Career, &format!("There is a {id}"), false, "");
                    break;
                };
                if career.level(facility) + 1 >= level {
                    continue;
                }
                let price = next_price(facility, career.level(facility));
                match career.upgrade(facility) {
                    Ok(said) => bought.push(format!("{said} for {}", money(price))),
                    Err(_) => break,
                }
            }
        }
    }
    w.universe.career = Some(career);
    bought
}

fn next_price(facility: &FacilityDef, level: usize) -> f64 {
    facility.levels.get(level + 1).map_or(0.0, |l| l.price)
}

fn speed(w: &World) -> f64 {
    let v = w.vessel();
    v.state_at(&w.universe.system, w.now()).velocity.length()
}

pub(crate) fn periapsis(w: &World) -> f64 {
    let (system, now) = (&w.universe.system, w.now());
    let v = w.vessel();
    v.orbit(system, now).periapsis() - system.body(v.parent()).radius
}

pub(crate) fn apoapsis(w: &World) -> f64 {
    let (system, now) = (&w.universe.system, w.now());
    let v = w.vessel();
    let radius = system.body(v.parent()).radius;
    v.orbit(system, now)
        .apoapsis()
        .map_or(f64::INFINITY, |a| a - radius)
}
