//! How a new part from the parts list is sized for where it goes on, as KSP's procedural parts
//! take the size of what they join. A tank, tube or decoupler put on a node is as wide as the node,
//! and a nose cone's base as wide. One put on a hull's side, a booster beside its core, is half as
//! wide as the hull there. A fin or wing is sized for the hull it stands on. A tank put on an
//! engine's mount is sized to feed that engine through a stage's burn: as wide as the nozzle at
//! least, and holding what the engine burns in [`BOOSTER_BURN`] at sea level or [`UPPER_BURN`] in
//! vacuum, measured on the tank as built, filled with the engine's propellants. Under an engine's
//! nozzle a part is as wide as the stack the engine hangs from, and a decoupler there gets an
//! interstage up round the engine to that stack's base ([`below_engine`]): the next stage down
//! hangs there, as under a KSP engine, and the body stays whole for the air. A solid booster put
//! under a craft is sized for what it lifts ([`HOP_DELTA_V`], [`HOP_TWR`]), not as a tank: its grain
//! gives the stack above a first hop's speed, and its thrust lifts it briskly off the pad. A part
//! from a catalogue record (a pod, an engine) stays as it is, and so does a part being moved.

use crate::{
    assembly::Mount,
    component::{BOOSTER_NOZZLE_MASS, Component, ComponentDef, SolidMotorDef},
    constants::G0,
    craft::Craft,
    data::{CraftDef, CraftPartDef, EngineDef, PartCatalog},
    geometry::preset::{AirfoilDef, ShapeDef},
    properties::{self, Mix},
    shaping::{self, Limits},
    sizing::ProceduralDef,
};

/// How long a tank on a sea-level engine's mount feeds it at full thrust, s: a first stage's burn.
pub const BOOSTER_BURN: f64 = 150.0;
/// How long a tank on a vacuum engine's mount feeds it at full thrust, s: an upper stage's burn.
pub const UPPER_BURN: f64 = 350.0;
/// An engine whose Isp at sea level is at least this share of its vacuum Isp is a sea-level one.
const SEA_LEVEL: f64 = 0.7;
/// The least thrust to weight a tank sized for an engine put under it leaves its stage at
/// ignition: a sea-level engine's at sea level, lifting off; a vacuum engine's in vacuum, on an
/// upper stage, weighed at the surface.
const BOOSTER_TWR: f64 = 1.4;
const UPPER_TWR: f64 = 0.6;
/// A tank's length for its width, when nothing else sets it.
const ASPECT: f64 = 3.0;
/// How wide a part put on a hull's side is, for the hull's width: a booster beside its core.
const BESIDE: f64 = 0.5;
/// The speed a solid booster put under a craft gives all of it, m/s, with no air or gravity to
/// take their share, at the mean of its impulse at sea level and in a vacuum: a first sounding
/// flight. Under a C-1 Capsule the air and gravity take three quarters of it (a light rocket with a
/// blunt nose), and it goes some 20 km up and comes down at a few g; a heavier stack goes higher, and
/// a longer booster, typed in, takes it to space.
pub const HOP_DELTA_V: f64 = 2000.0;
/// What a solid booster sized for what it lifts pushes for the weight of all of it at sea level.
pub const HOP_TWR: f64 = 2.5;
/// The narrowest a booster sized for what it lifts is made, m.
const NARROWEST: f64 = 0.3;

/// What a new part goes on, for [`fit`].
#[derive(Debug, Clone, Copy)]
pub struct Against<'a> {
    /// How wide what it goes on is there, m: the node's size, or the hull's width across where it
    /// goes on a surface.
    pub width: f64,
    /// It goes on a surface: beside the hull, not on an end of it.
    pub beside: bool,
    /// The engine whose mount it goes on ([`feeding`]): a tank is sized to feed it.
    pub engine: Option<&'a EngineDef>,
    /// It goes under an engine's nozzle ([`below_engine`]).
    pub under: Option<Stack>,
    /// What it holds up, kg, if it goes under a craft ([`lifted`]): a solid booster is sized for it.
    pub lifts: Option<f64>,
    /// The biggest the game lets a part be made.
    pub limits: Limits,
}

