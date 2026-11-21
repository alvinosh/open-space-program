//! A career's first steps, in Casual and in Realistic (T291). A new career on its mode's tech tree
//! takes on the contracts it is offered, builds a first rocket in the builder from the parts its
//! tree's roots unlock, sized as the editor sizes them and within what its first facilities take,
//! and flies it with a player's controls. What its instruments read reaches the ground and earns
//! science, the contract pays, and the science and the money are spent as a player spends them:
//! the cheapest node of the tree, and the cheapest facility upgrade there is money for. The figures
//! the player reads on the way are in the report.

use std::collections::BTreeSet;

use osp_sim::{
    assembly::Side,
    attitude::SasMode,
    builder::{self, Allowed, Builder, Place},
    career::{Career, money},
    component::ComponentDef,
    contract::ContractDef,
    craft::Craft,
    data::{
        CraftDef, CraftPartDef, FacilityDef, LaunchSiteDef, LaunchSiteStartDef, PartCatalog,
        StartDef, SurfaceAtDef, TechNodeDef, TechTreeDef,
    },
    fitting::{self, Against},
    geometry::preset::ShapeDef,
    palette::{self, Entry},
    tech::{ScienceTree, science_tree},
};

use crate::{
    Mission, audit,
    world::{Abort, Area, Flight, PhaseSpec, World},
};

pub static CASUAL: Mission = Mission {
    id: "career_casual",
    title: "Casual career: first launch to the first unlock",
    summary: "A new Casual career takes on the contract it is offered and builds its first rocket \
              from the First Steps parts: a one-seat capsule with its parachute on a solid \
              booster with four fins, and a thermometer, a barometer and a telemetry unit on the \
              booster, the booster sized by the parts list for what it lifts. It climbs some 20 km \
              with the telemetry sending what the instruments read; at the top the capsule lets \
              go, comes down on its parachute and is recovered with its crew alive. \
              The science unlocks the cheapest node of the tree and the money buys the first \
              facility upgrade it can pay for.",
    scenario: "pad",
    mode: Some("casual"),
    plan: PLAN,
    fly: fly_casual,
};

pub static REALISTIC: Mission = Mission {
    id: "career_realistic",
    title: "Realistic career: first sounding rocket to the first unlock",
    summary: "A new Realistic career on RP-1's tree takes on the contract it is offered and builds \
              a sounding rocket from what its start offers: a solid motor cut down to a sounding \
              rocket's size, a nose cone, four fins, a thermometer, a barometer and a telemetry \
              unit. Unguided, it climbs on its fins, and the telemetry sends the readings to the \
              ground at the link's rate until it falls back and is lost. The science unlocks the \
              cheapest node of the tree and the money buys the first facility upgrade it can pay \
              for.",
    scenario: "pad",
    mode: Some("realistic"),
    plan: PLAN,
    fly: fly_realistic,
};

const PLAN: &[PhaseSpec] = &[
    PhaseSpec {
        id: "career",
        title: "A new career",
        goal: "The career starts on its mode's tech tree with the tree's funds, no science, the \
               roots unlocked and every facility at its first level, and takes on the contracts \
               it is offered.",
    },
    PhaseSpec {
        id: "build",
        title: "The first rocket",
        goal: "A rocket is put together in the builder from the parts the roots unlock, each \
               sized for where it goes on as the editor sizes it, and the first facilities take \
               it.",
    },
    PhaseSpec {
        id: "flight",
        title: "The flight",
        goal: "It lifts off, climbs high with its instruments reading, and comes back down; a \
               crew come home alive.",
    },
    PhaseSpec {
        id: "home",
        title: "The data home",
        goal: "What the instruments read reaches the ground, sent or brought back, and earns \
               science; the contract taken on pays.",
    },
    PhaseSpec {
        id: "spend",
        title: "Spending it",
        goal: "The science unlocks a node of the tree and the money upgrades a facility: the \
               parts list and the limits grow.",
    },
];

/// Above this the flight counts as under way, m: what falls back below it has come down.
const CLEAR: f64 = 1_000.0;
/// The softest a capsule lands at that is a recovery and not a crash, m/s.
const SOFT: f64 = 10.0;

