//! Engine exhaust plumes: one volume per cluster of engines that share a look, ray-marched
//! in `shaders/plume.wesl`.
//!
//! The look comes from a `plumes` record: the colours the gas glows in, the particles
//! (soot) it carries and how hot they burn, and the haze it condenses into as it cools.
//! The shape of each engine's jet comes from its state: its thrust level (a start grows the
//! plume out of the nozzle, throttling shortens and dims it) and its exit pressure against
//! the ambient pressure. A jet that leaves the nozzle at or below the ambient pressure stays
//! a narrow column with shock diamonds; as the air thins it overtakes the ambient pressure,
//! the cells stretch and the plume fans out and fades, until in vacuum it is a wide, faint
//! cone. The jets of a cluster spread into each other and merge into one plume, which burns
//! as long as one engine of the cluster's whole exit area would.
//!
//! The plume hangs off the vessel, not the gimbals. Its axis is where the exhaust has been
//! going lately (the cluster's nozzle axis smoothed over time), bent towards the airflow in
//! air; each jet leaves along its nozzle's current axis and turns onto the plume's within a
//! few metres, so a quick gimbal move shows near the nozzles instead of swinging the whole
//! plume.

use bevy::{
    asset::embedded_asset,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};
use serde::{Deserialize, Serialize};

use crate::settings::Quality;

/// 1 atm, Pa.
const ATM: f64 = 101_325.0;

pub struct PlumePlugin;

impl Plugin for PlumePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/plume.wesl");
        app.add_plugins(MaterialPlugin::<PlumeMaterial>::default());
    }
}

/// How an exhaust looks (a `plumes` record). Colours are linear RGB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlumeDef {
    pub id: String,
    /// The gas's own glow: the hot core just behind the nozzle...
    pub core: [f32; 3],
    /// ...the body of the plume...
    pub flame: [f32; 3],
    /// ...and the shock diamonds.
    pub shock: [f32; 3],
    /// Brightness of a metre of the glowing gas at full thrust, cd/m² per m.
    pub luminance: f32,
    /// Particles in the exhaust (soot, or alumina from a solid motor), 0..1: they hide what
    /// is behind them, glow at their temperature and are smoke once they cool.
    #[serde(default)]
    pub soot: f32,
    /// Hottest temperature of the particles, K, where the exhaust afterburns with the air;
    /// 0: they don't glow.
    #[serde(default)]
    pub temperature: f32,
    /// Colour of the particles as smoke in daylight (their albedo).
    #[serde(default = "soot_colour")]
    pub smoke: [f32; 3],
    /// Exhaust that condenses as it cools (mostly water), 0..1: the white trail in the air
    /// and the sunlit cloud around a plume in thin air.
    #[serde(default)]
    pub haze: f32,
    /// What it is the exhaust of: engines without a `plume` of their own get the look whose
    /// propellants they all burn (the one that names the most). An entry is one propellant
    /// or several alternatives joined by `|` (`"Kerosene|RP-1"`: either will do); a cooled
    /// propellant counts as the plain one.
    #[serde(default)]
    pub propellants: Vec<String>,
    /// Kinds of engine (`solid`, `liquid`) it is the exhaust of, whatever they burn; empty:
    /// any kind. Together with `propellants`, both must hold.
    #[serde(default)]
    pub kinds: Vec<String>,
    /// The look for an engine that no other look is for, so none goes without a plume.
    /// Electric engines are never given one.
    #[serde(default)]
    pub fallback: bool,
}

impl PlumeDef {
    /// The look for an engine: the one it names, or else the one for what it burns and what
    /// kind of engine it is (the most specific), or else the fallback. Electric engines have
    /// none unless they name one.
    pub fn for_engine<'a>(
        looks: &'a [PlumeDef],
        named: Option<&str>,
        burns: &[&str],
        kind: Option<&str>,
    ) -> Option<&'a PlumeDef> {
        if let Some(id) = named {
            return looks.iter().find(|l| l.id == id);
        }
        if kind == Some("electric") {
            return None;
        }
        // A cooled (densified) propellant burns as the plain one does.
        let plain = |p: &str| p.strip_prefix("Cooled").unwrap_or(p).to_owned();
        let burns: Vec<String> = burns.iter().map(|b| plain(b)).collect();
        let burnt = |entry: &String| entry.split('|').any(|p| burns.contains(&plain(p.trim())));
        looks
            .iter()
            .filter(|l| !l.fallback && !(l.propellants.is_empty() && l.kinds.is_empty()))
            .filter(|l| l.kinds.is_empty() || kind.is_some_and(|k| l.kinds.iter().any(|x| x == k)))
            .filter(|l| l.propellants.iter().all(burnt))
            .max_by_key(|l| l.propellants.len() + l.kinds.len())
            .or_else(|| looks.iter().find(|l| l.fallback))
    }
}

fn soot_colour() -> [f32; 3] {
    [0.2, 0.18, 0.16]
}

