//! Serializable record definitions (the moddable data formats).
//!
//! The game loads these from mod collections (`bodies`, `engines`, `tank_types`, `craft`,
//! `scenarios`, ...); every record has a string `id`. They are turned into runtime types
//! (`SolarSystem`, `Vessel`) which precompute derived values.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::atmosphere::Atmosphere;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemDef {
    /// In any order; parents are resolved by id.
    pub bodies: Vec<BodyDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BodyDef {
    /// Unique id, also the display name.
    pub id: String,
    #[serde(default)]
    pub parent: Option<String>,
    /// Gravitational parameter GM, m³/s².
    pub gm: f64,
    /// Mean radius, m.
    pub radius: f64,
    /// Highest terrain above `radius`, m. Below this (plus a margin) vessels are integrated
    /// so they can collide with the ground.
    #[serde(default)]
    pub max_terrain_height: f64,
    /// Baked terrain dataset (a directory relative to the mod root, e.g.
    /// `"terrain/earth"`). Without one the body is a smooth sphere.
    #[serde(default)]
    pub terrain: Option<String>,
    pub rotation: RotationDef,
    #[serde(default)]
    pub atmosphere: Option<Atmosphere>,
    #[serde(default)]
    pub orbit: Option<OrbitDef>,
    /// Linear RGB used until bodies get real textures.
    pub color: [f32; 3],
}

/// IAU/WGCCRE rotation model, referenced to the ICRF (J2000 equator and equinox).
///
/// The body's north pole points at right ascension `pole_ra` and declination `pole_dec`.
/// The prime meridian's angle W = `prime_meridian` + `rate` · d, where d is days since
/// J2000, measured along the body's equator from its ascending node on the ICRF equator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationDef {
    /// Degrees.
    pub pole_ra: f64,
    pub pole_dec: f64,
    /// W at J2000, degrees.
    pub prime_meridian: f64,
    /// Degrees per day; negative for retrograde rotation.
    pub rate: f64,
}

/// Keplerian elements in degrees, referenced to the J2000 ecliptic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitDef {
    pub semi_major_axis: f64,
    pub eccentricity: f64,
    pub inclination: f64,
    pub longitude_of_ascending_node: f64,
    pub argument_of_periapsis: f64,
    pub mean_anomaly_at_epoch: f64,
    /// Seconds since J2000.
    #[serde(default)]
    pub epoch: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartCatalog {
    pub engines: Vec<EngineDef>,
    #[serde(default)]
    pub resources: Vec<ResourceDef>,
    #[serde(default)]
    pub materials: Vec<MaterialDef>,
    #[serde(default)]
    pub tank_types: Vec<TankTypeDef>,
    #[serde(default)]
    pub reaction_wheels: Vec<ReactionWheelDef>,
    #[serde(default)]
    pub rcs: Vec<RcsDef>,
    #[serde(default)]
    pub pods: Vec<PodDef>,
}

/// A propellant or other stored fluid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDef {
    pub id: String,
    /// Density as stored, kg/m³.
    pub density: f64,
    /// Storage temperature, K. Only tanks insulated down to it can hold it.
    #[serde(default = "room_temperature")]
    pub temperature: f64,
    /// Storage pressure, Pa (compressed gases). Only tanks rated for it can hold it.
    #[serde(default)]
    pub pressure: f64,
}

/// A structural material for tanks and fins.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialDef {
    pub id: String,
    /// kg/m³.
    pub density: f64,
    /// Stress at which it yields, Pa.
    pub yield_strength: f64,
    /// Young's modulus, Pa.
    pub elastic_modulus: f64,
}

/// How a procedural tank is built. Its wall is sized from these (see [`crate::structure`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TankTypeDef {
    pub id: String,
    /// Id of a `materials` record.
    pub material: String,
    /// Operating (ullage) pressure, Pa.
    pub pressure: f64,
    /// Thinnest wall that can be built, m.
    pub min_gauge: f64,
    /// Mass of the stiffening (isogrid ribs, stringers, liner) as a multiple of the bare wall.
    #[serde(default = "one")]
    pub stiffening: f64,
    /// How much more axial load the stiffened wall carries before buckling than a bare
    /// wall of the same thickness.
    #[serde(default = "one")]
    pub buckling_factor: f64,
    /// Insulation, kg per m² of tank surface.
    #[serde(default)]
    pub insulation: f64,
    /// Coldest propellant it can hold, K.
    #[serde(default = "uninsulated")]
    pub min_temperature: f64,
    /// Share of the internal volume that holds propellant; the rest is ullage, baffles and
    /// plumbing.
    #[serde(default = "default_utilization")]
    pub utilization: f64,
    /// Yield strength over the design stress.
    #[serde(default = "default_safety_factor")]
    pub safety_factor: f64,
    /// Acceleration, in g, at which a full tank's hydrostatic pressure is designed for.
    #[serde(default = "default_design_load")]
    pub design_load: f64,
    /// If given, only these resources (on top of the temperature and pressure limits).
    #[serde(default)]
    pub propellants: Option<Vec<String>>,
}