impl<'a> Against<'a> {
    /// At a node `width` across, with no more known.
    pub fn node(width: f64) -> Self {
        Self {
            width,
            beside: false,
            engine: None,
            under: None,
            lifts: None,
            limits: Limits::NONE,
        }
    }

    /// On a surface of a hull `width` across.
    pub fn surface(width: f64) -> Self {
        Self {
            beside: true,
            ..Self::node(width)
        }
    }
}

/// What hangs under an engine's nozzle is in: the stack the engine hangs from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stack {
    /// How wide the stack is where the engine hangs from it, m.
    pub width: f64,
    /// From the nozzle's exit up to there, m: how tall an interstage round the engine is.
    pub height: f64,
}

/// Whether `node` of the part at `index` in `craft` is an engine's back, under its nozzle.
pub fn engine_back(craft: &Craft, index: usize, node: &str) -> bool {
    let Some(part) = craft.parts.get(index) else {
        return false;
    };
    let engine = (part.built.components.iter()).any(|c| matches!(c, Component::Engine(_)));
    let node = (craft.assembly.parts().get(index)).and_then(|p| p.node(node));
    engine && node.is_some_and(|n| n.direction.z < -0.5)
}

/// The stack that `node` of the part at `index` in `craft` hangs under, if that is an engine's back
/// ([`engine_back`]) and the engine hangs by its front from something as wide as a stack: how wide
/// that is, and how far above the node.
pub fn below_engine(craft: &Craft, index: usize, node: &str) -> Option<Stack> {
    if !engine_back(craft, index, node) {
        return None;
    }
    let parts = craft.assembly.parts();
    let back = parts.get(index)?.node(node)?;
    craft.assembly.joints().iter().find_map(|joint| {
        let [a, b] = &joint.ends;
        let (own, other) = match (a.part == index, b.part == index) {
            (true, _) => (a, b),
            (_, true) => (b, a),
            _ => return None,
        };
        let (Mount::Node(own), Mount::Node(held_by)) = (&own.mount, &other.mount) else {
            return None;
        };
        let front = parts[index].node(own).filter(|n| n.direction.z > 0.5)?;
        let width = parts[other.part].node(held_by)?.size;
        let height = front.position.z - back.position.z;
        (width > 0.0 && height > 0.0).then_some(Stack { width, height })
    })
}

/// Whether `node` of the part at `index` in `craft` is an engine's front, where what feeds it
/// goes on: a thrust structure, which takes a part of any width.
pub fn engine_front(craft: &Craft, index: usize, node: &str) -> bool {
    let Some(part) = craft.parts.get(index) else {
        return false;
    };
    let engine = (part.built.components.iter()).any(|c| matches!(c, Component::Engine(_)));
    // It pushes along +Z: its front is that way.
    let node = (craft.assembly.parts().get(index)).and_then(|p| p.node(node));
    engine && node.is_some_and(|n| n.direction.z > 0.5)
}

/// The engine whose mount `node` of the part at `holder` in `craft` is ([`engine_front`]): what a
/// tank put there feeds. None if the part is no engine, or one a tank does not feed (a solid holds
/// its own propellant, an electric thruster burns little for a long time).
pub fn feeding<'a>(craft: &'a Craft, holder: usize, node: &str) -> Option<&'a EngineDef> {
    if !engine_front(craft, holder, node) {
        return None;
    }
    let components = &craft.parts.get(holder)?.built.components;
    if (components.iter()).any(|c| matches!(c, Component::Storage(_))) {
        return None;
    }
    let engine = components.iter().find_map(|c| match c {
        Component::Engine(engine) => Some(&**engine),
        _ => None,
    })?;
    if matches!(engine.kind.as_deref(), Some("solid" | "electric")) {
        return None;
    }
    Some(engine)
}

