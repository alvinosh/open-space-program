//! Everything a player can set on a part in the editor's part window, for every kind of part with
//! no editor code for any one of them, as in KSP with its procedural parts: [`fields`] lists them
//! in sections, each with its value and what it can be; [`set`] sets one from what the player
//! typed or chose; [`readouts`] says what the part comes to (what it holds and weighs).
//!
//! A procedural tank can be a cylinder, a cone or a sphere, with its sizes, domes and skirts, what
//! it holds, its tank type, the share of it that holds propellant and the load it is built for. A
//! shell is a tube, a taper or a point, with its profile and caps. A wing has its chords, span,
//! sweep, dihedral and twist, and its airfoil. A part's material and wall, and every number a
//! sizer asks for (a decoupler's load), are there too. Values are in the units the player reads:
//! metres, millimetres, percent, degrees, g.
//!
//! What a tank can hold is what the engines it feeds burn, each at its engine's ratio
//! ([`mixes`]); one that feeds no engine can only be emptied. A tank still as the parts list gave
//! it follows the engines it feeds when it or an engine goes on ([`fill_from_engines`]).

use std::collections::BTreeMap;

use serde_json::{Value as Json, json};

use crate::{
    component::{
        BOOSTER_ISP, BOOSTER_NOZZLE_MASS, CURVE_PRESETS, ComponentDef, SolidMotorDef,
        storage::StorageDef,
    },
    craft::Craft,
    data::{CraftDef, CraftPartDef, PartCatalog, ShellProfile},
    geometry::preset::{AirfoilDef, ConeTankDef, CylinderTankDef, ShapeDef, SphereDef},
    shaping,
};

/// What a field's number is in, as the player reads and types it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Metres,
    Millimetres,
    Percent,
    Degrees,
    G,
    /// A plain number, such as a dome's height over its radius.
    Ratio,
    /// A thrust, kN.
    Kilonewtons,
    /// What a sizer's field says it is in.
    Other(&'static str),
}

impl Unit {
    /// What follows the number.
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Metres => "m",
            Unit::Millimetres => "mm",
            Unit::Percent => "%",
            Unit::Degrees => "°",
            Unit::G => "g",
            Unit::Ratio => "",
            Unit::Kilonewtons => "kN",
            Unit::Other(unit) => unit,
        }
    }
}

/// What a field holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Number {
        value: f64,
        least: f64,
        most: f64,
        unit: Unit,
    },
    /// One of `options`, each `(id, label)`.
    Choice {
        value: String,
        options: Vec<(String, String)>,
    },
    Toggle(bool),
}

/// One thing that can be set.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// What [`set`] knows it by.
    pub id: String,
    pub label: String,
    /// The heading it is under.
    pub section: &'static str,
    pub value: Value,
}

fn field(id: &str, label: &str, section: &'static str, value: Value) -> Field {
    Field {
        id: id.into(),
        label: label.into(),
        section,
        value,
    }
}

fn number(value: f64, least: f64, most: f64, unit: Unit) -> Value {
    Value::Number {
        value,
        least,
        most,
        unit,
    }
}

fn choice(value: &str, options: &[(&str, &str)]) -> Value {
    Value::Choice {
        value: value.into(),
        options: (options.iter())
            .map(|(id, label)| ((*id).into(), (*label).into()))
            .collect(),
    }
}

/// The mixes the game has names for: an id, what the player reads, and the mass ratio. They name
/// what an engine burns when it burns the same resources. The first that the mods loaded have all
/// the resources of is what a new tank holds ([`new_tank`]).
const MIXES: &[(&str, &str, &[(&str, f64)])] = &[
    (
        "kerolox",
        "Kerosene and liquid oxygen",
        &[("Kerosene", 1.0), ("LqdOxygen", 2.36)],
    ),
    (
        "rp1_lox",
        "RP-1 and liquid oxygen",
        &[("RP-1", 1.0), ("LqdOxygen", 2.36)],
    ),
    (
        "hydrolox",
        "Liquid hydrogen and oxygen",
        &[("LqdHydrogen", 1.0), ("LqdOxygen", 5.5)],
    ),
    (
        "methalox",
        "Methane and liquid oxygen",
        &[("LqdMethane", 1.0), ("LqdOxygen", 3.6)],
    ),
    (
        "aerozine_nto",
        "Aerozine 50 and NTO",
        &[("Aerozine50", 1.0), ("NTO", 1.6)],
    ),
    ("mmh_nto", "MMH and NTO", &[("MMH", 1.0), ("NTO", 1.65)]),
    ("udmh_nto", "UDMH and NTO", &[("UDMH", 1.0), ("NTO", 2.6)]),
    (
        "liquid_fuel",
        "Liquid fuel and oxidiser",
        &[("Liquid Fuel", 1.0), ("Oxidiser", 2.36)],
    ),
    ("kerosene", "Kerosene", &[("Kerosene", 1.0)]),
    ("lox", "Liquid oxygen", &[("LqdOxygen", 1.0)]),
    ("lh2", "Liquid hydrogen", &[("LqdHydrogen", 1.0)]),
    ("hydrazine", "Hydrazine", &[("Hydrazine", 1.0)]),
    ("nitrogen", "Nitrogen", &[("Nitrogen", 1.0)]),
    ("helium", "Helium", &[("Helium", 1.0)]),
];

/// What a tank holding nothing is called: it is then structure.
const NOTHING: &str = "none";
/// What contents none of the mixes offered match are called.
const CUSTOM: &str = "custom";

/// What a tank can be filled with: what [`set`] knows it by, what the player reads, and the
/// resources in it with their ratio by mass.
#[derive(Debug, Clone, PartialEq)]
pub struct Mix {
    pub id: String,
    pub label: String,
    pub contents: BTreeMap<String, f64>,
}

/// What a tank from the parts list holds (`palette`'s tank): the first of [`MIXES`] that `catalog`
/// has, or nothing if it has none. A tank that still holds it follows the engines it feeds
/// ([`fill_from_engines`]); one the player filled keeps what it has.
pub fn new_tank(catalog: &PartCatalog) -> Mix {
    usual(catalog).into_iter().next().unwrap_or(Mix {
        id: NOTHING.to_owned(),
        label: "Nothing".to_owned(),
        contents: BTreeMap::new(),
    })
}

/// What tank `part` of `craft` can be filled with: what the engines and thrusters it feeds burn,
/// those joined to it through joints that pass propellant ([`fed_from`]), each mix once and at its
/// engine's own ratio, in the order the craft lists them, its label naming the engines that burn
/// it. The same resources burnt at two ratios are two mixes, the second called `<id>_2`. A tank that
/// feeds nothing has nothing to be filled with: it can only be emptied.
pub fn mixes(craft: &CraftDef, catalog: &PartCatalog, part: &str) -> Vec<Mix> {
    let Some(index) = craft.parts.iter().position(|p| p.id == part) else {
        return Vec::new();
    };
    let Ok(built) = Craft::build(craft, catalog) else {
        return Vec::new();
    };
    let known = |contents: &BTreeMap<String, f64>| {
        (contents.keys()).all(|name| catalog.resources.iter().any(|r| &r.id == name))
    };
    // Each mix with the names of what burns it.
    let mut found: Vec<(BTreeMap<String, f64>, Vec<String>)> = Vec::new();
    for at in fed_from(&built, index) {
        let part = &craft.parts[at];
        for (contents, _) in burnt_by(part, catalog) {
            if !known(&contents) {
                continue;
            }
            let name = engine_name(part);
            match found.iter_mut().find(|(c, _)| same_mix(c, &contents)) {
                Some((_, names)) if names.contains(&name) => {}
                Some((_, names)) => names.push(name),
                None => found.push((contents, vec![name])),
            }
        }
    }
    let mut out: Vec<Mix> = Vec::new();
    for (contents, names) in found {
        let (base, ..) = names_of(&contents);
        let before = (out.iter())
            .filter(|m| m.contents.keys().eq(contents.keys()))
            .count();
        let id = match before {
            0 => base,
            n => format!("{base}_{}", n + 1),
        };
        out.push(Mix {
            id,
            label: format!("{} ({})", label_of(&contents), names.join(", ")),
            contents,
        });
    }
    out
}