/// A procedural tank in a stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TankDef {
    /// Id of a `tank_types` record.
    #[serde(rename = "type")]
    pub tank_type: String,
    #[serde(default)]
    pub shape: TankShape,
    /// Cylinder length including its domes, m. Cylinders stack up the stage from its base
    /// in the order listed.
    #[serde(default)]
    pub length: f64,
    /// Outer diameter, m. Cylinders default to the stage diameter; spheres need it.
    #[serde(default)]
    pub diameter: Option<f64>,
    /// Acceleration, in g, at which the full tank's hydrostatic pressure is designed for;
    /// defaults to the tank type's. An upper stage rides its booster's peak acceleration
    /// with full tanks, so it needs more than a first stage, which is only full at liftoff.
    #[serde(default)]
    pub design_load: Option<f64>,
    /// What it holds: resource -> mass ratio (normalised). It is filled to its usable
    /// volume in this ratio.
    pub propellants: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TankShape {
    /// A cylinder with elliptical (√2:1) domes, carrying the stack's load in its wall.
    #[default]
    Cylinder,
    /// A sphere mounted inside the stage (e.g. a pressurant bottle), off the load path.
    Sphere,
}

/// A set of identical procedural fins around the base of a stage: flat, solid, trapezoidal
/// plates with their trailing edge at the stage base.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinDef {
    #[serde(default = "default_fin_count")]
    pub count: u32,
    /// Chord along the hull and at the tip, m.
    pub root_chord: f64,
    pub tip_chord: f64,
    /// How far the fin sticks out from the hull, m.
    pub span: f64,
    /// How far the tip's leading edge sits behind the root's, m.
    #[serde(default)]
    pub sweep: f64,
    pub thickness: f64,
    /// Id of a `materials` record.
    pub material: String,
}

/// A crew cabin (capsule or cockpit). It sits at the top of its stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodDef {
    pub id: String,
    /// Mass with crew and consumables, kg.
    pub mass: f64,
    pub crew: u32,
    /// Base diameter and height, m.
    pub diameter: f64,
    pub height: f64,
    /// glTF model, relative to the mod root.
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineDef {
    pub id: String,
    /// Dry mass, kg.
    pub mass: f64,
    /// Vacuum thrust, N.
    pub thrust_vac: f64,
    /// Specific impulse in vacuum and at 1 atm, s.
    pub isp_vac: f64,
    pub isp_sl: f64,
    /// Lowest throttle setting while running, 0..1. 1.0 means not throttleable.
    #[serde(default = "one")]
    pub min_throttle: f64,
    /// Number of ignitions available; `None` = unlimited.
    #[serde(default)]
    pub ignitions: Option<u32>,
    /// Whether the engine needs settled propellant (positive acceleration) to ignite.
    #[serde(default)]
    pub requires_ullage: bool,
    /// How far the engine can swivel from the vessel axis to steer, degrees. 0 = fixed.
    #[serde(default)]
    pub gimbal_range: f64,
    /// Propellants consumed: resource -> mass ratio (normalised at load).
    pub propellants: BTreeMap<String, f64>,
    /// From the gimbal pivot to the nozzle exit, m.
    #[serde(default)]
    pub length: f64,
    /// Nozzle exit diameter, m.
    #[serde(default)]
    pub exit_diameter: f64,
    /// glTF model, relative to the mod root: origin at the gimbal pivot, nozzle down −Y.
    #[serde(default)]
    pub model: Option<String>,
}

/// Torque without propellant, for turning while the engines are off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactionWheelDef {
    pub id: String,
    pub mass: f64,
    /// Torque about each axis (pitch, yaw and roll) at full command, N·m.
    pub torque: f64,
}

