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
        ControlSurfaceDef, EngineDef, EngineResponse, PartCatalog, PodDef, RcsDef,
        ReactionWheelDef, ShellProfile,
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
    /// Panel seams, bands and the like to suit its shape and size, under its own layers
    /// ([`PartDef::build`]): they follow the part when it is resized.
    #[serde(default, skip_serializing_if = "is_false")]
    pub trim: bool,
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
    /// Seams between skin panels, `rings` m apart up the part and `around` to the turn, each row
    /// offset from the last; `seam` m wide. With `around` 0 they lie along the surface's own
    /// coordinates, `across` m apart that way (a wing's skin). Riveted, seen close to.
    Panels {
        rings: f64,
        around: u32,
        #[serde(default, skip_serializing_if = "is_zero")]
        across: f64,
        seam: f64,
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
            LayerDef::Panels { .. } => {}
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
    DockingPort(DockingPortDef),
    LiftingSurface(LiftingSurfaceDef),
    Container(ContainerDef),
    Gear(GearLegDef),
    SurfaceLayer(SurfaceLayerDef),
    FuelCell(FuelCellDef),
    SolarArray(SolarArrayDef),
    Rtg(RtgDef),
    ElectricLoad(ElectricLoadDef),
    Avionics(AvionicsUnitDef),
    Antenna(AntennaDef),
    SolidMotor(SolidMotorDef),
    Parachute(ParachuteDef),
    Swing(SwingDef),
    HeatShield(HeatShieldDef),
    LifeSupport(LifeSupportDef),
    Process(ProcessComponentDef),
    FuelLine(FuelLineDef),
    Experiment(ExperimentComponentDef),
    Drive(DriveDef),
}

/// A scientific instrument's experiment: an `experiments` record, run where it works.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentComponentDef {
    /// The record it runs.
    pub experiment: String,
    /// Running from the start, wherever it works.
    #[serde(default = "yes")]
    pub on: bool,
}

/// Where a part keeps what experiments make: a drive for their data and room for their samples.
/// The experiments on the parts it is wired to write to it (see [`crate::systems::Drive`]).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriveDef {
    /// Bytes of data it holds.
    #[serde(default)]
    pub data: f64,
    /// kg of samples it holds.
    #[serde(default)]
    pub samples: f64,
}

impl DriveDef {
    /// What is wrong with it, if anything.
    pub fn problem(&self) -> Option<String> {
        let room = |v: f64| v.is_finite() && v >= 0.0;
        if !(room(self.data) && room(self.samples)) {
            Some("a drive's room is a figure, nothing or more".into())
        } else if self.data == 0.0 && self.samples == 0.0 {
            Some("a drive has room for data or samples".into())
        } else {
            None
        }
    }
}

/// A line that carries propellant one way, from what the part is on to the part `to` (by its id),
/// across decouplers and all. What it feeds drains what feeds it first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FuelLineDef {
    /// Empty until it is told what to feed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub to: String,
}

/// A part that folds out once when told to: `unfold`. It turns `angle` degrees about `axis`, in
/// its own frame and through its origin, in `seconds`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwingDef {
    pub axis: [f64; 3],
    pub angle: f64,
    pub seconds: f64,
}

/// A canopy, packed until it is armed and low enough.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParachuteDef {
    /// m.
    pub diameter: f64,
    /// Drag coefficient.
    #[serde(default = "default_canopy_drag")]
    pub drag: f64,
    /// It opens below this, m.
    pub deploy_altitude: f64,
    /// It lets go below this, for the next to take over, m; it stays on if left out.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub cut_altitude: f64,
    /// It tears in more dynamic pressure than this, Pa.
    #[serde(default = "default_canopy_pressure")]
    pub max_pressure: f64,
    /// How long it takes to fill, s.
    #[serde(default = "default_opening_time")]
    pub opening_time: f64,
}

impl ParachuteDef {
    /// A canopy `diameter` m across that opens below `deploy_altitude` m and stays on, with the
    /// usual drag, strength and opening time.
    pub fn new(diameter: f64, deploy_altitude: f64) -> Self {
        Self {
            diameter,
            drag: default_canopy_drag(),
            deploy_altitude,
            cut_altitude: 0.0,
            max_pressure: default_canopy_pressure(),
            opening_time: default_opening_time(),
        }
    }
}

fn default_canopy_drag() -> f64 {
    0.8
}

fn default_canopy_pressure() -> f64 {
    30_000.0
}

fn default_opening_time() -> f64 {
    3.0
}

