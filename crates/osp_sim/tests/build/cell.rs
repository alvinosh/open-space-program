//! The rows and columns of the placement matrix, and the small craft each cell builds: a
//! host with the part under test placed on it in one way.

use osp_sim::{
    DVec3,
    assembly::Side,
    craft::Craft,
    data::{CraftDef, CraftPartDef, SurfaceAtDef},
    geometry::preset::NodeDef,
};

use crate::content;

/// What the sim should do with a part of a row, beyond weighing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aero {
    /// The body's skin: outside, it changes the outline; inside, it does not.
    Body,
    /// Has a model of its own in the air (a wing, a gear leg): outside, it changes the
    /// loads; inside, it does not.
    Own,
    /// Left out of the body and modelled apart (an engine): only inside is checked.
    Apart,
}

/// A kind of part: one component the sim has, on a small part of its own (and a pod, and a
/// part with two components).
pub struct Row {
    pub id: &'static str,
    pub title: &'static str,
    /// The part, as a craft file writes it; its id and placement are filled in.
    part: &'static str,
    /// Its own connector that holds it when it stands on a node, hangs under one, and
    /// everywhere else (on a surface, inside).
    pub stack: &'static str,
    pub hang: &'static str,
    pub aero: Aero,
}

pub const ROWS: [Row; 12] = [
    Row {
        id: "storage",
        title: "Storage",
        part: r#"{ id: "x", shape: { cylinder_tank: { diameter: 0.4, length: 0.8 } },
            components: [{ storage: { type: "Stiffened Al 2219",
                contents: { Kerosene: 1, LqdOxygen: 2.36 } } }] }"#,
        stack: "bottom",
        hang: "top",
        aero: Aero::Body,
    },
    Row {
        id: "engine",
        title: "Engine",
        part: r#"{ id: "x", template: "K-0" }"#,
        stack: "top",
        hang: "top",
        aero: Aero::Apart,
    },
    Row {
        id: "rcs",
        title: "RCS",
        part: r#"{ id: "x", template: "Cold Gas Quad" }"#,
        stack: "mount",
        hang: "mount",
        aero: Aero::Body,
    },
    Row {
        id: "wheel",
        title: "Reaction wheel",
        part: r#"{ id: "x", template: "RW-1" }"#,
        stack: "bottom",
        hang: "top",
        aero: Aero::Body,
    },
    Row {
        id: "battery",
        title: "Battery",
        part: r#"{ id: "x", shape: { ring: { diameter: 0.4, height: 0.3, thickness: 0.2 } },
            structure: { mass: 12 }, components: [{ battery: { charge: 3600 } }] }"#,
        stack: "bottom",
        hang: "top",
        aero: Aero::Body,
    },
    Row {
        id: "crew",
        title: "Crew",
        part: r#"{ id: "x", shape: { ring: { diameter: 0.5, height: 0.6, thickness: 0.25 } },
            structure: { mass: 150 }, components: [{ crew: { seats: 1 } }] }"#,
        stack: "bottom",
        hang: "top",
        aero: Aero::Body,
    },
    Row {
        id: "decoupler",
        title: "Decoupler",
        part: r#"{ id: "x", shape: { ring: { diameter: 0.6, height: 0.1, thickness: 0.3 } },
            structure: { mass: 4 }, components: [{ decoupler: { node: "top" } }] }"#,
        stack: "bottom",
        hang: "top",
        aero: Aero::Body,
    },
    Row {
        id: "wing",
        title: "Lifting surface",
        part: r#"{ id: "x", shape: { wing: { root_chord: 0.6, tip_chord: 0.3, span: 1.0,
                sweep: 0.2, airfoil: { naca: { thickness: 0.1 } } } },
            structure: { mass: 8 }, components: [{ lifting_surface: {} }] }"#,
        stack: "root",
        hang: "root",
        aero: Aero::Own,
    },
    Row {
        id: "container",
        title: "Container",
        part: r#"{ id: "x", shape: { shell: { bottom_diameter: 0.5, sections: [{ length: 0.6 }],
                cap_bottom: true, cap_top: true } },
            structure: { mass: 8 }, components: [{ container: {} }] }"#,
        stack: "bottom",
        hang: "top",
        aero: Aero::Body,
    },
    Row {
        id: "gear",
        title: "Gear",
        part: r#"{ id: "x", shape: { ring: { diameter: 0.12, height: 0.2, thickness: 0.06 } },
            structure: { mass: 1 },
            nodes: [{ name: "mount", position: [0, 0, 0], direction: [0, 0, 1], size: 0.1 }],
            components: [{ gear: { length: 0.8, load: 300, mass: 10 } }] }"#,
        stack: "mount",
        hang: "mount",
        aero: Aero::Own,
    },
    Row {
        id: "pod",
        title: "Pod",
        part: r#"{ id: "x", template: "C-1 Capsule" }"#,
        stack: "bottom",
        hang: "top",
        aero: Aero::Body,
    },
    Row {
        id: "two",
        title: "Two components",
        part: r#"{ id: "x", shape: { cylinder_tank: { diameter: 0.4, length: 0.8 } },
            components: [
                { storage: { type: "COPV", contents: { Nitrogen: 1 } } },
                { rcs: { id: "Cold Gas Quad" } },
            ] }"#,
        stack: "bottom",
        hang: "top",
        aero: Aero::Body,
    },
];