/// The shape of one engine's jet for its state, in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlumeShape {
    pub exit_radius: f32,
    pub level: f32,
    /// 0 for a jet at or below the ambient pressure (a column), 1 in vacuum (a wide fan).
    pub expansion: f32,
    /// Length of a shock cell, and how strongly the cells show, 0..1.
    pub cell: f32,
    pub shock: f32,
    /// How bright the Mach disk ending each cell is, 0..1, and its radius as a share of the
    /// exit radius: a jet well below the ambient pressure (an RS-25 at sea level) or well
    /// above it ends its cells in a flat, white-hot disk of reheated exhaust.
    pub disk: f32,
    pub disk_radius: f32,
    /// Distance over which the glow falls to 1/e.
    pub fade: f32,
    /// How far the flame stays hot before mixing (in air) or expanding (in vacuum) cools it.
    pub flame: f32,
    /// Tangent of the half-angle the jet's edge spreads at.
    pub spread: f32,
    /// Brightness of the gas relative to the plume at sea level: exhaust expanding into
    /// vacuum cools fast and has no air to keep burning with.
    pub glow: f32,
    /// Temperature of the exhaust leaving the nozzle as a share of the particles' hottest:
    /// the more the nozzle expands it, the cooler it leaves.
    pub heat: f32,
    /// How many times the look's particles the jet carries, and over how many metres from
    /// the exit it catches fire with the air.
    pub soot: f32,
    /// How far it takes to catch fire with the air, m.
    pub afterburn: f32,
    /// Fuel-rich turbine exhaust lining the nozzle wall, 0..1: a dark, sooty collar around
    /// the jet that only catches fire a nozzle diameter or so out (the F-1's).
    pub film: f32,
    /// Distance from the nozzle exit along the axis to the ground (infinite when clear): the
    /// jet stops there and spreads out along it.
    pub ground: f32,
    /// The nozzle axis against the plume's, as the tangent of the angle between them in the
    /// plume's cross-section (x, z): the jet leaves along the nozzle and turns onto the plume
    /// axis over `bend_length`.
    pub bend: Vec2,
    pub bend_length: f32,
    /// Bounding cone: length, radius at the nozzle and growth per metre.
    pub length: f32,
    pub bound_radius: f32,
    pub bound_spread: f32,
}

/// How much wider the jet gets where it splashes on the ground.
const SPLASH: f32 = 2.5;
/// Share of the fade length over which the jet turns from the nozzle axis onto the plume's.
const BEND_LENGTH: f32 = 0.3;
/// Largest bend drawn, as the tangent of the angle between nozzle and plume axis.
const MAX_BEND: f32 = 1.0;
/// Seconds over which the plume's axis follows the nozzle's.
pub const AXIS_LAG: f32 = 0.4;
/// How strongly the airflow bends the plume, against the square root of the ratio of the
/// air's dynamic pressure to the jet's momentum flux, and the most it can (below 1, so
/// the airflow never turns a plume blowing into it all the way round).
const CROSSFLOW: f32 = 2.0;
const MAX_CROSSFLOW: f32 = 0.6;
/// The bounds reach this many jet widths out: past them the jet's edge, however the eddies
/// push it, is too thin to see.
const BOUND_WIDTHS: f32 = 2.6;
/// Exit pressure of a typical sea-level engine at full thrust, Pa: its exhaust leaves at
/// `SEA_LEVEL_HEAT` of the flame's hottest.
const SEA_LEVEL_EXIT: f64 = 70_000.0;
const SEA_LEVEL_HEAT: f64 = 0.9;
/// Extinction by a plume's particles, per metre, for a load of 1 at the exhaust's density
/// at the exit of a nozzle 1 m across: soot makes a kerosene plume opaque.
const PARTICLE_EXTINCTION: f32 = 6.0;
/// Air temperature for the particles' glow, K.
const AIR: f32 = 290.0;

impl PlumeShape {
    /// `merged_radius` is the radius of a nozzle with the whole exit area of the engine's
    /// cluster (its own for a single engine): merged jets burn longer. `exit_pressure` is
    /// the engine's at full thrust, `ambient` the air's, both Pa; `level` the thrust share;
    /// `ground` how far the ground is from the nozzle exit along the axis, m (infinite when
    /// clear); `bend` the nozzle axis against the plume's (see [`plume_axes`]).
    pub fn new(
        exit_radius: f32,
        merged_radius: f32,
        exit_pressure: f64,
        level: f64,
        ambient: f64,
        ground: f32,
        bend: Vec2,
    ) -> Self {
        let level = level.clamp(0.0, 1.0);
        // Exit pressure scales with chamber pressure, so with thrust.
        let ratio = exit_pressure * level / ambient.max(1e-3);
        let expansion = smoothstep(0.0, 1.0, ratio.max(1.0).log10() / 2.5) as f32;
        let air = (ambient / ATM).clamp(0.0, 1.0);
        // Cells stretch as the jet's pressure overtakes the ambient and fade out once it
        // fans out; they show best in dense air.
        let cell = 2.0 * exit_radius * (0.8 + 0.7 * ratio.min(9.0).sqrt() as f32);
        let shock = smoothstep(0.02, 0.25, air) as f32 * (1.0 - expansion).powi(2);
        let (disk, disk_radius) = if ratio < 1.0 {
            (smoothstep(0.95, 0.35, ratio), 0.3 + 0.35 * (1.0 - ratio))
        } else {
            (
                smoothstep(1.5, 4.0, ratio),
                (0.3 + 0.1 * ratio.sqrt()).min(0.8),
            )
        };
        let fade_per_radius = (24.0 + 36.0 * (level as f32).sqrt()) * (1.0 - 0.5 * expansion);
        let fade = merged_radius.max(exit_radius) * fade_per_radius;
        // In air the jet mixes with it and widens slowly; in vacuum it fans out (the faint
        // edge goes wider still, but the glow stays within about 35°).
        let spread = 0.1 + 0.6 * expansion.powf(1.3);
        let heat = (SEA_LEVEL_HEAT * (exit_pressure * level / SEA_LEVEL_EXIT).max(1e-9).powf(0.1))
            .clamp(0.45, 0.95) as f32;
        let ground = ground.max(0.0);
        let length = (4.0 * fade).min(ground);
        // Room for the shock-cell bulges and the eddies at the edge, and for the splash.
        let bound_spread = BOUND_WIDTHS * spread;
        let mut bound_radius = BOUND_WIDTHS * exit_radius;
        if ground <= length {
            let splash = BOUND_WIDTHS * (1.0 + SPLASH) * (exit_radius + ground * spread);
            bound_radius = bound_radius.max(splash - bound_spread * ground);
        }
        // The bent jet's centre is at most `bend · bend_length` off the axis; it turns as
        // fast as a lone jet would.
        let bend = bend.clamp_length_max(MAX_BEND);
        let bend_length = BEND_LENGTH * exit_radius * fade_per_radius;
        bound_radius += bend.length() * bend_length;
        Self {
            exit_radius,
            level: level as f32,
            expansion,
            cell,
            shock,
            disk: disk as f32,
            disk_radius: disk_radius as f32,
            fade,
            flame: 0.8 * fade,
            spread,
            glow: 1.0 - 0.85 * expansion,
            heat,
            soot: 1.0,
            afterburn: 6.0 * exit_radius,
            film: 0.0,
            ground,
            bend,
            bend_length,
            length,
            bound_radius,
            bound_spread,
        }
    }
}