/// An ablator over the surfaces of the part that face one way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeatShieldDef {
    /// kg.
    pub ablator: f64,
    /// The way it faces, in the part's frame.
    #[serde(default = "default_shield_faces")]
    pub faces: [f64; 3],
    /// How far round from there it covers, degrees.
    #[serde(default = "default_shield_angle")]
    pub half_angle: f64,
    /// What burning a kilogram takes away, J.
    #[serde(default = "default_ablation")]
    pub heat_of_ablation: f64,
    /// How far across the air's push moves towards the side it comes from, per unit of the tangent
    /// of the angle it comes at, m: a blunt face turns to meet the air, and with its centre of mass
    /// off the axis it trims at an angle and so lifts.
    #[serde(default = "default_trim_arm")]
    pub trim_arm: f64,
}

fn default_trim_arm() -> f64 {
    0.55
}

fn default_shield_faces() -> [f64; 3] {
    [0.0, 0.0, -1.0]
}

fn default_shield_angle() -> f64 {
    60.0
}

fn default_ablation() -> f64 {
    3e7
}

/// Life support for the people in the part's seats: a cabin. What they need, what its air can take
/// and what it is fitted with come from the rules ([`crate::data::RulesDef::life_support`]); under
/// rules with none, the part is built without it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifeSupportDef {
    /// Days of everything its seats need, and its fitted processes burn, carried in its lockers.
    #[serde(default, skip_serializing_if = "is_zero_days")]
    pub days: f64,
    /// Its cabin, m³; the rules' room per seat if left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
    /// What else its lockers carry, kg.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub carries: BTreeMap<String, f64>,
}

fn is_zero_days(days: &f64) -> bool {
    *days == 0.0
}

/// Runs a record of `processes` on the tanks the part can reach.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessComponentDef {
    /// The record it runs.
    pub process: String,
    /// How many of it: its rates and power times this.
    #[serde(default = "one")]
    pub scale: f64,
    /// When the tanks or the power cannot give everything all it wants, a higher one goes first
    /// and a lower one is shed first.
    #[serde(default = "default_process_priority")]
    pub priority: u8,
    /// Running from the start.
    #[serde(default = "yes")]
    pub on: bool,
}

/// The cabin a `life_support` makes for `seats` people under `profile`, and the processes it is
/// fitted with. Its lockers carry `days` of what they need and what the processes burn, besides
/// what it says it carries.
fn cabin(
    def: &LifeSupportDef,
    profile: &crate::data::LifeSupportProfileDef,
    seats: u32,
    catalog: &PartCatalog,
) -> Result<(LifeSupport, Vec<ProcessUnit>), String> {
    if !(def.days >= 0.0 && def.days.is_finite()) || def.volume.is_some_and(|v| v <= 0.0) {
        return Err("life support carries days of nothing less than nothing, in a cabin".into());
    }
    let people = f64::from(seats);
    let volume = def.volume.unwrap_or(profile.room_per_seat * people);
    if let Some(air) = &profile.air {
        let bad = |gas: &crate::data::GasDef| {
            !(gas.molar_mass > 0.0 && gas.keep >= 0.0 && gas.most() > 0.0 && gas.most() >= gas.keep)
        };
        if !(air.temperature > 0.0 && air.refill > 0.0 && air.vent >= 0.0)
            || air.gases.iter().any(bad)
        {
            return Err(
                "a cabin's air has gases of some weight, kept below their most, at some \
                        temperature, and a regulator that fills it in some time"
                    .into(),
            );
        }
    }
    let mut fitted = Vec::new();
    let mut lockers: BTreeMap<String, f64> = BTreeMap::new();
    // What is breathed from the cabin's air is carried as what tops the air up.
    for need in &profile.needs {
        *lockers
            .entry(profile.supply_of(&need.resource).to_owned())
            .or_default() += need.daily * people * def.days;
    }
    for kit in &profile.fitted {
        let record = (catalog.processes.iter())
            .find(|p| p.id == kit.process)
            .ok_or_else(|| {
                format!(
                    "life support is fitted with `{}`, which is no process",
                    kit.process
                )
            })?;
        let scale = kit.per_seat * people;
        if scale <= 0.0 {
            continue;
        }
        // What it burns that the cabin's air does not give it is carried.
        for (resource, rate) in &record.inputs {
            if !profile.is_air(resource) {
                *lockers.entry(resource.clone()).or_default() += rate * scale * 86_400.0 * def.days;
            }
        }
        fitted.push(ProcessUnit {
            def: record.clone(),
            scale,
            priority: default_process_priority(),
            on: true,
        });
    }
    for (resource, kg) in &def.carries {
        *lockers.entry(resource.clone()).or_default() += kg;
    }
    lockers.retain(|_, kg| *kg > 0.0);
    let life_support = LifeSupport {
        needs: profile.needs.clone(),
        dump: profile.dump.clone(),
        cabin: profile.air.clone().map(|air| CabinAir {
            volume,
            air,
            limits: profile.limits.clone(),
        }),
        lockers: lockers.into_iter().collect(),
    };
    Ok((life_support, fitted))
}

fn default_process_priority() -> u8 {
    100
}

fn yes() -> bool {
    true
}