/// The parts of `craft` joined to part `index` through joints that pass propellant, itself among
/// them, in the order the craft lists them: the engines a tank feeds and the tanks an engine draws
/// on are among them.
fn fed_from(craft: &Craft, index: usize) -> Vec<usize> {
    let joints = craft.assembly.joints();
    let mut seen = std::collections::BTreeSet::from([index]);
    let mut open = vec![index];
    while let Some(at) = open.pop() {
        for (j, joint) in joints.iter().enumerate() {
            let [a, b] = &joint.ends;
            let next = if a.part == at {
                b.part
            } else if b.part == at {
                a.part
            } else {
                continue;
            };
            if craft.assembly.carries(j).fluid && seen.insert(next) {
                open.push(next);
            }
        }
    }
    seen.into_iter().collect()
}

/// What the player calls an engine part: its name, or what it was made from.
fn engine_name(part: &CraftPartDef) -> String {
    (part.name.clone())
        .or_else(|| part.template.clone())
        .unwrap_or_else(|| part.id.clone())
}

/// Fill the tanks that the parts `put`, just put on `craft`, are joined to through joints that pass
/// propellant: a tank that still holds what a new one comes with ([`new_tank`]) takes what the
/// engines it feeds burn, at their ratio, an engine's mix before thrusters'. So a tank put on an
/// engine, and an engine put under a tank, need nothing set. A tank the player filled stays as it
/// is, and so does one that feeds nothing. The ids of the tanks filled.
pub fn fill_from_engines(
    craft: &mut CraftDef,
    catalog: &PartCatalog,
    put: &[String],
) -> Vec<String> {
    let concerned =
        |part: &CraftPartDef| storage(part).is_some() || !burnt_by(part, catalog).is_empty();
    let starts: Vec<usize> = (craft.parts.iter().enumerate())
        .filter(|(_, p)| put.contains(&p.id) && concerned(p))
        .map(|(i, _)| i)
        .collect();
    if starts.is_empty() {
        return Vec::new();
    }
    let Ok(built) = Craft::build(craft, catalog) else {
        return Vec::new();
    };
    let new_tank = new_tank(catalog).contents;
    let mut seen = std::collections::BTreeSet::new();
    let mut filled = Vec::new();
    for start in starts {
        if seen.contains(&start) {
            continue;
        }
        let group = fed_from(&built, start);
        seen.extend(group.iter().copied());
        let burnt: Vec<(BTreeMap<String, f64>, bool)> = (group.iter())
            .flat_map(|&i| burnt_by(&craft.parts[i], catalog))
            .collect();
        let wanted = (burnt.iter().find(|(_, thrusters)| !thrusters))
            .or(burnt.first())
            .map(|(contents, _)| contents.clone());
        let Some(wanted) = wanted.filter(|w| !same_mix(w, &new_tank)) else {
            continue;
        };
        let mix = Mix {
            id: "engines".into(),
            label: String::new(),
            contents: wanted,
        };
        for &i in &group {
            let part = &mut craft.parts[i];
            if !storage(part).is_some_and(|s| same_mix(&s.contents, &new_tank)) {
                continue;
            }
            let mut tank = part.clone();
            if set_contents(&mut tank, &mix.id, std::slice::from_ref(&mix), catalog).is_ok() {
                filled.push(tank.id.clone());
                *part = tank;
            }
        }
    }
    filled
}

/// The usual mixes: those of [`MIXES`] `catalog` has every resource of.
fn usual(catalog: &PartCatalog) -> Vec<Mix> {
    (MIXES.iter())
        .filter(|(_, _, parts)| {
            (parts.iter()).all(|(name, _)| catalog.resources.iter().any(|r| r.id == *name))
        })
        .map(|(id, label, parts)| Mix {
            id: (*id).to_owned(),
            label: (*label).to_owned(),
            contents: (parts.iter())
                .map(|(name, ratio)| ((*name).to_owned(), *ratio))
                .collect(),
        })
        .collect()
}

/// What `part` burns, each as its record has it and whether it is thrusters' (RCS): a template's,
/// those among its components, and those a sizer makes. A sized part that holds everything its
/// engine burns (a separation motor and its grain) draws on no tank, and is left out.
fn burnt_by(part: &CraftPartDef, catalog: &PartCatalog) -> Vec<(BTreeMap<String, f64>, bool)> {
    let mut out = Vec::new();
    if let Some(id) = &part.template {
        let engine = (catalog.engines.iter()).find(|e| &e.id == id);
        let rcs = (catalog.rcs.iter()).find(|r| &r.id == id);
        out.extend(engine.map(|e| (e.propellants.clone(), false)));
        out.extend(rcs.map(|r| (r.propellants.clone(), true)));
    }
    out.extend(part.components.iter().filter_map(|c| burns(c, catalog)));
    let design = (part.procedural.as_ref()).and_then(|p| p.size(catalog).ok());
    if let Some(design) = design {
        let held: Vec<&String> = (design.components.iter())
            .filter_map(|c| match c {
                ComponentDef::Storage(s) => Some(s.contents.keys()),
                _ => None,
            })
            .flatten()
            .collect();
        out.extend(
            (design.components.iter())
                .filter_map(|c| burns(c, catalog))
                .filter(|(burns, _)| !burns.keys().all(|name| held.contains(&name))),
        );
    }
    out
}

/// What `component` burns, if it is an engine or thrusters, and whether it is thrusters.
fn burns(component: &ComponentDef, catalog: &PartCatalog) -> Option<(BTreeMap<String, f64>, bool)> {
    match component {
        ComponentDef::Engine(r) => (catalog.engines.iter())
            .find(|e| e.id == r.id)
            .map(|e| (e.propellants.clone(), false)),
        ComponentDef::Rcs(r) => (catalog.rcs.iter())
            .find(|b| b.id == r.id)
            .map(|b| (b.propellants.clone(), true)),
        ComponentDef::SolidMotor(motor) => Some(([(motor.propellant.clone(), 1.0)].into(), false)),
        _ => None,
    }
}

/// Whether `a` and `b` are the same resources in the same shares by mass.
fn same_mix(a: &BTreeMap<String, f64>, b: &BTreeMap<String, f64>) -> bool {
    let (total_a, total_b): (f64, f64) = (a.values().sum(), b.values().sum());
    a.len() == b.len()
        && (a.iter()).all(|(name, ratio)| {
            (b.get(name)).is_some_and(|r| (ratio / total_a - r / total_b).abs() < 1e-3)
        })
}

/// Whether a mix of [`MIXES`] is of the resources `contents` is, in whatever ratio.
fn same_resources(parts: &[(&str, f64)], contents: &BTreeMap<String, f64>) -> bool {
    parts.len() == contents.len() && (parts.iter()).all(|(name, _)| contents.contains_key(*name))
}

/// What `contents` is called: its id, what the player reads, and the order its resources are
/// named in. As [`MIXES`] calls the same resources, or by their ids from the least of it by mass to
/// the most, which puts a fuel before its oxidiser.
fn names_of(contents: &BTreeMap<String, f64>) -> (String, String, Vec<&str>) {
    match (MIXES.iter()).find(|(_, _, parts)| same_resources(parts, contents)) {
        Some((id, label, parts)) => (
            (*id).to_owned(),
            (*label).to_owned(),
            parts.iter().map(|p| p.0).collect(),
        ),
        None => {
            let mut order: Vec<&str> = contents.keys().map(String::as_str).collect();
            order.sort_by(|a, b| contents[*a].total_cmp(&contents[*b]));
            (order.join("+"), order.join(" and "), order)
        }
    }
}

