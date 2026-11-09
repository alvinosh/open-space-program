//! The palette: what a player can put on a craft in the editor. A few procedural parts (a tank, a
//! structural tube, a nose cone, a decoupler, a fin, a wing and landing wheels) whose sizes the
//! player then sets,
//! as in KSP with its procedural parts, and the catalogue's pods, engines, RCS blocks and reaction
//! wheels as they are. Each is listed by what it does ([`Group`], KSP's function filter) and says
//! what else the parts list narrows it by: how wide it is ([`Entry::width`]) and what it burns
//! ([`Propellant`]).
//!
//! An entry is a part to copy ([`Entry::part`], with no id: the editor names it). A procedural one
//! is [`fit`] to what it goes on, so a tank put under a 3 m tank is 3 m across and a fin is sized
//! for the hull it stands on. What goes on a surface says by which of its own nodes, and how far it
//! is turned about the joint so that it stands the right way up ([`Entry::surface`]).

use serde_json::json;

use crate::{
    craft::Craft,
    data::{CatalogInfo, CraftDef, CraftPartDef, PartCatalog},
    geometry::preset::{AirfoilDef, ShapeDef},
    shaping,
    sizing::ProceduralDef,
};

/// Where an entry is listed: by what it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Pods,
    Tanks,
    Engines,
    /// Tubes, nose cones and decouplers.
    Structure,
    /// Fins and wings.
    Aero,
    /// RCS blocks and reaction wheels.
    Control,
}

impl Group {
    pub const ALL: [Group; 6] = [
        Group::Pods,
        Group::Tanks,
        Group::Engines,
        Group::Structure,
        Group::Aero,
        Group::Control,
    ];

    /// As actions write it.
    pub fn id(self) -> &'static str {
        match self {
            Group::Pods => "pods",
            Group::Tanks => "tanks",
            Group::Engines => "engines",
            Group::Structure => "structure",
            Group::Aero => "aero",
            Group::Control => "control",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.id() == text)
    }

    /// What the player reads.
    pub fn name(self) -> &'static str {
        match self {
            Group::Pods => "Pods",
            Group::Tanks => "Tanks",
            Group::Engines => "Engines",
            Group::Structure => "Structure",
            Group::Aero => "Aero",
            Group::Control => "Control",
        }
    }
}

/// What an engine or an RCS block burns, by kind, as players name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Propellant {
    /// The base game's one mix of liquid fuel and oxidiser.
    LiquidFuel,
    /// Liquid oxygen and kerosene (RP-1, RG-1, syntin).
    Kerolox,
    /// Liquid oxygen and liquid hydrogen.
    Hydrolox,
    /// Liquid oxygen and liquid methane.
    Methalox,
    /// Propellants kept at room temperature: nitrogen tetroxide or nitric acid with a hydrazine or
    /// kerosene.
    Hypergolic,
    Solid,
    /// One propellant broken down over a catalyst: hydrazine or peroxide.
    Monopropellant,
    /// Gas let out of a nozzle, burning nothing.
    ColdGas,
    /// Gas pushed out by electric power: xenon, krypton, argon.
    Electric,
    /// Propellant heated by a reactor.
    Nuclear,
    /// Anything else: alcohol and oxygen, fluorine, boranes.
    Other,
}

impl Propellant {
    pub const ALL: [Propellant; 11] = [
        Propellant::LiquidFuel,
        Propellant::Kerolox,
        Propellant::Hydrolox,
        Propellant::Methalox,
        Propellant::Hypergolic,
        Propellant::Solid,
        Propellant::Monopropellant,
        Propellant::ColdGas,
        Propellant::Electric,
        Propellant::Nuclear,
        Propellant::Other,
    ];

