//! Components: what a part does, on top of its shape (see `docs/vessel-overhaul.md`).
//!
//! A part is a shape ([`ShapeDef`]), what its structure is made of ([`StructureDef`]) and a
//! list of components, like KSP's part modules: storage in a cell, an engine, RCS, a
//! reaction wheel, a battery, crew seats, a decoupler, a lifting surface, a container and
//! landing gear. A component works with any shape that has what it needs (a cell to fill,
//! a node to let go of, a loft to lift with), so every shape works with every component.
//!
//! Building a part ([`PartDef::build`]) measures its shape, sizes the tank walls around its
//! storage, weighs its structure and the components' hardware, and tells the assembly what
//! its connectors pass and which of them let go ([`BuiltPart::assembly_part`]).
//!
//! Templates make parts of catalogue records ([`PartDef::engine`], [`PartDef::pod`],
//! [`PartDef::rcs`], [`PartDef::reaction_wheel`]): a record's model and nodes become the
//! shape, and the record itself a component.
//!
//! Frames follow the geometry's ([`crate::geometry`]). A part that hangs from a node at its
//! origin faces +Z into what holds it: an engine's gimbal pivot, with its nozzle down −Z and
//! its thrust up +Z; a gear leg's mount, its strut out along −Z.

pub mod storage;

use std::{collections::BTreeMap, sync::Arc};

use glam::{DMat2, DVec2, DVec3};
use serde::{Deserialize, Serialize};

use self::storage::{Sized, SizedWall};
pub use self::storage::{Storage, StorageDef, TankContent};
use crate::{
    assembly::{AssemblyPart, Carries},
    data::{
        ControlSurfaceDef, EngineDef, PartCatalog, PodDef, RcsDef, ReactionWheelDef, ShellProfile,
    },
    geometry::{
        Element, Fill, Geometry, Kind, Measure, Node, ShapeDef,
        preset::{
            BoxDef, CurveDef, ModelDef, NodeDef, PlacedShapeDef, RegionDef, RevolvedDef, RingDef,
            SegmentDef, ShellDef, ShellSection,
        },
    },
    mass::Mass,
    propulsion::CraftError,
    skip::*,
};

/// Wall thickness and member section that weigh a structure given only its mass, m and m².
const WEIGHING_THICKNESS: f64 = 1e-3;
const WEIGHING_SECTION: f64 = 1e-4;
/// How thick lattice members are drawn without a cross-section of their own, m.
const DRAWN_MEMBER: f64 = 0.05;
/// kJ in a watt-hour, and litres in a cubic metre.
const KJ_PER_WH: f64 = 3.6;
const LITRES: f64 = 1e3;

/// A part: its shape, what it is made of and what it does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartDef {
    #[serde(default)]
    pub name: String,
    pub shape: ShapeDef,
    #[serde(default)]
    pub structure: StructureDef,
    #[serde(default)]
    pub components: Vec<ComponentDef>,
    /// Nodes besides its shape's, in its frame: a template's, or where other parts are
    /// mounted on it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeDef>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub paint: PaintDef,
}

/// How a part looks: one colour and finish all over, some of its pieces otherwise, layers
/// of paint over that, and the back of its walls (inside a skirt or a fairing) its own way.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PaintDef {
    /// Linear RGB; the finish's own if left out (white paint, black composite, bare
    /// aluminium, soot).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub finish: Finish,
    /// By element name: a wall, solid or skin of its shape.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pieces: BTreeMap<String, Look>,
    /// Over every piece's outside, in order: each covers those before it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<LayerDef>,
    /// The back of its walls; bare metal by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside: Option<Look>,
}

/// A layer of paint, laid out round the part's axis (its +Z): `around` in turns from its +X
/// towards +Y, `along` in metres up the axis from the part's origin. A layer without a
/// `finish` takes the finish of what it covers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum LayerDef {
    /// A ring between two heights.
    Band {
        along: [f64; 2],
        color: [f32; 3],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finish: Option<Finish>,
    },
    /// A stripe up the part from one turn to another (through 0 if the second is smaller),
    /// between two heights if given.
    Stripe {
        around: [f64; 2],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        along: Option<[f64; 2]>,
        color: [f32; 3],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finish: Option<Finish>,
    },
    /// A roll pattern: `around` equal sectors and rows between the heights in `along`,
    /// every other cell painted, starting with the second sector of the first row (or the
    /// first, if `first`).
    Checker {
        around: u32,
        along: Vec<f64>,
        #[serde(default, skip_serializing_if = "is_false")]
        first: bool,
        color: [f32; 3],
        #[serde(default, skip_serializing_if = "Option::is_none")]
        finish: Option<Finish>,
    },
    /// Lettering, its middle at `at` (`[around, along]`), its capitals `height` m tall,
    /// reading round the part, or down it if `vertical`.
    Text {
        text: String,
        at: [f64; 2],
        height: f64,
        #[serde(default, skip_serializing_if = "is_false")]
        vertical: bool,
        color: [f32; 3],
    },
    /// A picture (an SVG file a mod ships, such as a flag), its middle at `at`, `size` m
    /// wide round the part and tall along it (the other way round if `vertical`).
    Image {
        file: String,
        at: [f64; 2],
        size: [f64; 2],
        #[serde(default, skip_serializing_if = "is_false")]
        vertical: bool,
    },
    /// Stiffening stringers standing `depth` m proud of the skin between two heights,
    /// `count` round it: relief only, in whatever colour is under them.
    Stringers {
        along: [f64; 2],
        count: u32,
        #[serde(default = "stringer_depth")]
        depth: f64,
    },
}