/// `part`, a palette entry's new part, sized for what it goes on ([`Against`]). A part from a
/// catalogue record stays as it is.
pub fn fit(part: &mut CraftPartDef, against: &Against, catalog: &PartCatalog) {
    // Under an engine's nozzle, as wide as the stack it hangs from.
    let width = against.under.map_or(against.width, |stack| stack.width);
    if !(width.is_finite() && width > 0.0) {
        return;
    }
    // A booster beside its core: half as wide. A fin or wing is sized for the whole hull.
    let wing = matches!(&part.shape, Some(ShapeDef::Wing(_)));
    let width = if against.beside && !wing {
        width * BESIDE
    } else {
        width
    };
    if let Some(ProceduralDef::Decoupler(decoupler)) = &mut part.procedural {
        decoupler.diameter = width;
        // Up round the engine above to the base of its stack.
        decoupler.interstage = against.under.map_or(0.0, |stack| stack.height);
        return;
    }
    // A fuel line is a small fitting, however big the tank it is on.
    if (part.components.iter()).any(|c| matches!(c, ComponentDef::FuelLine(_))) {
        return;
    }
    if part.template.is_some() || part.procedural.is_some() {
        return;
    }
    if let Some(mass) = against.lifts.filter(|_| !against.beside)
        && boost(part, width, mass, against.limits, catalog)
    {
        return;
    }
    if let Some(engine) = against.engine
        && feed(
            part,
            engine,
            width,
            None,
            f64::INFINITY,
            against.limits,
            catalog,
        )
    {
        return;
    }
    let Some(shape) = part.shape.as_ref() else {
        return;
    };
    let sizes: Vec<(&str, f64)> = match shape {
        // Longer first, so its domes have room for the width.
        ShapeDef::CylinderTank(_) => vec![("length", ASPECT * width), ("diameter", width)],
        ShapeDef::Shell(d) if d.sections.last().and_then(|s| s.top_diameter) == Some(0.0) => {
            vec![("length", 1.5 * width), ("bottom_diameter", width)]
        }
        // A tube as wide at both ends, whichever it goes on by.
        ShapeDef::Shell(_) => vec![
            ("length", 0.5 * width),
            ("bottom_diameter", width),
            ("top_diameter", width),
        ],
        // On a hull `width` across: big enough to keep a short rocket pointing into the air as it
        // burns its tank down and goes supersonic (a C-1 Capsule on its tank and a K-1 Booster,
        // with these, keeps its centre of pressure behind its centre of mass to Mach 8).
        ShapeDef::Wing(d) if matches!(d.airfoil, AirfoilDef::Plate { .. }) => vec![
            ("root_chord", 1.8 * width),
            ("tip_chord", 0.75 * width),
            ("span", 1.2 * width),
            ("sweep", 0.9 * width),
        ],
        ShapeDef::Wing(_) => vec![
            ("root_chord", 1.5 * width),
            ("tip_chord", 0.6 * width),
            ("span", 2.0 * width),
            ("sweep", 0.6 * width),
        ],
        _ => Vec::new(),
    };
    for (size, value) in sizes {
        // Sizes that a shape does not take are left as they are.
        let _ = shaping::resize(part, size, value);
    }
}

/// What `part`, about to go on `node` of the part at `holder` in `craft` (built from `draft`), holds
/// up there: all of the craft, if it is a solid booster going under it, on a node facing down. None
/// for any other part or place.
pub fn lifted(
    part: &CraftPartDef,
    draft: &CraftDef,
    craft: &Craft,
    holder: usize,
    node: &str,
    catalog: &PartCatalog,
) -> Option<f64> {
    booster(part)?;
    let node = craft.assembly.parts().get(holder)?.node(node)?;
    if node.direction.z > -0.5 {
        return None;
    }
    let mass = crate::propulsion::Propulsion::from_craft(draft, catalog)
        .ok()?
        .mass();
    (mass > 0.0).then_some(mass)
}

fn booster(part: &CraftPartDef) -> Option<&SolidMotorDef> {
    part.components.iter().find_map(|c| match c {
        ComponentDef::SolidMotor(motor) if motor.booster => Some(motor),
        _ => None,
    })
}

