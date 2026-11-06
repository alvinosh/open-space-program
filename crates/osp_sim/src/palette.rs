//! The palette: what a player can put on a craft in the editor. A few procedural parts (a tank, a
//! structural tube, a nose cone, a decoupler, a fin and a wing) whose sizes the player then sets,
//! as in KSP with its procedural parts, and the catalogue's pods, engines, RCS blocks and reaction
//! wheels as they are.
//!
//! An entry is a part to copy ([`Entry::part`], with no id: the editor names it). A procedural one
//! is [`fit`] to what it goes on, so a tank put under a 3 m tank is 3 m across and a fin is sized
//! for the hull it stands on. What goes on a surface says by which of its own nodes, and how far it
//! is turned about the joint so that it stands the right way up ([`Entry::surface`]).

use serde_json::json;

use crate::{
    data::{CraftPartDef, PartCatalog},
    geometry::preset::{AirfoilDef, ShapeDef},
    shaping,
    sizing::ProceduralDef,
};

/// Where an entry is listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Procedural,
    Pods,
    Engines,
    Control,
}

impl Group {
    pub const ALL: [Group; 4] = [
        Group::Procedural,
        Group::Pods,
        Group::Engines,
        Group::Control,
    ];

    /// As actions write it.
    pub fn id(self) -> &'static str {
        match self {
            Group::Procedural => "procedural",
            Group::Pods => "pods",
            Group::Engines => "engines",
            Group::Control => "control",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.id() == text)
    }

    /// What the player reads.
    pub fn name(self) -> &'static str {
        match self {
            Group::Procedural => "Procedural",
            Group::Pods => "Pods",
            Group::Engines => "Engines",
            Group::Control => "Control",
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
    /// A short line about it: how hard an engine pushes, how many a pod carries.
    pub detail: String,
    /// What a new part starts as, with no id.
    pub part: CraftPartDef,
    /// The node of its own it is held by on a surface, and how far it is turned about the joint
    /// there, degrees; none if it only goes on nodes.
    pub surface: Option<(String, f64)>,
    /// What a new part's id starts with.
    pub stem: String,
}

impl Entry {
    /// Whether `search` (any case) is in its name, its detail or its group's name.
    pub fn matches(&self, search: &str) -> bool {
        let search = search.trim().to_lowercase();
        search.is_empty()
            || [self.name.as_str(), self.detail.as_str(), self.group.name()]
                .iter()
                .any(|text| text.to_lowercase().contains(&search))
    }
}

fn part(value: serde_json::Value) -> CraftPartDef {
    serde_json::from_value(value).expect("the palette's own parts are well formed")
}

/// The paint of a new procedural part: white, as most rockets are.
const WHITE: [f64; 3] = [0.85, 0.85, 0.83];
const GREY: [f64; 3] = [0.55, 0.57, 0.6];

/// The procedural parts, in the order they are listed.
fn procedural() -> Vec<Entry> {
    let entry = |id: &str, name: &str, detail: &str, value, surface: Option<(&str, f64)>| Entry {
        id: id.into(),
        name: name.into(),
        group: Group::Procedural,
        detail: detail.into(),
        part: part(value),
        surface: surface.map(|(node, roll)| (node.to_owned(), roll)),
        stem: id.into(),
    };
    vec![
        entry(
            "tank",
            "Tank",
            "Kerosene and liquid oxygen",
            json!({
                "id": "", "name": "Tank",
                "shape": { "cylinder_tank": { "diameter": 2.0, "length": 4.0 } },
                "components": [{ "storage": {
                    "type": "Stiffened Al 2219",
                    "contents": { "Kerosene": 1.0, "LqdOxygen": 2.36 },
                } }],
                "paint": { "color": WHITE },
            }),
            None,
        ),
        entry(
            "structural",
            "Structural",
            "A tube: an interstage or a skirt",
            json!({
                "id": "", "name": "Structural",
                "shape": { "shell": { "bottom_diameter": 2.0, "sections": [{ "length": 1.0 }] } },
                "structure": { "material": "Al 2219-T87", "thickness": 0.004 },
                "paint": { "color": WHITE },
            }),
            None,
        ),
        entry(
            "nose_cone",
            "Nose cone",
            "An ogive, closed at its base",
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
        entry(
            "decoupler",
            "Decoupler",
            "Lets go of what is below it",
            json!({
                "id": "", "name": "Decoupler",
                "procedural": { "decoupler": { "diameter": 2.0, "load": 1e6 } },
            }),
            None,
        ),
        entry(
            "fin",
            "Fin",
            "A flat plate, swept",
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
        entry(
            "wing",
            "Wing",
            "An airfoil, tapered and swept",
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
    ]
}

/// Everything a craft can be built from, `catalog`'s records after the procedural parts, each group
/// by name.
pub fn entries(catalog: &PartCatalog) -> Vec<Entry> {
    let template = |id: &str, name: &str| part(json!({ "id": "", "name": name, "template": id }));
    let mut out = procedural();
    let mut pods: Vec<Entry> = (catalog.pods.iter())
        .map(|pod| Entry {
            id: format!("pod:{}", pod.id),
            name: pod.id.clone(),
            group: Group::Pods,
            detail: match pod.crew {
                1 => format!("1 crew, {:.1} m", pod.diameter),
                n => format!("{n} crew, {:.1} m", pod.diameter),
            },
            part: template(&pod.id, &pod.id),
            surface: None,
            stem: "pod".into(),
        })
        .collect();
    let mut engines: Vec<Entry> = (catalog.engines.iter())
        .map(|engine| {
            let name = (engine.catalog.title.clone()).unwrap_or_else(|| engine.id.clone());
            Entry {
                id: format!("engine:{}", engine.id),
                detail: format!("{:.0} kN", engine.thrust_vac / 1e3),
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
        .map(|rcs| Entry {
            id: format!("rcs:{}", rcs.id),
            name: rcs.id.clone(),
            group: Group::Control,
            detail: format!("RCS, {:.0} N a nozzle", rcs.thrust),
            part: template(&rcs.id, &rcs.id),
            surface: Some(("mount".into(), 90.0)),
            stem: "rcs".into(),
        })
        .chain(catalog.reaction_wheels.iter().map(|wheel| Entry {
            id: format!("wheel:{}", wheel.id),
            name: wheel.id.clone(),
            group: Group::Control,
            detail: format!("Reaction wheel, {:.0} N·m", wheel.torque),
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
    use crate::{craft::Craft, data::CraftDef};

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
        assert!(tank.matches("TAN") && tank.matches("oxygen") && tank.matches("procedural"));
        assert!(tank.matches("  ") && !tank.matches("wheel"));
        assert_eq!(Group::parse(Group::Engines.id()), Some(Group::Engines));
    }
}