    /// As actions write it.
    pub fn id(self) -> &'static str {
        match self {
            Propellant::LiquidFuel => "liquid_fuel",
            Propellant::Kerolox => "kerolox",
            Propellant::Hydrolox => "hydrolox",
            Propellant::Methalox => "methalox",
            Propellant::Hypergolic => "hypergolic",
            Propellant::Solid => "solid",
            Propellant::Monopropellant => "monopropellant",
            Propellant::ColdGas => "cold_gas",
            Propellant::Electric => "electric",
            Propellant::Nuclear => "nuclear",
            Propellant::Other => "other",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == text)
    }

    /// What the player reads.
    pub fn name(self) -> &'static str {
        match self {
            Propellant::LiquidFuel => "Liquid fuel",
            Propellant::Kerolox => "Kerolox",
            Propellant::Hydrolox => "Hydrolox",
            Propellant::Methalox => "Methalox",
            Propellant::Hypergolic => "Hypergolic",
            Propellant::Solid => "Solid",
            Propellant::Monopropellant => "Monoprop",
            Propellant::ColdGas => "Cold gas",
            Propellant::Electric => "Electric",
            Propellant::Nuclear => "Nuclear",
            Propellant::Other => "Other",
        }
    }

    /// The kind of what burns `resources` (resource ids) in an engine of `kind` (`liquid`, `solid`
    /// or `electric`), if it says.
    pub fn of<'a>(kind: Option<&str>, resources: impl IntoIterator<Item = &'a str>) -> Self {
        match kind {
            Some("solid") => return Propellant::Solid,
            Some("electric") => return Propellant::Electric,
            _ => {}
        }
        // Chilled propellants are the same propellants.
        let burns: Vec<&str> = (resources.into_iter())
            .map(|r| r.strip_prefix("Cooled").unwrap_or(r))
            .collect();
        let has = |names: &[&str]| burns.iter().any(|r| names.contains(r));
        let oxygen = has(&["LqdOxygen"]);
        let storable = (burns.iter()).any(|r| {
            ["NTO", "MON", "IRFNA", "IWFNA", "AK2"]
                .iter()
                .any(|s| r.starts_with(s))
        });
        if has(&["Liquid Fuel"]) {
            Propellant::LiquidFuel
        } else if has(&["EnrichedUranium"]) {
            Propellant::Nuclear
        } else if has(&["XenonGas", "KryptonGas", "ArgonGas"]) {
            Propellant::Electric
        } else if oxygen && has(&["LqdHydrogen"]) {
            Propellant::Hydrolox
        } else if oxygen && has(&["LqdMethane"]) {
            Propellant::Methalox
        } else if oxygen && has(&["RP-1", "Kerosene", "RG-1", "Syntin"]) {
            Propellant::Kerolox
        } else if storable {
            Propellant::Hypergolic
        } else if burns.len() == 1 && has(&["Hydrazine", "HTP"]) {
            Propellant::Monopropellant
        } else if burns.len() == 1 && has(&["Nitrogen", "Helium"]) {
            Propellant::ColdGas
        } else {
            Propellant::Other
        }
    }
}

/// One thing that can be put on a craft.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Unique in the palette: `tank`, `engine:<record id>` and so on.
    pub id: String,
    pub name: String,
    pub group: Group,
    /// The figures that matter for what it is, on a line: thrust and Isp for an engine, crew for
    /// a pod, size and volume for a tank.
    pub detail: String,
    /// What it is: who made it and what for, as its catalogue record says, or how a procedural
    /// part is sized.
    pub about: String,
    /// How wide it is where it goes on, m: a pod's base, an engine's nozzle exit. None for what
    /// takes the width of what it goes on (the procedural parts) or has none to speak of.
    pub width: Option<f64>,
    /// What it burns. None for what burns nothing, and for a tank, which holds what it is set to.
    pub propellant: Option<Propellant>,
    /// What a new part starts as, with no id.
    pub part: CraftPartDef,
    /// The node of its own it is held by on a surface, and how far it is turned about the joint
    /// there, degrees; none if it only goes on nodes.
    pub surface: Option<(String, f64)>,
    /// What a new part's id starts with.
    pub stem: String,
}