/// Size `part`, a solid booster under `lifts` kg of craft on a node `width` across, for what it
/// lifts: grain enough to give all of it [`HOP_DELTA_V`], and thrust for [`HOP_TWR`] at sea level.
/// As wide as the node if that leaves it at least [`ASPECT`] times as long as it is wide, narrower
/// if not; no bigger than `limits`. False, and `part` as it was, if it is no booster or cannot be.
fn boost(
    part: &mut CraftPartDef,
    width: f64,
    lifts: f64,
    limits: Limits,
    catalog: &PartCatalog,
) -> bool {
    let Some(motor) = booster(part).cloned() else {
        return false;
    };
    if !matches!(&part.shape, Some(ShapeDef::CylinderTank(_))) || motor.isp_sl <= 0.0 {
        return false;
    }
    let ratio = (HOP_DELTA_V / (G0 * 0.5 * (motor.isp_vac + motor.isp_sl))).exp();
    // Its thrust is rated in a vacuum; at sea level it is less, as its impulse is.
    let rating = motor.isp_vac / motor.isp_sl;
    // Measured on the booster as built at width `d`: its own mass and its grain's are each so much
    // with no length and so much a metre (its domes, walls and nozzle count). The nozzle weighs
    // with the thrust.
    let measure = |d: f64| -> Option<[f64; 4]> {
        let at = |length: f64| -> Option<(f64, f64)> {
            let mut probe = part.clone();
            set_both(&mut probe, d, length).ok()?;
            let built = properties::alone(&probe, catalog).ok()?;
            let built = &built.parts[0].built;
            let grain = (built.storages())
                .flat_map(|s| s.contents.iter().map(|c| c.capacity))
                .sum::<f64>();
            Some((built.mass.mass, grain))
        };
        let (short, long) = (ASPECT * d, 2.0 * ASPECT * d);
        let ((dry_1, grain_1), (dry_2, grain_2)) = (at(short)?, at(long)?);
        let dry = (dry_2 - dry_1) / (long - short);
        let grain = (grain_2 - grain_1) / (long - short);
        Some([dry_1 - dry * short, dry, grain_1 - grain * short, grain])
    };
    // The length and thrust that give it all, at width `d`.
    let solve = |d: f64| -> Option<(f64, f64)> {
        let [dry_0, dry, grain_0, grain] = measure(d)?;
        let (mut thrust, mut length) = (motor.thrust, 0.0);
        for _ in 0..8 {
            let nozzle = BOOSTER_NOZZLE_MASS * (thrust - motor.thrust);
            // Full is `ratio` times empty: the grain is `ratio - 1` times all that is left.
            let empty = lifts + dry_0 + nozzle;
            length = ((ratio - 1.0) * empty - grain_0) / (grain - (ratio - 1.0) * dry);
            let full = empty + (dry + grain) * length + grain_0;
            thrust = HOP_TWR * G0 * full * rating;
        }
        (length.is_finite() && length > 0.0 && thrust.is_finite()).then_some((length, thrust))
    };
    // As wide as the node, or narrower until it is a rocket's shape: its grain goes as the cube of
    // its width at that shape.
    let mut d = width.min(limits.diameter);
    let mut sized = None;
    for _ in 0..8 {
        let Some((length, thrust)) = solve(d) else {
            break;
        };
        sized = Some((d, length, thrust));
        if length >= 0.95 * ASPECT * d || d <= NARROWEST {
            break;
        }
        d = (d * (length / (ASPECT * d)).cbrt()).max(NARROWEST);
    }
    let Some((d, ..)) = sized else {
        return false;
    };
    // As a player would type them, and the length again for the width rounded.
    let d = if d < width.min(limits.diameter) {
        round_up(d, width_step(d)).min(width)
    } else {
        d
    };
    let Some((length, thrust)) = solve(d) else {
        return false;
    };
    let length = round(length, length_step(length)).min(limits.length);
    let step = if thrust < 100e3 { 1e3 } else { 5e3 };
    let thrust = round_up(thrust, step);
    let mut sized = part.clone();
    if set_both(&mut sized, d, length).is_err() {
        return false;
    }
    let Some(motor) = sized.components.iter_mut().find_map(|c| match c {
        ComponentDef::SolidMotor(motor) if motor.booster => Some(motor),
        _ => None,
    }) else {
        return false;
    };
    motor.thrust = thrust;
    motor.mass = BOOSTER_NOZZLE_MASS * thrust;
    *part = sized;
    true
}