fn stringer_depth() -> f64 {
    0.012
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl LayerDef {
    /// The same layer `rise` m lower down the part (for a part whose origin is `rise` up
    /// the frame the layer was laid out in).
    pub fn lowered(&self, rise: f64) -> LayerDef {
        let down = |[a, b]: [f64; 2]| [a - rise, b - rise];
        let mut layer = self.clone();
        match &mut layer {
            LayerDef::Band { along, .. } | LayerDef::Stringers { along, .. } => {
                *along = down(*along)
            }
            LayerDef::Stripe { along, .. } => *along = along.map(down),
            LayerDef::Checker { along, .. } => along.iter_mut().for_each(|z| *z -= rise),
            LayerDef::Text { at, .. } | LayerDef::Image { at, .. } => at[1] -= rise,
        }
        layer
    }
}

/// A colour and a finish.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Look {
    /// Linear RGB; the finish's own if left out (white paint, black composite, bare
    /// aluminium, soot).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub finish: Finish,
}

/// What a surface is, which decides how light plays on it and the detail it shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finish {
    /// Glossy paint over a metal skin.
    #[default]
    Paint,
    /// Carbon composite, black unless painted.
    Composite,
    /// Bare or brushed metal.
    Metal,
    /// Charred ablator or soot.
    Char,
    /// Spray-on foam insulation, rough and mottled.
    Foam,
    /// Cork insulation.
    Cork,
    /// Bare stainless steel rings welded together, the seams showing.
    Stainless,
}

impl Finish {
    /// Its colour when none is given, linear RGB.
    pub fn color(self) -> [f32; 3] {
        match self {
            Finish::Paint => [0.85, 0.85, 0.83],
            Finish::Composite => [0.028, 0.028, 0.032],
            Finish::Metal => [0.55, 0.55, 0.57],
            Finish::Char => [0.05, 0.045, 0.04],
            Finish::Foam => [0.42, 0.14, 0.035],
            Finish::Cork => [0.33, 0.2, 0.1],
            Finish::Stainless => [0.62, 0.62, 0.64],
        }
    }
}

impl PaintDef {
    /// The same paint on a part whose origin is `rise` m up the frame its layers were laid
    /// out in.
    pub fn lowered(&self, rise: f64) -> PaintDef {
        PaintDef {
            layers: self.layers.iter().map(|l| l.lowered(rise)).collect(),
            ..self.clone()
        }
    }

    /// How piece `element` looks, front and back.
    pub fn of(&self, element: &str) -> (Look, Look) {
        let own = Look {
            color: self.color,
            finish: self.finish,
        };
        let front = self.pieces.get(element).copied().unwrap_or(own);
        let back = self.inside.unwrap_or(Look {
            color: None,
            finish: Finish::Metal,
        });
        (front, back)
    }
}

impl Look {
    pub fn color(&self) -> [f32; 3] {
        self.color.unwrap_or(self.finish.color())
    }
}

/// What the part's walls, solids and members are made of, where no component says
/// otherwise (a storage sizes the walls around its cell, a battery fills its solid). With
/// neither a material nor a mass the structure weighs nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureDef {
    /// Id of a `materials` record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// Of walls and skins, m.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thickness: Option<f64>,
    /// Share of a solid that is material: 1 solid through, less for a built-up structure
    /// (a wing's spars and ribs).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub solidity: f64,
    /// Cross-section of a lattice's members, m².
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_area: Option<f64>,
    /// kg, instead of what the material gives, spread over the structure as the material
    /// would be (as 1 mm walls and 1 cm² members of something 1 kg/m³ without one). On a
    /// model with nothing else to spread over, it fills the model's proxy shape, or sits at
    /// the origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mass: Option<f64>,
}

