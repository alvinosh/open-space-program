//! The palette: what a player can put on a craft in the editor. A few procedural parts (a tank, a
//! structural tube, a nose cone, a decoupler, a fin, a wing and landing wheels) whose sizes the
//! player then sets,
//! as in KSP with its procedural parts, and the catalogue's pods, engines, RCS blocks and reaction
//! wheels as they are. Each is listed by what it does ([`Group`], KSP's function filter) and says
//! what else the parts list narrows it by: how wide it is ([`Entry::width`]) and what it burns
//! ([`Propellant`]).
//!
//! An entry is a part to copy ([`Entry::part`], with no id: the editor names it). A procedural one
//! is fitted to what it goes on ([`crate::fitting`]), so a tank put under a 3 m tank is 3 m across,
//! one on an engine feeds it, and a fin is sized for the hull it stands on. What goes on a surface says by which of its own nodes, and how far it
//! is turned about the joint so that it stands the right way up ([`Entry::surface`]).

use std::f64::consts::FRAC_1_SQRT_2;

use serde_json::json;

use crate::{
    comms,
    craft::Craft,
    data::{CatalogInfo, CraftDef, CraftPartDef, PartCatalog},
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
    /// Power, radios and what commands a vessel with no crew.
    Systems,
    /// Scientific instruments.
    Science,
}