impl Entry {
    /// Whether `search` (any case) is in its name, its figures, what it is, what it burns or its
    /// group's name.
    pub fn matches(&self, search: &str) -> bool {
        let search = search.trim().to_lowercase();
        let burns = self.propellant.map_or("", Propellant::name);
        let texts = [
            self.name.as_str(),
            self.detail.as_str(),
            self.about.as_str(),
            burns,
            self.group.name(),
        ];
        search.is_empty() || texts.iter().any(|t| t.to_lowercase().contains(&search))
    }
}

fn part(value: serde_json::Value) -> CraftPartDef {
    serde_json::from_value(value).expect("the palette's own parts are well formed")
}

/// A force as the player reads it: `850 N`, `914 kN`, `7.78 MN`.
fn force(newtons: f64) -> String {
    let (value, unit) = if newtons >= 1e6 {
        (newtons / 1e6, "MN")
    } else if newtons >= 1e3 {
        (newtons / 1e3, "kN")
    } else {
        (newtons, "N")
    };
    format!("{} {unit}", three_figures(value))
}

/// A mass as the player reads it: `470 kg`, `8.44 t`.
fn mass(kg: f64) -> String {
    if kg >= 1e3 {
        format!("{} t", three_figures(kg / 1e3))
    } else {
        format!("{} kg", three_figures(kg))
    }
}

/// A length to the centimetre at most, without noughts at the end: `2 m`, `3.72 m`.
fn metres(m: f64) -> String {
    let text = format!("{m:.2}");
    format!("{} m", text.trim_end_matches('0').trim_end_matches('.'))
}