/// A way to put a part on the host.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Column {
    /// On the host's top node.
    Top,
    /// Under the host's bottom node.
    Bottom,
    /// On the side of the host's wall.
    Radial,
    /// Round the wall, 2, 3, 4 or 6 of them.
    Ring(u32),
    /// Inside the host, on its inside node.
    Inside,
    /// On a deck on the top node, with a fairing round it.
    Fairing,
    /// On the top node, turned this many degrees off the host's axis.
    Canted(f64),
    /// A pair on the wall, one the mirror image of the other.
    Mirrored,
}

pub const COLUMNS: [Column; 12] = [
    Column::Top,
    Column::Bottom,
    Column::Radial,
    Column::Ring(2),
    Column::Ring(3),
    Column::Ring(4),
    Column::Ring(6),
    Column::Inside,
    Column::Fairing,
    Column::Canted(15.0),
    Column::Canted(90.0),
    Column::Mirrored,
];

impl Column {
    pub fn id(self) -> String {
        match self {
            Column::Top => "top".into(),
            Column::Bottom => "bottom".into(),
            Column::Radial => "radial".into(),
            Column::Ring(n) => format!("ring{n}"),
            Column::Inside => "inside".into(),
            Column::Fairing => "fairing".into(),
            Column::Canted(d) => format!("cant{d}"),
            Column::Mirrored => "mirror".into(),
        }
    }

    pub fn title(self) -> String {
        match self {
            Column::Top => "On a top node".into(),
            Column::Bottom => "Under a bottom node".into(),
            Column::Radial => "Radial, on a surface".into(),
            Column::Ring(n) => format!("Ring of {n}"),
            Column::Inside => "Inside a container".into(),
            Column::Fairing => "Inside a fairing".into(),
            Column::Canted(d) => format!("Canted {d}°"),
            Column::Mirrored => "Mirrored pair".into(),
        }
    }

    /// Whether the part is meant to be in the air stream: outside the host's skin and not
    /// under a fairing.
    pub fn exposed(self) -> bool {
        !matches!(self, Column::Inside | Column::Fairing)
    }
}

/// The host every part goes on: a closed shell, 1 m across and 3 m long, that holds things
/// (it has an inside node).
const HOST: &str = r#"{ id: "host",
    shape: { shell: { bottom_diameter: 1.0, sections: [{ length: 3.0 }],
        cap_bottom: true, cap_top: true } },
    structure: { mass: 120 },
    components: [{ container: {} }],
    nodes: [
        { name: "seat", position: [0, 0, 1.5], direction: [0, 0, 1], size: 0.5 },
    ] }"#;

/// The deck a fairing stands on, the craft's root with the host hanging under it: a short
/// closed shell that lets go of the host, with an inside node for the payload and a node on
/// its rim for each panel.
const DECK: &str = r#"{ id: "deck",
    shape: { shell: { bottom_diameter: 1.0, sections: [{ length: 0.1 }],
        cap_bottom: true, cap_top: true } },
    structure: { mass: 10 },
    components: [{ container: {} }, { decoupler: { node: "bottom" } }],
    nodes: [
        { name: "seat", position: [0, 0, 0.1], direction: [0, 0, 1], size: 0.5 },
        { name: "fairing_1", position: [0.5, 0, 0], direction: [0, 0, 1], size: 1.0 },
        { name: "fairing_2", position: [-0.5, 0, 0], direction: [0, 0, 1], size: 1.0 },
    ] }"#;

/// One half of a fairing round the deck's payload; fitted by the fairing tool.
fn panel(index: u32) -> CraftPartDef {
    let text = format!(
        r#"{{ id: "fairing_{index}", on: "deck.fairing_{index}", roll: {roll},
            shape: {{ shell: {{ bottom_diameter: 1.0, sections: [{{ length: 1.0 }}], span: [-90, 90] }} }},
            structure: {{ mass: 10 }}, components: [{{ decoupler: {{ node: "bottom" }} }}] }}"#,
        roll = if index == 2 { -180 } else { 0 }
    );
    json5::from_str(&text).expect("the fairing panel is valid")
}