/// A block of reaction control thrusters: four small fixed nozzles pushing forward, back and
/// both ways around the vessel axis. They turn the vessel and push it along, e.g. to settle
/// propellant before an engine start, burning their stage's propellant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RcsDef {
    pub id: String,
    /// Dry mass, kg.
    pub mass: f64,
    /// Vacuum thrust of each nozzle, N.
    pub thrust: f64,
    /// Specific impulse in vacuum and at 1 atm, s.
    pub isp_vac: f64,
    pub isp_sl: f64,
    /// Propellants consumed: resource -> mass ratio (normalised at load).
    pub propellants: BTreeMap<String, f64>,
    /// glTF model, relative to the mod root: origin on the hull, +X outward, +Y forward.
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CraftDef {
    pub id: String,
    /// Display name; defaults to the id.
    #[serde(default)]
    pub name: String,
    /// Stages in firing order: index 0 is the first stage (bottom of the stack).
    pub stages: Vec<StageDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageDef {
    pub name: String,
    /// Engine part names; repeat a name for clusters.
    #[serde(default)]
    pub engines: Vec<String>,
    /// Procedural tanks.
    #[serde(default)]
    pub tanks: Vec<TankDef>,
    #[serde(default)]
    pub fins: Option<FinDef>,
    /// Reaction wheel part names.
    #[serde(default)]
    pub reaction_wheels: Vec<String>,
    /// RCS block part names; the blocks sit evenly around the stage's forward rim.
    #[serde(default)]
    pub rcs: Vec<String>,
    /// Crew pod part name, at the top of the stage.
    #[serde(default)]
    pub pod: Option<String>,
    /// Extra structural mass (interstage, thrust structure, fairings, avionics, payload),
    /// kg.
    #[serde(default)]
    pub structure_mass: f64,
    /// Outer diameter, m. Used for drag reference area, inertia and rendering.
    pub diameter: f64,
    /// Length, m. Used for the centre of mass, inertia and rendering.
    pub length: f64,
}

/// Where and what to spawn when a new game starts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Seconds since J2000.
    pub start_time: f64,
    pub craft: String,
    pub start: StartDef,
    /// Other vessels in the game from the start (stations, targets).
    #[serde(default)]
    pub others: Vec<VesselStartDef>,
    /// What the player's vessel starts targeting: a body id or another vessel's name.
    #[serde(default)]
    pub target: Option<String>,
}

/// A vessel placed at the start of a game besides the player's.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VesselStartDef {
    pub craft: String,
    /// Overrides the craft's name.
    #[serde(default)]
    pub name: Option<String>,
    pub start: StartDef,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StartDef {
    /// On the pad of a `launch_sites` record.
    LaunchSite(LaunchSiteStartDef),
    /// Already in orbit. Angles in degrees relative to the body's equator; altitudes in m.
    Orbit(OrbitStartDef),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchSiteStartDef {
    /// Id of a `launch_sites` record.
    pub site: String,
}

/// A place to launch from (collection `launch_sites`). Its pad and roads are built into
/// the terrain, so they exist for collision as well as rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchSiteDef {
    pub id: String,
    /// Display name; defaults to the id.
    #[serde(default)]
    pub name: String,
    pub body: String,
    /// Degrees. Vessels stand at this point, on the pad deck if there is one.
    pub latitude: f64,
    pub longitude: f64,
    /// Direction of the site layout's forward axis, degrees clockwise from north.
    #[serde(default)]
    pub heading: f64,
    #[serde(default)]
    pub pad: Option<PadDef>,
    #[serde(default)]
    pub roads: Vec<RoadDef>,
}

impl LaunchSiteDef {
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }
}

/// A level concrete deck on an embankment, centred on the site.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PadDef {
    /// Deck length along the site heading and width across it, m.
    pub size: [f64; 2],
    #[serde(default)]
    pub corner_radius: f64,
    /// Deck height above the mean radius (sea level), m. Defaults to 1 m above the ground
    /// at the centre.
    #[serde(default)]
    pub elevation: Option<f64>,
    /// Embankment steepness, rise over run.
    #[serde(default = "default_slope")]
    pub slope: f64,
}

/// A paved road, level across its width, following the ground along its length.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoadDef {
    /// Centre line as `[right, forward]` metres from the site centre in the site frame.
    pub points: Vec<[f64; 2]>,
    /// m.
    pub width: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrbitStartDef {
    pub body: String,
    pub periapsis_altitude: f64,
    pub apoapsis_altitude: f64,
    #[serde(default)]
    pub inclination: f64,
    #[serde(default)]
    pub longitude_of_ascending_node: f64,
    #[serde(default)]
    pub argument_of_periapsis: f64,
    #[serde(default)]
    pub mean_anomaly: f64,
}

fn one() -> f64 {
    1.0
}

fn default_slope() -> f64 {
    0.4
}

fn room_temperature() -> f64 {
    293.0
}

/// Bare metal holds liquid oxygen for a launch (it frosts over) but not liquid hydrogen.
fn uninsulated() -> f64 {
    77.0
}

fn default_utilization() -> f64 {
    0.97
}

fn default_safety_factor() -> f64 {
    1.25
}

fn default_design_load() -> f64 {
    2.5
}

fn default_fin_count() -> u32 {
    4
}
