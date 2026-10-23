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
    /// Height of its base above the stage base, m, to place it higher than it would stack
    /// (a cylinder must still clear the one before it). A sphere sits on the stage base
    /// without it.
    #[serde(default)]
    pub position: Option<f64>,
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

/// A lifting surface on a stage: a wing, tailplane or tail fin. Unlike rocket fins it has
/// an airfoil (so it stalls), can be set at an angle to the hull, and can carry a control
/// surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WingDef {
    #[serde(default)]
    pub name: String,
    /// Height of the root's leading edge above the stage base, m.
    pub position: f64,
    /// Where around the hull the root stands, degrees from the pilot's right (vessel +X)
    /// towards their feet (+Y): 0 a right wing, 270 a fin on top.
    #[serde(default)]
    pub angle: f64,
    /// Chord at the root and at the tip, m.
    pub root_chord: f64,
    pub tip_chord: f64,
    /// From the hull to the tip, m.
    pub span: f64,
    /// How far the tip's leading edge sits behind the root's, m.
    #[serde(default)]
    pub sweep: f64,
    /// Degrees the panel turns about the hull's axis towards smaller `angle` (a right
    /// wing's tip goes up).
    #[serde(default)]
    pub dihedral: f64,
    /// Degrees the leading edge turns towards the lifting side (up, for a wing).
    #[serde(default)]
    pub incidence: f64,
    /// Airfoil thickness over chord.
    #[serde(default = "default_wing_thickness")]
    pub thickness: f64,
    /// All panels together, kg.
    pub mass: f64,
    /// A second panel mirrored across the vessel's plane of symmetry (a left wing for a
    /// right one).
    #[serde(default)]
    pub mirror: bool,
    #[serde(default)]
    pub control: Option<ControlSurfaceDef>,
}

/// A hinged trailing-edge surface: aileron, elevator, rudder, or a mix such as elevons.
/// Which way it moves for a command follows from where it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlSurfaceDef {
    /// The rotation commands that move it.
    pub axes: Vec<ControlAxis>,
    /// Share of the chord, from the trailing edge.
    #[serde(default = "default_control_chord")]
    pub chord: f64,
    /// Where it runs, as shares of the span from the root.
    #[serde(default = "default_control_span")]
    pub span: [f64; 2],
    /// Largest deflection each way, degrees.
    #[serde(default = "default_control_deflection")]
    pub deflection: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ControlAxis {
    Pitch,
    Yaw,
    Roll,
}

impl ControlAxis {
    /// The vessel axis it turns about: X, Y or Z.
    pub fn index(self) -> usize {
        match self {
            ControlAxis::Pitch => 0,
            ControlAxis::Yaw => 1,
            ControlAxis::Roll => 2,
        }
    }
}

fn default_wing_thickness() -> f64 {
    0.12
}

fn default_control_chord() -> f64 {
    0.25
}

fn default_control_span() -> [f64; 2] {
    [0.0, 1.0]
}

fn default_control_deflection() -> f64 {
    20.0
}

/// A landing gear leg: a sprung, damped strut standing out from the hull with a wheel at
/// its end. It rolls freely along the vessel's nose, grips sideways like a tyre, and can
/// brake and steer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GearDef {
    #[serde(default)]
    pub name: String,
    /// Height of the strut's mount above the stage base, m.
    pub position: f64,
    /// Where around the hull it stands, degrees from the pilot's right (vessel +X) towards
    /// their feet (+Y): 90 straight down, less to splay a right-hand leg outwards.
    #[serde(default = "default_gear_angle")]
    pub angle: f64,
    /// From the hull to the wheel's axle, strut fully extended, m.
    pub length: f64,
    /// Wheel radius, m.
    #[serde(default = "default_wheel_radius")]
    pub wheel_radius: f64,
    /// Strut travel, m.
    #[serde(default = "default_stroke")]
    pub stroke: f64,
    /// Mass the leg is sprung for: standing still under it, the strut sits a third of the
    /// way into its travel, kg.
    pub load: f64,
    /// All legs together (with `mirror`), kg.
    pub mass: f64,
    /// A second leg mirrored across the vessel's plane of symmetry.
    #[serde(default)]
    pub mirror: bool,
    /// The wheel has brakes.
    #[serde(default)]
    pub brakes: bool,
    /// The wheel steers with the yaw command, this many degrees each way.
    #[serde(default)]
    pub steering: f64,
}

fn default_gear_angle() -> f64 {
    90.0
}

fn default_wheel_radius() -> f64 {
    0.3
}