/// One jet of a cluster as it is this frame, in vessel render axes.
#[derive(Debug, Clone, Copy)]
pub struct Jet {
    /// Centre of the nozzle exit (or pipe outlet), and the way it points.
    pub exit: Vec3,
    pub nozzle: Vec3,
    pub radius: f32,
    /// Nozzle exit pressure at full thrust, Pa.
    pub exit_pressure: f64,
    pub level: f64,
    pub kind: JetKind,
}

/// What leaves a jet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JetKind {
    /// An engine's exhaust; `film`: its turbine exhaust lines the nozzle wall.
    Main { film: bool },
    /// The fuel-rich exhaust that drove an open-cycle engine's turbine, from a pipe of its
    /// own: cooler, sooty and dim, a dark streak beside the plume that only catches fire
    /// once it has mixed with the air.
    TurbineExhaust,
}

/// Turbine exhaust leaves its pipe at about the ambient pressure, at this share of the
/// flame's hottest temperature, with this many times the main exhaust's soot, and catches
/// fire this many pipe diameters out.
const TURBINE_EXHAUST_HEAT: f32 = 0.42;
const TURBINE_EXHAUST_SOOT: f32 = 3.0;
const TURBINE_EXHAUST_IGNITION: f32 = 40.0;

/// The merged plume of a cluster of engines: a volume from the exit plane (where it
/// crosses the plume's axis) down the axis.
#[derive(Debug, Clone)]
pub struct PlumeCluster {
    pub origin: Vec3,
    /// Turns the volume (exhaust along −Y) onto the plume's axis.
    pub rotation: Quat,
    /// Each jet's shape, where its nozzle exit is across the axis (x, z) and how far down
    /// the axis.
    pub jets: Vec<(PlumeShape, Vec2, f32)>,
    /// Distance from the exit plane along the axis to the ground (infinite when clear).
    pub ground: f32,
    /// Bounding cone: length, radius at the exit plane and growth per metre.
    pub length: f32,
    pub bound_radius: f32,
    pub bound_spread: f32,
}

impl PlumeCluster {
    /// `axis` is where the plume runs (see [`plume_axis`]), `ambient` the air's pressure,
    /// Pa. `height` is how high the vessel's base is above the ground, m, and `up` the local
    /// vertical, for where the plume meets the ground.
    pub fn new(jets: &[Jet], axis: Vec3, ambient: f64, height: f32, up: Vec3) -> Self {
        let rotation = Quat::from_rotation_arc(Vec3::NEG_Y, axis);
        // The axis through the middle of the nozzles by exit area (a turbine exhaust pipe
        // hardly counts), the exit plane through the outlet nearest the vessel.
        let area = jets
            .iter()
            .map(|j| j.radius * j.radius)
            .sum::<f32>()
            .max(1e-9);
        let mean = jets
            .iter()
            .map(|j| j.exit * j.radius * j.radius)
            .sum::<Vec3>()
            / area;
        let first = jets
            .iter()
            .map(|j| (j.exit - mean).dot(axis))
            .fold(f32::INFINITY, f32::min);
        let origin = mean + axis * first.min(0.0);
        let falling = -axis.dot(up);
        let ground = if falling > 0.2 {
            (height + origin.dot(up)) / falling
        } else {
            f32::INFINITY
        };
        let merged = jets.iter().map(|j| j.radius * j.radius).sum::<f32>().sqrt();
        let jets: Vec<_> = jets
            .iter()
            .map(|jet| {
                let local = rotation.inverse() * (jet.exit - origin);
                let along = (-local.y).max(0.0);
                let (_, bend) = plume_axes(axis, jet.nozzle);
                let mut shape = PlumeShape::new(
                    jet.radius,
                    merged,
                    jet.exit_pressure,
                    jet.level,
                    ambient,
                    ground - along,
                    bend,
                );
                match jet.kind {
                    JetKind::Main { film } => shape.film = if film { 1.0 } else { 0.0 },
                    JetKind::TurbineExhaust => {
                        shape.heat = TURBINE_EXHAUST_HEAT;
                        shape.soot = TURBINE_EXHAUST_SOOT;
                        shape.afterburn = TURBINE_EXHAUST_IGNITION * 2.0 * jet.radius;
                        shape.glow *= 0.1;
                        shape.shock = 0.0;
                        shape.disk = 0.0;
                    }
                }
                (shape, Vec2::new(local.x, local.z), along)
            })
            .collect();
        // A cone around every jet's.
        let mut length = 0.0f32;
        let mut bound_radius = 0.0f32;
        let mut bound_spread = 0.0f32;
        for (shape, across, along) in &jets {
            length = length.max(along + shape.length);
            bound_radius =
                bound_radius.max(across.length() + shape.bound_radius - shape.bound_spread * along);
            bound_spread = bound_spread.max(shape.bound_spread);
        }
        Self {
            origin,
            rotation,
            jets,
            ground,
            length,
            bound_radius,
            bound_spread,
        }
    }