fn fly_casual(w: &mut World) -> Flight {
    let game = new_career(w)?;
    w.phase("build");
    let craft = build(w, &game, "First Rocket", |h| first_rocket(h, None))?;
    launch(w, &game, craft, true)?;
    w.phase("flight");
    capsule_hop(w)?;
    // The telemetry sent what it could on the way; the capsule is fetched home.
    w.phase("home");
    let sent = game.career(w).science;
    w.info(
        Area::Comms,
        "Sent home by telemetry on the way",
        format!("{sent:.1} science: {}", subjects(game.career(w))),
    );
    recover(w, &game)?;
    earned(w, &game);
    spend(w, &game)
}

/// The sizes of a solid booster as a player types them into its settings: diameter and length, m,
/// and thrust, kN.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Typed {
    pub diameter: f64,
    pub length: f64,
    pub thrust: f64,
}

/// A Casual career's first rocket, from First Steps: a C-1 with its parachute on a solid booster
/// as the parts list sizes it under the capsule, or `typed` into its settings; four fins round its
/// base, and a thermometer, a barometer and a telemetry unit on it near the top.
pub(crate) fn first_rocket(h: &mut Hangar, typed: Option<Typed>) -> Result<(), String> {
    h.root("pod:C-1 Capsule", "capsule")?;
    h.on_node("parachute", "parachute", "capsule", "top", "bottom")?;
    h.on_node("decoupler", "decoupler", "capsule", "bottom", "top")?;
    h.on_node("solid_booster", "booster", "decoupler", "bottom", "top")?;
    if let Some(typed) = typed {
        let sizes = [("diameter", typed.diameter), ("length", typed.length)];
        h.resize("booster", &sizes)?;
        h.set("booster", "motor_thrust", &format!("{}", typed.thrust))?;
    }
    h.on_surface("fin", "fin", "booster", "barrel", 0.125, 0.12, 4)?;
    let instruments = [
        ("instrument:thermometer", "thermometer", 0.0),
        ("instrument:barometer", "barometer", 0.25),
        ("telemetry", "telemetry", 0.5),
    ];
    for (entry, id, u) in instruments {
        h.on_surface(entry, id, "booster", "barrel", u, 0.9, 1)?;
    }
    Ok(())
}

/// A capsule on a booster flown up and down as the keys say: SAS on, full throttle, the next
/// sequence; at the top the capsule lets go of the booster and arms its parachute, and comes down
/// on it with its crew alive. How high it went, m.
pub(crate) fn capsule_hop(w: &mut World) -> Result<f64, Abort> {
    w.sas(SasMode::Stability);
    w.vessel_mut().set_throttle(1.0);
    w.stage();
    let lifted = w.fly(10.0, |w| w.liftoff.is_some());
    w.must(
        Area::Propulsion,
        "The booster lifts it off the pad",
        lifted,
        "",
    )?;
    w.fly(1.0, |_| false);
    liftoff_thrust(w);
    let mut top: f64 = 0.0;
    let burnt = w.fly(600.0, |w| {
        top = top.max(height(w).0);
        w.vessel().flight.thrust <= 0.0
    });
    w.must(Area::Propulsion, "The booster burns out", burnt, "")?;
    let pitch = w.samples.last().map_or(f64::NAN, |s| s.pitch);
    w.info(
        Area::Aero,
        "Pointing at burnout",
        format!(
            "{pitch:.0} degrees above the horizon, {:.1} km up",
            height(w).0 / 1e3
        ),
    );
    let fell = w.fly(1800.0, |w| {
        let (altitude, climb) = height(w);
        top = top.max(altitude);
        altitude > CLEAR && climb < 0.0
    });
    w.must(
        Area::Propulsion,
        "It climbs and turns back down",
        fell,
        format!("{:.1} km at the top", top / 1e3),
    )?;
    w.info(
        Area::Propulsion,
        "How high it went",
        format!("{:.1} km", top / 1e3),
    );
    // At the top the capsule lets go of the booster, and its parachute is armed: the next two
    // sequences.
    let before = w.vessel().propulsion.parts().parts().len();
    w.stage();
    w.stage();
    let after = w.vessel().propulsion.parts().parts().len();
    w.check(
        Area::Staging,
        "The capsule lets go of the booster",
        after < before,
        format!("{before} parts before, {after} after"),
    );
    let down = w.fly(3600.0, |w| w.touchdown.is_some() || w.flown().is_none());
    let touchdown = w.touchdown.filter(|_| down);
    w.must(
        Area::Recovery,
        "The capsule comes down",
        touchdown.is_some(),
        "",
    )?;
    let (speed, whole) = touchdown.unwrap_or_default();
    w.check(
        Area::Recovery,
        "On its parachute, whole",
        whole && speed <= SOFT,
        format!("{speed:.1} m/s"),
    );
    w.fly(10.0, |_| false);
    let phase = w.phase_id().unwrap_or_default().to_owned();
    let peak = w.peak(&phase, |s| s.g).map_or(0.0, |(g, _)| g);
    w.info(Area::Crew, "The most the crew felt", format!("{peak:.1} g"));
    let (aboard, lost) = crew(w);
    let why = (w.vessel().log.iter())
        .find(|e| e.message.starts_with("Crew lost"))
        .map_or(String::new(), |e| format!(": {}", e.message));
    w.check(
        Area::Crew,
        "The crew come home alive",
        aboard > 0 && lost == 0,
        format!("{aboard} aboard, {lost} lost{why}"),
    );
    Ok(top)
}