fn default_stroke() -> f64 {
    0.3
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
    /// Diameter at the top, m, for the aerodynamics: the pod is a frustum to the air. A
    /// third of the base by default.
    #[serde(default)]
    pub top_diameter: Option<f64>,
    /// glTF model, relative to the mod root.
    #[serde(default)]
    pub model: Option<String>,
}

impl PodDef {
    pub fn top_diameter(&self) -> f64 {
        self.top_diameter.unwrap_or(self.diameter / 3.0)
    }
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
    /// Lowest thrust share while running, 0..1: throttle settings below it run the engine
    /// at this floor. 1.0 means not throttleable.
    #[serde(default = "one")]
    pub min_throttle: f64,
    /// How the thrust follows the throttle: start-up, throttling and shutdown.
    #[serde(default)]
    pub response: EngineResponse,
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
    /// Scales the model evenly, to make a smaller or larger engine from it (`length` and
    /// `exit_diameter` are the scaled engine's).
    #[serde(default = "one")]
    pub model_scale: f64,
    /// Pressure of the exhaust leaving the nozzle at full thrust, Pa. Against the ambient
    /// pressure it decides how the plume expands; estimated from the Isp lost at sea level
    /// if missing.
    #[serde(default)]
    pub exit_pressure: Option<f64>,
    /// Look of the exhaust: id of a `plumes` record. Without one the engine gets the look for
    /// the propellants it burns, if there is one.
    #[serde(default)]
    pub plume: Option<String>,
    /// Where an open-cycle (gas-generator) engine dumps the exhaust that drove its turbine,
    /// which shows beside its plume. Closed-cycle engines burn it in the chamber.
    #[serde(default)]
    pub turbine_exhaust: Option<TurbineExhaust>,
}

/// Where a gas-generator engine's turbine exhaust leaves.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TurbineExhaust {
    /// Out of a pipe of its own (the Merlin's): the centre of its outlet in the engine's
    /// axes (from the gimbal pivot, x and z across, the nozzle down −y) and its diameter,
    /// m, at the scale the model is drawn. It points along the nozzle.
    Pipe { exit: [f64; 3], diameter: f64 },
    /// Into the nozzle, where it lines the wall (the F-1's and J-2's extensions).
    Nozzle,
}

impl EngineDef {
    /// Nozzle exit pressure at full thrust, Pa: the given one, or a guess from how much
    /// Isp the engine loses at sea level (sea-level engines exit near 1 atm, vacuum engines
    /// at about 1 kPa).
    pub fn exit_pressure(&self) -> f64 {
        self.exit_pressure.unwrap_or_else(|| {
            let kept = self.isp_sl / self.isp_vac.max(1.0);
            101_325.0 * ((kept - 0.4) * 1.6).clamp(0.01, 1.0)
        })
    }
}

/// How an engine's thrust follows its throttle, in shares of full thrust. All zero (the
/// default) is an ideal engine that does whatever the throttle says at once.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineResponse {
    /// From ignition to full thrust, s: turbopumps spinning up, chamber pressure building.
    pub spool_up: f64,
    /// Shape of the start as `[time, thrust]` points, both as shares (time of `spool_up`,
    /// thrust of full), from `[0, 0]` to `[1, 1]`, piecewise linear. A start throttled
    /// below full follows it until it reaches the setting.
    pub startup: Vec<[f64; 2]>,
    /// From cutoff until the thrust has tailed off to 1%, s.
    pub spool_down: f64,
    /// Fastest throttling: time to sweep the thrust from zero to full, s.
    pub throttle_time: f64,
}

impl Default for EngineResponse {
    fn default() -> Self {
        Self {
            spool_up: 0.0,
            startup: vec![[0.0, 0.0], [1.0, 1.0]],
            spool_down: 0.0,
            throttle_time: 0.0,
        }
    }
}