impl Default for StructureDef {
    fn default() -> Self {
        Self {
            material: None,
            thickness: None,
            solidity: 1.0,
            member_area: None,
            mass: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentDef {
    Storage(StorageDef),
    /// An `engines` record.
    Engine(RecordRef),
    /// An `rcs` record.
    Rcs(RecordRef),
    /// A `reaction_wheels` record.
    ReactionWheel(RecordRef),
    Battery(BatteryPackDef),
    Crew(CrewDef),
    Decoupler(DecouplerDef),
    LiftingSurface(LiftingSurfaceDef),
    Container(ContainerDef),
    Gear(GearLegDef),
}

/// A catalogue record, by id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordRef {
    pub id: String,
}

/// Stored electricity: a set amount, or as much as a battery technology packs into one of
/// the part's solids, which then weighs what such a battery does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BatteryPackDef {
    /// kJ, its mass counted elsewhere (a pod's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charge: Option<f64>,
    /// Id of a `batteries` record, with `solid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technology: Option<String>,
    /// The solid it fills, by element name; the part's only one if left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrewDef {
    pub seats: u32,
}

/// Lets go of one of the part's nodes in flight: whatever is joined there leaves, and which
/// node it is decides which side keeps the part. One at each end of an interstage
/// separates in two planes; on a fairing panel it is the jettison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecouplerDef {
    pub node: String,
    /// What it pushes the two sides apart with, N·s.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub impulse: f64,
    /// Propellant flows across until it lets go.
    #[serde(default, skip_serializing_if = "is_false")]
    pub crossfeed: bool,
}

/// A wing, fin or tail surface: the part's loft is its planform and airfoil.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiftingSurfaceDef {
    /// The loft, by element name; the part's only one if left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loft: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<ControlSurfaceDef>,
}

/// A hollow part that holds other parts: they mount on the inside of its walls and on its
/// inside nodes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContainerDef {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeDef>,
}

/// A landing gear leg: a sprung, damped strut out from the part's origin along −Z with a
/// wheel at its end. It rolls along the part's +Y, grips sideways like a tyre, and can
/// brake and steer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GearLegDef {
    /// From the mount to the axle, strut fully extended, m.
    pub length: f64,
    #[serde(
        default = "default_wheel_radius",
        skip_serializing_if = "is_point_three"
    )]
    pub wheel_radius: f64,
    /// Strut travel, m.
    #[serde(default = "default_stroke", skip_serializing_if = "is_point_three")]
    pub stroke: f64,
    /// Mass it is sprung for: standing still under it, the strut sits a third of the way
    /// into its travel, kg.
    pub load: f64,
    /// The leg, wheel and strut, kg: halfway down the strut.
    pub mass: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub brakes: bool,
    /// Steers with the yaw command, this many degrees each way.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub steering: f64,
}

/// A part, built: measured, sized and weighed.
#[derive(Debug, Clone)]
pub struct BuiltPart {
    pub name: String,
    pub geometry: Arc<Geometry>,
    pub components: Vec<Component>,
    /// Nodes besides its geometry's.
    pub nodes: Vec<Node>,
    pub paint: PaintDef,
    /// How thick its lattices' members are drawn, m.
    pub member: f64,
    /// What each weighed piece of its geometry is made of.
    pub pieces: Vec<Piece>,
    /// Dry, in the part's frame: its structure, tank walls and the components' hardware.
    pub mass: Mass,
}

/// A piece of a part's geometry as built.
#[derive(Debug, Clone)]
pub struct Piece {
    /// Its element's name.
    pub element: String,
    pub material: Option<String>,
    /// Of a wall or skin, m; 0 for other pieces.
    pub thickness: f64,
    pub mass: Mass,
}