impl Group {
    pub const ALL: [Group; 8] = [
        Group::Pods,
        Group::Tanks,
        Group::Engines,
        Group::Structure,
        Group::Aero,
        Group::Control,
        Group::Systems,
        Group::Science,
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
            Group::Systems => "systems",
            Group::Science => "science",
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
            Group::Systems => "Power and comms",
            Group::Science => "Science",
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

/// The tank type a solid booster's case is built as, and what it holds.
pub const BOOSTER_CASE: &str = "Booster Case";

/// A solid booster as the parts list gives it: a case 1.25 m across and 7.5 m long full of the
/// grain its tank type holds (Solid Fuel, or a realistic one), lit once with 400 kN in a vacuum,
/// burning evenly for about a minute and a half. None if no tank type can hold its grain.
fn solid_booster(catalog: &PartCatalog) -> Option<Entry> {
    use crate::component::{BOOSTER_ISP, BOOSTER_NOZZLE_MASS};
    let case = catalog.tank_types.iter().find(|t| t.id == BOOSTER_CASE)?;
    let grain = case.propellants.as_ref()?.first()?.clone();
    let thrust = 400e3;
    let [isp_vac, isp_sl] = BOOSTER_ISP[0];
    let value = json!({
        "id": "", "name": "Solid booster",
        // Skirts to its domes' tips, a cylinder outside; the nozzle hangs under its base.
        "shape": { "cylinder_tank": {
            "diameter": 1.25, "length": 7.5,
            "aft_skirt": 0.625 * FRAC_1_SQRT_2, "forward_skirt": 0.625 * FRAC_1_SQRT_2,
        } },
        "components": [
            { "storage": { "type": BOOSTER_CASE, "contents": { grain.clone(): 1.0 } } },
            { "solid_motor": {
                "thrust": thrust, "isp_vac": isp_vac, "isp_sl": isp_sl,
                "mass": BOOSTER_NOZZLE_MASS * thrust, "propellant": grain, "booster": true,
            } },
        ],
        "paint": { "color": WHITE, "trim": true },
    });
    Some(Entry {
        id: "solid_booster".into(),
        name: "Solid booster".into(),
        group: Group::Engines,
        detail: String::new(),
        about:
            "A case of solid propellant and its nozzle in one part. Put under a craft it is sized \
                for what it lifts: a first hop's worth of grain and thrust to match. Lit once, \
                it burns until it is empty: nothing stops it"
                .into(),
        width: None,
        propellant: Some(Propellant::Solid),
        part: part(value),
        surface: None,
        stem: "solid_booster".into(),
    })
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
                    // Skirts down to its domes' tips (√2:1, 0.71 m on 2 m): a cylinder outside.
                    "shape": { "cylinder_tank": {
                        "diameter": 2.0, "length": 4.0,
                        "aft_skirt": FRAC_1_SQRT_2, "forward_skirt": FRAC_1_SQRT_2,
                    } },
                    "components": [{ "storage": {
                        "type": "Stiffened Al 2219",
                        "contents": new_tank.contents,
                    } }],
                    "paint": { "color": WHITE, "trim": true },
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
                    "paint": { "color": WHITE, "trim": true },
                }),
                None,
            ),
            "2 m × 1 m",
        ),
        (
            entry(
                "fuel_line",
                "Fuel line",
                Group::Structure,
                "Carries propellant one way, from the tank it is on to the part you click next",
                json!({
                    "id": "", "name": "Fuel line",
                    "shape": { "compound": [
                        {
                            "name": "body",
                            "shape": { "shell": {
                                "bottom_diameter": 0.18,
                                "sections": [{ "length": 0.17 }],
                                "cap_bottom": true,
                                "cap_top": true,
                            } },
                            "unseen": true,
                        },
                        {
                            "name": "fitting",
                            "shape": { "model": { "path": "models/control/fuel_line.glb" } },
                        },
                    ] },
                    "nodes": [{
                        "name": "bottom",
                        "position": [0.0, 0.0, 0.0],
                        "direction": [0.0, 0.0, -1.0],
                        "size": 0.18,
                    }],
                    "structure": { "mass": 8.0 },
                    "components": [{ "fuel_line": {} }],
                    "paint": { "color": GREY, "finish": "metal" },
                }),
                Some(("bottom", 0.0)),
            ),
            "8 kg",
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
                    "paint": { "color": WHITE, "trim": true },
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
                    "paint": { "color": [0.035, 0.04, 0.05], "trim": true },
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
                    "paint": { "color": GREY, "finish": "metal", "trim": true },
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
                    "paint": { "color": WHITE, "trim": true },
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
        (
            entry(
                "probe_core",
                "Probe core",
                Group::Pods,
                "Commands a vessel with no crew. It needs power and a link to the ground to answer",
                json!({
                    "id": "", "name": "Probe core",
                    "shape": { "ring": { "diameter": 0.6, "height": 0.2, "thickness": 0.3 } },
                    "model": { "path": "models/systems/probe_core.glb" },
                    "structure": { "mass": 30.0 },
                    "components": [
                        { "avionics": {
                            "controlled_mass": 20000.0, "interplanetary": true, "needs_link": true,
                        } },
                        { "drive": { "data": 2e6 } },
                    ],
                }),
                None,
            ),
            "0.6 m",
        ),
        (
            entry(
                "solar_panel",
                "Solar panel",
                Group::Systems,
                "Fixed to the hull, making power in sunlight when it faces the Sun",
                json!({
                    "id": "", "name": "Solar panel",
                    "procedural": { "solar_array": { "power": 200.0 } },
                    "model": { "path": "models/systems/solar_panel.glb", "fit": 1.0 },
                    "nodes": [{
                        "name": "mount",
                        "position": [-0.01, 0.0, 0.0],
                        "direction": [-1.0, 0.0, 0.0],
                        "size": 0.2,
                    }],
                }),
                Some(("mount", 0.0)),
            ),
            "200 W",
        ),
        (
            entry(
                "rtg",
                "RTG",
                Group::Systems,
                "Steady power anywhere, day or night, for decades",
                json!({
                    "id": "", "name": "RTG",
                    "procedural": { "rtg": { "power": 100.0 } },
                    "model": { "path": "models/systems/rtg.glb", "fit": 1.0 },
                }),
                None,
            ),
            "100 W",
        ),
        (
            entry(
                "parachute",
                "Parachute",
                Group::Systems,
                "Opens by itself once armed and low enough, the last thing staged. Size the canopy \
                 for what it brings down: a one-seat capsule wants about 24 m, a sounding \
                 rocket's nose a few",
                json!({
                    "id": "", "name": "Parachute",
                    "procedural": { "parachute": { "diameter": 24.0, "deploy_altitude": 3000.0 } },
                    "nodes": [{
                        "name": "bottom",
                        "position": [0.0, 0.0, 0.0],
                        "direction": [0.0, 0.0, -1.0],
                        "size": 0.4,
                    }],
                    "paint": { "color": [0.85, 0.45, 0.15], "finish": "paint" },
                }),
                Some(("bottom", 0.0)),
            ),
            "24 m",
        ),
        (
            entry(
                "telemetry",
                "Telemetry unit",
                Group::Systems,
                "A sounding rocket's transmitter on its own battery: it sends what the instruments \
                 read to the ground, nearby, and hears no commands",
                json!({
                    "id": "", "name": "Telemetry unit",
                    "shape": { "box": { "size": [0.2, 0.14, 0.18] } },
                    "structure": { "mass": 10.0 },
                    "components": [
                        { "antenna": { "power": 1.0, "data_rate": 512.0, "telemetry": true } },
                        { "battery": { "charge": 40.0 } },
                    ],
                    "nodes": [{
                        "name": "bottom",
                        "position": [0.0, 0.0, 0.0],
                        "direction": [0.0, 0.0, -1.0],
                        "size": 0.2,
                    }],
                    "paint": { "color": [0.35, 0.38, 0.42], "finish": "metal" },
                }),
                Some(("bottom", 0.0)),
            ),
            "512 bit/s",
        ),
        (
            entry(
                "antenna",
                "Antenna",
                Group::Systems,
                &reach("A whip that sends all ways, for short range", 0.0, 2.0, 1000.0),
                antenna_part("Antenna", 0.0, 2.0, 1000.0, false),
                Some(("bottom", 0.0)),
            ),
            "2 W",
        ),
        (
            entry(
                "small_dish",
                "Small dish",
                Group::Systems,
                &reach("Light, for near the Earth", 0.5, 5.0, 2000.0),
                antenna_part("Small dish", 0.5, 5.0, 2000.0, false),
                Some(("bottom", 0.0)),
            ),
            "0.5 m",
        ),
        (
            entry(
                "dish",
                "Dish",
                Group::Systems,
                &reach("For the Moon and the planets nearby", 1.0, 20.0, 10000.0),
                antenna_part("Dish", 1.0, 20.0, 10000.0, false),
                Some(("bottom", 0.0)),
            ),
            "1 m",
        ),
        (
            entry(
                "large_dish",
                "Large dish",
                Group::Systems,
                &reach("Deep space, at a good deal of power", 3.0, 60.0, 10000.0),
                antenna_part("Large dish", 3.0, 60.0, 10000.0, false),
                Some(("bottom", 0.0)),
            ),
            "3 m",
        ),
        (
            entry(
                "relay_dish",
                "Relay dish",
                Group::Systems,
                &reach(
                    "Passes other vessels' signals to the ground, or on to the next relay",
                    2.0,
                    40.0,
                    10000.0,
                ),
                antenna_part("Relay dish", 2.0, 40.0, 10000.0, true),
                Some(("bottom", 0.0)),
            ),
            "2 m",
        ),
        (
            entry(
                "deep_relay",
                "Deep space relay",
                Group::Systems,
                &reach("A relay for the far planets. Heavy and hungry", 5.0, 150.0, 10000.0),
                antenna_part("Deep space relay", 5.0, 150.0, 10000.0, true),
                Some(("bottom", 0.0)),
            ),
            "5 m",
        ),
    ]
    .into_iter()
    .chain(systems(catalog))
    .collect()
}