/// A process as built: its record, as many times over as `scale` says.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessUnit {
    pub def: crate::data::ProcessDef,
    pub scale: f64,
    pub priority: u8,
    pub on: bool,
}

/// Life support as built: what each person aboard needs, and the cabin's stores.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct LifeSupport {
    /// A save from before the rules said what people need has none.
    #[serde(default)]
    pub needs: Vec<crate::data::NeedDef>,
    /// What people make that is let go of when there is nowhere to keep it.
    #[serde(default)]
    pub dump: Vec<String>,
    /// Its air, if the rules give cabins air.
    #[serde(default)]
    pub cabin: Option<CabinAir>,
    /// What its lockers carry, full, kg.
    #[serde(default)]
    pub lockers: Vec<(String, f64)>,
}

/// A cabin's air as built: how big it is, what the air is made of and kept at, and what it has to
/// be for people to be well.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CabinAir {
    /// m³.
    pub volume: f64,
    pub air: crate::data::AirDef,
    pub limits: Vec<crate::data::AirLimitDef>,
}

/// What talks to the ground and to other vessels, in S-band.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AntennaDef {
    /// Of its dish, m; an omni if left out.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub diameter: f64,
    /// What it sends, W; it draws about four times that.
    pub power: f64,
    /// bit/s.
    pub data_rate: f64,
    /// It passes on other vessels' signals to the ground.
    #[serde(default, skip_serializing_if = "is_false")]
    pub relay: bool,
    /// It only sends what the vessel's drives hold, as a sounding rocket's telemetry does: it
    /// hears no commands and passes nothing on, so a vessel does not wait on its link to be flown.
    #[serde(default, skip_serializing_if = "is_false")]
    pub telemetry: bool,
}

/// Guidance and control electronics: with power, it controls a vessel of up to its
/// `controlled_mass`. A vessel with no unit at all flies as it always has, until T087.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvionicsUnitDef {
    /// kg.
    pub controlled_mass: f64,
    /// What it draws, W; none if it is on another load's account.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub power: f64,
    /// It works beyond the Earth's sphere of influence.
    #[serde(default, skip_serializing_if = "is_false")]
    pub interplanetary: bool,
    /// A probe core: in a Realistic game it answers only over a link.
    #[serde(default, skip_serializing_if = "is_false")]
    pub needs_link: bool,
}

/// Burns hydrogen and oxygen from the tanks its part can reach for electricity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FuelCellDef {
    /// The most it makes, W.
    pub power: f64,
}

/// Makes electricity from sunlight. Its face is the part's +X unless it tracks the Sun.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolarArrayDef {
    /// m².
    pub area: f64,
    /// The share of the sunlight it turns into electricity, new.
    #[serde(default = "default_solar_efficiency")]
    pub efficiency: f64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub tracking: bool,
    /// What it loses a year, as a share.
    #[serde(default = "default_solar_degradation")]
    pub degradation: f64,
}

fn default_solar_efficiency() -> f64 {
    0.14
}

fn default_solar_degradation() -> f64 {
    0.01
}

/// Makes electricity from a decaying isotope's heat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RtgDef {
    /// New, W.
    pub power: f64,
    /// Years; plutonium-238's if left out.
    #[serde(default = "default_half_life")]
    pub half_life: f64,
}

fn default_half_life() -> f64 {
    87.7
}

/// Uses electricity: avionics, a transmitter, a cabin, heaters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElectricLoadDef {
    /// W.
    pub watts: f64,
    /// The lowest on a network is shed first.
    #[serde(default = "default_load_priority")]
    pub priority: u8,
    /// Without it the vessel has no SAS and lights no engine.
    #[serde(default, skip_serializing_if = "is_false")]
    pub essential: bool,
}

fn default_load_priority() -> u8 {
    5
}

/// A coat on a part's walls: an ablator, insulation, solar cells, a radiator's panel. It weighs
/// by its area, thickness and material and looks as it says, and has no shape of its own: what
/// has one (a dish, a leg, a hinged panel) is a part. A wall can carry any number of them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceLayerDef {
    /// The wall it covers, by element name; every wall of the part if left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall: Option<String>,
    /// Id of a `materials` record.
    pub material: String,
    /// m.
    pub thickness: f64,
    /// What it looks like from outside; the wall keeps its own look if left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<Look>,
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
    /// How many start aboard; all of them if left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aboard: Option<u32>,
    /// Who sits in the seats, in order; those not named are given names from `crew_names`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
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
    /// What the joint carries before it breaks, N; without limit if left out.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub strength: f64,
}

/// What a docking port mates with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortKind {
    /// The active half of a pair: goes into a drogue.
    Probe,
    Drogue,
    /// Mates with its own kind, either side active.
    #[default]
    Androgynous,
    /// A wide hatch for modules brought together: mates with its own kind.
    Berthing,
}