/// Whether `engine` is one that fires from the ground: its Isp at sea level not far below its
/// vacuum Isp.
fn sea_level(engine: &EngineDef) -> bool {
    engine.isp_sl >= SEA_LEVEL * engine.isp_vac
}

/// Size `part`, a cylinder tank on `engine`'s mount (a node `width` across), to feed the engine at
/// full thrust for a stage's burn, holding no more than `most` kg, no bigger than `limits`; `keep`
/// as wide, if it is to stay so (it is as wide as what stands on it). False, and `part` as it was,
/// if it is no cylinder tank or cannot hold what the engine burns.
fn feed(
    part: &mut CraftPartDef,
    engine: &EngineDef,
    width: f64,
    keep: Option<f64>,
    most: f64,
    limits: Limits,
    catalog: &PartCatalog,
) -> bool {
    if !matches!(&part.shape, Some(ShapeDef::CylinderTank(_))) {
        return false;
    }
    let flow = engine.thrust_vac / (G0 * engine.isp_vac);
    if !(flow.is_finite() && flow > 0.0) {
        return false;
    }
    let burn = if sea_level(engine) {
        BOOSTER_BURN
    } else {
        UPPER_BURN
    };
    let wanted = (flow * burn).min(most);
    // Measured on the tank as built, filled with what the engine burns: its domes, walls and
    // ullage count. What it holds is `at_none` plus `per_metre` for each metre of length, at a
    // width of `d0`; at another width those go as its cube and square.
    let mut probe = part.clone();
    let mix = [Mix {
        id: "engine".into(),
        label: String::new(),
        contents: engine.propellants.clone(),
    }];
    if properties::set_contents(&mut probe, "engine", &mix, catalog).is_err() {
        return false;
    }
    let least = width.max(engine.exit_diameter);
    let d0 = keep.unwrap_or(least.min(limits.diameter));
    let (Some(short), Some(long)) = (
        holds(&probe, d0, ASPECT * d0, catalog),
        holds(&probe, d0, 2.0 * ASPECT * d0, catalog),
    ) else {
        return false;
    };
    let per_metre = (long - short) / (ASPECT * d0);
    if per_metre.is_nan() || per_metre <= 0.0 {
        return false;
    }
    let at_none = short - per_metre * ASPECT * d0;
    // At the usual length for its width, what it holds goes as the cube of the width.
    let diameter = match keep {
        Some(width) => width,
        None => {
            let diameter = (d0 * (wanted / short).cbrt())
                .max(least)
                .min(limits.diameter);
            round_up(diameter, width_step(diameter)).min(limits.diameter)
        }
    };
    let scale = diameter / d0;
    let length = (wanted - at_none * scale.powi(3)) / (per_metre * scale.powi(2));
    let length = round(length, length_step(length)).min(limits.length);
    set_both(part, diameter, length).is_ok()
}