/// What the player reads for `contents`: its name (see [`names_of`]), then the ratio by mass of
/// its resources when there is more than one (`Kerosene and liquid oxygen, 1 : 2.27`).
fn label_of(contents: &BTreeMap<String, f64>) -> String {
    let (_, name, order) = names_of(contents);
    let first = order.first().and_then(|n| contents.get(*n)).copied();
    match first {
        Some(first) if order.len() > 1 && first > 0.0 => {
            let ratio: Vec<String> = (order.iter())
                .map(|n| short(contents.get(*n).copied().unwrap_or_default() / first))
                .collect();
            format!("{name}, {}", ratio.join(" : "))
        }
        _ => name,
    }
}

/// `value` to two places, without the zeros at its end: `2.27`, `5.5`, `1`.
fn short(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The shape a part is drawn as, when it is a procedural one.
fn own_shape(part: &CraftPartDef) -> Option<&ShapeDef> {
    let alone = part.template.is_none() && part.procedural.is_none() && part.model.is_none();
    part.shape.as_ref().filter(|_| alone).map(unmirrored)
}

fn unmirrored(shape: &ShapeDef) -> &ShapeDef {
    match shape {
        ShapeDef::Mirrored(inner) => unmirrored(inner),
        other => other,
    }
}

fn unmirrored_mut(shape: &mut ShapeDef) -> &mut ShapeDef {
    match shape {
        ShapeDef::Mirrored(inner) => unmirrored_mut(inner),
        other => other,
    }
}

fn storage(part: &CraftPartDef) -> Option<&StorageDef> {
    part.components.iter().find_map(|c| match c {
        ComponentDef::Storage(s) => Some(s),
        _ => None,
    })
}

fn storage_mut(part: &mut CraftPartDef) -> Option<&mut StorageDef> {
    part.components.iter_mut().find_map(|c| match c {
        ComponentDef::Storage(s) => Some(s),
        _ => None,
    })
}

/// A shape that has room inside it to hold something.
fn holds_things(shape: &ShapeDef) -> bool {
    matches!(
        shape,
        ShapeDef::CylinderTank(_)
            | ShapeDef::ConeTank(_)
            | ShapeDef::Sphere(_)
            | ShapeDef::Torus(_)
    )
}

const PROFILES: &[(&str, &str)] = &[
    ("conic", "Straight"),
    ("ogive", "Ogive"),
    ("elliptical", "Elliptical"),
    ("parabolic", "Parabolic"),
    ("haack", "Von Kármán"),
];

fn profile_id(profile: ShellProfile) -> &'static str {
    match profile {
        ShellProfile::Conic => "conic",
        ShellProfile::Ogive => "ogive",
        ShellProfile::Elliptical => "elliptical",
        ShellProfile::Parabolic => "parabolic",
        ShellProfile::Haack => "haack",
    }
}

/// Which of its family's configurations an engine is, if it is one of several: each with what sets
/// it apart, its thrust, Isp, propellants, mass and year.
fn configuration(part: &CraftPartDef, catalog: &PartCatalog) -> Option<Field> {
    let engine = part.template.as_deref()?;
    let configs = catalog.configurations(engine);
    if configs.len() < 2 {
        return None;
    }
    let options = (configs.iter())
        .map(|c| {
            let title = (c.catalog.title.clone()).filter(|t| !t.starts_with('#'));
            let burns: Vec<&str> = c.propellants.keys().map(String::as_str).collect();
            let mut label = format!(
                "{} · {:.0} kN · Isp {:.0} s · {} · {:.0} kg",
                title.unwrap_or_else(|| c.id.clone()),
                c.thrust_vac / 1e3,
                c.isp_vac,
                burns.join("/"),
                c.mass
            );
            if let Some(year) = c.catalog.year {
                label.push_str(&format!(" · {year}"));
            }
            (c.id.clone(), label)
        })
        .collect();
    Some(field(
        "configuration",
        "Configuration",
        "Engine",
        Value::Choice {
            value: engine.to_owned(),
            options,
        },
    ))
}

/// Everything that can be set on `part`, in the order the window shows it. Its tank can hold one of
/// `mixes` (those of its craft, see [`mixes`]) or nothing; what it holds now stays offered, as
/// `custom`, if it is none of them.
pub fn fields(part: &CraftPartDef, mixes: &[Mix], catalog: &PartCatalog) -> Vec<Field> {
    let name = part.name.clone().unwrap_or_else(|| part.id.clone());
    let mut out = vec![field("name", "Name", "Part", Value::Text(name))];
    if let Some(field) = configuration(part, catalog) {
        out.push(field);
    }
    let shape = own_shape(part);
    if let Some(shape) = shape {
        shape_fields(shape, &mut out);
    }
    // A solid booster's motor: how hard it pushes (and so how long it burns), its nozzle, and where
    // the rules let it be set, the shape its grain is cast in.
    if let Some(motor) = booster(part) {
        out.push(field(
            "motor_thrust",
            "Thrust",
            "Motor",
            number(motor.thrust / 1000.0, 5.0, 20_000.0, Unit::Kilonewtons),
        ));
        out.push(field(
            "motor_nozzle",
            "Nozzle",
            "Motor",
            choice(
                if motor.vacuum { "vacuum" } else { "sea_level" },
                &[("sea_level", "For sea level"), ("vacuum", "For a vacuum")],
            ),
        ));
        if catalog.rules.shaped_solids {
            let mut options: Vec<(&str, &str)> = (CURVE_PRESETS.iter())
                .map(|(id, label, _)| (*id, *label))
                .collect();
            let preset = motor.curve_preset().unwrap_or(CUSTOM);
            if preset == CUSTOM {
                options.push((CUSTOM, "Your own"));
            }
            out.push(field(
                "motor_curve",
                "Burn",
                "Motor",
                choice(preset, &options),
            ));
            let labels = [
                "Thrust at first",
                "A quarter burnt",
                "Half burnt",
                "Three quarters",
                "At the end",
            ];
            for (k, (label, share)) in labels.iter().zip(motor.curve_points()).enumerate() {
                out.push(field(
                    &format!("motor_curve_{k}"),
                    label,
                    "Motor",
                    number(100.0 * share, 0.0, 150.0, Unit::Percent),
                ));
            }
        }
    }
    // What it holds, and how its tank is built.
    if shape.is_some_and(holds_things) || storage(part).is_some() {
        let mut options: Vec<(String, String)> = (mixes.iter())
            .map(|m| (m.id.clone(), m.label.clone()))
            .collect();
        let value = match storage(part) {
            None => NOTHING.to_owned(),
            Some(s) => match mixes.iter().find(|m| same_mix(&m.contents, &s.contents)) {
                Some(mix) => mix.id.clone(),
                None => {
                    options.push((CUSTOM.into(), label_of(&s.contents)));
                    CUSTOM.to_owned()
                }
            },
        };
        options.push((NOTHING.into(), "Nothing: structure".into()));
        out.push(field(
            "contents",
            "Holds",
            "Tank",
            Value::Choice { value, options },
        ));
    }
    if let Some(s) = storage(part) {
        // The tank types that can hold what it holds, and the one it is.
        let holds = |kind: &crate::data::TankTypeDef| {
            s.contents.keys().all(|name| {
                (catalog.resources.iter().find(|r| &r.id == name))
                    .is_some_and(|r| crate::component::storage::cannot_hold(kind, r).is_none())
            })
        };
        let types: Vec<(String, String)> = (catalog.tank_types.iter())
            .filter(|t| t.id == s.tank_type || holds(t))
            .map(|t| (t.id.clone(), t.id.clone()))
            .collect();
        out.push(field(
            "tank_type",
            "Tank type",
            "Tank",
            Value::Choice {
                value: s.tank_type.clone(),
                options: types,
            },
        ));
        let kind = catalog.tank_types.iter().find(|t| t.id == s.tank_type);
        let utilization = s.utilization.or(kind.map(|k| k.utilization)).unwrap_or(1.0);
        let load = s.design_load.or(kind.map(|k| k.design_load)).unwrap_or(1.0);
        out.push(field(
            "utilization",
            "Utilization",
            "Tank",
            number(100.0 * utilization, 1.0, 100.0, Unit::Percent),
        ));
        out.push(field(
            "design_load",
            "Built for",
            "Tank",
            number(load, 0.5, 30.0, Unit::G),
        ));
    }
    // What it is made of, for what is not a tank's own wall.
    if shape.is_some() && storage(part).is_none() {
        let materials: Vec<(String, String)> = (catalog.materials.iter())
            .map(|m| (m.id.clone(), m.id.clone()))
            .collect();
        let value = (part.structure.material.clone())
            .or_else(|| materials.first().map(|m| m.0.clone()))
            .unwrap_or_default();
        out.push(field(
            "material",
            "Material",
            "Structure",
            Value::Choice {
                value,
                options: materials,
            },
        ));
        if !matches!(shape, Some(ShapeDef::Wing(_))) {
            let wall = part.structure.thickness.unwrap_or(0.004);
            out.push(field(
                "wall",
                "Wall",
                "Structure",
                number(1000.0 * wall, 0.2, 200.0, Unit::Millimetres),
            ));
        }
    }
    // How it sits on what holds it.
    if part.on.is_some() {
        out.push(field(
            "roll",
            "Turned",
            "Placement",
            number(part.roll, -180.0, 180.0, Unit::Degrees),
        ));
    }
    // What a sizer asks for.
    if let Some(procedural) = &part.procedural {
        let asked = requirement(procedural);
        for f in procedural.fields() {
            let value = asked.get(f.id).and_then(Json::as_f64).unwrap_or(f.default);
            out.push(field(
                f.id,
                f.label,
                "Requirement",
                number(value, f.range[0], f.range[1], unit_of(f.unit)),
            ));
        }
    }
    out
}