impl PortKind {
    pub fn mates(self, other: Self) -> bool {
        match self {
            Self::Probe => other == Self::Drogue,
            Self::Drogue => other == Self::Probe,
            Self::Androgynous | Self::Berthing => other == self,
        }
    }
}

/// A ring at one of the part's nodes, as wide as the node, that can capture another vessel's in
/// flight ([`crate::docking`]). The two vessels are then one until they undock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockingPortDef {
    pub node: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: PortKind,
    /// What its springs push the two vessels apart with when they undock, N·s.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub impulse: f64,
    /// Propellant flows across while docked. Power and data always do.
    #[serde(default, skip_serializing_if = "is_false")]
    pub crossfeed: bool,
}

/// A solid motor's nozzle and case: burns what the part's own grain holds, lit once, at full thrust
/// (or as its thrust curve says) until it is gone; it cannot be stopped or throttled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolidMotorDef {
    /// N, in a vacuum.
    pub thrust: f64,
    /// s, in a vacuum and at sea level.
    pub isp_vac: f64,
    pub isp_sl: f64,
    /// Its nozzle and what is not the part's own walls, kg.
    pub mass: f64,
    /// What it burns.
    pub propellant: String,
    /// A booster: it lights with its sequence as any engine does, its nozzle drawn under the part's
    /// base, as wide as its thrust needs. Without, a small separation motor, which fires with the
    /// decoupler it is beside.
    #[serde(default, skip_serializing_if = "is_false")]
    pub booster: bool,
    /// Its nozzle is made for a vacuum: wider, more impulse up high and less at sea level.
    #[serde(default, skip_serializing_if = "is_false")]
    pub vacuum: bool,
    /// Thrust and mass flow as shares of full against the share of its grain left, from full (1)
    /// to empty (0): the shape its grain is cast in. An even burn without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thrust_curve: Option<Vec<[f64; 2]>>,
}

/// How much thrust a booster's nozzle exit makes per square metre of it, N/m²: about a sea-level
/// one's (the Shuttle's boosters, 12.5 MN from 11 m²) and a vacuum one's, which is far wider.
const BOOSTER_EXIT_LOAD: [f64; 2] = [1.1e6, 0.3e6];

/// A booster's specific impulse in a vacuum and at sea level, s, with a nozzle for sea level and
/// with one for a vacuum.
pub const BOOSTER_ISP: [[f64; 2]; 2] = [[265.0, 238.0], [290.0, 190.0]];
/// What a booster's nozzle and its fittings weigh for each newton of thrust, kg/N.
pub const BOOSTER_NOZZLE_MASS: f64 = 1.0e-3;
/// Where a booster's thrust curve is set: the grain left at each point, from full to empty.
pub const CURVE_POINTS: [f64; 5] = [1.0, 0.75, 0.5, 0.25, 0.0];
/// The shapes a grain is cast in, each its thrust at [`CURVE_POINTS`] as a share of its rating, the
/// same on average, so that a booster burns as long whichever it has: an even burn; one that rises
/// as the bore opens; one that falls; and one with a dip for the thickest air, as the Shuttle's had.
pub const CURVE_PRESETS: &[(&str, &str, [f64; 5])] = &[
    ("even", "Even", [1.0, 1.0, 1.0, 1.0, 1.0]),
    ("rising", "Rising", [0.75, 0.88, 1.0, 1.12, 1.25]),
    ("falling", "Falling", [1.25, 1.12, 1.0, 0.88, 0.75]),
    ("max_q", "Dip for max-Q", [1.2, 0.75, 1.05, 1.1, 0.9]),
];

impl SolidMotorDef {
    /// Its nozzle's exit diameter, m, from its thrust: what a booster's is drawn with.
    pub fn exit_diameter(&self) -> f64 {
        let load = BOOSTER_EXIT_LOAD[usize::from(self.vacuum)];
        2.0 * (self.thrust / load / std::f64::consts::PI).sqrt()
    }

    /// Its thrust at each of [`CURVE_POINTS`], as a share of its rating.
    pub fn curve_points(&self) -> [f64; 5] {
        let curve = self.thrust_curve.as_deref().unwrap_or_default();
        CURVE_POINTS.map(|left| {
            (curve.iter())
                .find(|[at, _]| (at - left).abs() < 1e-6)
                .map_or(1.0, |[_, share]| *share)
        })
    }

    /// The preset its curve is, if it is one.
    pub fn curve_preset(&self) -> Option<&'static str> {
        let now = self.curve_points();
        (CURVE_PRESETS.iter())
            .find(|(_, _, points)| points.iter().zip(now).all(|(a, b)| (a - b).abs() < 1e-6))
            .map(|(id, ..)| *id)
    }

    /// Set its thrust at each of [`CURVE_POINTS`]: no curve at all for an even burn.
    pub fn set_curve(&mut self, points: [f64; 5]) {
        self.thrust_curve = (points.iter().any(|share| (share - 1.0).abs() > 1e-9)).then(|| {
            (CURVE_POINTS.iter().zip(points))
                .map(|(&left, share)| [left, share])
                .collect()
        });
    }
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
    /// It ends in a foot, which does not roll: `wheel_radius` is the pad's.
    #[serde(default, skip_serializing_if = "is_false")]
    pub foot: bool,
    /// Where it points folded, in the craft's axes: it stands on nothing until it is deployed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stowed: Option<[f64; 3]>,
}