/// The vessel flown, down where it can be fetched, is recovered.
pub(crate) fn recover(w: &mut World, game: &Game) -> Flight {
    let id = w.focus;
    let fetched = w.universe.recoverable(id, &game.sites);
    w.must(
        Area::Recovery,
        "It can be recovered where it came down",
        fetched,
        "",
    )?;
    let recovered = w.universe.recover_vessel(id);
    w.check(
        Area::Recovery,
        "It is recovered",
        recovered.is_some(),
        recovered.as_ref().map_or(String::new(), |r| r.notice()),
    );
    Ok(())
}

fn fly_realistic(w: &mut World) -> Flight {
    let game = new_career(w)?;
    w.phase("build");
    let craft = build(w, &game, "Sounding Rocket", |h| {
        h.root("solid_booster", "motor")?;
        // Cut down to a sounding rocket, and its thrust for that, in the part's settings.
        h.resize("motor", &[("diameter", 0.6), ("length", 4.0)])?;
        h.set("motor", "motor_thrust", "60")?;
        h.on_node("nose_cone", "nose", "motor", "top", "bottom")?;
        h.on_surface("fin", "fin", "motor", "barrel", 0.125, 0.1, 4)?;
        h.on_surface(
            "instrument:thermometer",
            "thermometer",
            "motor",
            "barrel",
            0.0,
            0.85,
            1,
        )?;
        h.on_surface(
            "instrument:barometer",
            "barometer",
            "motor",
            "barrel",
            0.25,
            0.85,
            1,
        )?;
        h.on_surface("telemetry", "telemetry", "motor", "barrel", 0.5, 0.85, 1)
    })?;
    launch(w, &game, craft, true)?;

    // Nothing to steer it: the motor is lit and the fins hold it straight.
    w.phase("flight");
    w.vessel_mut().set_throttle(1.0);
    w.stage();
    let lifted = w.fly(10.0, |w| w.liftoff.is_some());
    w.must(
        Area::Propulsion,
        "The motor lifts it off the pad",
        lifted,
        "",
    )?;
    w.fly(1.0, |_| false);
    liftoff_thrust(w);
    let mut top: f64 = 0.0;
    let ended = w.fly(3600.0, |w| match w.flown() {
        Some(v) if !v.is_destroyed() && w.touchdown.is_none() => {
            top = top.max(height(w).0);
            false
        }
        _ => true,
    });
    w.must(
        Area::Propulsion,
        "It climbs and falls back",
        ended && top > CLEAR,
        format!("{:.1} km at the top", top / 1e3),
    )?;
    w.info(
        Area::Propulsion,
        "How high it went",
        format!("{:.1} km", top / 1e3),
    );

    // The rocket is lost: what came home came by telemetry.
    w.phase("home");
    let sent = game.career(w).science;
    w.check(
        Area::Comms,
        "The telemetry sends what the instruments read",
        sent > 0.0,
        format!("{sent:.2} science: {}", subjects(game.career(w))),
    );
    earned(w, &game);
    spend(w, &game)
}

/// What the career is played with: its tree, what it can be offered, and the parts list.
pub(crate) struct Game {
    pub tree: ScienceTree,
    pub facilities: Vec<FacilityDef>,
    pub contracts: Vec<ContractDef>,
    /// The contracts it took on at the start.
    pub taken: Vec<ContractDef>,
    pub sites: Vec<LaunchSiteDef>,
    pub entries: Vec<Entry>,
    /// Its funds when it took them on.
    pub funds: f64,
}

