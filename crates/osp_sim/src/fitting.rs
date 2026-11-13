//! How a new part from the parts list is sized for where it goes on, as KSP's procedural parts
//! take the size of what they join. A tank, tube or decoupler put on a node is as wide as the node,
//! and a nose cone's base as wide. One put on a hull's side, a booster beside its core, is half as
//! wide as the hull there. A fin or wing is sized for the hull it stands on. A tank put on an
//! engine's mount is sized to feed that engine through a stage's burn: as wide as the nozzle at
//! least, and holding what the engine burns in [`BOOSTER_BURN`] at sea level or [`UPPER_BURN`] in
//! vacuum, measured on the tank as built, filled with the engine's propellants. A part from a
//! catalogue record (a pod, an engine) stays as it is, and so does a part being moved.

use crate::{
    component::{Component, ComponentDef},
    constants::G0,
    craft::Craft,
    data::{CraftPartDef, EngineDef, PartCatalog},
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
/// A tank's length for its width, when nothing else sets it.
const ASPECT: f64 = 3.0;
/// How wide a part put on a hull's side is, for the hull's width: a booster beside its core.
const BESIDE: f64 = 0.5;

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
    let width = against.width;
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
        return;
    }
    // A fuel line is a small fitting, however big the tank it is on.
    if (part.components.iter()).any(|c| matches!(c, ComponentDef::FuelLine(_))) {
        return;
    }
    if part.template.is_some() || part.procedural.is_some() {
        return;
    }
    if let Some(engine) = against.engine
        && feed(part, engine, width, against.limits, catalog)
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
        // On a hull `width` across: big enough to keep a short rocket pointing into the air.
        ShapeDef::Wing(d) if matches!(d.airfoil, AirfoilDef::Plate { .. }) => vec![
            ("root_chord", 1.2 * width),
            ("tip_chord", 0.5 * width),
            ("span", 0.8 * width),
            ("sweep", 0.6 * width),
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

/// Size `part`, a cylinder tank on `engine`'s mount (a node `width` across), to feed the engine at
/// full thrust for a stage's burn, no bigger than `limits`. False, and `part` as it was, if it is
/// no cylinder tank or cannot hold what the engine burns.
fn feed(
    part: &mut CraftPartDef,
    engine: &EngineDef,
    width: f64,
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
    let burn = if engine.isp_sl >= SEA_LEVEL * engine.isp_vac {
        BOOSTER_BURN
    } else {
        UPPER_BURN
    };
    let wanted = flow * burn;
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
    let d0 = least.min(limits.diameter);
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
    let diameter = (d0 * (wanted / short).cbrt())
        .max(least)
        .min(limits.diameter);
    let diameter = round_up(diameter, width_step(diameter)).min(limits.diameter);
    let scale = diameter / d0;
    let length = (wanted - at_none * scale.powi(3)) / (per_metre * scale.powi(2));
    let length = round(length, length_step(length)).min(limits.length);
    set_both(part, diameter, length).is_ok()
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
    use crate::{
        data::CraftDef,
        palette::{self, Group},
        shaping::part_sizes,
    };

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

    #[test]
    fn a_procedural_part_takes_the_width_it_goes_on() {
        let catalog = catalog();
        let at = Against::node(3.0);
        let tank = fitted("tank", &at, &catalog);
        assert_eq!((size(&tank, "length"), size(&tank, "diameter")), (9.0, 3.0));
        let nose = fitted("nose_cone", &at, &catalog);
        assert_eq!(size(&nose, "bottom_diameter"), 3.0);
        let decoupler = fitted("decoupler", &at, &catalog);
        assert!(
            matches!(&decoupler.procedural, Some(ProceduralDef::Decoupler(d)) if d.diameter == 3.0)
        );
        let fin = fitted("fin", &Against::surface(2.0), &catalog);
        assert_eq!((size(&fin, "span"), size(&fin, "root_chord")), (1.6, 2.4));
        // A catalogue part stays as it was.
        let all = palette::entries(&catalog);
        let engine = all.iter().find(|e| e.group == Group::Engines).unwrap();
        let mut copy = engine.part.clone();
        fit(&mut copy, &Against::node(9.0), &catalog);
        assert_eq!(copy, engine.part);
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

    /// Put on a 2 m hull's side, a tank, a decoupler and a tube are half as wide: a booster beside
    /// its core. A fin there is still sized for the whole hull.
    #[test]
    fn a_part_beside_a_hull_is_half_as_wide() {
        let catalog = catalog();
        let side = Against::surface(2.0);
        let tank = fitted("tank", &side, &catalog);
        assert_eq!((size(&tank, "diameter"), size(&tank, "length")), (1.0, 3.0));
        let decoupler = fitted("decoupler", &side, &catalog);
        assert!(
            matches!(&decoupler.procedural, Some(ProceduralDef::Decoupler(d)) if d.diameter == 1.0)
        );
        let tube = fitted("structural", &side, &catalog);
        assert_eq!(size(&tube, "bottom_diameter"), 1.0);
        let fin = fitted("fin", &side, &catalog);
        assert_eq!(size(&fin, "root_chord"), 2.4);
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
