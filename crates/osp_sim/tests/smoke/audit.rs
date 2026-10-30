//! Looks at the craft as a player would build it: part by part, what each one is, what the
//! sim runs for it, and what a real vehicle has that this one lacks.

use osp_sim::{component::Component, craft::Craft};
use serde::Serialize;

use crate::world::{Area, World};

/// Part names (lower case) that say what a part is for, where the sim gives it no component.
/// Each is checked against the sim's own list of what it models.
const FUNCTIONS: [(&str, Area, &str); 14] = [
    ("fuel_cell", Area::Power, "fuel cell"),
    ("battery", Area::Power, "battery"),
    ("solar", Area::Power, "solar panel"),
    ("antenna", Area::Comms, "antenna"),
    ("dish", Area::Comms, "antenna"),
    ("transponder", Area::Comms, "transponder"),
    ("avionics", Area::Avionics, "avionics"),
    ("instrument_unit", Area::Avionics, "instrument unit"),
    ("computer", Area::Avionics, "guidance computer"),
    ("heat_shield", Area::Thermal, "heat shield"),
    ("parachute", Area::Recovery, "parachute"),
    ("drogue", Area::Recovery, "parachute"),
    ("docking", Area::Docking, "docking port"),
    ("landing_leg", Area::Landing, "landing leg"),
];