fn unit_of(unit: &'static str) -> Unit {
    match unit {
        "m" => Unit::Metres,
        "mm" => Unit::Millimetres,
        "%" => Unit::Percent,
        "g" => Unit::G,
        other => Unit::Other(other),
    }
}

/// A sizer's requirement as its fields: `{ "diameter": 2, ... }`.
fn requirement(procedural: &crate::sizing::ProceduralDef) -> serde_json::Map<String, Json> {
    match serde_json::to_value(procedural) {
        Ok(Json::Object(outer)) => (outer.into_iter().next())
            .and_then(|(_, inner)| match inner {
                Json::Object(fields) => Some(fields),
                _ => None,
            })
            .unwrap_or_default(),
        _ => Default::default(),
    }
}

fn shape_fields(shape: &ShapeDef, out: &mut Vec<Field>) {
    const SHAPE: &str = "Shape";
    match shape {
        ShapeDef::CylinderTank(_) | ShapeDef::ConeTank(_) | ShapeDef::Sphere(_) => {
            let value = match shape {
                ShapeDef::CylinderTank(_) => "cylinder",
                ShapeDef::ConeTank(_) => "cone",
                _ => "sphere",
            };
            let options = [
                ("cylinder", "Cylinder"),
                ("cone", "Cone"),
                ("sphere", "Sphere"),
            ];
            out.push(field("shape", "Shape", SHAPE, choice(value, &options)));
        }
        ShapeDef::Shell(d) if d.span.is_none() => {
            let top = (d.sections.last()).map(|s| s.top_diameter).unwrap_or(None);
            let value = match top {
                None => "tube",
                Some(t) if t <= 0.0 => "point",
                Some(_) => "taper",
            };
            let options = [("tube", "Tube"), ("taper", "Taper"), ("point", "Point")];
            out.push(field(
                "shell_shape",
                "Shape",
                SHAPE,
                choice(value, &options),
            ));
        }
        _ => {}
    }
    for size in shaping::sizes(shape) {
        let unit = Unit::Metres;
        out.push(field(
            size.name,
            size.label,
            SHAPE,
            number(size.value, size.least, size.most, unit),
        ));
    }
    match shape {
        ShapeDef::CylinderTank(d) => {
            let most_dome = (d.length / d.diameter).min(2.0);
            out.push(field(
                "dome",
                "Domes",
                SHAPE,
                number(d.dome, 0.0, most_dome, Unit::Ratio),
            ));
            out.push(field(
                "aft_skirt",
                "Aft skirt",
                SHAPE,
                number(d.aft_skirt, 0.0, d.length, Unit::Metres),
            ));
            out.push(field(
                "forward_skirt",
                "Forward skirt",
                SHAPE,
                number(d.forward_skirt, 0.0, d.length, Unit::Metres),
            ));
            out.push(field(
                "thrust_plate",
                "Thrust plate",
                SHAPE,
                Value::Toggle(d.thrust_plate),
            ));
        }
        ShapeDef::ConeTank(d) => {
            let room = 2.0 * d.length / (d.bottom_diameter + d.top_diameter).max(1e-9);
            out.push(field(
                "dome",
                "Domes",
                SHAPE,
                number(d.dome, 0.0, room.min(2.0) * 0.999, Unit::Ratio),
            ));
        }
        ShapeDef::Shell(d) if d.span.is_none() => {
            let profile = d.sections.last().map(|s| s.profile).unwrap_or_default();
            out.push(field(
                "profile",
                "Profile",
                SHAPE,
                choice(profile_id(profile), PROFILES),
            ));
            out.push(field(
                "cap_bottom",
                "Closed at the bottom",
                SHAPE,
                Value::Toggle(d.cap_bottom),
            ));
            out.push(field(
                "cap_top",
                "Closed at the top",
                SHAPE,
                Value::Toggle(d.cap_top),
            ));
        }
        ShapeDef::Ring(d) => {
            out.push(field(
                "ring_thickness",
                "Thickness",
                SHAPE,
                number(d.thickness, 0.005, d.diameter / 2.0, Unit::Metres),
            ));
        }
        ShapeDef::Wing(d) => {
            const WING: &str = "Wing";
            out.push(field(
                "dihedral",
                "Dihedral",
                WING,
                number(d.dihedral, -30.0, 30.0, Unit::Degrees),
            ));
            out.push(field(
                "twist",
                "Twist",
                WING,
                number(d.twist, -15.0, 15.0, Unit::Degrees),
            ));
            match &d.airfoil {
                AirfoilDef::Plate { thickness, bevel } => {
                    let options = [("plate", "Flat plate"), ("naca", "Airfoil")];
                    out.push(field("airfoil", "Section", WING, choice("plate", &options)));
                    out.push(field(
                        "thickness",
                        "Thickness",
                        WING,
                        number(1000.0 * thickness, 1.0, 500.0, Unit::Millimetres),
                    ));
                    out.push(field(
                        "bevel",
                        "Edges bevelled over",
                        WING,
                        number(100.0 * bevel, 0.0, 50.0, Unit::Percent),
                    ));
                }
                AirfoilDef::Naca {
                    thickness, camber, ..
                } => {
                    let options = [("plate", "Flat plate"), ("naca", "Airfoil")];
                    out.push(field("airfoil", "Section", WING, choice("naca", &options)));
                    out.push(field(
                        "thickness",
                        "Thickness",
                        WING,
                        number(100.0 * thickness, 1.0, 40.0, Unit::Percent),
                    ));
                    out.push(field(
                        "camber",
                        "Camber",
                        WING,
                        number(100.0 * camber, 0.0, 9.0, Unit::Percent),
                    ));
                }
            }
        }
        _ => {}
    }
}

/// A number from what the player typed: digits, a sign, a point (or a comma), an exponent, with
/// anything after (a unit) left off.
fn parse_number(text: &str) -> Result<f64, String> {
    let text = text.trim().replace(',', ".");
    let end = text
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
        .map_or(text.len(), |(i, _)| i);
    text[..end]
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| format!("`{}` is not a number", text.trim()))
}

fn parse_toggle(text: &str) -> Result<bool, String> {
    match text.trim() {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        other => Err(format!("`{other}` is neither on nor off")),
    }
}