impl Game {
    pub(crate) fn career<'a>(&self, w: &'a World) -> &'a Career {
        w.universe.career.as_ref().expect("a career game")
    }
}

/// A new career game, as New game starts one: nothing flies yet, and the career takes on what it
/// is offered.
pub(crate) fn new_career(w: &mut World) -> Result<Game, Abort> {
    w.clear();
    w.phase("career");
    let registry = &w.content.registry;
    let mut problems = Vec::new();
    let trees: Vec<TechTreeDef> = registry.all("tech_trees", &mut problems);
    let nodes: Vec<TechNodeDef> = registry.all("tech_nodes", &mut problems);
    let facilities: Vec<FacilityDef> = registry.all("facilities", &mut problems);
    let contracts: Vec<ContractDef> = registry.all("contracts", &mut problems);
    let sites: Vec<LaunchSiteDef> = registry.all("launch_sites", &mut problems);
    let problems: Vec<String> = problems.iter().map(ToString::to_string).collect();
    w.check(
        Area::Career,
        "The tree, the facilities and the contracts load",
        problems.is_empty(),
        problems.join("; "),
    );
    let found = science_tree(&w.catalog, &trees, &nodes);
    let detail = match &found {
        Ok(Some(tree)) => format!("{} ({})", tree.def.name, tree.def.id),
        Ok(None) => "the rules name none".into(),
        Err(e) => e.clone(),
    };
    let named = matches!(found, Ok(Some(_)));
    w.must(Area::Science, "The mode has a tech tree", named, detail)?;
    let tree = found.ok().flatten().expect("checked");

    let mut career = Career::new(&tree);
    w.check(
        Area::Career,
        "It starts with the tree's funds",
        career.funds > 0.0 && career.funds == tree.def.funds,
        money(career.funds),
    );
    w.check(
        Area::Science,
        "It starts with no science",
        career.science == 0.0,
        format!("{:.1}", career.science),
    );
    let roots = tree.roots();
    let titles: Vec<&str> = (roots.iter())
        .filter_map(|id| tree.node(id))
        .map(|node| node.title.as_str())
        .collect();
    w.check(
        Area::Science,
        "The tree's roots are unlocked",
        !roots.is_empty() && career.unlocked == roots,
        titles.join(", "),
    );
    let entries = palette::entries(&w.catalog);
    let open: Vec<&str> = (entries.iter())
        .filter(|e| career.has_entry(&tree, &e.id))
        .map(|e| e.name.as_str())
        .collect();
    w.info(
        Area::Build,
        "The parts list",
        format!(
            "{} of its {} entries can be built: {}",
            open.len(),
            entries.len(),
            open.join(", ")
        ),
    );
    w.info(Area::Build, "How big", sizes(&career.allowed(&tree)));
    for facility in &facilities {
        let level = career.level(facility);
        w.check(
            Area::Career,
            &format!("The {} starts at its first level", facility.title()),
            level == 0,
            format!(
                "level {} of {}: {}",
                level + 1,
                facility.levels.len(),
                facility.levels[level].describe()
            ),
        );
    }

    // Every contract offered is taken on.
    let offered: Vec<ContractDef> = career.offers(&contracts).into_iter().cloned().collect();
    let titles: Vec<&str> = offered.iter().map(|c| c.title.as_str()).collect();
    w.must(
        Area::Career,
        "A contract is offered",
        !offered.is_empty(),
        titles.join(", "),
    )?;
    for def in &offered {
        let taken = career.accept(def, &contracts, &sites, &w.universe.system);
        let detail = match &taken {
            Ok(said) => format!("{said}: {}, pays {}", def.description, money(def.reward)),
            Err(e) => e.clone(),
        };
        w.check(
            Area::Career,
            &format!("Contract taken on: {}", def.title),
            taken.is_ok(),
            detail,
        );
    }
    let funds = career.funds;
    w.universe.career = Some(career);
    Ok(Game {
        tree,
        facilities,
        contracts,
        taken: offered,
        sites,
        entries,
        funds,
    })
}