#[derive(Debug, Clone, Serialize)]
pub struct PartRow {
    pub id: String,
    pub name: String,
    /// Dry mass, kg.
    pub mass: f64,
    /// What the sim runs for it.
    pub runs: Vec<String>,
    /// Made from a catalogue record, or shaped by hand in the craft file.
    pub catalogue: bool,
    /// A flat mass written into the craft file, standing in for what the part is made of.
    pub lump: Option<f64>,
    /// What the part is for in the real vehicle, where the sim does nothing with it.
    pub inert: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Audit {
    pub parts: Vec<PartRow>,
    /// Dry mass, kg, and propellant, kg.
    pub dry: f64,
    pub propellant: f64,
    /// Stored electricity, kJ.
    pub charge: f64,
    pub crew: u32,
}

/// Builds the craft from its part list and writes down what is in it.
pub fn inspect(w: &mut World) -> Audit {
    let craft = Craft::build(&w.craft, &w.catalog).expect("the craft builds");
    let mut audit = Audit {
        parts: Vec::new(),
        dry: 0.0,
        propellant: 0.0,
        charge: 0.0,
        crew: 0,
    };
    for part in &craft.parts {
        let def = w.craft.parts.iter().find(|p| p.id == part.id);
        let mut runs = Vec::new();
        for component in &part.built.components {
            match component {
                Component::Storage(s) => {
                    let kg: f64 = s.contents.iter().map(|c| c.capacity).sum();
                    audit.propellant += kg;
                    let what: Vec<_> = s.contents.iter().map(|c| c.resource.as_str()).collect();
                    runs.push(format!("tank: {:.0} kg of {}", kg, what.join(" + ")));
                }
                Component::Engine(e) => {
                    runs.push(format!("engine {}: {:.0} kN", e.id, e.thrust_vac / 1e3))
                }
                Component::Rcs(r) => runs.push(format!("RCS {}", r.id)),
                Component::ReactionWheel(r) => runs.push(format!("reaction wheel {}", r.id)),
                Component::Battery(b) => {
                    audit.charge += b.charge;
                    runs.push(format!("battery: {:.0} kJ (a stored number)", b.charge));
                }
                Component::Crew { seats } => {
                    audit.crew += seats;
                    runs.push(format!("crew: {seats} seats"));
                }
                Component::Decoupler(_) => runs.push("decoupler".into()),
                Component::LiftingSurface(_) => runs.push("lifting surface".into()),
                Component::Container(_) => runs.push("container".into()),
                Component::Gear(_) => runs.push("landing gear".into()),
            }
        }
        let mass = part.built.mass.mass;
        audit.dry += mass;
        let text = format!("{} {}", part.id, part.name).to_lowercase();
        let inert = runs
            .iter()
            .all(|r| r == "container")
            .then(|| FUNCTIONS.iter().find(|(key, ..)| text.contains(key)))
            .flatten()
            .map(|(_, _, what)| (*what).to_owned());
        audit.parts.push(PartRow {
            id: part.id.clone(),
            name: part.name.clone(),
            mass,
            runs,
            catalogue: part.template.is_some(),
            lump: def.and_then(|d| d.structure.mass),
            inert,
        });
    }
    audit
}

/// Writes the build findings: whether it builds from catalogue parts, what is lumped, and
/// which systems of the real vehicle have no part in it.
pub fn report(w: &mut World, audit: &Audit) {
    let hand = audit.parts.iter().filter(|p| !p.catalogue).count();
    let lumped: f64 = audit.parts.iter().filter_map(|p| p.lump).sum();
    w.info(
        Area::Build,
        "Parts",
        format!(
            "{} parts: {} from catalogue records, {} shaped by hand in the craft file. {:.0} t dry, {:.0} t of propellant.",
            audit.parts.len(),
            audit.parts.len() - hand,
            hand,
            audit.dry / 1e3,
            audit.propellant / 1e3
        ),
    );
    let lumps: Vec<_> = audit
        .parts
        .iter()
        .filter(|p| p.lump.is_some_and(|m| m > 1_000.0))
        .map(|p| format!("{} {:.1} t", p.name, p.lump.unwrap() / 1e3))
        .collect();
    w.info(
        Area::Build,
        "Flat masses standing in for hardware",
        format!(
            "{:.0} t of the dry mass ({:.0}%) is written into the craft file as a flat number per part, not built from tanks, structure or equipment: {}. A player assembling the Saturn V in an editor would have to place these as real parts.",
            lumped / 1e3,
            100.0 * lumped / audit.dry.max(1.0),
            lumps.join(", ")
        ),
    );

    // One finding per kind of inert part, however many there are.
    let mut kinds: Vec<&str> = Vec::new();
    for part in &audit.parts {
        if let Some(what) = part.inert.as_deref()
            && !kinds.contains(&what)
        {
            kinds.push(what);
        }
    }
    for what in kinds {
        let parts: Vec<_> = audit
            .parts
            .iter()
            .filter(|p| p.inert.as_deref() == Some(what))
            .collect();
        let area = FUNCTIONS
            .iter()
            .find(|(_, _, name)| *name == what)
            .map_or(Area::Build, |(_, area, _)| *area);
        let ids: Vec<_> = parts.iter().map(|p| p.id.as_str()).collect();
        w.gap(
            area,
            &format!("{}s do nothing ({})", capitalised(what), parts.len()),
            format!(
                "{} {} in the real vehicle, but here only shapes with {:.0} kg of mass between them: the sim has no component for it.",
                ids.join(", "),
                if parts.len() == 1 { format!("is a {what}") } else { format!("are {what}s") },
                parts.iter().map(|p| p.mass).sum::<f64>()
            ),
        );
    }

    if audit.charge > 0.0 {
        w.gap(
            Area::Power,
            "Electrical power is a stored number",
            format!(
                "The Command Module carries {:.0} kJ ({:.1} kWh) of battery, but the vessel has no electrical state: batteries are dropped when the part tree is built (parts/gather.rs), so nothing draws or recharges them. Engines name an `ElectricCharge` ignition cost and nothing pays it. The real Apollo flew on three fuel cells and five batteries.",
                audit.charge,
                audit.charge / 3600.0
            ),
        );
    }
    w.gap(
        Area::Avionics,
        "No avionics or instrument unit to build",
        "The Saturn V's guidance (the instrument unit, the CSM computer) is part of each stage's flat structure mass. The `avionics` records (RP-1's procedural avionics, with mass and power formulas) are loaded but nothing in the sim reads them: a vessel needs none to fly, and cannot lose control without one.",
    );
    w.gap(
        Area::Comms,
        "No communications",
        "There is no antenna part, link budget, ground station or signal delay. The real mission was in contact through the S-band network and the Honeysuckle Creek and Parkes dishes; here every craft is always controllable.",
    );
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
}