/// Size the tanks that the engines `put` (just put on `craft`) hang from by their fronts for those
/// engines, as a tank put on an engine's mount is ([`fit`]), when the tank is still the size the
/// parts list gave it where it is ([`as_given`]): built top down, a capsule, a tank and then an
/// engine, the tank holds the engine's burn, or as much as leaves the engine lifting all the craft
/// ([`BOOSTER_TWR`], [`UPPER_TWR`]). It keeps its width if something stands on it, and is no bigger
/// than `limits`. A tank the parts list gave too heavy for the engine to lift is made smaller; one
/// the player has sized stays as it is. The ids of the tanks sized.
pub fn size_for_engines(
    craft: &mut CraftDef,
    catalog: &PartCatalog,
    put: &[String],
    limits: Limits,
) -> Vec<String> {
    if !craft.parts.iter().any(|p| put.contains(&p.id)) {
        return Vec::new();
    }
    let Ok(built) = Craft::build(craft, catalog) else {
        return Vec::new();
    };
    let joints = built.assembly.joints();
    let mut sized = Vec::new();
    for joint in joints {
        for (engine_end, tank_end) in [
            (&joint.ends[0], &joint.ends[1]),
            (&joint.ends[1], &joint.ends[0]),
        ] {
            let (Mount::Node(front), Mount::Node(_)) = (&engine_end.mount, &tank_end.mount) else {
                continue;
            };
            let (engine, tank) = (engine_end.part, tank_end.part);
            if !put.contains(&craft.parts[engine].id) || sized.contains(&craft.parts[tank].id) {
                continue;
            }
            let Some(fed) = feeding(&built, engine, front) else {
                continue;
            };
            let Some(ShapeDef::CylinderTank(shape)) = &craft.parts[tank].shape else {
                continue;
            };
            if !as_given(&built, &craft.parts[tank], catalog) {
                continue;
            }
            // As wide as what stands on it, if anything does: joined by another node.
            let stood_on = (joints.iter()).any(|j| {
                !std::ptr::eq(j, joint)
                    && j.ends
                        .iter()
                        .any(|e| e.part == tank && matches!(e.mount, Mount::Node(_)))
            });
            let keep = stood_on.then_some(shape.diameter);
            let Some(most) = liftable(craft, tank, fed, catalog) else {
                continue;
            };
            let mut tank_part = craft.parts[tank].clone();
            if feed(
                &mut tank_part,
                fed,
                shape.diameter,
                keep,
                most,
                limits,
                catalog,
            ) {
                sized.push(tank_part.id.clone());
                craft.parts[tank] = tank_part;
            }
        }
    }
    sized
}

/// The most propellant the tank at `tank` in `craft` may hold for `engine` under it to lift all the
/// craft at the least thrust to weight it is sized for, kg, more or less than it holds now; none if
/// the engine cannot lift the rest of the craft even with the tank empty.
fn liftable(
    craft: &CraftDef,
    tank: usize,
    engine: &EngineDef,
    catalog: &PartCatalog,
) -> Option<f64> {
    use crate::propulsion::Propulsion;
    let all = Propulsion::from_craft(craft, catalog).ok()?.mass();
    let alone = properties::alone(&craft.parts[tank], catalog).ok()?;
    let part = &alone.parts[0];
    let held: f64 = (part.built.components.iter())
        .filter_map(|c| match c {
            Component::Storage(s) => Some(s.contents.iter().map(|c| c.capacity).sum::<f64>()),
            _ => None,
        })
        .sum();
    let mut only = craft.parts[tank].clone();
    (only.on, only.at) = (None, None);
    let lone = CraftDef {
        id: "alone".into(),
        name: String::new(),
        parts: vec![only],
        sequences: None,
        triggers: Vec::new(),
        rcs_shut: Vec::new(),
        abort: Vec::new(),
    };
    let tank_mass = Propulsion::from_craft(&lone, catalog).ok()?.mass();
    if !(held > 0.0 && tank_mass > 0.0) {
        return None;
    }
    let (thrust, least) = if sea_level(engine) {
        (
            engine.thrust_vac * engine.isp_sl / engine.isp_vac,
            BOOSTER_TWR,
        )
    } else {
        (engine.thrust_vac, UPPER_TWR)
    };
    // What the rest weighs, and how much the tank weighs for each kg it holds.
    let rest = all - tank_mass;
    let most = (thrust / (G0 * least) - rest) * held / tank_mass;
    (most > 0.0).then_some(most)
}