/// Puts a rocket called `name` together in the builder with `parts`, as the career may build it.
pub(crate) fn build(
    w: &mut World,
    game: &Game,
    name: &str,
    parts: impl FnOnce(&mut Hangar) -> Result<(), String>,
) -> Result<CraftDef, Abort> {
    let career = game.career(w);
    let (made, mut craft, warnings) = {
        let mut hangar = Hangar {
            catalog: &w.catalog,
            entries: &game.entries,
            builder: Builder::new(name.to_lowercase().replace(' ', "_")),
        };
        hangar.builder.allow(career.allowed(&game.tree));
        let made = parts(&mut hangar);
        let craft = hangar.builder.into_draft();
        let warnings = builder::warnings(&craft, &w.catalog);
        (made, craft, warnings)
    };
    craft.name = name.to_owned();
    let detail = match &made {
        Ok(()) => format!("{} parts", craft.parts.len()),
        Err(e) => e.clone(),
    };
    w.must(
        Area::Build,
        "It goes together in the builder",
        made.is_ok(),
        detail,
    )?;
    let career = game.career(w);
    let locked = career.locked_parts(&game.tree, &w.catalog, &game.entries, &craft);
    w.must(
        Area::Build,
        "Every part of it is unlocked",
        locked.is_empty(),
        locked.join(", "),
    )?;
    w.check(
        Area::Build,
        "The builder has nothing to warn of",
        warnings.is_empty(),
        warnings.join("; "),
    );
    // What the parts list made of its tanks and boosters, sized where they went on.
    let sizes: Vec<String> = (craft.parts.iter())
        .filter_map(|part| {
            let Some(ShapeDef::CylinderTank(tank)) = &part.shape else {
                return None;
            };
            let motor = part.components.iter().find_map(|c| match c {
                ComponentDef::SolidMotor(motor) => Some(motor.thrust),
                _ => None,
            });
            let thrust = motor.map_or(String::new(), |n| format!(", {:.0} kN", n / 1e3));
            Some(format!(
                "{} {:.2} m by {:.2} m{thrust}",
                part.id, tank.diameter, tank.length
            ))
        })
        .collect();
    w.info(Area::Build, "Tanks and boosters", sizes.join("; "));
    Ok(craft)
}

/// Puts `craft` on the first launch pad, as the editor's Launch does (starting the game, if it is
/// the `first` launch of a new one), and takes the flight over. The pad must be clear and the
/// facilities must take it.
pub(crate) fn launch(w: &mut World, game: &Game, craft: CraftDef, first: bool) -> Flight {
    let pad = game.sites.iter().find(|site| site.pad.is_some());
    w.must(Area::Build, "There is a launch pad", pad.is_some(), "")?;
    let pad = pad.expect("checked");
    let standing = w.universe.on_launch_ground(pad, false);
    w.must(
        Area::Build,
        "The pad is clear",
        standing.is_empty(),
        format!("{} vessels on it", standing.len()),
    )?;
    let start = StartDef::LaunchSite(LaunchSiteStartDef {
        site: pad.id.clone(),
    });
    let now = w.now();
    let new_game = first.then_some(now);
    let launched = (w.universe).launch(&game.sites, &w.catalog, &craft, &start, None, new_game);
    let detail = match &launched {
        Ok(_) => format!("{} on {}", craft.name, pad.id),
        Err(e) => e.to_string(),
    };
    w.must(
        Area::Build,
        "It stands on the pad",
        launched.is_ok(),
        detail,
    )?;
    w.take_over(launched.expect("checked"), craft);
    w.audit = Some(audit::inspect(w));

    // What the facilities hold it to, measured as the editor measures it.
    let size = w.universe.craft_size(w.focus).unwrap_or_default();
    let career = game.career(w);
    let over = career.refuses(&game.facilities, &size, true);
    let limits: Vec<String> = (game.facilities.iter())
        .map(|f| format!("{}: {}", f.title(), f.levels[career.level(f)].describe()))
        .collect();
    let detail = if over.is_empty() {
        format!(
            "{:.1} t, {:.1} m tall, {:.1} m wide, {} parts; {}",
            size.mass / 1e3,
            size.height,
            size.width,
            size.parts,
            limits.join("; ")
        )
    } else {
        over.join(". ")
    };
    w.must(
        Area::Career,
        "The first facilities take it",
        over.is_empty(),
        detail,
    )
}