    /// Radius of the bounding cone at its far end.
    pub fn end_radius(&self) -> f32 {
        self.bound_radius + self.bound_spread * self.length
    }

    /// The hottest part of the flame, which lights its smoke and the vessel and ground
    /// around it: its luminance (linear RGB, cd/m²), radius and length, m.
    pub fn flame_glow(&self, look: &PlumeDef) -> (Vec3, f32, f32) {
        // By exit area: turbine exhaust pipes hardly count.
        let area = self
            .jets
            .iter()
            .map(|(s, ..)| s.exit_radius.powi(2))
            .sum::<f32>()
            .max(1e-9);
        let biggest = self
            .jets
            .iter()
            .map(|(s, ..)| s.exit_radius)
            .fold(1e-6, f32::max);
        let n = area / (biggest * biggest);
        let mean = |f: &dyn Fn(&PlumeShape) -> f32| {
            self.jets
                .iter()
                .map(|(s, ..)| f(s) * s.exit_radius.powi(2))
                .sum::<f32>()
                / area
        };
        let level = mean(&|s| s.level);
        // Expanding into vacuum the exhaust cools within a few nozzle radii.
        let hot =
            mean(&|s| s.flame + (4.0 * s.exit_radius - s.flame) * s.expansion).min(self.ground);
        let radius = self
            .jets
            .iter()
            .map(|(s, across, _)| across.length() + s.exit_radius + 0.5 * hot * s.spread)
            .fold(0.0, f32::max);
        let exit = mean(&|s| s.exit_radius);
        // The gas: a column of it about as wide as the merged jets.
        let gas = look.luminance * mean(&|s| s.level * s.glow) * 2.0 * exit * n.sqrt();
        let mut luminance = Vec3::from(look.flame) * gas;
        // The particles: a black body at the flame's temperature, as far as they are dense
        // enough to hide what is behind them.
        if look.temperature > 0.0 && look.soot > 0.0 {
            let share = mean(&|s| s.heat + (1.0 - s.heat) * (1.0 - s.expansion));
            let kelvin = AIR + (look.temperature - AIR) * 0.92 * share;
            let depth = PARTICLE_EXTINCTION * look.soot * level * 1.3 * n.sqrt();
            let albedo = Vec3::from(look.smoke);
            luminance += blackbody(kelvin) * (Vec3::ONE - albedo) * (1.0 - (-depth).exp());
        }
        (luminance, radius, hot)
    }
}

/// Where the far part of a plume runs, in vessel axes: along `exhaust` (where the nozzles
/// have been pointing lately), bent towards `airflow` (the air's velocity past the vessel,
/// m/s) the more, the stronger the air is against the jet. `dynamic_pressure` is the air's,
/// `jet` the jet's momentum flux (its thrust over the exit area), both Pa.
pub fn plume_axis(exhaust: Vec3, airflow: Vec3, dynamic_pressure: f32, jet: f32) -> Vec3 {
    let Some(downwind) = airflow.try_normalize() else {
        return exhaust;
    };
    let push = if jet > 0.0 {
        (CROSSFLOW * (dynamic_pressure / jet).sqrt()).min(MAX_CROSSFLOW)
    } else {
        MAX_CROSSFLOW
    };
    (exhaust + push * downwind).normalize()
}

/// The rotation that turns a plume volume (exhaust along −Y) onto `axis`, and the bend of
/// `nozzle` against it for [`PlumeShape::new`].
pub fn plume_axes(axis: Vec3, nozzle: Vec3) -> (Quat, Vec2) {
    let rotation = Quat::from_rotation_arc(Vec3::NEG_Y, axis);
    let local = rotation.inverse() * nozzle;
    let bend = Vec2::new(local.x, local.z) / (-local.y).max(0.5);
    (rotation, bend)
}

/// Radiance of a black body at `kelvin` in linear Rec. 709, cd/m² of luminance: Planck's
/// law at one wavelength per channel, fitted to the CIE 1931 observer from 1100 to 3500 K.
/// The shader has the same.
pub fn blackbody(kelvin: f32) -> Vec3 {
    let channel =
        |amplitude: f32, c2: f32| amplitude / ((c2 / kelvin.max(1.0)).min(80.0).exp() - 1.0);
    Vec3::new(
        channel(1.144e11, 23_021.0),
        channel(1.951e11, 26_768.0),
        channel(1.199e12, 35_747.0),
    )
}