impl EngineResponse {
    /// Thrust share `elapsed` seconds into a start.
    pub fn startup_level(&self, elapsed: f64) -> f64 {
        if self.spool_up <= 0.0 || elapsed >= self.spool_up {
            return 1.0;
        }
        let x = (elapsed / self.spool_up).max(0.0);
        let points = &self.startup;
        let Some(end) = points.iter().position(|p| p[0] >= x) else {
            return 1.0;
        };
        if end == 0 {
            return points[0][1].clamp(0.0, 1.0);
        }
        let ([x0, y0], [x1, y1]) = (points[end - 1], points[end]);
        let f = if x1 > x0 { (x - x0) / (x1 - x0) } else { 1.0 };
        (y0 + (y1 - y0) * f).clamp(0.0, 1.0)
    }
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
    /// What each press of the stage key does, in order: lists of activations written
    /// `"<what>:<stage name>"`, where `<what>` is `engines` (light the stage's engines),
    /// `decoupler` (release the joint above the stage, dropping it and everything below),
    /// `fairing` (jettison the stage's fairing) or `rcs` (switch RCS on). Without it the
    /// sequences are made from the stages: see `Propulsion::default_sequences`.
    #[serde(default)]
    pub sequences: Option<Vec<Vec<String>>>,
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
    /// Wings, tailplanes and tail fins.
    #[serde(default)]
    pub wings: Vec<WingDef>,
    /// Landing gear legs.
    #[serde(default)]
    pub gear: Vec<GearDef>,
    /// Reaction wheel part names.
    #[serde(default)]
    pub reaction_wheels: Vec<String>,
    /// RCS block part names; the blocks sit evenly around the stage's forward rim.
    #[serde(default)]
    pub rcs: Vec<String>,
    /// Crew pod part name, at the top of the stage.
    #[serde(default)]
    pub pod: Option<String>,
    /// A payload fairing around the stage, jettisoned by a sequence.
    #[serde(default)]
    pub fairing: Option<FairingDef>,
    /// Extra structural mass (interstage, thrust structure, fairings, avionics, payload),
    /// kg.
    #[serde(default)]
    pub structure_mass: f64,
    /// Outer diameter, m. Used for drag reference area, inertia and rendering.
    pub diameter: f64,
    /// Length, m. Used for the centre of mass, inertia and rendering.
    pub length: f64,
    /// How the stage looks; no effect on physics.
    #[serde(default)]
    pub appearance: StageAppearance,
}

/// A payload fairing: two shells around a stage and a nose, jettisoned in flight.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FairingDef {
    /// Both halves, kg.
    pub mass: f64,
}

/// How a stage is drawn. The hull is built from the stage itself: its tanks are painted
/// barrels with weld seams, the structure above them is an interstage (dark, around the
/// next stage's engines) or more hull, and a top stage without tanks, engines or a pod is a
/// payload fairing with an ogive nose. Colours are linear RGB.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StageAppearance {
    /// Hull paint; default white.
    pub paint: Option<[f32; 3]>,
    /// The interstage; default black composite.
    pub interstage: Option<[f32; 3]>,
    /// Landing legs folded against the base of the stage (for show until legs are parts).
    pub legs: u32,
    /// Grid fins stowed at the top of the stage.
    pub grid_fins: u32,
    /// A cable raceway running up the side of the tanks.
    pub raceway: bool,
}

/// Where and what to spawn when a new game starts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioDef {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// A line for the main menu.
    #[serde(default)]
    pub description: String,
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
    /// At the start of a `launch_sites` record's runway, lined up along it, on its wheels.
    Runway(LaunchSiteStartDef),
    /// Already in orbit. Angles in degrees relative to the body's equator; altitudes in m.
    Orbit(OrbitStartDef),
    /// Flying through the air, as if dropped from a carrier aircraft.
    Flight(FlightStartDef),
}

/// Nose along the flight path, feet towards the ground. Degrees and metres.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlightStartDef {
    pub body: String,
    pub latitude: f64,
    pub longitude: f64,
    /// Above sea level.
    pub altitude: f64,
    /// Clockwise from north.
    #[serde(default)]
    pub heading: f64,
    /// Airspeed, m/s.
    pub speed: f64,
    /// Climb angle of the flight path.
    #[serde(default)]
    pub climb: f64,
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
    #[serde(default)]
    pub runway: Option<RunwayDef>,
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
    /// Height of the launch mount above the deck, m: vehicles stand on it with their
    /// engines over the flame trench.
    #[serde(default)]
    pub mount_height: f64,
}

/// A level, paved runway centred on the site and running along its heading. Vessels start
/// at its near end (`start: { type: "Runway", site }`), facing along it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunwayDef {
    /// Length along the heading and width across it, m.
    pub size: [f64; 2],
    /// Surface height above the mean radius (sea level), m. Defaults to 1 m above the
    /// ground at its centre.
    #[serde(default)]
    pub elevation: Option<f64>,
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
    /// Start in the orbital plane of this body (one that orbits `body`, like the Moon)
    /// instead of `inclination` and `longitude_of_ascending_node`.
    #[serde(default)]
    pub plane: Option<String>,
    #[serde(default)]
    pub argument_of_periapsis: f64,
    #[serde(default)]
    pub mean_anomaly: f64,
    /// Turning at the start, degrees per second about pitch, yaw and roll (vessel X, Y and
    /// Z).
    #[serde(default)]
    pub spin: [f64; 3],
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