/// An antenna of the parts list: a whip with no `diameter`, and a dish, its model scaled to it.
fn antenna_part(
    name: &str,
    diameter: f64,
    power: f64,
    data_rate: f64,
    relay: bool,
) -> serde_json::Value {
    let (model, fit, mount) = if relay {
        ("models/systems/relay_dish.glb", 1.0, 0.32)
    } else if diameter > 0.0 {
        ("models/systems/dish.glb", 1.0, 0.16)
    } else {
        ("models/systems/antenna_omni.glb", 0.1, 0.1)
    };
    json!({
        "id": "", "name": name,
        "procedural": { "antenna": {
            "diameter": diameter, "power": power, "data_rate": data_rate, "relay": relay,
        } },
        "model": { "path": model, "fit": fit },
        "nodes": [{
            "name": "bottom",
            "position": [0.0, 0.0, 0.0],
            "direction": [0.0, 0.0, -1.0],
            "size": mount * diameter.max(1.0),
        }],
    })
}

/// What an antenna is for, and how far it reaches a 26 m ground station.
fn reach(about: &str, diameter: f64, power: f64, data_rate: f64) -> String {
    let gain = if diameter > 0.0 {
        comms::dish_gain(diameter, comms::S_BAND)
    } else {
        1.0
    };
    let radio = comms::Radio {
        power,
        gain,
        data_rate,
    };
    let metres = comms::reach(&radio, comms::dish_gain(26.0, comms::S_BAND), 50.0);
    let distance = if metres < 1e9 {
        format!("{:.0} thousand km", metres / 1e6)
    } else {
        format!("{:.1} million km", metres / 1e9)
    };
    format!(
        "{about}. Reaches the ground from {distance}, drawing {:.0} W",
        4.0 * power + 4.0 * diameter
    )
}