/// Luminance of linear Rec. 709.
fn luminance(rgb: Vec3) -> f32 {
    rgb.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Most engines drawn in one plume; a bigger cluster is drawn as several.
pub const MAX_JETS: usize = 32;

/// One jet, for the shader.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
pub struct JetParams {
    /// xy: the nozzle exit across the plume axis (x, z), z: how far down the axis it is,
    /// w: exit radius; metres.
    pub exit: Vec4,
    /// xy: tangent of the nozzle axis against the plume axis (x, z), z: bend length, w:
    /// thrust level.
    pub bend: Vec4,
    /// x: expansion, y: shock cell length, z: shock strength, w: glow fade length.
    pub flow: Vec4,
    /// x: spread (tan of the half-angle), y: the gas's brightness (level times glow), z:
    /// exit temperature share, w: hot length.
    pub heat: Vec4,
    /// x: Mach disk brightness, y: its radius as a share of the exit radius.
    pub disk: Vec4,
    /// x: particles as a multiple of the look's, y: how far it takes to catch fire with the
    /// air, m, z: turbine exhaust film on the nozzle wall.
    pub mix: Vec4,
}

#[derive(Debug, Clone, Copy, Default, ShaderType)]
pub struct PlumeParams {
    /// x: bounding length, y: bounding radius at the far end (the mesh is a unit cylinder
    /// scaled to these), z: noise seed, w: number of jets.
    pub shape: Vec4,
    /// x: bounding cone radius at the exit plane, y: its growth per metre, z: distance to
    /// the ground along the axis, w: how much wider a jet gets where it splashes on it.
    pub bounds: Vec4,
    /// The gas's colours; `core.w`: its luminance (cd/m² per m at full thrust).
    pub core: Vec4,
    pub flame: Vec4,
    pub shock: Vec4,
    /// rgb: the particles' albedo, w: their load.
    pub smoke: Vec4,
    /// x: the particles' hottest temperature (K, 0 if they don't glow), y: haze, z: eddy
    /// size at the exit, m.
    pub medium: Vec4,
    /// The hottest part of the flame, which lights the smoke: rgb its luminance (cd/m²),
    /// w its radius.
    pub glow: Vec4,
    /// xyz: the local vertical (world axes), w: the hot flame's length.
    pub up: Vec4,
    /// x: ray-marching steps, y: steps towards the sun, z: noise octaves.
    pub quality: Vec4,
    pub jets: [JetParams; MAX_JETS],
}

/// How finely a plume is drawn at each `effects` setting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlumeQuality {
    /// Ray-marching steps through the plume (the most; rays across it take fewer).
    pub steps: u32,
    /// Steps towards the sun for the smoke's shadows (0: estimated from the smoke's width).
    pub shadow_steps: u32,
    /// Octaves of turbulence.
    pub octaves: u32,
}

impl PlumeQuality {
    pub fn new(quality: Quality) -> Self {
        let (steps, shadow_steps, octaves) = match quality {
            Quality::Off | Quality::Low => (16, 0, 2),
            Quality::Medium => (32, 2, 3),
            Quality::High => (48, 3, 4),
            Quality::Ultra => (96, 5, 5),
        };
        Self {
            steps,
            shadow_steps,
            octaves,
        }
    }
}