#[derive(Debug, Clone)]
pub enum Component {
    Storage(Storage),
    /// Propellant ratios normalised. It pushes the part along +Z, from its origin.
    Engine(Box<EngineDef>),
    /// Propellant ratios normalised.
    Rcs(RcsDef),
    ReactionWheel(ReactionWheelDef),
    Battery(Battery),
    Crew {
        seats: u32,
    },
    Decoupler(DecouplerDef),
    LiftingSurface(LiftingSurface),
    Container(Container),
    Gear(GearLegDef),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Battery {
    /// kJ.
    pub charge: f64,
    /// The `batteries` record, when it fills a solid.
    pub technology: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Container {
    /// Its inside nodes, in the part's frame.
    pub nodes: Vec<Node>,
}

/// A lifting surface as its loft lays it out: a panel between each pair of stations.
#[derive(Debug, Clone, PartialEq)]
pub struct LiftingSurface {
    /// The loft's element name.
    pub loft: String,
    /// Its loft runs from the tip to the root (a mirrored wing's does).
    pub reversed: bool,
    pub panels: Vec<Planform>,
    pub control: Option<ControlSurfaceDef>,
}

/// One panel of a lifting surface, in the part's frame. A loft that lifts has its
/// section's chord along its x, the leading edge first, and lifts towards its −y, as the
/// `wing` preset's does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Planform {
    /// The leading edge at its inner and outer station, m.
    pub root: DVec3,
    pub tip: DVec3,
    pub root_chord: f64,
    pub tip_chord: f64,
    /// Unit: from the leading edge back along the chord, at the root.
    pub chord: DVec3,
    /// Unit, square to its span and chord: the side it lifts towards.
    pub normal: DVec3,
    /// Thickness over chord at the root and tip.
    pub thickness: [f64; 2],
}

impl Planform {
    pub fn area(&self) -> f64 {
        let span = (self.tip - self.root).reject_from(self.chord).length();
        (self.root_chord + self.tip_chord) / 2.0 * span
    }
}

impl PartDef {
    /// Build it from `catalog`'s records, or say what is wrong with it.
    pub fn build(&self, catalog: &PartCatalog) -> Result<BuiltPart, CraftError> {
        let bad = |problem: String| CraftError::BadPart {
            part: self.name.clone(),
            problem,
        };
        let geometry = Arc::new(self.shape.build().map_err(&bad)?);
        let elements = geometry.elements();
        let nodes: Vec<Node> = self
            .nodes
            .iter()
            .map(NodeDef::node)
            .collect::<Result<_, _>>()
            .map_err(&bad)?;

        // Storage sizes the walls around its cell, each as thick as the storage that
        // needs it thickest asks.
        let mut sized: Vec<Sized> = Vec::new();
        for component in &self.components {
            if let ComponentDef::Storage(def) = component {
                let storage = def.size(&geometry, catalog)?;
                if sized.iter().any(|s| s.storage.cell == storage.storage.cell) {
                    return Err(bad(format!(
                        "two storages fill cell `{}`",
                        storage.storage.cell
                    )));
                }
                sized.push(storage);
            }
        }
        let mut walls: BTreeMap<String, SizedWall> = BTreeMap::new();
        for wall in sized.iter().flat_map(|s| &s.walls) {
            let heavier = walls
                .get(&wall.element)
                .is_none_or(|w| wall.areal_density > w.areal_density);
            if heavier {
                walls.insert(wall.element.clone(), wall.clone());
            }
        }
        for storage in &mut sized {
            storage.built(&walls);
        }
        let mut storages = sized.into_iter().map(|s| s.storage);

        let mut pieces = Vec::new();
        let mut taken: Vec<String> = Vec::new();
        for wall in walls.values() {
            let element = element(&elements, &wall.element).map_err(&bad)?;
            pieces.push(Piece {
                element: wall.element.clone(),
                material: Some(wall.material.clone()),
                thickness: wall.thickness,
                mass: element.moments.mass(wall.areal_density),
            });
            taken.push(wall.element.clone());
        }
        let mut hardware = Mass::ZERO;
        let mut components = Vec::new();
        for component in &self.components {
            let built = match component {
                ComponentDef::Storage(_) => {
                    Component::Storage(storages.next().expect("one storage each"))
                }
                ComponentDef::Engine(r) => {
                    let mut def = find(&catalog.engines, &r.id, |e| &e.id)
                        .ok_or_else(|| CraftError::UnknownEngine(r.id.clone()))?
                        .clone();
                    normalise(&mut def.propellants);
                    hardware = hardware + Mass::point(def.mass, DVec3::ZERO);
                    Component::Engine(Box::new(def))
                }
                ComponentDef::Rcs(r) => {
                    let mut def = find(&catalog.rcs, &r.id, |b| &b.id)
                        .ok_or_else(|| CraftError::UnknownRcs(r.id.clone()))?
                        .clone();
                    normalise(&mut def.propellants);
                    hardware = hardware + Mass::point(def.mass, DVec3::ZERO);
                    Component::Rcs(def)
                }
                ComponentDef::ReactionWheel(r) => {
                    let def = find(&catalog.reaction_wheels, &r.id, |w| &w.id)
                        .ok_or_else(|| CraftError::UnknownReactionWheel(r.id.clone()))?
                        .clone();
                    hardware = hardware + Mass::point(def.mass, DVec3::ZERO);
                    Component::ReactionWheel(def)
                }
                ComponentDef::Battery(def) => {
                    let (battery, filled) = battery(def, &elements, catalog).map_err(&bad)?;
                    if let Some(piece) = filled {
                        if taken.contains(&piece.element) {
                            return Err(bad(format!("`{}` is filled twice", piece.element)));
                        }
                        taken.push(piece.element.clone());
                        pieces.push(piece);
                    }
                    Component::Battery(battery)
                }
                ComponentDef::Crew(def) => {
                    if def.seats == 0 {
                        return Err(bad("crew needs a seat".into()));
                    }
                    Component::Crew { seats: def.seats }
                }
                ComponentDef::Decoupler(def) => {
                    if !(def.impulse >= 0.0 && def.impulse.is_finite()) {
                        return Err(bad("a decoupler's impulse cannot be negative".into()));
                    }
                    Component::Decoupler(def.clone())
                }
                ComponentDef::LiftingSurface(def) => {
                    Component::LiftingSurface(lifting_surface(def, &geometry).map_err(&bad)?)
                }
                ComponentDef::Container(def) => {
                    if geometry.surfaces().is_empty() {
                        return Err(bad("a container needs walls to hold things inside".into()));
                    }
                    let nodes = def
                        .nodes
                        .iter()
                        .map(NodeDef::node)
                        .collect::<Result<_, _>>()
                        .map_err(&bad)?;
                    Component::Container(Container { nodes })
                }
                ComponentDef::Gear(def) => {
                    let positive = [def.length, def.wheel_radius, def.stroke, def.load];
                    if !positive.iter().all(|v| *v > 0.0 && v.is_finite()) {
                        return Err(bad(
                            "a gear leg needs a length, wheel radius, stroke and load".into(),
                        ));
                    }
                    if !(def.mass >= 0.0 && (0.0..=90.0).contains(&def.steering)) {
                        return Err(bad(
                            "a gear leg's mass cannot be negative, and it steers up to 90°".into(),
                        ));
                    }
                    hardware = hardware + Mass::point(def.mass, DVec3::NEG_Z * def.length / 2.0);
                    Component::Gear(def.clone())
                }
            };
            components.push(built);
        }
        // What a decoupler lets go of must be there.
        let assembly = assembly_part("", &geometry, &nodes, &components);
        for component in &components {
            if let Component::Decoupler(d) = component
                && assembly.node(&d.node).is_none()
            {
                return Err(bad(format!("it has no node `{}` to let go of", d.node)));
            }
        }

        // A solid's skin or sides are its surface, not a wall of their own.
        let solids: Vec<&str> = elements
            .iter()
            .filter(|e| e.fill == Some(Fill::Solid))
            .map(|e| e.name.as_str())
            .collect();
        let surface_of_solid = |e: &Element| {
            e.measure == Measure::Area
                && solids.iter().any(|s| {
                    e.name
                        .strip_prefix(s)
                        .is_some_and(|r| r == "/skin" || r == "/sides")
                })
        };
        let rest: Vec<&Element> = elements
            .iter()
            .filter(|e| !taken.contains(&e.name) && !surface_of_solid(e))
            .collect();
        let (structure, unspread) = structure(&self.structure, &rest, catalog).map_err(&bad)?;
        pieces.extend(structure);
        let mass = pieces.iter().map(|p| p.mass).sum::<Mass>() + hardware + unspread;
        Ok(BuiltPart {
            name: self.name.clone(),
            geometry,
            components,
            nodes,
            paint: self.paint.clone(),
            member: self
                .structure
                .member_area
                .map_or(DRAWN_MEMBER, |a| 2.0 * (a / std::f64::consts::PI).sqrt()),
            pieces,
            mass,
        })
    }