/// Set field `id` of `part` to `text`: a number in the field's unit, a choice's id or a toggle's
/// `true` or `false`. A number is kept within what the field can be, and a tank holds one of
/// `mixes` (see [`fields`]): a mix no engine it feeds burns is refused. What is mounted on a
/// shape is carried along as it changes, as when a size is dragged. Says why if it cannot be set;
/// the part is then as it was.
pub fn set(
    part: &mut CraftPartDef,
    id: &str,
    text: &str,
    mixes: &[Mix],
    catalog: &PartCatalog,
) -> Result<(), String> {
    let mut changed = part.clone();
    set_on(&mut changed, id, text, mixes, catalog)?;
    if let Some(shape) = &changed.shape {
        shape.build()?;
    }
    alone(&changed, catalog).map_err(|e| e.to_string())?;
    *part = changed;
    Ok(())
}

fn set_on(
    part: &mut CraftPartDef,
    id: &str,
    text: &str,
    mixes: &[Mix],
    catalog: &PartCatalog,
) -> Result<(), String> {
    // A number is kept within what its field can be.
    let field = fields(part, mixes, catalog)
        .into_iter()
        .find(|f| f.id == id);
    let Some(field) = field else {
        return Err(format!("`{}` has nothing called `{id}` to set", part.id));
    };
    let value = match &field.value {
        Value::Number { least, most, .. } => Some(parse_number(text)?.clamp(*least, *most)),
        Value::Choice { options, .. } if !options.iter().any(|(o, _)| o == text) => {
            // One of the usual mixes, which no engine the tank feeds burns.
            if id == "contents" && MIXES.iter().any(|(mix, ..)| *mix == text) {
                return Err(format!(
                    "{} cannot be `{text}`: no engine it feeds burns it",
                    field.label
                ));
            }
            return Err(format!("{} cannot be `{text}`", field.label));
        }
        _ => None,
    };
    let number = || value.unwrap_or_default();
    if id == "name" {
        let name = text.trim();
        part.name = (!name.is_empty()).then(|| name.to_owned());
        return Ok(());
    }
    if id == "roll" {
        part.roll = number();
        return Ok(());
    }
    if id == "configuration" {
        part.template = Some(text.to_owned());
        return Ok(());
    }
    // The shape's own sizes, dragged or typed alike.
    if shaping::part_sizes(part).iter().any(|s| s.name == id) {
        return shaping::resize(part, id, number());
    }
    if let Some(procedural) = &mut part.procedural {
        let mut asked = requirement(procedural);
        if procedural.fields().iter().any(|f| f.id == id) {
            asked.insert(id.into(), json!(number()));
            let kind = serde_json::to_value(&*procedural)
                .ok()
                .and_then(|v| v.as_object().and_then(|o| o.keys().next().cloned()))
                .unwrap_or_default();
            *procedural =
                serde_json::from_value(json!({ kind: asked })).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    match id {
        "contents" => return set_contents(part, text, mixes, catalog),
        "tank_type" => {
            if let Some(s) = storage_mut(part) {
                s.tank_type = text.to_owned();
            }
            return Ok(());
        }
        "utilization" => {
            if let Some(s) = storage_mut(part) {
                s.utilization = Some(number() / 100.0);
            }
            return Ok(());
        }
        "design_load" => {
            if let Some(s) = storage_mut(part) {
                s.design_load = Some(number());
            }
            return Ok(());
        }
        "material" => {
            part.structure.material = Some(text.to_owned());
            return Ok(());
        }
        "wall" => {
            part.structure.thickness = Some(number() / 1000.0);
            return Ok(());
        }
        // A booster's motor: its nozzle grows and weighs with its thrust.
        "motor_thrust" => {
            if let Some(motor) = booster_mut(part) {
                motor.thrust = 1000.0 * number();
                motor.mass = BOOSTER_NOZZLE_MASS * motor.thrust;
            }
            return Ok(());
        }
        "motor_nozzle" => {
            if let Some(motor) = booster_mut(part) {
                motor.vacuum = text == "vacuum";
                [motor.isp_vac, motor.isp_sl] = BOOSTER_ISP[usize::from(motor.vacuum)];
            }
            return Ok(());
        }
        "motor_curve" => {
            let preset = CURVE_PRESETS.iter().find(|(id, ..)| *id == text);
            if let (Some((_, _, points)), Some(motor)) = (preset, booster_mut(part)) {
                motor.set_curve(*points);
            }
            return Ok(());
        }
        _ => {}
    }
    if let Some(k) = (id.strip_prefix("motor_curve_")).and_then(|k| k.parse::<usize>().ok())
        && let Some(motor) = booster_mut(part)
    {
        let mut points = motor.curve_points();
        if let Some(point) = points.get_mut(k) {
            *point = number() / 100.0;
        }
        motor.set_curve(points);
        return Ok(());
    }
    let Some(shape) = part.shape.as_mut() else {
        return Err(format!("`{}` has no shape", part.id));
    };
    let before = shape.clone();
    let shape = unmirrored_mut(shape);
    match (shape, id) {
        (shape, "shape") => *shape = switched(shape, text),
        (ShapeDef::Shell(d), "shell_shape") => {
            let base = (d.sections.iter().rev().skip(1))
                .find_map(|s| s.top_diameter)
                .unwrap_or(d.bottom_diameter);
            if let Some(last) = d.sections.last_mut() {
                match text {
                    "tube" => {
                        last.top_diameter = None;
                        last.profile = ShellProfile::Conic;
                    }
                    "taper" => last.top_diameter = Some(0.6 * base),
                    _ => {
                        last.top_diameter = Some(0.0);
                        if last.profile == ShellProfile::Conic {
                            last.profile = ShellProfile::Ogive;
                        }
                    }
                }
            }
        }
        (ShapeDef::Shell(d), "profile") => {
            let profile: ShellProfile =
                serde_json::from_value(json!(text)).map_err(|e| e.to_string())?;
            if let Some(last) = d.sections.last_mut() {
                last.profile = profile;
            }
        }
        (ShapeDef::Shell(d), "cap_bottom") => d.cap_bottom = parse_toggle(text)?,
        (ShapeDef::Shell(d), "cap_top") => d.cap_top = parse_toggle(text)?,
        (ShapeDef::CylinderTank(d), "dome") => {
            let domes = d.dome_height();
            d.dome = number();
            d.keep_skirts(domes);
        }
        (ShapeDef::ConeTank(d), "dome") => d.dome = number(),
        (ShapeDef::CylinderTank(d), "aft_skirt") => d.aft_skirt = number(),
        (ShapeDef::CylinderTank(d), "forward_skirt") => d.forward_skirt = number(),
        (ShapeDef::CylinderTank(d), "thrust_plate") => d.thrust_plate = parse_toggle(text)?,
        (ShapeDef::Ring(d), "ring_thickness") => d.thickness = number(),
        (ShapeDef::Wing(d), "dihedral") => d.dihedral = number(),
        (ShapeDef::Wing(d), "twist") => d.twist = number(),
        (ShapeDef::Wing(d), "airfoil") => {
            let now = matches!(d.airfoil, AirfoilDef::Plate { .. });
            d.airfoil = match (text, now) {
                ("plate", false) => {
                    serde_json::from_value(json!({ "plate": { "thickness": 0.02 } }))
                }
                ("naca", true) => serde_json::from_value(json!({ "naca": { "thickness": 0.12 } })),
                _ => Ok(d.airfoil.clone()),
            }
            .map_err(|e| e.to_string())?;
        }
        (ShapeDef::Wing(d), "thickness") => match &mut d.airfoil {
            AirfoilDef::Plate { thickness, .. } => *thickness = number() / 1000.0,
            AirfoilDef::Naca { thickness, .. } => *thickness = number() / 100.0,
        },
        (ShapeDef::Wing(d), "bevel") => {
            if let AirfoilDef::Plate { bevel, .. } = &mut d.airfoil {
                *bevel = number() / 100.0;
            }
        }
        (ShapeDef::Wing(d), "camber") => {
            if let AirfoilDef::Naca { camber, .. } = &mut d.airfoil {
                *camber = number() / 100.0;
            }
        }
        _ => return Err(format!("`{}` has nothing called `{id}` to set", part.id)),
    }
    shaping::carry_nodes(part, &before);
    Ok(())
}

/// `shape` as a tank of kind `kind` (`cylinder`, `cone`, `sphere`), as like it as it can be: as
/// long and as wide, with the same domes.
fn switched(shape: &ShapeDef, kind: &str) -> ShapeDef {
    let (length, bottom, top, dome) = match shape {
        ShapeDef::CylinderTank(d) => (d.length, d.diameter, d.diameter, d.dome),
        ShapeDef::ConeTank(d) => (d.length, d.bottom_diameter, d.top_diameter, d.dome),
        ShapeDef::Sphere(d) => (
            d.diameter,
            d.diameter,
            d.diameter,
            std::f64::consts::FRAC_1_SQRT_2,
        ),
        other => return other.clone(),
    };
    let widest = bottom.max(top);
    match kind {
        "cylinder" => ShapeDef::CylinderTank(
            CylinderTankDef {
                diameter: widest,
                // Long enough for its domes.
                length: length.max(dome * widest * 1.001),
                dome,
                common_bulkhead: None,
                aft_skirt: 0.0,
                forward_skirt: 0.0,
                thrust_plate: false,
            }
            .skirted(),
        ),
        "cone" => ShapeDef::ConeTank(ConeTankDef {
            length: length.max(dome * (bottom + top) / 2.0 * 1.001),
            bottom_diameter: bottom,
            top_diameter: top,
            dome,
        }),
        "sphere" => ShapeDef::Sphere(SphereDef { diameter: widest }),
        _ => shape.clone(),
    }
}

/// Fill `part` with the mix of `mixes` called `id`, or empty it for `none`. A tank type that cannot
/// hold the mix gives way to the first that can; an empty tank is structure, and gets a wall if it
/// had none.
pub(crate) fn set_contents(
    part: &mut CraftPartDef,
    id: &str,
    mixes: &[Mix],
    catalog: &PartCatalog,
) -> Result<(), String> {
    if id == CUSTOM {
        return Ok(());
    }
    if id == NOTHING {
        part.components
            .retain(|c| !matches!(c, ComponentDef::Storage(_)));
        if part.structure.material.is_none() {
            part.structure.material = catalog.materials.first().map(|m| m.id.clone());
        }
        if part.structure.thickness.is_none() {
            part.structure.thickness = Some(0.004);
        }
        return Ok(());
    }
    let Some(mix) = mixes.iter().find(|m| m.id == id) else {
        return Err(format!("there is no mix `{id}`"));
    };
    let contents = mix.contents.clone();
    match storage_mut(part) {
        Some(s) => s.contents = contents,
        None => part.components.push(ComponentDef::Storage(StorageDef {
            cell: None,
            tank_type: catalog
                .tank_types
                .first()
                .map(|t| t.id.clone())
                .unwrap_or_default(),
            contents,
            design_load: None,
            utilization: None,
        })),
    }
    if alone(part, catalog).is_ok() {
        return Ok(());
    }
    // The first tank type that can hold it.
    for kind in &catalog.tank_types {
        if let Some(s) = storage_mut(part) {
            s.tank_type = kind.id.clone();
        }
        if alone(part, catalog).is_ok() {
            return Ok(());
        }
    }
    Err("no tank type can hold that".into())
}

/// `part` built by itself.
pub(crate) fn alone(
    part: &CraftPartDef,
    catalog: &PartCatalog,
) -> Result<Craft, crate::propulsion::CraftError> {
    let mut only = part.clone();
    only.on = None;
    only.at = None;
    let craft = CraftDef {
        id: "alone".into(),
        name: String::new(),
        parts: vec![only],
        sequences: None,
        triggers: Vec::new(),
        rcs_shut: Vec::new(),
        abort: Vec::new(),
    };
    Craft::build(&craft, catalog)
}

/// What `part` comes to, each a label and its value as read: what its tank holds and how much,
/// and what it weighs dry and full.
pub fn readouts(part: &CraftPartDef, catalog: &PartCatalog) -> Vec<(String, String)> {
    let Ok(craft) = alone(part, catalog) else {
        return Vec::new();
    };
    let built = &craft.parts[0].built;
    let mut out = Vec::new();
    let mut held = 0.0;
    for storage in built.storages() {
        let holds: f64 = storage.contents.iter().map(|c| c.capacity).sum();
        held += holds;
        out.push(("Volume".into(), format!("{:.2} m³", storage.volume)));
        out.push(("Propellant".into(), tonnes(holds)));
    }
    out.push(("Dry mass".into(), tonnes(built.mass.mass)));
    if held > 0.0 {
        out.push(("Full mass".into(), tonnes(built.mass.mass + held)));
    }
    // A booster: how long its grain lasts at its thrust, and what it pushes with in the thick air
    // of the pad.
    if let Some(motor) = booster(part) {
        let curve = motor.thrust_curve.as_deref();
        let mean = curve.map_or(1.0, crate::propulsion::mean_share);
        let flow = motor.thrust / (motor.isp_vac * crate::constants::G0) * mean;
        if flow > 0.0 {
            out.push(("Burns for".into(), format!("{:.0} s", held / flow)));
        }
        let at_pad = motor.thrust * motor.isp_sl / motor.isp_vac;
        out.push((
            "Thrust at sea level".into(),
            format!("{:.0} kN", at_pad / 1000.0),
        ));
    }
    out
}

/// `part`'s solid booster motor, if it has one.
fn booster(part: &CraftPartDef) -> Option<&SolidMotorDef> {
    part.components.iter().find_map(|c| match c {
        ComponentDef::SolidMotor(motor) if motor.booster => Some(motor),
        _ => None,
    })
}

fn booster_mut(part: &mut CraftPartDef) -> Option<&mut SolidMotorDef> {
    part.components.iter_mut().find_map(|c| match c {
        ComponentDef::SolidMotor(motor) if motor.booster => Some(motor),
        _ => None,
    })
}

fn tonnes(kg: f64) -> String {
    if kg >= 1000.0 {
        format!("{:.2} t", kg / 1000.0)
    } else {
        format!("{kg:.0} kg")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette;

    fn catalog() -> PartCatalog {
        crate::vessel::tests::catalog()
    }

    fn part(id: &str) -> CraftPartDef {
        let all = palette::entries(&catalog());
        let mut part = all.iter().find(|e| e.id == id).unwrap().part.clone();
        part.id = "p".into();
        part
    }

    fn value(part: &CraftPartDef, id: &str) -> Value {
        let catalog = catalog();
        fields(part, &usual(&catalog), &catalog)
            .into_iter()
            .find(|f| f.id == id)
            .unwrap()
            .value
    }

    /// Set a field of a part on a craft with no engine, whose tanks hold the usual mixes.
    fn set_alone(
        part: &mut CraftPartDef,
        id: &str,
        text: &str,
        catalog: &PartCatalog,
    ) -> Result<(), String> {
        set(part, id, text, &usual(catalog), catalog)
    }

    /// `tank` with `under` (palette ids) under it, each on the one above.
    fn craft_of(tank: &CraftPartDef, under: &[&str]) -> CraftDef {
        let mut parts = vec![tank.clone()];
        for (k, id) in under.iter().enumerate() {
            let mut next = part(id);
            next.id = format!("under_{k}");
            next.on = Some(format!("{}.bottom", parts[parts.len() - 1].id));
            next.by = "top".into();
            parts.push(next);
        }
        CraftDef {
            id: "test".into(),
            name: String::new(),
            parts,
            sequences: None,
            triggers: Vec::new(),
            rcs_shut: Vec::new(),
            abort: Vec::new(),
        }
    }

    /// `tank` with `below` (a palette id) under it, and over it another tank with `above` on that
    /// tank's top, held by its `by`: two things that burn, both fed from `tank`. An engine has no
    /// node under it, so a second one goes on top.
    fn two_ends(tank: &CraftPartDef, below: &str, above: &str, by: &str) -> CraftDef {
        let mut craft = craft_of(tank, &[below]);
        let mut over = part("tank");
        over.id = "over".into();
        over.on = Some(format!("{}.top", tank.id));
        over.by = "bottom".into();
        let mut last = part(above);
        last.id = "above".into();
        last.on = Some("over.top".into());
        last.by = by.into();
        craft.parts.extend([over, last]);
        craft
    }

    /// What a tank's contents can be set to on `craft`, each `(id, label)`, and what it is: what
    /// the engines its copy on `craft` (by id) feeds burn.
    fn holds(tank: &CraftPartDef, craft: &CraftDef) -> (String, Vec<(String, String)>) {
        let catalog = catalog();
        let mixes = mixes(craft, &catalog, &tank.id);
        let found = fields(tank, &mixes, &catalog);
        match found
            .into_iter()
            .find(|f| f.id == "contents")
            .unwrap()
            .value
        {
            Value::Choice { value, options } => (value, options),
            other => panic!("{other:?}"),
        }
    }

    fn ids(options: &[(String, String)]) -> Vec<&str> {
        options.iter().map(|(id, _)| id.as_str()).collect()
    }

    /// Fill `tank` on `craft` with mix `id`.
    fn fill(tank: &mut CraftPartDef, id: &str, craft: &CraftDef) -> Result<(), String> {
        let catalog = catalog();
        set(
            tank,
            "contents",
            id,
            &mixes(craft, &catalog, &tank.id),
            &catalog,
        )
    }

    /// Whether `tank` holds `parts`, in their ratio.
    fn holds_mix(tank: &CraftPartDef, parts: &[(&str, f64)]) -> bool {
        let wanted = (parts.iter())
            .map(|(name, ratio)| ((*name).to_owned(), *ratio))
            .collect();
        storage(tank).is_some_and(|s| same_mix(&s.contents, &wanted))
    }

    #[test]
    fn a_tank_holds_what_the_engine_it_feeds_burns_at_its_ratio() {
        // The tank holds kerolox at 2.36, the K-5 burns it at 2.27: the tank's own stays shown, as
        // custom, beside the engine's, named with the engine.
        let mut tank = part("tank");
        let one = craft_of(&tank, &["engine:K-5"]);
        let (value, options) = holds(&tank, &one);
        assert_eq!(value, "custom");
        assert_eq!(ids(&options), ["kerolox", "custom", "none"]);
        assert!(
            options[0]
                .1
                .starts_with("Kerosene and liquid oxygen, 1 : 2.27 (")
                && options[0].1.contains("K-5"),
            "{}",
            options[0].1
        );
        assert_eq!(options[1].1, "Kerosene and liquid oxygen, 1 : 2.36");
        // Set, it holds what the engine burns, in its ratio, and nothing else is offered.
        fill(&mut tank, "kerolox", &one).unwrap();
        assert!(holds_mix(&tank, &[("Kerosene", 1.0), ("LqdOxygen", 2.27)]));
        let (value, options) = holds(&tank, &one);
        assert_eq!(
            (value.as_str(), ids(&options)),
            ("kerolox", vec!["kerolox", "none"])
        );
        // Its engine does not burn hydrogen: the tank is not filled with it.
        let before = tank.clone();
        let why = fill(&mut tank, "hydrolox", &one).unwrap_err();
        assert!(why.contains("no engine"), "{why}");
        assert_eq!(tank, before);
        // A hydrolox engine it feeds as well adds hydrolox at its ratio.
        let two = two_ends(&tank, "engine:K-5", "engine:H-25", "top");
        let (_, options) = holds(&tank, &two);
        assert_eq!(ids(&options), ["kerolox", "hydrolox", "none"]);
        fill(&mut tank, "hydrolox", &two).unwrap();
        assert!(holds_mix(
            &tank,
            &[("LqdHydrogen", 1.0), ("LqdOxygen", 6.0)]
        ));
    }

    #[test]
    fn a_tank_is_offered_nothing_an_engine_past_a_decoupler_burns() {
        let tank = part("tank");
        let craft = craft_of(&tank, &["decoupler", "engine:H-25"]);
        let (value, options) = holds(&tank, &craft);
        assert_eq!(
            (value.as_str(), ids(&options)),
            ("custom", vec!["custom", "none"])
        );
        let (_, options) = holds(&tank, &craft_of(&tank, &["decoupler"]));
        assert_eq!(ids(&options), ["custom", "none"]);
    }

    #[test]
    fn the_same_propellants_burnt_at_two_ratios_are_two_mixes() {
        let tank = part("tank");
        // The K-1 Booster burns the tank's own 2.36, the K-5 2.27.
        let craft = two_ends(&tank, "engine:K-5", "engine:K-1 Booster", "top");
        let (value, options) = holds(&tank, &craft);
        assert_eq!(ids(&options), ["kerolox", "kerolox_2", "none"]);
        assert_eq!(value, "kerolox_2");
        // The same mix burnt by two engines is offered once, naming both.
        let craft = two_ends(&tank, "engine:K-1 Booster", "engine:K-1 Vacuum", "top");
        let (value, options) = holds(&tank, &craft);
        assert_eq!(
            (value.as_str(), ids(&options)),
            ("kerolox", vec!["kerolox", "none"])
        );
        assert!(
            options[0]
                .1
                .starts_with("Kerosene and liquid oxygen, 1 : 2.36 (")
                && options[0].1.contains("K-1 Booster")
                && options[0].1.contains("K-1 Vacuum"),
            "{}",
            options[0].1
        );
    }

    #[test]
    fn what_a_solid_or_thrusters_burn_is_offered() {
        let mut tank = part("tank");
        // A separation motor carries its own grain, and asks nothing of the tanks.
        let motor: CraftPartDef = serde_json::from_value(json!({
            "id": "motor",
            "procedural": { "separation_motor": { "impulse": 60000, "burn_time": 4 } },
        }))
        .unwrap();
        assert!(burnt_by(&motor, &catalog()).is_empty());
        // A solid's grain, and the gas that thrusters it feeds burn.
        let craft = two_ends(&tank, "engine:S-1", "rcs:Cold Gas Quad", "mount");
        let (_, options) = holds(&tank, &craft);
        assert_eq!(ids(&options), ["HTPB", "nitrogen", "custom", "none"]);
        fill(&mut tank, "HTPB", &craft).unwrap();
        assert!(holds_mix(&tank, &[("HTPB", 1.0)]));
    }

    #[test]
    fn a_new_tank_takes_what_its_engine_burns_whichever_goes_on_last() {
        let catalog = catalog();
        let k5 = [("Kerosene", 1.0), ("LqdOxygen", 2.27)];
        // The engine put on under the tank.
        let mut craft = craft_of(&part("tank"), &["engine:K-5"]);
        let filled = fill_from_engines(&mut craft, &catalog, &["under_0".into()]);
        assert_eq!(filled, ["p"]);
        assert!(holds_mix(&craft.parts[0], &k5));
        // The tank put on over the engine: the same.
        let mut craft = craft_of(&part("tank"), &["engine:K-5"]);
        assert_eq!(
            fill_from_engines(&mut craft, &catalog, &["p".into()]),
            ["p"]
        );
        assert!(holds_mix(&craft.parts[0], &k5));
        // Hydrolox, in a tank type that holds it.
        let mut craft = craft_of(&part("tank"), &["engine:H-25"]);
        assert_eq!(
            fill_from_engines(&mut craft, &catalog, &["p".into()]),
            ["p"]
        );
        assert!(holds_mix(
            &craft.parts[0],
            &[("LqdHydrogen", 1.0), ("LqdOxygen", 6.0)]
        ));
        assert!(alone(&craft.parts[0], &catalog).is_ok());
    }

    #[test]
    fn a_tank_the_player_filled_or_past_a_decoupler_is_left_as_it_is() {
        let catalog = catalog();
        let mut tank = part("tank");
        if let Some(s) = storage_mut(&mut tank) {
            s.contents = [("Kerosene".to_owned(), 1.0), ("LqdOxygen".to_owned(), 2.0)].into();
        }
        let mut craft = craft_of(&tank, &["engine:K-5"]);
        assert!(fill_from_engines(&mut craft, &catalog, &["under_0".into()]).is_empty());
        assert_eq!(craft.parts[0], tank);
        let mut craft = craft_of(&part("tank"), &["decoupler", "engine:H-25"]);
        assert!(fill_from_engines(&mut craft, &catalog, &["under_1".into()]).is_empty());
        assert_eq!(craft.parts[0], part("tank"));
        // A part that has nothing to do with propellant fills nothing.
        let mut craft = craft_of(&part("tank"), &["engine:K-5", "fin"]);
        assert!(fill_from_engines(&mut craft, &catalog, &["under_1".into()]).is_empty());
    }

    #[test]
    fn every_field_of_every_procedural_part_can_be_set_to_every_value_it_offers() {
        let catalog = catalog();
        for id in [
            "tank",
            "structural",
            "nose_cone",
            "decoupler",
            "fin",
            "wing",
        ] {
            let start = part(id);
            for f in fields(&start, &usual(&catalog), &catalog) {
                let tries: Vec<String> = match &f.value {
                    Value::Number { least, most, .. } => {
                        vec![
                            least.to_string(),
                            most.to_string(),
                            ((least + most) / 2.0).to_string(),
                        ]
                    }
                    Value::Choice { options, .. } => options.iter().map(|o| o.0.clone()).collect(),
                    Value::Toggle(on) => vec![(!on).to_string()],
                    Value::Text(_) => vec!["Renamed".into()],
                };
                for text in tries {
                    let mut changed = start.clone();
                    match set_alone(&mut changed, &f.id, &text, &catalog) {
                        // What was set builds, and reads back as set.
                        Ok(()) => {
                            alone(&changed, &catalog).unwrap();
                            if let (Value::Choice { .. }, Some(back)) = (
                                &f.value,
                                fields(&changed, &usual(&catalog), &catalog)
                                    .into_iter()
                                    .find(|g| g.id == f.id),
                            ) && let Value::Choice { value, .. } = back.value
                            {
                                assert_eq!(value, text, "{id} {}", f.id);
                            }
                        }
                        // A refusal leaves it as it was, and says why.
                        Err(why) => {
                            assert_eq!(changed, start);
                            assert!(!why.is_empty());
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_tank_switches_shape_keeping_its_size() {
        let catalog = catalog();
        let mut tank = part("tank");
        set_alone(&mut tank, "diameter", "3", &catalog).unwrap();
        set_alone(&mut tank, "length", "8", &catalog).unwrap();
        set_alone(&mut tank, "shape", "cone", &catalog).unwrap();
        assert_eq!(
            value(&tank, "bottom_diameter"),
            number(3.0, 0.0, value_most(&tank, "bottom_diameter"), Unit::Metres)
        );
        set_alone(&mut tank, "top_diameter", "1.5", &catalog).unwrap();
        set_alone(&mut tank, "shape", "cylinder", &catalog).unwrap();
        let Value::Number { value: d, .. } = value(&tank, "diameter") else {
            panic!()
        };
        assert_eq!(d, 3.0);
        set_alone(&mut tank, "shape", "sphere", &catalog).unwrap();
        assert!(
            matches!(tank.shape, Some(ShapeDef::Sphere(SphereDef { diameter })) if diameter == 3.0)
        );
    }

    fn value_most(part: &CraftPartDef, id: &str) -> f64 {
        match value(part, id) {
            Value::Number { most, .. } => most,
            _ => panic!(),
        }
    }

    #[test]
    fn a_tank_holds_what_it_is_set_to_in_the_share_of_it_set() {
        let catalog = catalog();
        let mut tank = part("tank");
        let held = |tank: &CraftPartDef| -> f64 {
            let craft = alone(tank, &catalog).unwrap();
            let storage = craft.parts[0].built.storages().next().unwrap();
            storage.contents.iter().map(|c| c.capacity).sum()
        };
        set_alone(&mut tank, "utilization", "100", &catalog).unwrap();
        let full = held(&tank);
        set_alone(&mut tank, "utilization", "50", &catalog).unwrap();
        assert!((held(&tank) / full - 0.5).abs() < 1e-9);
        // Hydrogen needs an insulated tank: the tank type gives way to one.
        set_alone(&mut tank, "contents", "hydrolox", &catalog).unwrap();
        assert!(
            matches!(value(&tank, "contents"), Value::Choice { value, .. } if value == "hydrolox")
        );
        // Emptied, it is structure with a wall; filled again, a tank.
        set_alone(&mut tank, "contents", "none", &catalog).unwrap();
        assert!(storage(&tank).is_none() && tank.structure.thickness.is_some());
        set_alone(&mut tank, "contents", "kerolox", &catalog).unwrap();
        assert!(storage(&tank).is_some());
        assert!(
            readouts(&tank, &catalog)
                .iter()
                .any(|(l, _)| l == "Propellant")
        );
    }

    #[test]
    fn a_number_is_read_with_its_unit_and_kept_in_its_range() {
        assert_eq!(parse_number(" 3,5 m").unwrap(), 3.5);
        assert_eq!(parse_number("88%").unwrap(), 88.0);
        assert!(parse_number("tall").is_err());
        let catalog = catalog();
        let mut decoupler = part("decoupler");
        set_alone(&mut decoupler, "diameter", "9999", &catalog).unwrap();
        let Value::Number { value, most, .. } = value(&decoupler, "diameter") else {
            panic!()
        };
        assert_eq!(value, most);
        assert!(set_alone(&mut decoupler, "colour", "red", &catalog).is_err());
    }

    /// Two engines made configurations of one: the parts list shows one part, its settings pick
    /// the other, and the part is then the other engine.
    #[test]
    fn an_engine_family_is_one_part_with_its_configuration_picked() {
        let mut catalog = catalog();
        for engine in &mut catalog.engines {
            if engine.id.starts_with("K-1 ") {
                engine.family = Some("K-1".into());
            }
        }
        catalog.engine_families = crate::test_data::records(
            r#"[{ id: "K-1", title: "K-1", default_config: "K-1 Booster",
                  configs: ["K-1 Booster", "K-1 Vacuum"] }]"#,
        );
        let all = palette::entries(&catalog);
        let k1 = |e: &&palette::Entry| e.id == "engine:K-1 Booster" || e.id == "engine:K-1 Vacuum";
        let listed: Vec<_> = all.iter().filter(k1).collect();
        let ids: Vec<&str> = listed.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["engine:K-1 Booster"]);
        assert_eq!(listed[0].name, "K-1");
        assert!(
            listed[0].detail.contains("2 configurations"),
            "{}",
            listed[0].detail
        );
        assert!(
            listed[0].matches("vacuum"),
            "found by another configuration's name"
        );

        let mut part = listed[0].part.clone();
        part.id = "p".into();
        let field = (fields(&part, &[], &catalog).into_iter())
            .find(|f| f.id == "configuration")
            .unwrap();
        let Value::Choice { value, options } = field.value else {
            panic!("a choice");
        };
        assert_eq!(value, "K-1 Booster");
        let ids: Vec<&str> = options.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["K-1 Booster", "K-1 Vacuum"]);

        set(&mut part, "configuration", "K-1 Vacuum", &[], &catalog).unwrap();
        assert_eq!(part.template.as_deref(), Some("K-1 Vacuum"));
        assert!(set(&mut part, "configuration", "H-10", &[], &catalog).is_err());
        let vacuum = catalog
            .engines
            .iter()
            .find(|e| e.id == "K-1 Vacuum")
            .unwrap();
        let built = alone(&part, &catalog).unwrap();
        let thrust = (built.parts[0].built.components.iter())
            .find_map(|c| match c {
                crate::component::Component::Engine(engine) => Some(engine.thrust_vac),
                _ => None,
            })
            .unwrap();
        assert_eq!(thrust, vacuum.thrust_vac);

        // An engine of no family has nothing to pick.
        let mut lone = part.clone();
        lone.template = Some("H-10".into());
        assert!(
            fields(&lone, &[], &catalog)
                .iter()
                .all(|f| f.id != "configuration")
        );
    }
}