/// The contracts taken on are done and paid, and what the flight earned.
pub(crate) fn earned(w: &mut World, game: &Game) {
    let career = game.career(w).clone();
    for def in &game.taken {
        let done = career.contracts.done.contains(&def.id);
        w.check(
            Area::Career,
            &format!("Contract done: {}", def.title),
            done,
            if done {
                format!("paid {}", money(def.reward))
            } else {
                "not met".into()
            },
        );
    }
    let paid: f64 = (game.taken.iter())
        .filter(|def| career.contracts.done.contains(&def.id))
        .map(|def| def.reward)
        .sum();
    w.check(
        Area::Career,
        "The funds grow by what they paid",
        (career.funds - game.funds - paid).abs() < 1e-6,
        format!("{} to {}", money(game.funds), money(career.funds)),
    );
    w.check(
        Area::Science,
        "The flight earns science",
        career.science > 0.0,
        format!("{:.1} science", career.science),
    );
}

/// The science on the cheapest node there is, and the money on the cheapest upgrade it pays for.
pub(crate) fn spend(w: &mut World, game: &Game) -> Flight {
    w.phase("spend");
    let tree = &game.tree;
    let career = game.career(w).clone();
    let mut ready = tree.ready(&career.unlocked);
    ready.sort_by(|a, b| tree.cost(&a.id).total_cmp(&tree.cost(&b.id)));
    let menu: Vec<String> = (ready.iter())
        .map(|node| format!("{} {:.0}", node.title, tree.cost(&node.id)))
        .collect();
    w.info(Area::Science, "What the tree offers next", menu.join(", "));
    let cheapest = ready.first().copied();
    let cost = cheapest.map_or(f64::INFINITY, |node| tree.cost(&node.id));
    w.must(
        Area::Science,
        "The flight's science pays for a node",
        cost <= career.science,
        format!(
            "{:.1} science; the cheapest node costs {cost:.0}",
            career.science
        ),
    )?;
    let node = cheapest.expect("checked");
    let mut after = career.clone();
    let unlocked = after.unlock(&node.id, tree);
    w.check(
        Area::Science,
        &format!("{} is unlocked", node.title),
        unlocked.is_ok() && (career.science - after.science - cost).abs() < 1e-9,
        match &unlocked {
            Ok(()) => format!("{cost:.0} science, {:.1} left", after.science),
            Err(e) => e.clone(),
        },
    );
    // What it lets be built: entries, sizes or tank types.
    let new: Vec<&str> = (game.entries.iter())
        .filter(|e| !career.has_entry(tree, &e.id) && after.has_entry(tree, &e.id))
        .map(|e| e.name.as_str())
        .collect();
    let (was, now) = (career.allowed(tree), after.allowed(tree));
    let grown = now != was;
    w.check(
        Area::Build,
        "It lets more be built",
        !new.is_empty() || grown,
        if grown {
            format!("{}; {}", new.join(", "), sizes(&now))
        } else {
            new.join(", ")
        },
    );

    // The cheapest upgrade it has the money for.
    let mut upgrades: Vec<(&FacilityDef, f64)> = (game.facilities.iter())
        .filter_map(|f| Some((f, f.levels.get(after.level(f) + 1)?.price)))
        .collect();
    upgrades.sort_by(|a, b| a.1.total_cmp(&b.1));
    let menu: Vec<String> = (upgrades.iter())
        .map(|(f, price)| {
            let next = &f.levels[after.level(f) + 1];
            format!("{} to {}, {}", f.title(), next.describe(), money(*price))
        })
        .collect();
    w.info(
        Area::Career,
        "Upgrades on offer",
        format!("{}; {} in funds", menu.join("; "), money(after.funds)),
    );
    let affordable = upgrades.iter().find(|(_, price)| *price <= after.funds);
    w.must(
        Area::Career,
        "The money pays for an upgrade",
        affordable.is_some(),
        money(after.funds),
    )?;
    let &(facility, price) = affordable.expect("checked");
    let funds = after.funds;
    let before = facility.levels[after.level(facility)].describe();
    let upgraded = after.upgrade(facility);
    let level = after.level(facility);
    w.check(
        Area::Career,
        &format!("The {} is upgraded", facility.title()),
        upgraded.is_ok() && level == 1 && (funds - after.funds - price).abs() < 1e-6,
        match &upgraded {
            Ok(said) => format!(
                "{said}: {before} to {}; {} left",
                facility.levels[level].describe(),
                money(after.funds)
            ),
            Err(e) => e.clone(),
        },
    );
    let offers: Vec<String> = (after.offers(&game.contracts).iter())
        .map(|c| format!("{} ({})", c.title, money(c.reward)))
        .collect();
    w.info(Area::Career, "Contracts offered next", offers.join(", "));
    w.universe.career = Some(after);
    Ok(())
}