    /// An engine record as a part: its model with the record's nodes (or a bell of its size
    /// without one), the bell standing in for the model in the air.
    pub fn engine(def: &EngineDef) -> Result<Self, CraftError> {
        let bell = (def.length > 0.0 && def.exit_diameter > 0.0).then(|| {
            ShapeDef::Compound(vec![PlacedShapeDef {
                name: String::new(),
                shape: ShapeDef::Shell(ShellDef {
                    bottom_diameter: def.exit_diameter,
                    sections: vec![ShellSection {
                        length: def.length,
                        top_diameter: Some(def.exit_diameter / 3.0),
                        profile: ShellProfile::Parabolic,
                    }],
                    span: None,
                    cap_bottom: false,
                    cap_top: false,
                }),
                position: [0.0, 0.0, -def.length],
                rotation: [0.0; 3],
                unseen: false,
            }])
        });
        let scale = def.model_scale;
        let mut nodes: Vec<NodeDef> = def
            .nodes
            .iter()
            .map(|n| NodeDef {
                position: n.position.map(|x| x * scale),
                size: n.size * scale,
                ..n.clone()
            })
            .collect();
        if nodes.is_empty() {
            nodes.push(node("top", [0.0; 3], [0.0, 0.0, 1.0], def.exit_diameter));
        }
        let shape = match (&def.model, bell) {
            (Some(path), bell) => ShapeDef::Model(ModelDef {
                path: path.clone(),
                scale,
                proxy: bell.map(Box::new),
                nodes,
            }),
            (None, Some(bell)) => bell,
            (None, None) => {
                return Err(CraftError::BadPart {
                    part: def.id.clone(),
                    problem: "an engine needs a model, or a length and exit diameter".into(),
                });
            }
        };
        Ok(Self {
            name: def.id.clone(),
            shape,
            structure: StructureDef::default(),
            components: vec![ComponentDef::Engine(RecordRef { id: def.id.clone() })],
            nodes: Vec::new(),
            paint: PaintDef::default(),
        })
    }