/// Whether `tank` is still the size the parts list gave it where it is in `craft`: as [`fit`] makes
/// a new tank on the node that holds it, or as the list has it if it is the root.
fn as_given(craft: &Craft, tank: &CraftPartDef, catalog: &PartCatalog) -> bool {
    let size = |part: &CraftPartDef| match &part.shape {
        Some(ShapeDef::CylinderTank(d)) => Some((d.diameter, d.length)),
        _ => None,
    };
    let Some(entry) = (crate::palette::entries(catalog).into_iter()).find(|e| e.id == "tank")
    else {
        return false;
    };
    let mut given = entry.part;
    if let Some(on) = &tank.on {
        let Some((holder, node)) = on.split_once('.') else {
            return false;
        };
        let holder = craft.parts.iter().position(|p| p.id == holder);
        let width = holder
            .and_then(|h| craft.assembly.parts().get(h))
            .and_then(|p| p.node(node))
            .map(|n| n.size);
        let Some(width) = width.filter(|w| *w > 0.0) else {
            return false;
        };
        fit(&mut given, &Against::node(width), catalog);
    }
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-6 * a.abs().max(1.0);
    match (size(tank), size(&given)) {
        (Some((d, l)), Some((gd, gl))) => close(d, gd) && close(l, gl),
        _ => false,
    }
}

/// What `tank` holds when full, kg, made `diameter` across and `length` long. None if it cannot
/// be built so.
fn holds(tank: &CraftPartDef, diameter: f64, length: f64, catalog: &PartCatalog) -> Option<f64> {
    let mut tank = tank.clone();
    set_both(&mut tank, diameter, length).ok()?;
    let built = properties::alone(&tank, catalog).ok()?;
    let held = (built.parts[0].built.components.iter())
        .filter_map(|c| match c {
            Component::Storage(s) => Some(s.contents.iter().map(|c| c.capacity).sum::<f64>()),
            _ => None,
        })
        .sum::<f64>();
    (held > 0.0).then_some(held)
}

/// A cylinder tank `diameter` across and `length` long, or as near as its limits let it be: the
/// length again after the width, since how short it can be goes with how wide it is.
fn set_both(tank: &mut CraftPartDef, diameter: f64, length: f64) -> Result<(), String> {
    shaping::resize(tank, "length", length)?;
    shaping::resize(tank, "diameter", diameter)?;
    shaping::resize(tank, "length", length)
}

/// The step a width made up (not taken from a node) is rounded to, m: as a player would type it.
fn width_step(width: f64) -> f64 {
    match width {
        w if w < 2.0 => 0.05,
        w if w < 5.0 => 0.1,
        _ => 0.25,
    }
}

/// The step a length made up is rounded to, m.
fn length_step(length: f64) -> f64 {
    if length < 2.0 { 0.05 } else { 0.1 }
}

fn round(value: f64, step: f64) -> f64 {
    (value / step).round() * step
}