/// The builder as the editor drives it: a part from the parts list, brought within what the career
/// may build, sized for where it goes on ([`fitting`]) and put on there.
pub(crate) struct Hangar<'a> {
    catalog: &'a PartCatalog,
    entries: &'a [Entry],
    builder: Builder,
}

impl Hangar<'_> {
    /// A new part from the list's `entry`, called `id`, and the node it goes on a surface by.
    fn take(&self, entry: &str, id: &str) -> Result<(CraftPartDef, Option<(String, f64)>), String> {
        let entry = (self.entries.iter())
            .find(|e| e.id == entry)
            .ok_or_else(|| format!("no `{entry}` in the parts list"))?;
        let mut part = entry.part.clone();
        part.id = id.to_owned();
        self.builder.allowed().bring_within(&mut part, self.catalog);
        Ok((part, entry.surface.clone()))
    }

    fn built(&self) -> Result<Craft, String> {
        Craft::build(self.builder.draft(), self.catalog).map_err(|e| e.to_string())
    }

    /// The first part, which the rest go on.
    pub(crate) fn root(&mut self, entry: &str, id: &str) -> Result<(), String> {
        let (part, _) = self.take(entry, id)?;
        (self.builder.place(self.catalog, part, None))
            .map(drop)
            .map_err(|r| format!("{id}: {}", r.message))
    }

    /// On `node` of `on` by its own `by`.
    pub(crate) fn on_node(
        &mut self,
        entry: &str,
        id: &str,
        on: &str,
        node: &str,
        by: &str,
    ) -> Result<(), String> {
        let (mut part, _) = self.take(entry, id)?;
        let built = self.built()?;
        let holder = built.find(on).ok_or_else(|| format!("no part `{on}`"))?;
        let width = (built.assembly.parts()[holder].node(node))
            .ok_or_else(|| format!("`{on}` has no node `{node}`"))?
            .size;
        let limits = self.builder.allowed().sizes;
        let draft = self.builder.draft();
        let against = Against {
            engine: fitting::feeding(&built, holder, node),
            under: fitting::below_engine(&built, holder, node),
            lifts: fitting::lifted(&part, draft, &built, holder, node, self.catalog),
            limits,
            ..Against::node(width)
        };
        fitting::fit(&mut part, &against, self.catalog);
        limits.shrink(&mut part);
        let place = Place::node(format!("{on}.{node}"), by);
        self.put(id, part, vec![(place, false)])
    }

    /// On `surface` of `on` at `u`, `v`, and `count` of them round it as radial symmetry puts them.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn on_surface(
        &mut self,
        entry: &str,
        id: &str,
        on: &str,
        surface: &str,
        u: f64,
        v: f64,
        count: u32,
    ) -> Result<(), String> {
        let (mut part, held) = self.take(entry, id)?;
        let (by, roll) = held.ok_or_else(|| format!("{id} does not go on a surface"))?;
        let built = self.built()?;
        let holder = built.find(on).ok_or_else(|| format!("no part `{on}`"))?;
        let point = (built.parts[holder]
            .built
            .geometry
            .surface_point(surface, u, v))
        .ok_or_else(|| format!("`{on}` has no surface `{surface}`"))?;
        let limits = self.builder.allowed().sizes;
        let against = Against {
            limits,
            ..Against::surface(2.0 * point.position.truncate().length())
        };
        fitting::fit(&mut part, &against, self.catalog);
        limits.shrink(&mut part);
        let places: Vec<(Place, bool)> = (0..count)
            .map(|k| {
                let at = SurfaceAtDef {
                    u: (u + f64::from(k) / f64::from(count)).rem_euclid(1.0),
                    v,
                    side: Side::Front,
                    standoff: 0.0,
                    axial: false,
                };
                let mut place = Place::surface(format!("{on}.{surface}"), by.clone(), at);
                place.roll = roll;
                (place, false)
            })
            .collect();
        self.put(id, part, places)
    }

    /// `count` of what `entry` makes, by symmetry round the end `surface` of `on` (a tank's bottom)
    /// at `u`, `v`: each goes on by its own node facing that end, along the axis, as engines under a
    /// tank do.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn under_end(
        &mut self,
        entry: &str,
        id: &str,
        on: &str,
        surface: &str,
        u: f64,
        v: f64,
        count: u32,
    ) -> Result<(), String> {
        let (part, _) = self.take(entry, id)?;
        let alone = builder::alone(&part, self.catalog).map_err(|r| r.message)?;
        // Its own node that faces up the craft, towards the end it goes under.
        let by = (alone.assembly.parts()[0].nodes())
            .filter(|n| n.direction.z > 0.5)
            .max_by(|a, b| a.position.z.total_cmp(&b.position.z))
            .map(|n| n.name.clone())
            .ok_or_else(|| format!("{id} has no node facing up"))?;
        let places = (0..count)
            .map(|k| {
                let at = SurfaceAtDef {
                    u: (u + f64::from(k) / f64::from(count)).rem_euclid(1.0),
                    v,
                    side: Side::Front,
                    standoff: 0.0,
                    axial: true,
                };
                (
                    Place::surface(format!("{on}.{surface}"), by.clone(), at),
                    false,
                )
            })
            .collect();
        self.put(id, part, places)
    }

    /// `part` put at each of `places`, as the editor puts what is in hand.
    fn put(
        &mut self,
        id: &str,
        part: CraftPartDef,
        places: Vec<(Place, bool)>,
    ) -> Result<(), String> {
        (self
            .builder
            .put_set(self.catalog, &[part], &places, &BTreeSet::new()))
        .map(drop)
        .map_err(|r| format!("{id}: {}", r.message))
    }

    /// Sizes of `id`'s shape, as its settings' boxes are typed in.
    pub(crate) fn resize(&mut self, id: &str, sizes: &[(&str, f64)]) -> Result<(), String> {
        (self.builder.resize(self.catalog, id, sizes))
            .map(drop)
            .map_err(|r| format!("{id}: {}", r.message))
    }

    /// One of `id`'s settings, as typed in.
    pub(crate) fn set(&mut self, id: &str, field: &str, text: &str) -> Result<(), String> {
        (self.builder.set_field(self.catalog, id, field, text))
            .map(drop)
            .map_err(|r| format!("{id}: {}", r.message))
    }
}