    /// A pod record as a part: its model with the record's nodes, a solid frustum of its
    /// size standing in for it and carrying its mass.
    pub fn pod(def: &PodDef) -> Self {
        let (bottom, top, height) = (def.diameter / 2.0, def.top_diameter() / 2.0, def.height);
        let mut segments = vec![
            SegmentDef::Line([bottom, 0.0]),
            SegmentDef::Line([top, height]),
        ];
        if top > 0.0 {
            segments.push(SegmentDef::Line([0.0, height]));
        }
        let frustum = ShapeDef::Revolved(RevolvedDef {
            span: None,
            walls: Vec::new(),
            regions: vec![RegionDef {
                name: "cabin".into(),
                boundary: CurveDef {
                    start: [0.0, 0.0],
                    segments,
                },
                fill: Fill::Solid,
            }],
        });
        let shape = match &def.model {
            Some(path) => {
                let mut nodes = def.nodes.clone();
                if nodes.is_empty() {
                    nodes.push(node("bottom", [0.0; 3], [0.0, 0.0, -1.0], def.diameter));
                    let up = [0.0, 0.0, def.height];
                    nodes.push(node("top", up, [0.0, 0.0, 1.0], def.top_diameter()));
                }
                ShapeDef::Model(ModelDef {
                    path: path.clone(),
                    scale: 1.0,
                    proxy: Some(Box::new(frustum)),
                    nodes,
                })
            }
            None => frustum,
        };
        let mut components = Vec::new();
        if def.crew > 0 {
            components.push(ComponentDef::Crew(CrewDef { seats: def.crew }));
        }
        if def.electric_charge > 0.0 {
            components.push(ComponentDef::Battery(BatteryPackDef {
                charge: Some(def.electric_charge),
                technology: None,
                solid: None,
            }));
        }
        Self {
            name: def.id.clone(),
            shape,
            structure: StructureDef {
                mass: Some(def.mass),
                ..StructureDef::default()
            },
            components,
            nodes: Vec::new(),
            paint: PaintDef::default(),
        }
    }

    /// An RCS record as a part: its model (or a small box) standing out along +X from a
    /// `mount` node at its origin, +Z towards the nose.
    pub fn rcs(def: &RcsDef) -> Self {
        let mount = node("mount", [0.0; 3], [-1.0, 0.0, 0.0], 0.0);
        let shape = match &def.model {
            Some(path) => ShapeDef::Model(ModelDef {
                path: path.clone(),
                scale: 1.0,
                proxy: None,
                nodes: vec![mount],
            }),
            None => ShapeDef::Compound(vec![PlacedShapeDef {
                name: String::new(),
                shape: ShapeDef::Cuboid(BoxDef {
                    size: [0.2, 0.2, 0.3],
                    fill: Fill::Solid,
                }),
                position: [0.1, 0.0, -0.15],
                rotation: [0.0; 3],
                unseen: false,
            }]),
        };
        Self {
            name: def.id.clone(),
            shape,
            structure: StructureDef::default(),
            components: vec![ComponentDef::Rcs(RecordRef { id: def.id.clone() })],
            nodes: Vec::new(),
            paint: PaintDef::default(),
        }
    }

    /// A reaction wheel record as a part: a squat drum, 0.5 m across, which the records do
    /// not size.
    pub fn reaction_wheel(def: &ReactionWheelDef) -> Self {
        Self {
            name: def.id.clone(),
            shape: ShapeDef::Ring(RingDef {
                diameter: 0.5,
                height: 0.2,
                thickness: 0.25,
            }),
            structure: StructureDef::default(),
            components: vec![ComponentDef::ReactionWheel(RecordRef {
                id: def.id.clone(),
            })],
            nodes: Vec::new(),
            paint: PaintDef::default(),
        }
    }
}

impl BuiltPart {
    pub fn storages(&self) -> impl Iterator<Item = &Storage> {
        self.components.iter().filter_map(|c| match c {
            Component::Storage(s) => Some(s),
            _ => None,
        })
    }

    /// The part as an assembly joins it: its geometry's nodes and its containers', what a
    /// decoupler's node passes until it lets go (power and data, and propellant with
    /// crossfeed), and which nodes let go.
    pub fn assembly_part(&self, id: impl Into<String>) -> AssemblyPart {
        assembly_part(id, &self.geometry, &self.nodes, &self.components)
    }
}

fn assembly_part(
    id: impl Into<String>,
    geometry: &Arc<Geometry>,
    nodes: &[Node],
    components: &[Component],
) -> AssemblyPart {
    let mut part = AssemblyPart::new(id, geometry.clone());
    part.extra_nodes.extend(nodes.iter().cloned());
    for component in components {
        match component {
            Component::Decoupler(d) => {
                let carries = Carries {
                    fluid: d.crossfeed,
                    ..Carries::NODE
                };
                part.carries.insert(d.node.clone(), carries);
                part.releases.insert(d.node.clone());
            }
            Component::Container(c) => {
                part.inside = true;
                part.extra_nodes.extend(c.nodes.iter().cloned());
            }
            _ => {}
        }
    }
    part
}

fn element<'a>(elements: &'a [Element], name: &str) -> Result<&'a Element, String> {
    elements
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| format!("no piece `{name}`"))
}

