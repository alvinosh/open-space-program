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

use serde_json::{Value as Json, json};

use crate::{
    component::{ComponentDef, storage::StorageDef},
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

/// What a tank can be filled with: an id, what the player reads, and the mass ratio. Those whose
/// resources the content has are offered.
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
    ("kerosene", "Kerosene", &[("Kerosene", 1.0)]),
    ("lox", "Liquid oxygen", &[("LqdOxygen", 1.0)]),
    ("lh2", "Liquid hydrogen", &[("LqdHydrogen", 1.0)]),
    ("hydrazine", "Hydrazine", &[("Hydrazine", 1.0)]),
    ("nitrogen", "Nitrogen", &[("Nitrogen", 1.0)]),
    ("helium", "Helium", &[("Helium", 1.0)]),
];

/// What a tank holding nothing is called: it is then structure.
const NOTHING: &str = "none";
/// What contents no mix above match are called.
const CUSTOM: &str = "custom";

/// The mixes `catalog` has every resource of.
fn mixes(
    catalog: &PartCatalog,
) -> Vec<(&'static str, &'static str, &'static [(&'static str, f64)])> {
    (MIXES.iter())
        .filter(|(_, _, parts)| {
            (parts.iter()).all(|(name, _)| catalog.resources.iter().any(|r| r.id == *name))
        })
        .copied()
        .collect()
}

/// Which mix `contents` is, if any: the same resources in the same ratio.
fn mix_of(contents: &std::collections::BTreeMap<String, f64>) -> Option<&'static str> {
    let total: f64 = contents.values().sum();
    MIXES.iter().find_map(|(id, _, parts)| {
        let mine: f64 = parts.iter().map(|(_, ratio)| ratio).sum();
        let same = parts.len() == contents.len()
            && (parts.iter()).all(|(name, ratio)| {
                (contents.get(*name)).is_some_and(|r| (r / total - ratio / mine).abs() < 1e-3)
            });
        same.then_some(*id)
    })
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

/// Everything that can be set on `part`, in the order the window shows it.
pub fn fields(part: &CraftPartDef, catalog: &PartCatalog) -> Vec<Field> {
    let name = part.name.clone().unwrap_or_else(|| part.id.clone());
    let mut out = vec![field("name", "Name", "Part", Value::Text(name))];
    let shape = own_shape(part);
    if let Some(shape) = shape {
        shape_fields(shape, &mut out);
    }
    // What it holds, and how its tank is built.
    if shape.is_some_and(holds_things) || storage(part).is_some() {
        let mut options: Vec<(String, String)> = mixes(catalog)
            .iter()
            .map(|(id, label, _)| ((*id).into(), (*label).into()))
            .collect();
        let value = match storage(part) {
            None => NOTHING.to_owned(),
            Some(s) => match mix_of(&s.contents) {
                Some(id) if options.iter().any(|(o, _)| o == id) => id.to_owned(),
                _ => {
                    let names: Vec<&str> = s.contents.keys().map(String::as_str).collect();
                    options.push((CUSTOM.into(), names.join(" and ")));
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
/// `true` or `false`. A number is kept within what the field can be. What is mounted on a shape
/// is carried along as it changes, as when a size is dragged. Says why if it cannot be set; the
/// part is then as it was.
pub fn set(
    part: &mut CraftPartDef,
    id: &str,
    text: &str,
    catalog: &PartCatalog,
) -> Result<(), String> {
    let mut changed = part.clone();
    set_on(&mut changed, id, text, catalog)?;
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
    catalog: &PartCatalog,
) -> Result<(), String> {
    // A number is kept within what its field can be.
    let field = fields(part, catalog).into_iter().find(|f| f.id == id);
    let Some(field) = field else {
        return Err(format!("`{}` has nothing called `{id}` to set", part.id));
    };
    let value = match &field.value {
        Value::Number { least, most, .. } => Some(parse_number(text)?.clamp(*least, *most)),
        Value::Choice { options, .. } if !options.iter().any(|(o, _)| o == text) => {
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
        "contents" => return set_contents(part, text, catalog),
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
        _ => {}
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
        (ShapeDef::CylinderTank(d), "dome") => d.dome = number(),
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
        "cylinder" => ShapeDef::CylinderTank(CylinderTankDef {
            diameter: widest,
            // Long enough for its domes.
            length: length.max(dome * widest * 1.001),
            dome,
            common_bulkhead: None,
            aft_skirt: 0.0,
            forward_skirt: 0.0,
            thrust_plate: false,
        }),
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

/// Fill `part` with mix `id`, or empty it for `none`. A tank type that cannot hold the mix gives way
/// to the first that can; an empty tank is structure, and gets a wall if it had none.
fn set_contents(part: &mut CraftPartDef, id: &str, catalog: &PartCatalog) -> Result<(), String> {
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
    let Some((_, _, mix)) = mixes(catalog).into_iter().find(|(m, ..)| *m == id) else {
        return Err(format!("there is no mix `{id}`"));
    };
    let contents = mix
        .iter()
        .map(|(name, ratio)| ((*name).to_owned(), *ratio))
        .collect();
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
fn alone(
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
    out
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
        fields(part, &catalog)
            .into_iter()
            .find(|f| f.id == id)
            .unwrap()
            .value
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
            for f in fields(&start, &catalog) {
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
                    match set(&mut changed, &f.id, &text, &catalog) {
                        // What was set builds, and reads back as set.
                        Ok(()) => {
                            alone(&changed, &catalog).unwrap();
                            if let (Value::Choice { .. }, Some(back)) = (
                                &f.value,
                                fields(&changed, &catalog)
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
        set(&mut tank, "diameter", "3", &catalog).unwrap();
        set(&mut tank, "length", "8", &catalog).unwrap();
        set(&mut tank, "shape", "cone", &catalog).unwrap();
        assert_eq!(
            value(&tank, "bottom_diameter"),
            number(3.0, 0.0, value_most(&tank, "bottom_diameter"), Unit::Metres)
        );
        set(&mut tank, "top_diameter", "1.5", &catalog).unwrap();
        set(&mut tank, "shape", "cylinder", &catalog).unwrap();
        let Value::Number { value: d, .. } = value(&tank, "diameter") else {
            panic!()
        };
        assert_eq!(d, 3.0);
        set(&mut tank, "shape", "sphere", &catalog).unwrap();
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
        set(&mut tank, "utilization", "100", &catalog).unwrap();
        let full = held(&tank);
        set(&mut tank, "utilization", "50", &catalog).unwrap();
        assert!((held(&tank) / full - 0.5).abs() < 1e-9);
        // Hydrogen needs an insulated tank: the tank type gives way to one.
        set(&mut tank, "contents", "hydrolox", &catalog).unwrap();
        assert!(
            matches!(value(&tank, "contents"), Value::Choice { value, .. } if value == "hydrolox")
        );
        // Emptied, it is structure with a wall; filled again, a tank.
        set(&mut tank, "contents", "none", &catalog).unwrap();
        assert!(storage(&tank).is_none() && tank.structure.thickness.is_some());
        set(&mut tank, "contents", "kerolox", &catalog).unwrap();
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
        set(&mut decoupler, "diameter", "9999", &catalog).unwrap();
        let Value::Number { value, most, .. } = value(&decoupler, "diameter") else {
            panic!()
        };
        assert_eq!(value, most);
        assert!(set(&mut decoupler, "colour", "red", &catalog).is_err());
    }
}