/// `value` to three figures, without noughts at the end after the point.
fn three_figures(value: f64) -> String {
    let decimals = match value {
        v if v >= 100.0 => 0,
        v if v >= 10.0 => 1,
        _ => 2,
    };
    let text = format!("{value:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

/// Who made a catalogue part and what it is, as its record says.
fn about(info: &CatalogInfo) -> String {
    let said: Vec<&str> = [info.manufacturer.as_deref(), info.description.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    said.join(". ")
}

/// The paint of a new procedural part: white, as most rockets are.
const WHITE: [f64; 3] = [0.85, 0.85, 0.83];
const GREY: [f64; 3] = [0.55, 0.57, 0.6];

/// The procedural parts, in the order they are listed, each with its size as it starts, which its
/// figures begin with.
fn procedural(catalog: &PartCatalog) -> Vec<(Entry, &'static str)> {
    let new_tank = crate::properties::new_tank(catalog);
    let entry =
        |id: &str, name: &str, group, about: &str, value, surface: Option<(&str, f64)>| Entry {
            id: id.into(),
            name: name.into(),
            group,
            detail: String::new(),
            about: about.into(),
            width: None,
            propellant: None,
            part: part(value),
            surface: surface.map(|(node, roll)| (node.to_owned(), roll)),
            stem: id.into(),
        };
    vec![
        (
            entry(
                "tank",
                "Tank",
                Group::Tanks,
                &format!(
                    "{}, or what it is set to hold. As wide as what it goes on",
                    new_tank.label
                ),
                json!({
                    "id": "", "name": "Tank",
                    "shape": { "cylinder_tank": { "diameter": 2.0, "length": 4.0 } },
                    "components": [{ "storage": {
                        "type": "Stiffened Al 2219",
                        "contents": new_tank.contents,
                    } }],
                    "paint": { "color": WHITE },
                }),
                None,
            ),
            "2 m × 4 m",
        ),
        (
            entry(
                "structural",
                "Structural",
                Group::Structure,
                "A tube: an interstage or a skirt. As wide as what it goes on",
                json!({
                    "id": "", "name": "Structural",
                    "shape": { "shell": { "bottom_diameter": 2.0, "sections": [{ "length": 1.0 }] } },
                    "structure": { "material": "Al 2219-T87", "thickness": 0.004 },
                    "paint": { "color": WHITE },
                }),
                None,
            ),
            "2 m × 1 m",
        ),
        (
            entry(
                "nose_cone",
                "Nose cone",
                Group::Structure,
                "An ogive, closed at its base. As wide as what it goes on",
                json!({
                    "id": "", "name": "Nose cone",
                    "shape": { "shell": {
                        "bottom_diameter": 2.0,
                        "sections": [{ "length": 3.0, "top_diameter": 0.0, "profile": "ogive" }],
                        "cap_bottom": true,
                    } },
                    "structure": { "material": "Al 2219-T87", "thickness": 0.003 },
                    "paint": { "color": WHITE },
                }),
                None,
            ),
            "2 m × 3 m",
        ),
        (
            entry(
                "decoupler",
                "Decoupler",
                Group::Structure,
                "Lets go of what is below it. As wide as what it goes on",
                json!({
                    "id": "", "name": "Decoupler",
                    "procedural": { "decoupler": { "diameter": 2.0, "load": 1e6 } },
                }),
                None,
            ),
            "2 m",
        ),
        (
            entry(
                "fin",
                "Fin",
                Group::Aero,
                "A flat plate, swept. Sized for the hull it stands on",
                json!({
                    "id": "", "name": "Fin",
                    "shape": { "wing": {
                        "root_chord": 1.6, "tip_chord": 0.7, "span": 0.9, "sweep": 0.8,
                        "airfoil": { "plate": { "thickness": 0.02 } },
                    } },
                    "structure": { "material": "Al 2219-T87" },
                    "components": [{ "lifting_surface": {} }],
                    "paint": { "color": GREY, "finish": "metal" },
                }),
                Some(("root", 0.0)),
            ),
            "0.9 m span",
        ),
        (
            entry(
                "wing",
                "Wing",
                Group::Aero,
                "An airfoil, tapered and swept. Sized for the hull it stands on",
                json!({
                    "id": "", "name": "Wing",
                    "shape": { "wing": {
                        "root_chord": 3.0, "tip_chord": 1.2, "span": 4.0, "sweep": 1.5,
                    } },
                    "structure": { "material": "Al 2219-T87" },
                    "components": [{ "lifting_surface": {} }],
                    "paint": { "color": WHITE },
                }),
                Some(("root", 0.0)),
            ),
            "4 m span",
        ),
        // A strut down from where it is put, a wheel at its end that rolls along the craft.
        (
            entry(
                "wheel",
                "Wheel",
                Group::Aero,
                "Landing gear with brakes, for a plane's belly. It rolls along the craft",
                json!({
                    "id": "", "name": "Wheel",
                    "shape": { "compound": [{
                        "shape": { "beam": { "length": 1.2, "section": { "circle": { "diameter": 0.1 } } } },
                        "rotation": [180, 0, 0],
                    }] },
                    "components": [{ "gear": {
                        "length": 1.2, "wheel_radius": 0.3, "stroke": 0.25, "load": 5000.0,
                        "mass": 60.0, "brakes": true,
                    } }],
                    "paint": { "color": GREY, "finish": "metal" },
                }),
                Some(("bottom", 0.0)),
            ),
            "1.2 m strut",
        ),
        (
            entry(
                "nose_wheel",
                "Nose wheel",
                Group::Aero,
                "Landing gear that steers, for under a plane's nose. It rolls along the craft",
                json!({
                    "id": "", "name": "Nose wheel",
                    "shape": { "compound": [{
                        "shape": { "beam": { "length": 1.2, "section": { "circle": { "diameter": 0.1 } } } },
                        "rotation": [180, 0, 0],
                    }] },
                    "components": [{ "gear": {
                        "length": 1.2, "wheel_radius": 0.25, "stroke": 0.25, "load": 3000.0,
                        "mass": 40.0, "steering": 25.0,
                    } }],
                    "paint": { "color": GREY, "finish": "metal" },
                }),
                Some(("bottom", 0.0)),
            ),
            "1.2 m strut",
        ),
    ]
}

/// A procedural part's figures as it starts, `size` first: what it weighs, and what a tank holds.
fn built_figures(part: &CraftPartDef, size: &str, catalog: &PartCatalog) -> String {
    let mut part = part.clone();
    part.id = "alone".into();
    let alone = CraftDef {
        id: "alone".into(),
        name: String::new(),
        parts: vec![part],
        sequences: None,
        triggers: Vec::new(),
        rcs_shut: Vec::new(),
        abort: Vec::new(),
    };
    let mut figures = vec![size.to_owned()];
    if let Ok(craft) = Craft::build(&alone, catalog)
        && let Some(built) = craft.parts.first().map(|p| &p.built)
    {
        let volume: f64 = built.storages().map(|s| s.volume).sum();
        if volume > 0.0 {
            figures.push(format!("{} m³", three_figures(volume)));
        }
        figures.push(mass(built.mass.mass));
    }
    figures.join(" · ")
}

/// Everything a craft can be built from, `catalog`'s records after the procedural parts, each group
/// by name.
pub fn entries(catalog: &PartCatalog) -> Vec<Entry> {
    let template = |id: &str, name: &str| part(json!({ "id": "", "name": name, "template": id }));
    let mut out: Vec<Entry> = (procedural(catalog).into_iter())
        .map(|(mut entry, size)| {
            entry.detail = built_figures(&entry.part, size, catalog);
            entry
        })
        .collect();
    let mut pods: Vec<Entry> = (catalog.pods.iter())
        .map(|pod| Entry {
            id: format!("pod:{}", pod.id),
            name: pod.id.clone(),
            group: Group::Pods,
            detail: [
                format!("{} crew", pod.crew),
                metres(pod.diameter),
                mass(pod.mass),
            ]
            .join(" · "),
            about: about(&pod.catalog),
            width: Some(pod.diameter),
            propellant: None,
            part: template(&pod.id, &pod.id),
            surface: None,
            stem: "pod".into(),
        })
        .collect();
    let mut engines: Vec<Entry> = (catalog.engines.iter())
        .map(|engine| {
            // Not a title still waiting to be translated (`#roBE3UTitle`).
            let title = (engine.catalog.title.clone()).filter(|t| !t.starts_with('#'));
            let name = title.unwrap_or_else(|| engine.id.clone());
            let burns = engine.propellants.keys().map(String::as_str);
            let propellant = Propellant::of(engine.kind.as_deref(), burns);
            let mut detail = vec![
                force(engine.thrust_vac),
                format!(
                    "Isp {:.0} s, {:.0} s at sea level",
                    engine.isp_vac, engine.isp_sl
                ),
                propellant.name().to_owned(),
            ];
            if engine.exit_diameter > 0.0 {
                detail.push(metres(engine.exit_diameter));
            }
            detail.push(mass(engine.mass));
            Entry {
                id: format!("engine:{}", engine.id),
                detail: detail.join(" · "),
                about: about(&engine.catalog),
                width: (engine.exit_diameter > 0.0).then_some(engine.exit_diameter),
                propellant: Some(propellant),
                part: template(&engine.id, &name),
                name,
                group: Group::Engines,
                surface: None,
                stem: "engine".into(),
            }
        })
        .collect();
    // RCS blocks stand with +Y towards the nose: turned a quarter about the joint from how a
    // surface holds them.
    let mut control: Vec<Entry> = (catalog.rcs.iter())
        .map(|rcs| {
            let burns = rcs.propellants.keys().map(String::as_str);
            let propellant = Propellant::of(None, burns);
            Entry {
                id: format!("rcs:{}", rcs.id),
                name: rcs.id.clone(),
                group: Group::Control,
                detail: [
                    format!("RCS, {} a nozzle", force(rcs.thrust)),
                    format!("Isp {:.0} s", rcs.isp_vac),
                    propellant.name().to_owned(),
                    mass(rcs.mass),
                ]
                .join(" · "),
                about: String::new(),
                width: None,
                propellant: Some(propellant),
                part: template(&rcs.id, &rcs.id),
                surface: Some(("mount".into(), 90.0)),
                stem: "rcs".into(),
            }
        })
        .chain(catalog.reaction_wheels.iter().map(|wheel| Entry {
            id: format!("wheel:{}", wheel.id),
            name: wheel.id.clone(),
            group: Group::Control,
            detail: format!(
                "Reaction wheel, {} N·m · {}",
                three_figures(wheel.torque),
                mass(wheel.mass)
            ),
            about: String::new(),
            width: None,
            propellant: None,
            part: template(&wheel.id, &wheel.id),
            surface: None,
            stem: "reaction_wheel".into(),
        }))
        .collect();
    for list in [&mut pods, &mut engines, &mut control] {
        list.sort_by_key(|entry| entry.name.to_lowercase());
        out.append(list);
    }
    out
}

/// `part`, a palette entry's procedural part, sized for what it goes on, `width` across: the node
/// it is held at, or the hull it stands on. A part from a catalogue record stays as it is.
pub fn fit(part: &mut CraftPartDef, width: f64) {
    if !(width.is_finite() && width > 0.0) {
        return;
    }
    if let Some(ProceduralDef::Decoupler(decoupler)) = &mut part.procedural {
        decoupler.diameter = width;
        return;
    }
    let stand_alone = part.template.is_none() && part.procedural.is_none();
    let Some(shape) = part.shape.as_mut().filter(|_| stand_alone) else {
        return;
    };
    let sizes: Vec<(&str, f64)> = match shape {
        // Longer first, so its domes have room for the width.
        ShapeDef::CylinderTank(_) => vec![("length", 3.0 * width), ("diameter", width)],
        ShapeDef::Shell(d) if d.sections.last().and_then(|s| s.top_diameter) == Some(0.0) => {
            vec![("length", 1.5 * width), ("bottom_diameter", width)]
        }
        ShapeDef::Shell(_) => vec![("length", 0.5 * width), ("bottom_diameter", width)],
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

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> PartCatalog {
        crate::vessel::tests::catalog()
    }

    #[test]
    fn every_entry_builds_by_itself() {
        let catalog = catalog();
        let all = entries(&catalog);
        assert!(all.iter().any(|e| e.group == Group::Engines));
        for entry in &all {
            let mut part = entry.part.clone();
            part.id = "only".into();
            let craft = CraftDef {
                id: "test".into(),
                name: String::new(),
                parts: vec![part],
                sequences: None,
                triggers: Vec::new(),
                rcs_shut: Vec::new(),
                abort: Vec::new(),
            };
            Craft::build(&craft, &catalog).unwrap_or_else(|e| panic!("{}: {e}", entry.id));
        }
        let mut ids: Vec<&str> = all.iter().map(|e| e.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "ids are unique");
    }

    #[test]
    fn a_procedural_part_takes_the_width_it_goes_on() {
        let all = entries(&catalog());
        let fitted = |id: &str, width: f64| {
            let mut part = all.iter().find(|e| e.id == id).unwrap().part.clone();
            fit(&mut part, width);
            part
        };
        let sizes = |part: &CraftPartDef| -> Vec<(&'static str, f64)> {
            shaping::part_sizes(part)
                .iter()
                .map(|s| (s.name, s.value))
                .collect()
        };
        assert_eq!(
            sizes(&fitted("tank", 3.0)),
            [("length", 9.0), ("diameter", 3.0)]
        );
        assert!(sizes(&fitted("nose_cone", 3.0)).contains(&("bottom_diameter", 3.0)));
        let decoupler = fitted("decoupler", 3.0);
        assert!(
            matches!(&decoupler.procedural, Some(ProceduralDef::Decoupler(d)) if d.diameter == 3.0)
        );
        let fin = sizes(&fitted("fin", 2.0));
        assert!(fin.contains(&("span", 1.6)) && fin.contains(&("root_chord", 2.4)));
        // A catalogue part stays as it was.
        let engine = all.iter().find(|e| e.group == Group::Engines).unwrap();
        let mut copy = engine.part.clone();
        fit(&mut copy, 9.0);
        assert_eq!(copy, engine.part);
    }

    /// A 4 m tank standing up, with `entry` held on its barrel's side facing +X, by the node and
    /// turn the palette gives: where that part sits.
    fn on_the_side(entry: &str) -> crate::assembly::Pose {
        let catalog = catalog();
        let all = entries(&catalog);
        let tank = all.iter().find(|e| e.id == "tank").unwrap();
        let held = all.iter().find(|e| e.id == entry).unwrap();
        let (by, roll) = held.surface.clone().unwrap();
        let mut root = tank.part.clone();
        root.id = "tank".into();
        let mut part = held.part.clone();
        part.id = "held".into();
        part.on = Some("tank.barrel".into());
        part.by = by;
        part.roll = roll;
        part.at = Some(crate::data::SurfaceAtDef {
            u: 0.0,
            v: 0.5,
            side: Default::default(),
            standoff: 0.0,
        });
        let def = CraftDef {
            id: "test".into(),
            name: String::new(),
            parts: vec![root, part],
            sequences: None,
            triggers: Vec::new(),
            rcs_shut: Vec::new(),
            abort: Vec::new(),
        };
        let craft = Craft::build(&def, &catalog).unwrap();
        craft.assembly.poses()[1]
    }

    #[test]
    fn a_fin_on_a_hull_reaches_out_with_its_leading_edge_up() {
        use glam::DVec3;
        let fin = on_the_side("fin");
        // On the wall, 1 m out from the axis.
        assert!((fin.position.x - 1.0).abs() < 1e-6, "{:?}", fin.position);
        // Its span (+X) out, its leading edge (+Z, the chord runs back along -Z) towards the nose.
        assert!((fin.rotation * DVec3::X).dot(DVec3::X) > 0.999);
        assert!((fin.rotation * DVec3::Z).dot(DVec3::Z) > 0.999);
    }

    #[test]
    fn an_rcs_block_on_a_hull_faces_out_with_its_top_towards_the_nose() {
        use glam::DVec3;
        let rcs = on_the_side("rcs:Cold Gas Quad");
        assert!((rcs.rotation * DVec3::X).dot(DVec3::X) > 0.999);
        assert!((rcs.rotation * DVec3::Y).dot(DVec3::Z) > 0.999);
    }

    #[test]
    fn a_point_on_a_wall_finds_the_coordinates_it_came_from() {
        let geometry = ShapeDef::CylinderTank(crate::geometry::preset::CylinderTankDef {
            diameter: 3.0,
            length: 8.0,
            dome: std::f64::consts::FRAC_1_SQRT_2,
            common_bulkhead: None,
            aft_skirt: 1.0,
            forward_skirt: 0.5,
            thrust_plate: false,
        })
        .build()
        .unwrap();
        for surface in geometry.surfaces() {
            for (u, v) in [(0.1, 0.2), (0.6, 0.5), (0.95, 0.9)] {
                let p = geometry.surface_point(&surface, u, v).unwrap();
                // Just off the wall, as a pointer touching its drawn mesh would be.
                let (found, fu, fv, off) = geometry
                    .surface_near(p.position + p.normal * 0.001)
                    .unwrap();
                assert_eq!(found, surface);
                assert!(
                    (fu - u).abs() < 1e-6 && (fv - v).abs() < 1e-3,
                    "{surface} {fu} {fv}"
                );
                assert!(off < 0.002, "{surface}: {off}");
            }
        }
    }

    #[test]
    fn a_search_finds_by_name_detail_or_group_in_any_case() {
        let all = entries(&catalog());
        let tank = all.iter().find(|e| e.id == "tank").unwrap();
        assert!(tank.matches("TAN") && tank.matches("oxygen") && tank.matches("tanks"));
        assert!(tank.matches("  ") && !tank.matches("wheel"));
        assert_eq!(Group::parse(Group::Engines.id()), Some(Group::Engines));
        // What an engine burns is found by its kind's name.
        let engine = all.iter().find(|e| e.group == Group::Engines).unwrap();
        let burns = engine.propellant.unwrap().name();
        assert!(engine.matches(&burns.to_uppercase()));
    }

    #[test]
    fn each_part_is_listed_by_what_it_does_and_says_its_figures() {
        let all = entries(&catalog());
        let group = |id: &str| all.iter().find(|e| e.id == id).unwrap().group;
        assert_eq!(group("tank"), Group::Tanks);
        assert_eq!(group("decoupler"), Group::Structure);
        assert_eq!(group("fin"), Group::Aero);
        let tank = all.iter().find(|e| e.id == "tank").unwrap();
        assert!(tank.detail.starts_with("2 m × 4 m · ") && tank.detail.contains("m³"));
        assert_eq!((tank.width, tank.propellant), (None, None));
        let engine = all.iter().find(|e| e.group == Group::Engines).unwrap();
        assert!(engine.detail.contains("Isp") && engine.width.is_some());
        let pod = all.iter().find(|e| e.group == Group::Pods).unwrap();
        assert!(pod.detail.contains("crew") && pod.width.is_some());
    }

    #[test]
    fn propellants_are_known_by_their_kind() {
        let of = |kind: Option<&str>, burns: &[&str]| Propellant::of(kind, burns.iter().copied());
        assert_eq!(
            of(None, &["Liquid Fuel", "Oxidiser"]),
            Propellant::LiquidFuel
        );
        assert_eq!(of(None, &["LqdOxygen", "RP-1"]), Propellant::Kerolox);
        assert_eq!(
            of(None, &["CooledLqdOxygen", "CooledRG-1"]),
            Propellant::Kerolox
        );
        assert_eq!(
            of(None, &["LqdHydrogen", "LqdOxygen"]),
            Propellant::Hydrolox
        );
        assert_eq!(of(None, &["LqdMethane", "LqdOxygen"]), Propellant::Methalox);
        assert_eq!(of(None, &["NTO", "UDMH"]), Propellant::Hypergolic);
        assert_eq!(of(None, &["MMH", "MON3"]), Propellant::Hypergolic);
        assert_eq!(of(None, &["AK27", "Kerosene"]), Propellant::Hypergolic);
        assert_eq!(of(Some("solid"), &["HTPB"]), Propellant::Solid);
        assert_eq!(of(Some("electric"), &["XenonGas"]), Propellant::Electric);
        assert_eq!(
            of(None, &["EnrichedUranium", "LqdHydrogen"]),
            Propellant::Nuclear
        );
        assert_eq!(of(None, &["Hydrazine"]), Propellant::Monopropellant);
        assert_eq!(of(None, &["Nitrogen"]), Propellant::ColdGas);
        assert_eq!(of(None, &["Ethanol75", "LqdOxygen"]), Propellant::Other);
        for kind in Propellant::ALL {
            assert_eq!(Propellant::parse(kind.id()), Some(kind));
        }
    }

    #[test]
    fn figures_read_as_a_player_would_say_them() {
        assert_eq!(force(914_000.0), "914 kN");
        assert_eq!(force(7_775_490.0), "7.78 MN");
        assert_eq!(force(445.0), "445 N");
        assert_eq!(mass(470.0), "470 kg");
        assert_eq!(mass(8444.1), "8.44 t");
        assert_eq!(metres(2.0), "2 m");
        assert_eq!(metres(3.723), "3.72 m");
        assert_eq!(three_figures(12.0), "12");
    }
}