/// A part, built: measured, sized and weighed.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// How hot it can get, K, and J/kg K: those of what most of it is made of, if that is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specific_heat: Option<f64>,
}

/// A piece of a part's geometry as built.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Piece {
    /// Its element's name.
    pub element: String,
    pub material: Option<String>,
    /// Of a wall or skin, m; 0 for other pieces.
    pub thickness: f64,
    pub mass: Mass,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
        #[serde(default)]
        aboard: Option<u32>,
        /// Who sits in each seat.
        #[serde(default)]
        names: Vec<String>,
        /// What radiation does to them, under rules with radiation.
        #[serde(default)]
        radiation: Option<crate::data::RadiationProfileDef>,
    },
    Decoupler(DecouplerDef),
    DockingPort(DockingPortDef),
    LiftingSurface(LiftingSurface),
    Container(Container),
    Gear(GearLegDef),
    SurfaceLayer(SurfaceLayer),
    FuelCell(FuelCellDef),
    SolarArray(SolarArrayDef),
    Rtg(RtgDef),
    ElectricLoad(ElectricLoadDef),
    Avionics(AvionicsUnitDef),
    Antenna(AntennaDef),
    Parachute(ParachuteDef),
    Swing(SwingDef),
    HeatShield(HeatShieldDef),
    LifeSupport(LifeSupport),
    Process(ProcessUnit),
    FuelLine(FuelLineDef),
    /// An experiment, with its record.
    Experiment {
        def: crate::data::ExperimentDef,
        on: bool,
    },
    Drive(DriveDef),
}