/// Up to the next step, leaving one already on a step (within rounding) as it is.
fn round_up(value: f64, step: f64) -> f64 {
    (value / step - 1e-6).ceil() * step
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{data::CraftDef, palette, shaping::part_sizes};

    fn catalog() -> PartCatalog {
        crate::vessel::tests::catalog()
    }

    fn entry(id: &str, catalog: &PartCatalog) -> CraftPartDef {
        let all = palette::entries(catalog);
        let mut part = (all.iter().find(|e| e.id == id))
            .unwrap_or_else(|| panic!("no entry {id}"))
            .part
            .clone();
        part.id = id.replace(':', "_");
        part
    }

    fn size(part: &CraftPartDef, name: &str) -> f64 {
        (part_sizes(part).iter())
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no size {name}"))
            .value
    }

    fn fitted(id: &str, against: &Against, catalog: &PartCatalog) -> CraftPartDef {
        let mut part = entry(id, catalog);
        fit(&mut part, against, catalog);
        part
    }

    /// An engine of `catalog`'s, built by itself, and the engine record its mount feeds.
    fn engine_alone(id: &str, catalog: &PartCatalog) -> Craft {
        let mut engine = entry(&format!("engine:{id}"), catalog);
        engine.id = "engine".into();
        let def = CraftDef {
            id: "test".into(),
            name: String::new(),
            parts: vec![engine],
            sequences: None,
            triggers: Vec::new(),
            rcs_shut: Vec::new(),
            abort: Vec::new(),
        };
        Craft::build(&def, catalog).unwrap()
    }

    /// What a tank holds of what `engine` burns, kg.
    fn held_for(tank: &CraftPartDef, engine: &EngineDef, catalog: &PartCatalog) -> f64 {
        let mut probe = tank.clone();
        let mix = [Mix {
            id: "engine".into(),
            label: String::new(),
            contents: engine.propellants.clone(),
        }];
        properties::set_contents(&mut probe, "engine", &mix, catalog).unwrap();
        let built = properties::alone(&probe, catalog).unwrap();
        (built.parts[0].built.components.iter())
            .filter_map(|c| match c {
                Component::Storage(s) => Some(s.contents.iter().map(|c| c.capacity).sum::<f64>()),
                _ => None,
            })
            .sum()
    }

    /// A structural tube put on a 1.25 m tank is a 1.25 m tube, not an adapter out to the width
    /// it came with.
    #[test]
    fn a_tube_is_as_wide_at_both_ends() {
        let catalog = catalog();
        let tube = fitted("structural", &Against::node(1.25), &catalog);
        assert_eq!(size(&tube, "bottom_diameter"), 1.25);
        assert_eq!(size(&tube, "top_diameter"), 1.25);
    }

    /// A tank put on a K-0's mount (a node as narrow as its nozzle) is not 0.3 m across: it holds
    /// what the K-0 burns in a first stage's burn, and is at least as wide as the nozzle.
    #[test]
    fn a_tank_on_an_engine_feeds_it_for_a_stage() {
        let catalog = catalog();
        for (id, burn) in [
            ("K-0", BOOSTER_BURN),
            ("K-1 Booster", BOOSTER_BURN),
            ("H-10", UPPER_BURN),
        ] {
            let alone = engine_alone(id, &catalog);
            let engine = feeding(&alone, 0, "top").unwrap_or_else(|| panic!("{id} is fed"));
            let node = alone.assembly.parts()[0].node("top").unwrap().size;
            let at = Against {
                engine: Some(engine),
                ..Against::node(node)
            };
            let tank = fitted("tank", &at, &catalog);
            let diameter = size(&tank, "diameter");
            let length = size(&tank, "length");
            assert!(diameter >= engine.exit_diameter - 1e-9, "{id}: {diameter}");
            // Neither a pancake nor a needle.
            assert!(
                (1.0..8.0).contains(&(length / diameter)),
                "{id}: {diameter} x {length}"
            );
            let wanted = engine.thrust_vac / (G0 * engine.isp_vac) * burn;
            let held = held_for(&tank, engine, &catalog);
            assert!(
                (held / wanted - 1.0).abs() < 0.1,
                "{id}: holds {held:.0} kg for {wanted:.0} ({diameter} x {length})"
            );
        }
    }

    /// In a career that allows no more than 1 m across, a tank on a big engine is 1 m across and
    /// longer instead, and no longer than allowed.
    #[test]
    fn a_tank_for_an_engine_keeps_to_the_limits() {
        let catalog = catalog();
        let alone = engine_alone("K-1 Booster", &catalog);
        let engine = feeding(&alone, 0, "top").unwrap();
        let limits = Limits {
            diameter: 1.0,
            length: 12.0,
        };
        let at = Against {
            engine: Some(engine),
            limits,
            ..Against::node(0.9)
        };
        let tank = fitted("tank", &at, &catalog);
        assert_eq!(size(&tank, "diameter"), 1.0);
        assert_eq!(size(&tank, "length"), 12.0);
    }

    /// The bottom of an engine is no mount, and a solid motor feeds itself.
    #[test]
    fn only_a_liquid_engine_s_front_is_fed() {
        let catalog = catalog();
        let k0 = engine_alone("K-0", &catalog);
        let bottom = (k0.assembly.parts()[0].nodes()).find(|n| n.direction.z < -0.5);
        if let Some(bottom) = bottom {
            assert!(feeding(&k0, 0, &bottom.name).is_none());
        }
        let solid = engine_alone("S-1", &catalog);
        assert!(feeding(&solid, 0, "top").is_none());
    }
}