/// The parts of the list that need records or resources not every game has.
fn systems(catalog: &PartCatalog) -> Vec<(Entry, &'static str)> {
    let entry = |id: &str, name: &str, about: &str, value| Entry {
        id: id.into(),
        name: name.into(),
        group: Group::Systems,
        detail: String::new(),
        about: about.into(),
        width: None,
        propellant: None,
        part: part(value),
        surface: None,
        stem: id.into(),
    };
    let mut out = Vec::new();
    if let Some(battery) = catalog.batteries.iter().find(|b| b.rechargeable) {
        out.push((
            entry(
                "battery",
                "Battery",
                "Stores power for the dark, and for when more is drawn than is made",
                json!({
                    "id": "", "name": "Battery",
                    "procedural": { "battery": {
                        "energy": 3600.0, "technology": battery.id, "rechargeable": true,
                    } },
                    "model": { "path": "models/systems/battery.glb", "fit": 1.0 },
                }),
            ),
            "1 kWh",
        ));
    }
    // What the crew live on, as the rules have it: a day's worth of each in proportion, in a tank
    // that keeps the coldest of them. What they breathe from the cabin's air is what tops it up.
    if let Ok(Some(profile)) = catalog.life_support_profile() {
        let supplies: Vec<(&str, f64)> = (profile.needs.iter())
            .map(|need| (profile.supply_of(&need.resource), need.daily))
            .collect();
        let contents: serde_json::Map<String, serde_json::Value> = (supplies.iter())
            .map(|(resource, daily)| ((*resource).to_owned(), json!(daily)))
            .collect();
        let cold = supplies.iter().any(|(resource, _)| {
            (catalog.resources.iter()).any(|r| r.id == *resource && r.temperature < 250.0)
        });
        let tank_type = if cold {
            "Insulated Al-Li"
        } else {
            "Stiffened Al 2219"
        };
        if catalog.tank_types.iter().any(|t| t.id == tank_type) {
            out.push((
                entry(
                    "supplies",
                    "Supplies",
                    "What the crew live on, for the cabins it is joined to",
                    json!({
                        "id": "", "name": "Supplies",
                        "shape": { "cylinder_tank": { "diameter": 1.0, "length": 1.0 } },
                        "components": [{ "storage": { "type": tank_type, "contents": contents } }],
                        "paint": { "color": WHITE, "trim": false },
                    }),
                ),
                "1 m × 1 m",
            ));
        }
    }
    let has = |name: &str| catalog.resources.iter().any(|r| r.id == name);
    if has("LqdHydrogen") && has("LqdOxygen") {
        out.push((
            entry(
                "fuel_cell",
                "Fuel cell",
                "Makes power from the hydrogen and oxygen it can reach",
                json!({
                    "id": "", "name": "Fuel cell",
                    "procedural": { "fuel_cell": { "power": 1000.0 } },
                    "model": { "path": "models/systems/fuel_cell.glb", "fit": 1.0 },
                }),
            ),
            "1 kW",
        ));
    }
    if let Some(unit) = catalog.avionics.first() {
        out.push((
            entry(
                "avionics",
                "Avionics ring",
                "Guidance rated for so much mass, in place of a probe core",
                json!({
                    "id": "", "name": "Avionics ring",
                    "procedural": { "avionics": {
                        "controlled_mass": 10000.0, "technology": unit.id,
                        "diameter": 1.0, "height": 0.3,
                    } },
                    "paint": { "color": GREY, "finish": "metal", "trim": true },
                }),
            ),
            "1 m",
        ));
    }
    out
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
/// The key a value is written under: an externally tagged enum's variant (`cylinder_tank`, `storage`).
fn tag<T: serde::Serialize>(value: &T) -> Option<String> {
    match serde_json::to_value(value).ok()? {
        serde_json::Value::Object(map) if map.len() == 1 => map.keys().next().cloned(),
        serde_json::Value::String(text) => Some(text),
        _ => None,
    }
}

/// How a part is made, to tell which entry it came from: what sizes it, the kind of its shape and the
/// kinds of its components.
fn made_as(part: &CraftPartDef) -> (Option<String>, Option<String>, Vec<String>) {
    let mut components: Vec<String> = part.components.iter().filter_map(tag).collect();
    components.sort();
    (
        part.procedural.as_ref().and_then(tag),
        part.shape.as_ref().and_then(tag),
        components,
    )
}

/// The entries of `entries` a placed part could have come from: one of the same template (an engine's
/// of its family), or those made the same way, and of those the one of its name if one is. None for
/// a part made some other way, by hand.
pub fn sources<'a>(
    part: &CraftPartDef,
    entries: &'a [Entry],
    catalog: &PartCatalog,
) -> Vec<&'a Entry> {
    if let Some(template) = &part.template {
        let same = |entry: &&Entry| {
            entry.part.template.as_ref().is_some_and(|t| {
                t == template || catalog.configurations(t).iter().any(|c| c.id == *template)
            })
        };
        return entries.iter().filter(same).collect();
    }
    let made = made_as(part);
    let like: Vec<&Entry> = (entries.iter())
        .filter(|e| e.part.template.is_none() && made_as(&e.part) == made)
        .collect();
    let named: Vec<&Entry> = (like.iter().copied())
        .filter(|e| part.name.is_some() && e.part.name == part.name)
        .collect();
    if named.is_empty() { like } else { named }
}

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
    // A solid booster, a case of grain and its nozzle in one part shaped as a tank is: procedural,
    // so listed first in the engines, before the records in their order.
    out.extend(solid_booster(catalog).map(|mut entry| {
        entry.detail = built_figures(&entry.part, "1.25 m × 7.5 m", catalog);
        entry
    }));
    // One entry an engine, and one a family of configurations of one engine, as its first: the
    // part settings pick the others (`properties`).
    let mut engines: Vec<Entry> = Vec::new();
    engines.extend(
        (catalog.engines.iter())
            .filter(|engine| {
                (catalog.first_configuration(&engine.id)).is_none_or(|first| first.id == engine.id)
            })
            .map(|engine| {
                // Not a title still waiting to be translated (`#roBE3UTitle`).
                let untranslated = |t: &String| !t.starts_with('#');
                let family = catalog.family_of(&engine.id);
                let title = (family.and_then(|f| f.catalog.title.clone()))
                    .filter(untranslated)
                    .or_else(|| engine.catalog.title.clone().filter(untranslated));
                let name = title.unwrap_or_else(|| engine.id.clone());
                let configs = catalog.configurations(&engine.id);
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
                let mut about = about(family.map_or(&engine.catalog, |f| &f.catalog));
                if configs.len() > 1 {
                    detail.push(format!("{} configurations", configs.len()));
                    let names: Vec<String> = (configs.iter())
                        .map(|c| {
                            c.catalog
                                .title
                                .clone()
                                .filter(untranslated)
                                .unwrap_or(c.id.clone())
                        })
                        .collect();
                    about = [about, format!("Configurations: {}", names.join(", "))]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(". ");
                }
                Entry {
                    id: format!("engine:{}", engine.id),
                    detail: detail.join(" · "),
                    about,
                    width: (engine.exit_diameter > 0.0).then_some(engine.exit_diameter),
                    propellant: Some(propellant),
                    part: template(&engine.id, &name),
                    name,
                    group: Group::Engines,
                    surface: None,
                    stem: "engine".into(),
                }
            }),
    );
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
    let mut science: Vec<Entry> = (catalog.instruments.iter())
        .map(|def| instrument(def, catalog))
        .collect();
    for list in [&mut pods, &mut engines, &mut control, &mut science] {
        list.sort_by_key(|entry| entry.name.to_lowercase());
        out.append(list);
    }
    out
}