/// A battery, and the solid it fills, weighed.
fn battery(
    def: &BatteryPackDef,
    elements: &[Element],
    catalog: &PartCatalog,
) -> Result<(Battery, Option<Piece>), String> {
    match (&def.technology, def.charge) {
        (None, Some(charge)) if charge > 0.0 && def.solid.is_none() => Ok((
            Battery {
                charge,
                technology: None,
            },
            None,
        )),
        (Some(id), None) => {
            let tech = find(&catalog.batteries, id, |b| &b.id)
                .ok_or_else(|| format!("unknown battery technology `{id}`"))?;
            if !(tech.specific_energy > 0.0 && tech.energy_density > 0.0) {
                return Err(format!("`{id}` says nothing of how much it stores"));
            }
            let solids: Vec<&Element> = elements
                .iter()
                .filter(|e| e.fill == Some(Fill::Solid) && !e.proxy)
                .collect();
            let solid = match &def.solid {
                Some(name) => solids
                    .iter()
                    .find(|e| &e.name == name)
                    .ok_or_else(|| format!("no solid `{name}` for the battery to fill"))?,
                None => match solids.as_slice() {
                    [only] => only,
                    _ => return Err("name the solid the battery fills".into()),
                },
            };
            // Wh/L and Wh/kg: the solid is as dense as such a battery.
            let charge = solid.moments.measure * LITRES * tech.energy_density * KJ_PER_WH;
            let density = LITRES * tech.energy_density / tech.specific_energy;
            let piece = Piece {
                element: solid.name.clone(),
                material: None,
                thickness: 0.0,
                mass: solid.moments.mass(density),
            };
            Ok((
                Battery {
                    charge,
                    technology: Some(id.clone()),
                },
                Some(piece),
            ))
        }
        _ => Err("a battery has a charge (kJ), or a technology and the solid it fills".into()),
    }
}