/// A cell's craft, and the same without the part under test.
pub struct Built {
    pub def: CraftDef,
    pub baseline: CraftDef,
    /// The ids of the parts under test.
    pub subject: Vec<String>,
}

/// Why a cell could not be set up as asked (the fairing tool refusing, say).
pub type Refusal = String;

pub fn build(row: &Row, column: Column) -> Result<Built, Refusal> {
    let catalog = &content::get().catalog;
    let part = |text: &str| -> CraftPartDef { json5::from_str(text).expect("the part is valid") };
    let host: CraftPartDef = part(HOST);
    let subject: CraftPartDef = part(row.part);

    // The held connector's place and direction on the part alone, to turn a copy of it.
    let alone = CraftDef {
        id: "alone".into(),
        name: String::new(),
        parts: vec![subject.clone()],
        sequences: None,
    };
    let alone = Craft::build(&alone, catalog).expect("a part builds alone");
    let mount = alone.assembly.parts()[0]
        .node(row.stack)
        .unwrap_or_else(|| panic!("`{}` has a node `{}`", row.id, row.stack))
        .clone();

    let wall = |u: f64| SurfaceAtDef {
        u,
        v: 0.5,
        side: Side::Front,
        standoff: 0.05,
    };
    let mut parts = vec![host];
    let mut subjects: Vec<CraftPartDef> = Vec::new();
    let place = |id: String, on: &str, by: &str, at: Option<SurfaceAtDef>| {
        let mut p = subject.clone();
        p.id = id;
        p.on = Some(on.into());
        p.by = by.into();
        p.at = at;
        p
    };
    match column {
        Column::Top => subjects.push(place("x".into(), "host.top", row.stack, None)),
        Column::Bottom => subjects.push(place("x".into(), "host.bottom", row.hang, None)),
        Column::Radial => {
            subjects.push(place("x".into(), "host.wall", row.stack, Some(wall(0.125))))
        }
        Column::Ring(n) => {
            for k in 0..n {
                let u = f64::from(k) / f64::from(n);
                subjects.push(place(
                    format!("x{k}"),
                    "host.wall",
                    row.stack,
                    Some(wall(u)),
                ));
            }
        }
        Column::Inside => subjects.push(place("x".into(), "host.seat", row.stack, None)),
        Column::Fairing => {
            subjects.push(place("x".into(), "deck.seat", row.stack, None));
            // The deck is the root, and the host hangs from it by its top.
            let host = parts.remove(0);
            parts.extend([
                part(DECK),
                CraftPartDef {
                    on: Some("deck.bottom".into()),
                    by: "top".into(),
                    ..host
                },
                panel(1),
                panel(2),
            ]);
        }
        Column::Canted(degrees) => {
            let turn = glam_rotation_y(degrees.to_radians());
            let direction = turn(mount.direction);
            let mut p = place("x".into(), "host.top", "cant", None);
            p.nodes.push(NodeDef {
                name: "cant".into(),
                position: mount.position.to_array(),
                direction: direction.to_array(),
                size: mount.size,
            });
            subjects.push(p);
        }
        Column::Mirrored => {
            subjects.push(place(
                "x_left".into(),
                "host.wall",
                row.stack,
                Some(wall(0.125)),
            ));
            // The other is the reflection across the host's YZ plane, where it has one
            // (a catalogue record is its own mirror image).
            let mut right = place("x_right".into(), "host.wall", row.stack, Some(wall(0.375)));
            if let Some(shape) = right.shape.take() {
                right.shape = Some(osp_sim::geometry::ShapeDef::Mirrored(Box::new(shape)));
            }
            subjects.push(right);
        }
    }
    let ids: Vec<String> = subjects.iter().map(|p| p.id.clone()).collect();
    parts.extend(subjects);
    let mut def = CraftDef {
        id: format!("cell_{}_{}", row.id, column.id()),
        name: String::new(),
        parts,
        sequences: None,
    };
    if column == Column::Fairing {
        def.fit_fairings(catalog).map_err(|e| e.to_string())?;
    }
    let mut baseline = def.clone();
    baseline.parts.retain(|p| !ids_contain(&ids, &p.id));
    Ok(Built {
        def,
        baseline,
        subject: ids,
    })
}

fn ids_contain(ids: &[String], id: &str) -> bool {
    ids.iter().any(|i| i == id)
}

/// A direction turned about the part's Y axis.
fn glam_rotation_y(angle: f64) -> impl Fn(DVec3) -> DVec3 {
    let (sin, cos) = angle.sin_cos();
    move |v| DVec3::new(v.x * cos + v.z * sin, v.y, -v.x * sin + v.z * cos)
}