/// A scientific instrument's entry: a box of its size and mass carrying its experiments, put on a
/// side by its bottom like an antenna, with a drive of its own for the sets it holds of them.
fn instrument(def: &crate::data::InstrumentDef, catalog: &PartCatalog) -> Entry {
    let name = if def.name.is_empty() {
        def.id.clone()
    } else {
        def.name.clone()
    };
    let mut experiments: Vec<serde_json::Value> = (def.experiments.iter())
        .map(|id| json!({ "experiment": { "experiment": id } }))
        .collect();
    let (mut data, mut samples) = (0.0, 0.0);
    for record in
        (def.experiments.iter()).filter_map(|id| catalog.experiments.iter().find(|e| &e.id == id))
    {
        match record.sample {
            Some(kg) => samples += def.holds * kg,
            None => data += def.holds * record.size,
        }
    }
    if data > 0.0 || samples > 0.0 {
        experiments.push(json!({ "drive": { "data": data, "samples": samples } }));
    }
    Entry {
        id: format!("instrument:{}", def.id),
        name: name.clone(),
        group: Group::Science,
        detail: mass(def.mass),
        about: def.catalog.description.clone().unwrap_or_default(),
        width: None,
        propellant: None,
        part: part(json!({
            "id": "", "name": name,
            "shape": { "box": { "size": def.size } },
            "structure": { "mass": def.mass },
            "components": experiments,
            "nodes": [{
                "name": "bottom",
                "position": [0.0, 0.0, 0.0],
                "direction": [0.0, 0.0, -1.0],
                "size": def.size[0].max(def.size[1]),
            }],
            "paint": { "color": def.color, "finish": "metal" },
        })),
        surface: Some(("bottom".to_owned(), 0.0)),
        stem: def.id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::preset::ShapeDef;

    fn catalog() -> PartCatalog {
        crate::vessel::tests::catalog()
    }

    /// Every entry's own part traces back to it, and to nothing else: a career can tell a placed
    /// part's entry from the part.
    #[test]
    fn every_entry_s_part_traces_back_to_it() {
        let catalog = catalog();
        let all = entries(&catalog);
        for entry in &all {
            let found: Vec<&str> = (sources(&entry.part, &all, &catalog).iter())
                .map(|e| e.id.as_str())
                .collect();
            assert_eq!(found, [entry.id.as_str()], "{}", entry.id);
        }
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
            axial: false,
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
}