impl PlumeParams {
    /// `up` is the local vertical in world axes.
    pub fn new(
        cluster: &PlumeCluster,
        look: &PlumeDef,
        seed: f32,
        quality: PlumeQuality,
        up: Vec3,
    ) -> Self {
        let rgb = |c: [f32; 3], w: f32| Vec4::new(c[0], c[1], c[2], w);
        let mut jets = [JetParams::default(); MAX_JETS];
        for (out, (shape, across, along)) in jets.iter_mut().zip(&cluster.jets) {
            *out = JetParams {
                exit: Vec4::new(across.x, across.y, *along, shape.exit_radius),
                bend: Vec4::new(shape.bend.x, shape.bend.y, shape.bend_length, shape.level),
                flow: Vec4::new(shape.expansion, shape.cell, shape.shock, shape.fade),
                heat: Vec4::new(
                    shape.spread,
                    shape.level * shape.glow,
                    shape.heat,
                    shape.flame,
                ),
                disk: Vec4::new(shape.disk, shape.disk_radius, 0.0, 0.0),
                mix: Vec4::new(shape.soot, shape.afterburn, shape.film, 0.0),
            };
        }
        let n = cluster.jets.len().min(MAX_JETS);
        let exit = cluster
            .jets
            .iter()
            .map(|(s, ..)| s.exit_radius)
            .fold(0.0, f32::max);
        let (glow, radius, hot) = cluster.flame_glow(look);
        Self {
            shape: Vec4::new(cluster.length, cluster.end_radius(), seed, n as f32),
            bounds: Vec4::new(
                cluster.bound_radius,
                cluster.bound_spread,
                cluster.ground.min(1e9),
                SPLASH,
            ),
            core: rgb(look.core, look.luminance),
            flame: rgb(look.flame, 0.0),
            shock: rgb(look.shock, 0.0),
            smoke: rgb(look.smoke, look.soot),
            medium: Vec4::new(look.temperature, look.haze, 1.4 * exit, 0.0),
            glow: glow.extend(radius),
            up: up.extend(hot),
            quality: Vec4::new(
                quality.steps as f32,
                quality.shadow_steps as f32,
                quality.octaves as f32,
                0.0,
            ),
            jets,
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct PlumeMaterial {
    #[uniform(0)]
    pub params: PlumeParams,
}

impl Material for PlumeMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://osp/shaders/plume.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Front faces march the plume from outside, back faces from a camera inside it.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// The light a plume casts on the vessel and the ground: lumens, its colour (linear RGB,
/// luminance 1) and where along the axis it is centred, m. The hot flame glows like a
/// column as wide and long as it is.
pub fn plume_light(cluster: &PlumeCluster, look: &PlumeDef) -> (f32, Vec3, f32) {
    let (glow, radius, hot) = cluster.flame_glow(look);
    let y = luminance(glow);
    if y <= 0.0 {
        return (0.0, Vec3::ONE, 0.0);
    }
    let lumens = std::f32::consts::PI * y * 2.0 * radius * hot;
    let middle = (0.5 * hot).min(cluster.ground - 1.0).max(0.0);
    (lumens, glow / y, middle)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEA_LEVEL_ENGINE: f64 = 70_000.0;

    /// A plume from a sea-level engine's nozzle, not bent.
    fn straight(level: f64, ambient: f64, ground: f32) -> PlumeShape {
        PlumeShape::new(
            0.45,
            0.45,
            SEA_LEVEL_ENGINE,
            level,
            ambient,
            ground,
            Vec2::ZERO,
        )
    }

    /// The jet's centre `a` metres from the nozzle exit, as the shader puts it.
    fn centre(shape: &PlumeShape, a: f32) -> Vec2 {
        let b = shape.bend_length;
        shape.bend * b * (1.0 - (-a / b).exp())
    }

    fn kerolox() -> PlumeDef {
        PlumeDef {
            id: "Kerolox".into(),
            core: [1.0, 0.82, 0.55],
            flame: [1.0, 0.45, 0.13],
            shock: [1.0, 0.9, 0.7],
            luminance: 20_000.0,
            soot: 0.5,
            temperature: 2300.0,
            smoke: soot_colour(),
            haze: 0.3,
            propellants: vec!["Kerosene".into(), "LqdOxygen".into()],
            kinds: Vec::new(),
            fallback: false,
        }
    }

    /// Every engine of every shipped mod has a look, except the electric ones; so none is
    /// drawn without exhaust.
    #[test]
    fn every_shipped_engine_has_a_plume() {
        use crate::content::{asset_root, mods_root};
        use osp_sim::data::EngineDef;

        let content = osp_mods::load(&asset_root().join("core"), Some(&mods_root()));
        let looks = content.registry.all::<PlumeDef>("plumes", &mut Vec::new());
        let engines = content
            .registry
            .all::<EngineDef>("engines", &mut Vec::new());
        assert!(engines.len() > 100, "{} engines", engines.len());
        let none: Vec<&str> = engines
            .iter()
            .filter(|e| {
                let burns: Vec<&str> = e.propellants.keys().map(String::as_str).collect();
                let look =
                    PlumeDef::for_engine(&looks, e.plume.as_deref(), &burns, e.kind.as_deref());
                look.is_none() != (e.kind.as_deref() == Some("electric"))
            })
            .map(|e| e.id.as_str())
            .collect();
        assert!(none.is_empty(), "wrong plume for {none:?}");
    }

    #[test]
    fn a_look_is_picked_by_propellant_alternatives_and_kind() {
        let looks = |_: ()| -> Vec<PlumeDef> {
            let mut kero = kerolox();
            kero.propellants = vec!["Kerosene|RP-1".into(), "LqdOxygen".into()];
            let mut solid = kerolox();
            solid.id = "Solid".into();
            solid.propellants.clear();
            solid.kinds = vec!["solid".into()];
            let mut any = kerolox();
            any.id = "Generic".into();
            any.propellants.clear();
            any.fallback = true;
            vec![kero, solid, any]
        }(());
        let pick = |burns: &[&str], kind: Option<&str>| {
            PlumeDef::for_engine(&looks, None, burns, kind).map(|l| l.id.as_str())
        };
        assert_eq!(
            pick(&["RP-1", "CooledLqdOxygen"], Some("liquid")),
            Some("Kerolox")
        );
        assert_eq!(pick(&["HTPB"], Some("solid")), Some("Solid"));
        assert_eq!(pick(&["HTPB"], Some("liquid")), Some("Generic"));
        assert_eq!(pick(&["LqdFluorine"], None), Some("Generic"));
        assert_eq!(pick(&["XenonGas"], Some("electric")), None);
    }

    /// Nine sea-level engines in a ring of eight around one, firing straight down.
    fn nine(level: f64) -> Vec<Jet> {
        (0..9)
            .map(|k| {
                let angle = k as f32 * std::f32::consts::TAU / 8.0;
                let at = if k == 8 {
                    Vec2::ZERO
                } else {
                    1.3 * Vec2::from_angle(angle)
                };
                Jet {
                    exit: Vec3::new(at.x, -2.0, at.y),
                    nozzle: Vec3::NEG_Y,
                    radius: 0.45,
                    exit_pressure: SEA_LEVEL_ENGINE,
                    level,
                    kind: JetKind::Main { film: false },
                }
            })
            .collect()
    }

    #[test]
    fn sea_level_plume_is_a_column_with_shock_diamonds() {
        let p = straight(1.0, ATM, f32::INFINITY);
        assert_eq!(p.expansion, 0.0);
        assert!(p.shock > 0.9, "{}", p.shock);
        assert!(p.spread < 0.15);
    }

    #[test]
    fn plume_fans_out_and_loses_its_diamonds_with_altitude() {
        let low = straight(1.0, 50_000.0, f32::INFINITY);
        let high = straight(1.0, 1_000.0, f32::INFINITY);
        let vacuum = straight(1.0, 0.0, f32::INFINITY);
        assert!(low.expansion < high.expansion && high.expansion < vacuum.expansion);
        assert!(low.spread < high.spread && high.spread < vacuum.spread);
        assert!(low.shock > high.shock);
        assert_eq!(vacuum.shock, 0.0);
        assert_eq!(vacuum.expansion, 1.0);
        // Cells stretch as the jet's pressure overtakes the air's.
        assert!(low.cell > straight(1.0, ATM, f32::INFINITY).cell);
    }

    #[test]
    fn the_ground_cuts_the_plume_and_its_bounds_make_room_for_the_splash() {
        let clear = straight(1.0, ATM, f32::INFINITY);
        let pad = straight(1.0, ATM, 5.0);
        assert_eq!(pad.length, 5.0);
        let at_ground = pad.bound_radius + pad.bound_spread * 5.0;
        assert!(at_ground > (1.0 + SPLASH) * (0.45 + 5.0 * pad.spread));
        assert!(clear.length > pad.length && clear.length.is_finite());
    }

    #[test]
    fn throttling_down_shortens_the_plume_and_tightens_the_cells() {
        let full = straight(1.0, 60_000.0, f32::INFINITY);
        let low = straight(0.4, 60_000.0, f32::INFINITY);
        assert!(low.fade < full.fade);
        assert!(low.cell < full.cell);
        assert!(low.expansion <= full.expansion);
        assert!(low.heat < full.heat);
    }

    #[test]
    fn overexpanded_jets_end_their_cells_in_mach_disks() {
        // An RS-25 at sea level leaves at a fifth of the air's pressure.
        let rs25 = PlumeShape::new(1.15, 1.15, 19_000.0, 1.0, ATM, f32::INFINITY, Vec2::ZERO);
        let matched = PlumeShape::new(0.45, 0.45, ATM, 1.0, ATM, f32::INFINITY, Vec2::ZERO);
        let thin = PlumeShape::new(
            1.15,
            1.15,
            19_000.0,
            1.0,
            2_000.0,
            f32::INFINITY,
            Vec2::ZERO,
        );
        assert!(
            rs25.disk > 0.95 && rs25.disk_radius > 0.5,
            "{} {}",
            rs25.disk,
            rs25.disk_radius
        );
        assert_eq!(matched.disk, 0.0);
        // In thin air the jet fans out and the cells are gone, disk and all.
        assert!(
            thin.shock * thin.disk < 0.05,
            "{} {}",
            thin.shock,
            thin.disk
        );
    }

    #[test]
    fn engines_get_the_look_for_what_they_burn_unless_they_name_one() {
        let kerolox = kerolox();
        let staged = PlumeDef {
            id: "Kerolox Staged".into(),
            propellants: Vec::new(),
            ..kerolox.clone()
        };
        let hydrolox = PlumeDef {
            id: "Hydrolox".into(),
            propellants: vec!["LqdHydrogen".into(), "LqdOxygen".into()],
            ..kerolox.clone()
        };
        let looks = [kerolox, staged, hydrolox];
        let id =
            |named, burns: &[&str]| PlumeDef::for_engine(&looks, named, burns, None).map(|l| &l.id);
        assert_eq!(id(None, &["Kerosene", "LqdOxygen"]).unwrap(), "Kerolox");
        assert_eq!(id(None, &["LqdOxygen", "LqdHydrogen"]).unwrap(), "Hydrolox");
        // Densified propellants burn as the plain ones.
        assert_eq!(
            id(None, &["CooledKerosene", "CooledLqdOxygen"]).unwrap(),
            "Kerolox"
        );
        assert_eq!(
            id(Some("Kerolox Staged"), &["Kerosene", "LqdOxygen"]).unwrap(),
            "Kerolox Staged"
        );
        assert_eq!(id(None, &["Hydrazine"]), None);
        assert_eq!(id(Some("Nonsense"), &["Kerosene", "LqdOxygen"]), None);
    }

    #[test]
    fn vacuum_engines_leave_their_exhaust_cooler() {
        let sea = straight(1.0, 0.0, f32::INFINITY);
        let vacuum = PlumeShape::new(1.5, 1.5, 4_000.0, 1.0, 0.0, f32::INFINITY, Vec2::ZERO);
        assert!((sea.heat - 0.9).abs() < 1e-3, "{}", sea.heat);
        assert!(vacuum.heat < 0.8 * sea.heat, "{}", vacuum.heat);
    }

    #[test]
    fn a_gimballed_jet_leaves_along_the_nozzle_and_stays_in_its_bounds() {
        let shape = |bend| {
            PlumeShape::new(
                0.45,
                0.45,
                SEA_LEVEL_ENGINE,
                1.0,
                20_000.0,
                f32::INFINITY,
                bend,
            )
        };
        let bend = Vec2::new(0.09, -0.03);
        let (plain, bent) = (shape(Vec2::ZERO), shape(bend));
        // Along the nozzle at first...
        let h = 1e-3;
        assert!((centre(&bent, h) / h - bend).length() < 1e-3);
        // ...then onto the plume axis: the far end is nowhere near where the nozzle points.
        let far = centre(&bent, bent.length);
        assert!(far.length() < 0.2 * bend.length() * bent.length);
        // The bounds make room for the offset.
        let room = bent.bound_radius - plain.bound_radius;
        for k in 0..=20 {
            let a = bent.length * k as f32 / 20.0;
            assert!(centre(&bent, a).length() <= room + 1e-4);
        }
    }

    #[test]
    fn a_cluster_merges_into_one_longer_plume_that_bounds_every_jet() {
        let up = Vec3::Y;
        let cluster = PlumeCluster::new(&nine(1.0), Vec3::NEG_Y, ATM, f32::INFINITY, up);
        let single = straight(1.0, ATM, f32::INFINITY);
        assert_eq!(cluster.jets.len(), 9);
        assert!((cluster.origin - Vec3::new(0.0, -2.0, 0.0)).length() < 1e-5);
        // Nine nozzles burn like one three times as wide.
        assert!((cluster.jets[0].0.fade / single.fade - 3.0).abs() < 1e-3);
        assert!(cluster.length > 2.0 * single.length);
        for (shape, across, along) in &cluster.jets {
            assert!(*along >= 0.0);
            for k in 0..=10 {
                let a = shape.length * k as f32 / 10.0;
                let jet = across.length() + shape.bound_radius + shape.bound_spread * a;
                let cone = cluster.bound_radius + cluster.bound_spread * (along + a);
                assert!(jet <= cone + 1e-4, "{jet} > {cone}");
            }
        }
        // One engine on its own is the single plume.
        let one = PlumeCluster::new(&nine(1.0)[8..], Vec3::NEG_Y, ATM, f32::INFINITY, up);
        assert_eq!(one.jets[0].0, single);
        assert_eq!(one.length, single.length);
    }

    #[test]
    fn turbine_exhaust_is_a_cool_sooty_jet_that_catches_fire_late() {
        let engine = nine(1.0)[8];
        let pipe = Jet {
            exit: engine.exit + Vec3::new(0.385, 1.0, 0.0),
            radius: 0.075,
            exit_pressure: 80_000.0,
            kind: JetKind::TurbineExhaust,
            ..engine
        };
        let alone = PlumeCluster::new(&[engine], Vec3::NEG_Y, ATM, f32::INFINITY, Vec3::Y);
        let both = PlumeCluster::new(&[engine, pipe], Vec3::NEG_Y, ATM, f32::INFINITY, Vec3::Y);
        let (main, gg) = (both.jets[0].0, both.jets[1].0);
        assert!(gg.heat < 0.5 * main.heat && gg.soot > 2.0 * main.soot);
        assert!(gg.afterburn > 2.0 * main.afterburn);
        assert_eq!((gg.shock, gg.disk), (0.0, 0.0));
        // The pipe leaves 1 m above the nozzle exit, so it sets the exit plane.
        assert!((both.jets[0].2 - 1.0).abs() < 1e-4 && both.jets[1].2 == 0.0);
        // It hardly changes the light.
        let look = kerolox();
        let ratio = plume_light(&both, &look).0 / plume_light(&alone, &look).0;
        assert!((ratio - 1.0).abs() < 0.1, "{ratio}");
        // A nozzle lined with it gets the film.
        let lined = Jet {
            kind: JetKind::Main { film: true },
            ..engine
        };
        let k5 = PlumeCluster::new(&[lined], Vec3::NEG_Y, ATM, f32::INFINITY, Vec3::Y);
        assert_eq!(k5.jets[0].0.film, 1.0);
    }

    #[test]
    fn the_cluster_meets_the_ground_below_its_exit_plane() {
        // The base 30 m up, the nozzle exits 2 m below it.
        let cluster = PlumeCluster::new(&nine(1.0), Vec3::NEG_Y, ATM, 30.0, Vec3::Y);
        assert!((cluster.ground - 28.0).abs() < 1e-4, "{}", cluster.ground);
        assert_eq!(cluster.length, 28.0);
        // Sideways it never does.
        let flat = PlumeCluster::new(&nine(1.0), Vec3::X, ATM, 30.0, Vec3::Y);
        assert_eq!(flat.ground, f32::INFINITY);
    }

    #[test]
    fn black_bodies_brighten_steeply_and_whiten_as_they_heat() {
        // Luminance from Planck's law with the CIE observer: 4.65e5 cd/m² at 2000 K.
        assert!((luminance(blackbody(2000.0)) / 4.65e5 - 1.0).abs() < 0.05);
        assert!((luminance(blackbody(2600.0)) / 8.30e6 - 1.0).abs() < 0.05);
        assert!(luminance(blackbody(1000.0)) < 10.0);
        let tint = |k| {
            let c = blackbody(k);
            c.z / c.x
        };
        assert!(tint(1800.0) < tint(2300.0) && tint(2300.0) < tint(3000.0));
        assert!(luminance(blackbody(0.0)) < 1e-20);
    }

    #[test]
    fn a_sooty_plume_glows_far_brighter_at_sea_level_than_in_vacuum() {
        let look = kerolox();
        let clean = PlumeDef {
            soot: 0.0,
            temperature: 0.0,
            ..look.clone()
        };
        let sea = PlumeCluster::new(&nine(1.0), Vec3::NEG_Y, ATM, f32::INFINITY, Vec3::Y);
        let vacuum = PlumeCluster::new(&nine(1.0), Vec3::NEG_Y, 0.0, f32::INFINITY, Vec3::Y);
        let lumens = |c: &PlumeCluster, l: &PlumeDef| plume_light(c, l).0;
        assert!(lumens(&sea, &look) > 10.0 * lumens(&sea, &clean));
        assert!(lumens(&sea, &look) > 10.0 * lumens(&vacuum, &look));
        // White-hot: yellow, not red.
        let (_, colour, _) = plume_light(&sea, &look);
        assert!(colour.y > 0.6 && colour.x < 2.6, "{colour}");
    }

    #[test]
    fn the_airflow_bends_the_plume_and_vacuum_does_not() {
        let exhaust = Vec3::NEG_Y;
        // At an angle of attack: the air comes past 10° off the axis.
        let airflow = Quat::from_rotation_z(10f32.to_radians()) * Vec3::NEG_Y * 400.0;
        let jet = 845e3 / (std::f32::consts::PI * 0.45 * 0.45);
        assert_eq!(plume_axis(exhaust, airflow, 0.0, jet), exhaust);
        assert_eq!(plume_axis(exhaust, Vec3::ZERO, 0.0, jet), exhaust);
        let angle = |q| plume_axis(exhaust, airflow, q, jet).angle_between(exhaust);
        assert!(angle(30e3) > angle(5e3) && angle(5e3) > 0.0);
        assert!(angle(30e3) < 10f32.to_radians());
        // A plume blowing into the airflow is pushed back but never turned round.
        assert!(plume_axis(exhaust, -airflow, 1e6, jet).dot(exhaust) > 0.5);
    }

    #[test]
    fn plume_axes_measure_the_nozzle_against_the_plume() {
        let axis = Vec3::new(0.3, -1.0, 0.2).normalize();
        let nozzle = Quat::from_axis_angle(axis.any_orthonormal_vector(), 0.1) * axis;
        let (rotation, bend) = plume_axes(axis, nozzle);
        assert!((rotation * Vec3::NEG_Y - axis).length() < 1e-6);
        assert!((bend.length() - 0.1f32.tan()).abs() < 1e-5);
        let along = rotation * Vec3::new(bend.x, -1.0, bend.y);
        assert!(along.normalize().dot(nozzle) > 0.99999);
    }
}