/// The people aboard the vessel flown, alive, and how many have died.
pub(crate) fn crew(w: &World) -> (u32, u32) {
    let systems = &w.vessel().propulsion.systems;
    (systems.all::<osp_sim::systems::Crew>()).fold((0, 0), |(aboard, lost), (_, crew)| {
        (aboard + crew.aboard, lost + crew.lost)
    })
}

/// How hard the vessel flown pushes for its weight, a second after liftoff.
fn liftoff_thrust(w: &mut World) {
    let flight = &w.vessel().flight;
    let weight = flight.mass * 9.80665;
    let detail = format!(
        "{:.0} kN on {:.1} t: {:.1} times its weight",
        flight.thrust / 1e3,
        flight.mass / 1e3,
        flight.thrust / weight.max(1e-9)
    );
    w.info(Area::Propulsion, "Thrust a second after liftoff", detail);
}

/// How high the vessel flown is above the mean radius of what it is near, m, and how fast it climbs.
pub(crate) fn height(w: &World) -> (f64, f64) {
    let (system, now) = (&w.universe.system, w.now());
    let v = w.vessel();
    let body = system.body(v.parent());
    let state = v.state_at(system, now);
    let up = state.position.normalize_or_zero();
    let ground = state.velocity - body.surface_velocity(state.position);
    (state.position.length() - body.radius, ground.dot(up))
}

/// What has reached the ground of each subject, as shares of a whole set.
pub(crate) fn subjects(career: &Career) -> String {
    let all: Vec<String> = (career.returned.iter())
        .map(|(subject, share)| format!("{subject} {share:.2}"))
        .collect();
    if all.is_empty() {
        "nothing".into()
    } else {
        all.join(", ")
    }
}

/// How big a career may build, as a player reads it.
pub(crate) fn sizes(allowed: &Allowed) -> String {
    let wide = allowed.sizes.diameter;
    let wide = if wide.is_finite() {
        format!("procedural parts up to {wide:.2} m across")
    } else {
        "procedural parts any width".into()
    };
    match &allowed.tank_types {
        Some(types) => format!("{wide}; tanks of {}", types.join(", ")),
        None => format!("{wide}; tanks of any type"),
    }
}