/// A coat on a part's walls, as built.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceLayer {
    /// The walls it covers, by element name.
    pub walls: Vec<String>,
    pub material: String,
    /// m.
    pub thickness: f64,
    /// m².
    pub area: f64,
    /// kg.
    pub mass: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Battery {
    /// kJ.
    pub charge: f64,
    /// The `batteries` record, when it fills a solid.
    pub technology: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Container {
    /// Its inside nodes, in the part's frame.
    pub nodes: Vec<Node>,
}

/// A lifting surface as its loft lays it out: a panel between each pair of stations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

/// The dark and the accent of a part's trim, linear RGB.
const TRIM_DARK: [f32; 3] = [0.035, 0.04, 0.05];
const TRIM_ACCENT: [f32; 3] = [0.87, 0.46, 0.012];

/// How a shape is trimmed ([`PaintDef::trim`]), from its own sizes.
fn trim_layers(shape: &ShapeDef) -> Vec<LayerDef> {
    let band = |low: f64, high: f64, color| LayerDef::Band {
        along: [low, high],
        color,
        finish: None,
    };
    let seam = |diameter: f64| (0.006 * diameter).clamp(0.008, 0.03);
    let panels = |diameter: f64| LayerDef::Panels {
        rings: (0.45 * diameter).clamp(0.4, 3.0),
        around: ((std::f64::consts::PI * diameter / (0.5 * diameter.max(1.5))).round() as u32)
            .clamp(6, 16),
        across: 0.0,
        seam: seam(diameter),
    };
    match shape {
        ShapeDef::CylinderTank(d) => {
            // At the domes' rims, where the barrel starts and ends.
            let dome = d.dome * d.diameter / 2.0;
            let low = dome.max(d.aft_skirt);
            let high = low + (d.length - 2.0 * dome).max(0.0);
            let width = (0.1 * d.diameter)
                .clamp(0.06, 0.6)
                .min(0.2 * (high - low).max(0.0));
            vec![
                panels(d.diameter),
                band(low, low + width, TRIM_DARK),
                band(low + width, low + 1.25 * width, TRIM_ACCENT),
                band(high - 1.25 * width, high - width, TRIM_ACCENT),
                band(high - width, high, TRIM_DARK),
            ]
        }
        ShapeDef::Shell(d) => {
            let length: f64 = d.sections.iter().map(|s| s.length).sum();
            let closes = d.sections.last().and_then(|s| s.top_diameter) == Some(0.0);
            let width = (0.1 * d.bottom_diameter).clamp(0.05, 0.5).min(0.2 * length);
            let mut layers = vec![panels(d.bottom_diameter), band(0.0, width, TRIM_DARK)];
            if closes {
                layers.push(band(0.84 * length, length + 1.0, TRIM_DARK));
                layers.push(band(
                    0.84 * length - 0.25 * width,
                    0.84 * length,
                    TRIM_ACCENT,
                ));
            } else {
                layers.push(band(length - width, length, TRIM_DARK));
                layers.push(LayerDef::Stringers {
                    along: [width, length - width],
                    count: ((std::f64::consts::PI * d.bottom_diameter / 0.3).round() as u32)
                        .clamp(12, 64),
                    depth: 0.012,
                });
            }
            layers
        }
        // A separation ring: marks that say so.
        ShapeDef::Ring(d) => vec![LayerDef::Checker {
            around: 2 * ((std::f64::consts::PI * d.diameter / 0.9).round() as u32).max(3),
            along: vec![-1.0, d.height + 1.0],
            first: false,
            color: TRIM_ACCENT,
            finish: None,
        }],
        ShapeDef::Wing(d) => vec![LayerDef::Panels {
            rings: (0.4 * d.root_chord).clamp(0.3, 1.5),
            around: 0,
            across: (0.5 * d.span).clamp(0.3, 2.0),
            seam: 0.012,
        }],
        _ => Vec::new(),
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
        let mut looks: Vec<(String, Look)> = Vec::new();
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
                    let names = (0..def.seats)
                        .map(|seat| match def.names.get(seat as usize) {
                            Some(name) => name.clone(),
                            None => catalog.crew_name(&self.name, seat),
                        })
                        .collect();
                    Component::Crew {
                        seats: def.seats,
                        aboard: def.aboard,
                        names,
                        radiation: catalog.radiation_profile().map_err(&bad)?.cloned(),
                    }
                }
                ComponentDef::Decoupler(def) => {
                    if !(def.impulse >= 0.0 && def.impulse.is_finite()) {
                        return Err(bad("a decoupler's impulse cannot be negative".into()));
                    }
                    Component::Decoupler(def.clone())
                }
                ComponentDef::DockingPort(def) => {
                    if !(def.impulse >= 0.0 && def.impulse.is_finite()) {
                        return Err(bad("a docking port's impulse cannot be negative".into()));
                    }
                    Component::DockingPort(def.clone())
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
                ComponentDef::FuelCell(def) => {
                    if !(def.power > 0.0 && def.power.is_finite()) {
                        return Err(bad("a fuel cell makes some power".into()));
                    }
                    Component::FuelCell(def.clone())
                }
                ComponentDef::SolarArray(def) => {
                    let fair = def.area > 0.0
                        && (0.0..=1.0).contains(&def.efficiency)
                        && (0.0..1.0).contains(&def.degradation);
                    if !fair {
                        return Err(bad(
                            "a solar array has an area, and its efficiency and degradation are \
                             shares"
                                .into(),
                        ));
                    }
                    Component::SolarArray(def.clone())
                }
                ComponentDef::Rtg(def) => {
                    if !(def.power > 0.0 && def.half_life > 0.0) {
                        return Err(bad("an RTG has a power and a half life".into()));
                    }
                    Component::Rtg(def.clone())
                }
                ComponentDef::ElectricLoad(def) => {
                    if !(def.watts >= 0.0 && def.watts.is_finite()) {
                        return Err(bad("a load draws some power, not less than none".into()));
                    }
                    Component::ElectricLoad(def.clone())
                }
                ComponentDef::SolidMotor(def) => {
                    let fair = def.thrust > 0.0 && def.isp_vac > 0.0 && def.mass >= 0.0;
                    if !fair {
                        return Err(bad("a solid motor has some thrust and impulse".into()));
                    }
                    let curve = def.thrust_curve.as_deref().unwrap_or_default();
                    if curve.iter().any(|[left, share]| {
                        !(0.0..=1.0).contains(left) || !(0.0..=2.0).contains(share)
                    }) {
                        return Err(bad(
                            "a thrust curve's points are a share of grain left (0 to 1) and of \
                             thrust (0 to 2)"
                                .into(),
                        ));
                    }
                    let mut engine = EngineDef {
                        id: format!("{} solid motor", self.name),
                        mass: def.mass,
                        thrust_vac: def.thrust,
                        isp_vac: def.isp_vac,
                        isp_sl: def.isp_sl,
                        min_throttle: 1.0,
                        response: EngineResponse::default(),
                        ignitions: Some(1),
                        requires_ullage: false,
                        gimbal_range: 0.0,
                        propellants: [(def.propellant.clone(), 1.0)].into_iter().collect(),
                        thrust_curve: def.thrust_curve.clone(),
                        // A booster's nozzle is drawn: a cone as long as it is wide.
                        length: if def.booster {
                            def.exit_diameter()
                        } else {
                            0.0
                        },
                        exit_diameter: if def.booster {
                            def.exit_diameter()
                        } else {
                            0.0
                        },
                        model: None,
                        model_scale: 1.0,
                        nodes: Vec::new(),
                        exit_pressure: None,
                        plume: None,
                        turbine_exhaust: None,
                        family: (!def.booster).then(|| crate::propulsion::SEPARATION_MOTOR.into()),
                        kind: Some("solid".into()),
                        pressure_fed: false,
                        electric_power: None,
                        reliability: None,
                        catalog: Default::default(),
                    };
                    normalise(&mut engine.propellants);
                    hardware = hardware + Mass::point(def.mass, DVec3::ZERO);
                    Component::Engine(Box::new(engine))
                }
                ComponentDef::Swing(def) => Component::Swing(def.clone()),
                ComponentDef::Parachute(def) => {
                    if !(def.diameter > 0.0 && def.drag > 0.0 && def.max_pressure > 0.0) {
                        return Err(bad("a parachute has a size, drag and a strength".into()));
                    }
                    Component::Parachute(def.clone())
                }
                ComponentDef::HeatShield(def) => {
                    if !(def.ablator > 0.0 && def.heat_of_ablation > 0.0) {
                        return Err(bad("a heat shield has some ablator to burn".into()));
                    }
                    Component::HeatShield(def.clone())
                }
                ComponentDef::Experiment(def) => {
                    let record = (catalog.experiments.iter())
                        .find(|e| e.id == def.experiment)
                        .ok_or_else(|| {
                            bad(format!("no experiment is called `{}`", def.experiment))
                        })?;
                    if let Some(problem) = record.problem() {
                        return Err(bad(problem));
                    }
                    Component::Experiment {
                        def: record.clone(),
                        on: def.on,
                    }
                }
                ComponentDef::Drive(def) => {
                    if let Some(problem) = def.problem() {
                        return Err(bad(problem));
                    }
                    Component::Drive(*def)
                }
                ComponentDef::Process(def) => {
                    let record = (catalog.processes.iter())
                        .find(|p| p.id == def.process)
                        .ok_or_else(|| bad(format!("no process is called `{}`", def.process)))?;
                    if let Some(problem) = record.problem() {
                        return Err(bad(problem));
                    }
                    if !(def.scale > 0.0 && def.scale.is_finite()) {
                        return Err(bad("a process runs at a scale above nothing".into()));
                    }
                    Component::Process(ProcessUnit {
                        def: record.clone(),
                        scale: def.scale,
                        priority: def.priority,
                        on: def.on,
                    })
                }
                ComponentDef::LifeSupport(def) => {
                    let Some(profile) = catalog.life_support_profile().map_err(&bad)? else {
                        continue;
                    };
                    let seats: u32 = (self.components.iter())
                        .map(|c| match c {
                            ComponentDef::Crew(crew) => crew.seats,
                            _ => 0,
                        })
                        .sum();
                    let (life_support, fitted) =
                        cabin(def, profile, seats, catalog).map_err(&bad)?;
                    components.extend(fitted.into_iter().map(Component::Process));
                    Component::LifeSupport(life_support)
                }
                ComponentDef::Antenna(def) => {
                    if !(def.power > 0.0 && def.data_rate > 0.0 && def.diameter >= 0.0) {
                        return Err(bad(
                            "an antenna sends some power at some data rate, from a dish of no \
                             negative size"
                                .into(),
                        ));
                    }
                    Component::Antenna(def.clone())
                }
                ComponentDef::FuelLine(def) => Component::FuelLine(def.clone()),
                ComponentDef::Avionics(def) => {
                    if !(def.controlled_mass > 0.0 && def.power >= 0.0) {
                        return Err(bad(
                            "avionics control some mass, on no negative power".into()
                        ));
                    }
                    Component::Avionics(def.clone())
                }
                ComponentDef::SurfaceLayer(def) => {
                    let (layer, mass) = surface_layer(def, &elements, catalog).map_err(&bad)?;
                    hardware = hardware + mass;
                    if let Some(look) = def.look {
                        for wall in &layer.walls {
                            looks.push((wall.clone(), look));
                        }
                    }
                    Component::SurfaceLayer(layer)
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
            if let Component::DockingPort(d) = component
                && assembly.node(&d.node).is_none()
            {
                return Err(bad(format!("it has no node `{}` to dock at", d.node)));
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
        let made_of = main_material(&pieces, catalog);
        Ok(BuiltPart {
            name: self.name.clone(),
            geometry,
            components,
            nodes,
            paint: {
                let mut paint = self.paint.clone();
                paint.pieces.extend(looks);
                if paint.trim {
                    let mut layers = trim_layers(&self.shape);
                    layers.append(&mut paint.layers);
                    paint.layers = layers;
                }
                paint
            },
            member: self
                .structure
                .member_area
                .map_or(DRAWN_MEMBER, |a| 2.0 * (a / std::f64::consts::PI).sqrt()),
            pieces,
            mass,
            max_temperature: made_of.map(|m| m.max_temperature),
            specific_heat: made_of.map(|m| m.specific_heat),
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
                    sections: if def.is_solid() {
                        // A straight cone from a wide throat, and the housing of the joint it
                        // swivels on above that: no bell, no pumps.
                        let length = def.nozzle_length();
                        vec![
                            ShellSection {
                                length: 0.75 * length,
                                top_diameter: Some(0.45 * def.exit_diameter),
                                profile: ShellProfile::Conic,
                            },
                            ShellSection {
                                length: 0.25 * length,
                                top_diameter: Some(0.7 * def.exit_diameter),
                                profile: ShellProfile::Conic,
                            },
                        ]
                    } else {
                        vec![ShellSection {
                            length: def.length,
                            top_diameter: Some(def.exit_diameter / 3.0),
                            profile: ShellProfile::Parabolic,
                        }]
                    },
                    span: None,
                    cap_bottom: false,
                    cap_top: false,
                }),
                position: [0.0, 0.0, -def.nozzle_length()],
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
        // Under the nozzle's exit: what the next stage down hangs from, as under a KSP engine. A
        // decoupler put there grows an interstage up round the engine ([`crate::fitting`]).
        if !nodes.iter().any(|n| n.name == "bottom") && def.exit_diameter > 0.0 {
            let exit = -def.nozzle_length();
            nodes.push(node(
                "bottom",
                [0.0, 0.0, exit],
                [0.0, 0.0, -1.0],
                def.exit_diameter,
            ));
        }
        let shape = match (&def.model, bell) {
            (Some(path), bell) => ShapeDef::Model(ModelDef {
                fit: 0.0,
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
                    fit: 0.0,
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
            components.push(ComponentDef::Crew(CrewDef {
                seats: def.crew,
                aboard: None,
                names: Vec::new(),
            }));
            if def.supplies > 0.0 {
                components.push(ComponentDef::LifeSupport(LifeSupportDef {
                    days: def.supplies,
                    ..LifeSupportDef::default()
                }));
            }
        }
        if let Some(drive) = def.drive() {
            components.push(ComponentDef::Drive(drive));
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
        let block = ShapeDef::Compound(vec![PlacedShapeDef {
            name: String::new(),
            shape: ShapeDef::Cuboid(BoxDef {
                size: [0.12, 0.12, 0.2],
                fill: Fill::Solid,
            }),
            // Below the mount, so that a block on a stage's rim stays on its stage.
            position: [0.06, 0.0, -0.2],
            rotation: [0.0; 3],
            unseen: false,
        }]);
        let shape = match &def.model {
            Some(path) => ShapeDef::Model(ModelDef {
                fit: 0.0,
                path: path.clone(),
                scale: 1.0,
                proxy: Some(Box::new(block)),
                nodes: vec![mount],
            }),
            None => block,
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
            // Built docked, it lets go as it would if it had docked in flight.
            Component::DockingPort(d) => {
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

/// A coat of `def` on its walls, and what it weighs where it lies.
fn surface_layer(
    def: &SurfaceLayerDef,
    elements: &[Element],
    catalog: &PartCatalog,
) -> Result<(SurfaceLayer, Mass), String> {
    if !(def.thickness > 0.0 && def.thickness.is_finite()) {
        return Err(format!("a layer's thickness is {} m", def.thickness));
    }
    let material = find(&catalog.materials, &def.material, |m| &m.id)
        .ok_or_else(|| format!("unknown material `{}`", def.material))?;
    let walls: Vec<&Element> = match &def.wall {
        Some(name) => {
            let wall = element(elements, name)?;
            if wall.measure != Measure::Area {
                return Err(format!("a layer lies on a wall, and `{name}` is none"));
            }
            vec![wall]
        }
        None => elements
            .iter()
            .filter(|e| e.measure == Measure::Area && !e.proxy)
            .collect(),
    };
    if walls.is_empty() {
        return Err("a layer needs a wall to lie on".into());
    }
    let areal = material.density * def.thickness;
    let mass = walls.iter().map(|w| w.moments.mass(areal)).sum();
    let area: f64 = walls.iter().map(|w| w.moments.measure).sum();
    Ok((
        SurfaceLayer {
            walls: walls.iter().map(|w| w.name.clone()).collect(),
            material: def.material.clone(),
            thickness: def.thickness,
            area,
            mass: area * areal,
        },
        mass,
    ))
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

/// What most of `pieces` is made of, by weight.
fn main_material<'a>(
    pieces: &[Piece],
    catalog: &'a PartCatalog,
) -> Option<&'a crate::data::MaterialDef> {
    let mut by_weight: BTreeMap<&str, f64> = BTreeMap::new();
    for piece in pieces {
        if let Some(material) = &piece.material {
            *by_weight.entry(material).or_default() += piece.mass.mass;
        }
    }
    let (id, _) = (by_weight.into_iter()).max_by(|a, b| a.1.total_cmp(&b.1))?;
    catalog.materials.iter().find(|m| m.id == id)
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