/// The panels of the loft named in `def`, or of the part's only loft.
fn lifting_surface(def: &LiftingSurfaceDef, geometry: &Geometry) -> Result<LiftingSurface, String> {
    let lofts: Vec<_> = geometry
        .subparts
        .iter()
        .filter(|s| !s.proxy)
        .filter_map(|s| match &s.kind {
            Kind::Lofted(l) => Some((s, l, s.named(&l.name))),
            _ => None,
        })
        .collect();
    let (sub, loft, name) = match &def.loft {
        Some(name) => lofts
            .iter()
            .find(|(_, _, n)| n == name)
            .ok_or_else(|| format!("no loft `{name}` to lift with"))?,
        None => match lofts.as_slice() {
            [only] => only,
            [] => return Err("a lifting surface needs a loft for its planform".into()),
            _ => return Err("the part has several lofts: name the one that lifts".into()),
        },
    };
    let section = loft.section();
    let leading = section
        .iter()
        .copied()
        .fold(section[0], |a, p| if p.x < a.x { p } else { a });
    let trailing = section
        .iter()
        .copied()
        .fold(section[0], |a, p| if p.x > a.x { p } else { a });
    let (lo, hi) = section.iter().fold(
        (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    );
    let place = |s: &crate::geometry::Station, p: DVec2| {
        let local = (DMat2::from_angle(s.twist) * (s.scale * p) + s.offset).extend(s.z);
        sub.position + sub.rotation * local
    };
    let thickness = |s: &crate::geometry::Station| {
        let chord = (hi.x - lo.x) * s.scale.x;
        if chord > 0.0 {
            (hi.y - lo.y) * s.scale.y / chord
        } else {
            0.0
        }
    };
    // Root first: from the end nearer the part's origin, where it joins.
    let mut stations = loft.stations().to_vec();
    let reversed = place(&stations[stations.len() - 1], leading).length()
        < place(&stations[0], leading).length();
    if reversed {
        stations.reverse();
    }
    let panels = stations
        .windows(2)
        .map(|w| {
            let (a, b) = (&w[0], &w[1]);
            let (root, tip) = (place(a, leading), place(b, leading));
            let along = |s: &crate::geometry::Station| place(s, trailing) - place(s, leading);
            let chord = along(a)
                .try_normalize()
                .unwrap_or_else(|| along(b).normalize());
            let lifting = sub.rotation * (DMat2::from_angle(a.twist) * DVec2::NEG_Y).extend(0.0);
            let normal = (tip - root)
                .cross(chord)
                .try_normalize()
                .map_or(lifting, |n| if n.dot(lifting) < 0.0 { -n } else { n });
            Planform {
                root,
                tip,
                root_chord: along(a).length(),
                tip_chord: along(b).length(),
                chord,
                normal,
                thickness: [thickness(a), thickness(b)],
            }
        })
        .collect();
    Ok(LiftingSurface {
        loft: name.clone(),
        reversed,
        panels,
        control: def.control.clone(),
    })
}

/// The structure's pieces: `elements` (those no component took) made of `def`. A mass with
/// nothing to spread over comes back on its own, at the origin.
fn structure(
    def: &StructureDef,
    elements: &[&Element],
    catalog: &PartCatalog,
) -> Result<(Vec<Piece>, Mass), String> {
    let finite = |v: Option<f64>| v.is_none_or(|v| v >= 0.0 && v.is_finite());
    if !(finite(def.thickness) && finite(def.member_area) && finite(def.mass))
        || !(0.0..=1.0).contains(&def.solidity)
    {
        return Err(
            "a structure's thickness, member area and mass cannot be negative, and its \
             solidity is a share"
                .into(),
        );
    }
    let material = match &def.material {
        Some(id) => Some(storage::material(catalog, id).map_err(|e| e.to_string())?),
        None if def.mass.is_none() => return Ok((Vec::new(), Mass::ZERO)),
        None => None,
    };
    let density = material.map_or(1.0, |m| m.density);
    let weighed = def.mass.is_some();
    // kg per unit of each piece's measure, for what the material makes of it.
    let weigh = |e: &Element| -> Result<(f64, f64), String> {
        Ok(match (e.measure, e.fill) {
            (Measure::Area, _) => {
                let t = match (def.thickness, weighed) {
                    (Some(t), _) => t,
                    (None, true) => WEIGHING_THICKNESS,
                    (None, false) => {
                        return Err(format!("its walls need a thickness, as `{}`", e.name));
                    }
                };
                (density * t, t)
            }
            (Measure::Volume, Some(Fill::Cell)) => (0.0, 0.0),
            (Measure::Volume, _) => (density * def.solidity, 0.0),
            (Measure::Length, _) => {
                let area = match (def.member_area, weighed) {
                    (Some(a), _) => a,
                    (None, true) => WEIGHING_SECTION,
                    (None, false) => {
                        return Err(format!("its members need a cross-section, as `{}`", e.name));
                    }
                };
                (density * area, 0.0)
            }
        })
    };
    // A model's mass fills its proxy when there is nothing else.
    let own: Vec<&Element> = elements.iter().copied().filter(|e| !e.proxy).collect();
    let over = if own.iter().any(|e| weigh(e).is_ok_and(|(w, _)| w > 0.0)) || !weighed {
        own
    } else {
        elements.iter().copied().filter(|e| e.proxy).collect()
    };
    let mut pieces = Vec::new();
    for e in over.into_iter().filter(|e| e.fill != Some(Fill::Cell)) {
        let (per, thickness) = weigh(e)?;
        pieces.push(Piece {
            element: e.name.clone(),
            material: def.material.clone(),
            thickness,
            mass: e.moments.mass(per),
        });
    }
    let Some(total) = def.mass else {
        return Ok((pieces, Mass::ZERO));
    };
    let sum: f64 = pieces.iter().map(|p| p.mass.mass).sum();
    if sum <= 0.0 {
        return Ok((pieces, Mass::point(total, DVec3::ZERO)));
    }
    for piece in &mut pieces {
        let m = &mut piece.mass;
        (m.mass, m.inertia) = (m.mass * total / sum, m.inertia * (total / sum));
    }
    Ok((pieces, Mass::ZERO))
}

fn node(name: &str, position: [f64; 3], direction: [f64; 3], size: f64) -> NodeDef {
    NodeDef {
        name: name.into(),
        position,
        direction,
        size,
    }
}

fn find<'a, T>(records: &'a [T], id: &str, key: impl Fn(&T) -> &String) -> Option<&'a T> {
    records.iter().find(|r| key(r) == id)
}

/// Mixture ratios as shares of one.
fn normalise(propellants: &mut BTreeMap<String, f64>) {
    let total: f64 = propellants.values().sum();
    for ratio in propellants.values_mut() {
        *ratio /= total;
    }
}

fn one() -> f64 {
    1.0
}

fn default_wheel_radius() -> f64 {
    0.3
}

fn default_stroke() -> f64 {
    0.3
}

fn is_point_three(x: &f64) -> bool {
    *x == 0.3
}

#[cfg(test)]
mod tests;
