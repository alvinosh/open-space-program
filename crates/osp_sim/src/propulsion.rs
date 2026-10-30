//! The vessel's engines, tanks and RCS blocks, each with its part, and RO-style engines:
//! pressure-dependent Isp, limited ignitions, ullage requirements, throttle limits and
//! thrust that spools up after ignition, follows the throttle at a limited rate and tails
//! off after cutoff. Engines draw from the tanks propellant reaches them from through the
//! part tree ([`crate::parts`]). The groups of parts between joints that let go are worked
//! out from the tree as [`Section`]s, which sequences name and the aero sees as a stack;
//! sizing tanks and fins is in [`crate::structure`], firing RCS in [`crate::rcs`]. What the
//! stage key does is a list of activation sequences ([`crate::staging`]), carried out here
//! by releasing joints on the tree.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};

use glam::{DVec2, DVec3};
use thiserror::Error;

use crate::{
    aero::Shape,
    component::Component,
    constants::{ATM, G0},
    craft::Craft,
    data::{
        CraftDef, EngineDef, GearDef, PartCatalog, PodDef, RcsDef, StagedCraftDef, TankShape,
        WingDef,
    },
    parts::{Part, PartId, PartKind, PartTree},
    shell::Shell,
    staging::{Activation, SectionActivation, Sequences},
    structure::{FinSet, Tank},
};

#[derive(Debug, Error)]
pub enum CraftError {
    #[error("unknown engine `{0}`")]
    UnknownEngine(String),
    #[error("unknown tank type `{0}`")]
    UnknownTankType(String),
    #[error("unknown material `{0}`")]
    UnknownMaterial(String),
    #[error("unknown resource `{0}`")]
    UnknownResource(String),
    #[error("unknown reaction wheel `{0}`")]
    UnknownReactionWheel(String),
    #[error("unknown RCS block `{0}`")]
    UnknownRcs(String),
    #[error("unknown pod `{0}`")]
    UnknownPod(String),
    #[error("a `{tank_type}` tank cannot hold {resource}: {reason}")]
    CannotHold {
        tank_type: String,
        resource: String,
        reason: String,
    },
    #[error("`{tank_type}` tank: {problem}")]
    BadTank { tank_type: String, problem: String },
    #[error("part `{part}`: {problem}")]
    BadPart { part: String, problem: String },
    #[error("unknown template `{0}`: no engine, pod, RCS block or reaction wheel has that id")]
    UnknownTemplate(String),
    #[error("craft {0}")]
    BadCraft(String),
    #[error("fins: {0}")]
    BadFins(String),
    #[error("wing `{wing}`: {problem}")]
    BadWing { wing: String, problem: String },
    #[error("stage `{stage}`: {problem}")]
    BadStage { stage: String, problem: String },
    #[error("sequences: {0}")]
    BadSequence(String),
    #[error("craft has no stages")]
    NoStages,
}

#[derive(Debug, Clone)]
pub struct Engine {
    pub def: EngineDef,
    /// The part it is, and its section.
    pub part: PartId,
    pub section: usize,
    pub running: bool,
    pub ignitions_left: Option<u32>,
    /// Where the engine is mounted: its gimbal pivot, in the assembly frame (vessel axes,
    /// from the axis at the height the craft was built from), m.
    pub mount: DVec3,
    /// Thrust in the last physics step, N.
    pub thrust: f64,
    /// Thrust as a share of full thrust at the end of the last physics step: what the
    /// throttle asked for, as far as the engine has got there.
    pub level: f64,
    /// Seconds since ignition while the engine is still spooling up.
    spooling: Option<f64>,
    /// Current gimbal angle as a vector in vessel X/Y (the side the thrust is tilted
    /// towards), radians.
    pub deflection: DVec2,
}

impl Engine {
    /// Full-throttle propellant mass flow, kg/s.
    pub fn max_mass_flow(&self) -> f64 {
        self.def.thrust_vac / (self.def.isp_vac * G0)
    }

    /// Thrust and mass flow as a share of full with `left` (0..1) of the propellant
    /// feeding it still there, by its thrust curve.
    pub fn curve_share(&self, left: f64) -> f64 {
        self.def
            .thrust_curve
            .as_deref()
            .map_or(1.0, |curve| curve_at(curve, left))
    }

    /// Full-throttle mass flow averaged over a whole burn as its thrust curve rises and
    /// falls, kg/s: the steady flow that would burn its propellant in the same time.
    pub fn mean_mass_flow(&self) -> f64 {
        self.max_mass_flow() * self.def.thrust_curve.as_deref().map_or(1.0, mean_share)
    }

    /// Specific impulse at a given ambient pressure (linear between vacuum and 1 atm).
    pub fn isp(&self, pressure: f64) -> f64 {
        isp_at(self.def.isp_vac, self.def.isp_sl, pressure)
    }

    /// Thrust share a throttle setting asks of a running engine: the setting, held within
    /// the engine's range (it cannot run below its minimum).
    pub fn commanded_level(&self, throttle: f64) -> f64 {
        throttle.clamp(self.def.min_throttle.clamp(0.0, 1.0), 1.0)
    }

    /// Whether the engine is still coming up to speed after ignition.
    pub fn spooling(&self) -> bool {
        self.spooling.is_some()
    }

    /// Seconds of the current thrust that a shutdown now would still give (the integral of
    /// the tail-off).
    pub fn tail_off_time(&self) -> f64 {
        self.def.response.spool_down / LN_100
    }

    /// Move the thrust level towards `target` over `dt` s: along the start-up curve after
    /// ignition, at most at the throttle rate while running, and decaying exponentially
    /// after cutoff. Exact for any `dt`, so it does not depend on the tick length.
    fn respond(&mut self, target: f64, dt: f64) {
        let response = &self.def.response;
        if target <= 0.0 {
            self.spooling = None;
            self.level = if response.spool_down > 0.0 {
                let left = self.level * (-dt / self.tail_off_time()).exp();
                if left < TAIL_END { 0.0 } else { left }
            } else {
                0.0
            };
            return;
        }
        if let Some(elapsed) = self.spooling {
            let elapsed = elapsed + dt;
            let along = response.startup_level(elapsed).max(self.level);
            if along >= target {
                self.level = target;
                self.spooling = None;
            } else {
                self.level = along;
                self.spooling = Some(elapsed);
            }
            return;
        }
        let step = if response.throttle_time > 0.0 {
            dt / response.throttle_time
        } else {
            f64::INFINITY
        };
        self.level += (target - self.level).clamp(-step, step);
    }
}

/// ln 100: a tail-off decays to 1% after `spool_down`.
const LN_100: f64 = 4.605_170_185_988_091;
/// Thrust share below which a tail-off counts as over.
const TAIL_END: f64 = 1e-3;

fn isp_at(isp_vac: f64, isp_sl: f64, pressure: f64) -> f64 {
    let x = pressure / ATM;
    (isp_vac + (isp_sl - isp_vac) * x).max(1.0)
}

/// Thrust curves never quite reach nothing: a share below this counts as this.
const CURVE_FLOOR: f64 = 1e-3;

/// A thrust curve's share at `left`: linear between its points, which run either way
/// along it, and level beyond its ends.
fn curve_at(curve: &[[f64; 2]], left: f64) -> f64 {
    let share = (curve.windows(2))
        .find(|w| (w[0][0].min(w[1][0])..=w[0][0].max(w[1][0])).contains(&left))
        .map(|w| {
            let [(x0, y0), (x1, y1)] = [(w[0][0], w[0][1]), (w[1][0], w[1][1])];
            if x1 == x0 {
                y0
            } else {
                y0 + (y1 - y0) * (left - x0) / (x1 - x0)
            }
        })
        .or_else(|| {
            // Beyond its ends: the nearer one.
            curve
                .iter()
                .min_by(|a, b| (a[0] - left).abs().total_cmp(&(b[0] - left).abs()))
                .map(|p| p[1])
        })
        .unwrap_or(1.0);
    share.max(CURVE_FLOOR)
}

/// The share of full flow a burn along `curve` averages: one over the mean of one over
/// the share across the propellant, so a steady burn at it lasts as long.
fn mean_share(curve: &[[f64; 2]]) -> f64 {
    // From empty to full, keeping the order of points at the same share (a step).
    let mut points: Vec<[f64; 2]> = curve.to_vec();
    if points
        .first()
        .zip(points.last())
        .is_some_and(|(a, b)| a[0] > b[0])
    {
        points.reverse();
    }
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return 1.0;
    };
    let share = |y: f64| y.max(CURVE_FLOOR);
    // Level beyond its ends, linear between its points.
    let mut time = first[0].clamp(0.0, 1.0) / share(first[1])
        + (1.0 - last[0].clamp(0.0, 1.0)) / share(last[1]);
    for w in points.windows(2) {
        let (dx, a, b) = (w[1][0] - w[0][0], share(w[0][1]), share(w[1][1]));
        time += if (b - a).abs() < 1e-12 {
            dx / a
        } else {
            dx * (b / a).ln() / (b - a)
        };
    }
    if time > 0.0 { 1.0 / time } else { 1.0 }
}

#[derive(Debug, Clone)]
pub struct RcsBlock {
    pub def: RcsDef,
    /// The part it is, and its section.
    pub part: PartId,
    pub section: usize,
    /// Where the block sits on the stage's forward rim, seen from behind: offset from the
    /// vessel axis in vessel X/Y, m.
    pub mount: DVec2,
    /// How hard each nozzle fired on the last physics tick, 0..1: the ones pushing forward,
    /// back, around the axis towards positive roll, and the other way.
    pub firing: [f64; 4],
}

impl RcsBlock {
    /// Propellant mass flow of one nozzle at full thrust, kg/s.
    pub fn nozzle_mass_flow(&self) -> f64 {
        self.def.thrust / (self.def.isp_vac * G0)
    }

    /// Thrust of one nozzle at a given ambient pressure, N.
    pub fn nozzle_thrust(&self, pressure: f64) -> f64 {
        self.nozzle_mass_flow() * isp_at(self.def.isp_vac, self.def.isp_sl, pressure) * G0
    }
}

#[derive(Debug, Clone)]
pub struct Propellant {
    pub name: String,
    pub amount: f64,
    pub capacity: f64,
}

/// A group of parts between joints that let go, seen whole: what a decoupler drops and a
/// sequence names (until sequences act on parts), and the stack the aero sees (until it
/// runs on voxels). It is worked out from the parts whenever they change; what changes in
/// flight (engines, tanks, RCS blocks, which sections are armed) is the [`Propulsion`]'s.
#[derive(Debug, Clone)]
pub struct Section {
    /// Its number in firing order; stays the same as other sections fall away.
    pub id: usize,
    pub name: String,
    /// The section it hangs from, if it is still there.
    pub parent: Option<usize>,
    /// It hangs beside its parent (a radial booster), not under it.
    pub beside: bool,
    /// The middle of its base, in the assembly frame, m.
    pub bottom: DVec3,
    /// How many engines, RCS blocks and tanks it has.
    pub engines: usize,
    pub rcs: usize,
    pub tanks: usize,
    /// Its engines, kg.
    pub engine_mass: f64,
    /// Empty tanks, kg.
    pub tank_mass: f64,
    /// Height of the top of its cylinder tanks above its base, m.
    pub tanks_top: f64,
    pub fins: Option<FinSet>,
    /// Wings, tailplanes and tail fins.
    pub wings: Vec<WingDef>,
    /// Landing gear legs.
    pub gear: Vec<GearDef>,
    /// Structure, avionics, payload, crew pod, reaction wheels, RCS and shells, kg.
    pub structure_mass: f64,
    /// Reaction wheel torque about each axis, N·m.
    pub wheel_torque: f64,
    pub pod: Option<PodDef>,
    pub fairing: Option<Fairing>,
    /// Stacked on the hull, bottom first; under a fairing, its nose.
    pub shells: Vec<Shell>,
    pub diameter: f64,
    /// The hull and the shells on it, m.
    pub length: f64,
}

impl Section {
    /// Section `id` of `tree` as its parts make it; its fairing gone if `jettisoned`. A
    /// fairing half on its own is a section of its own, facing its way.
    pub fn of(tree: &PartTree, id: usize, jettisoned: bool) -> Self {
        let mut section = Section {
            id,
            name: String::new(),
            parent: None,
            beside: false,
            bottom: DVec3::ZERO,
            engines: 0,
            rcs: 0,
            tanks: 0,
            engine_mass: 0.0,
            tank_mass: 0.0,
            tanks_top: 0.0,
            fins: None,
            wings: Vec::new(),
            gear: Vec::new(),
            structure_mass: 0.0,
            wheel_torque: 0.0,
            pod: None,
            fairing: None,
            shells: Vec::new(),
            diameter: 0.0,
            length: 0.0,
        };
        let mut hull = false;
        let mut half = None;
        for part in tree.stage(id) {
            match &part.kind {
                PartKind::Hull {
                    structure_mass,
                    diameter,
                    length,
                } => {
                    hull = true;
                    section.name = part.name.clone();
                    section.bottom = part.position;
                    section.structure_mass += structure_mass;
                    section.diameter = *diameter;
                    section.length = *length;
                }
                PartKind::Structure { mass } => section.structure_mass += mass,
                PartKind::Decoupler | PartKind::Mirror => {}
                PartKind::Tank(tank) => {
                    section.tanks += 1;
                    section.tank_mass += tank.dry_mass;
                    if tank.shape == TankShape::Cylinder {
                        section.tanks_top = section.tanks_top.max(tank.base + tank.length);
                    }
                }
                PartKind::Engine(def) => {
                    section.engines += 1;
                    section.engine_mass += def.mass;
                }
                PartKind::Rcs(def) => {
                    section.rcs += 1;
                    section.structure_mass += def.mass;
                }
                PartKind::ReactionWheel(def) => {
                    section.structure_mass += def.mass;
                    section.wheel_torque += def.torque;
                }
                PartKind::Pod(def) => {
                    section.structure_mass += def.mass;
                    section.pod = Some(def.clone());
                }
                PartKind::Fins(fins) => section.fins = Some(fins.clone()),
                PartKind::Wing(def) => section.wings.push(def.clone()),
                PartKind::Gear(def) => section.gear.push(def.clone()),
                PartKind::Shell(shell) => {
                    section.structure_mass += shell.mass;
                    section.shells.push(shell.clone());
                }
                PartKind::FairingHalf {
                    mass,
                    shells,
                    angle,
                    radius,
                    length,
                } => {
                    let fairing = section.fairing.get_or_insert(Fairing {
                        mass: 0.0,
                        attached: true,
                        half: None,
                    });
                    fairing.mass += mass;
                    if section.shells.is_empty() {
                        section.shells = shells.clone();
                    }
                    half = Some((part.position, *angle, *radius, *length));
                }
            }
        }
        if !hull && let Some((position, angle, radius, length)) = half {
            section.name = "Fairing".into();
            section.bottom = position;
            section.diameter = 2.0 * radius;
            section.length = length;
            if let Some(fairing) = &mut section.fairing {
                fairing.half = Some(angle);
            }
        }
        if jettisoned && section.fairing.is_none() {
            section.fairing = Some(Fairing {
                mass: 0.0,
                attached: false,
                half: None,
            });
        }
        section.length = section.shells.last().map_or(section.length, Shell::top);
        // What it hangs from: the section of the part above its own top part.
        let holder = tree
            .stage(id)
            .filter_map(|p| tree.parent(p.id))
            .find(|p| p.stage != id);
        if let Some(holder) = holder {
            section.parent = Some(holder.stage);
            let under = tree
                .stage(holder.stage)
                .find(|p| matches!(p.kind, PartKind::Hull { .. }))
                .map_or(DVec3::ZERO, |p| p.position);
            section.beside = (section.bottom - under).truncate().length() > 1e-6;
        }
        section
    }

    /// Length of the hull, below the shells, m.
    pub fn hull_length(&self) -> f64 {
        self.shells.first().map_or(self.length, |s| s.base)
    }

    pub fn fin_mass(&self) -> f64 {
        self.fins.as_ref().map_or(0.0, |f| f.mass)
    }

    pub fn wing_mass(&self) -> f64 {
        self.wings.iter().map(|w| w.mass).sum()
    }

    pub fn gear_mass(&self) -> f64 {
        self.gear.iter().map(|g| g.mass).sum()
    }

    /// The fairing, while it is still on, kg.
    pub fn fairing_mass(&self) -> f64 {
        self.fairing
            .as_ref()
            .filter(|f| f.attached)
            .map_or(0.0, |f| f.mass)
    }

    /// Everything but propellant, kg.
    pub fn dry_mass(&self) -> f64 {
        self.engine_mass
            + self.tank_mass
            + self.fin_mass()
            + self.wing_mass()
            + self.gear_mass()
            + self.structure_mass
            + self.fairing_mass()
    }
}

/// Height of the base of the lowest of `sections` in the assembly frame: the base of what
/// they make, m.
pub fn floor(sections: &[Section]) -> f64 {
    let floor = sections
        .iter()
        .map(|s| s.bottom.z)
        .fold(f64::INFINITY, f64::min);
    if floor.is_finite() { floor } else { 0.0 }
}

/// Each of `sections` with the height of its base above the lowest one's, m.
pub fn placed(sections: &[Section]) -> impl Iterator<Item = (f64, &Section)> {
    let floor = floor(sections);
    sections.iter().map(move |s| (s.bottom.z - floor, s))
}

/// A payload fairing around a section.
#[derive(Debug, Clone, PartialEq)]
pub struct Fairing {
    /// kg.
    pub mass: f64,
    /// Not jettisoned yet.
    pub attached: bool,
    /// Only one half of it, flying on its own after a jettison: the angle about the
    /// section's axis that the middle of the half faces (0 along vessel +X), rad.
    pub half: Option<f64>,
}

/// Angles the two halves of a fairing face; their seams are a quarter turn either side.
pub const FAIRING_HALVES: [f64; 2] = [0.0, std::f64::consts::PI];

/// Something a sequence released, to fly on as debris, with what its engines, RCS blocks and
/// tanks were doing.
#[derive(Debug, Clone)]
pub struct Released {
    /// Its sections, in firing order as they were on the vessel.
    pub sections: Vec<Section>,
    pub parts: PartTree,
    pub engines: Vec<Engine>,
    pub rcs: Vec<RcsBlock>,
    pub tanks: Vec<Tank>,
    /// Its engines that were armed.
    armed: BTreeSet<PartId>,
    /// Height of its base above the vessel's base before the sequence fired, m.
    pub base: f64,
    /// What its decouplers push it and the vessel apart with, N·s (0 for the usual push).
    pub impulse: f64,
}

impl Released {
    pub fn mass(&self) -> f64 {
        let dry: f64 = self.sections.iter().map(Section::dry_mass).sum();
        dry + self.tanks.iter().flat_map(|t| &t.amounts).sum::<f64>()
    }
}

/// Identical parts in a sequence, as the staging panel shows them: one icon with a count.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// As [`Activation::kind`].
    pub kind: &'static str,
    /// What makes them alike: their parts' name, or their engines' record.
    pub look: String,
    /// Where they are in the sequence, and their parts.
    pub members: Vec<usize>,
    pub parts: Vec<PartId>,
    /// The section of the first.
    pub section: usize,
}

/// What firing a sequence did.
#[derive(Debug, Clone, Default)]
pub struct Staged {
    /// For the flight log, one per activation.
    pub messages: Vec<String>,
    /// How far the vessel's base rose as sections under it fell away, m.
    pub dropped_length: f64,
    /// Mass left the vessel (sections fell away or a fairing was jettisoned).
    pub mass_changed: bool,
    /// An activation switched RCS on.
    pub rcs_on: bool,
    /// Sections that fell away and fairing halves that were jettisoned.
    pub released: Vec<Released>,
}

/// Per-section delta-v readout.
#[derive(Debug, Clone)]
pub struct StageInfo {
    pub name: String,
    pub delta_v_vac: f64,
    pub burn_time: f64,
    pub start_mass: f64,
}

/// The parts, the engines, RCS blocks and tanks among them and what they are doing, the
/// sections the parts make, the activation sequences, and throttle and ignition state.
#[derive(Debug, Clone)]
pub struct Propulsion {
    /// What the vessel is made of. Staging releases joints on it, and the rest follows.
    parts: PartTree,
    /// The sections left, in firing order: under and beside the sections they hang from.
    pub sections: Vec<Section>,
    /// Every engine, RCS block and tank left, each knowing its part and section.
    pub engines: Vec<Engine>,
    pub rcs: Vec<RcsBlock>,
    pub tanks: Vec<Tank>,
    /// Engines armed: they burn with the throttle once nothing hangs under their section.
    armed: BTreeSet<PartId>,
    /// Sections whose fairing has been jettisoned.
    jettisoned: BTreeSet<usize>,
    /// What the stage key does next; the player can rearrange them.
    pub sequences: Sequences,
    throttle: f64,
    ignition_pending: bool,
    /// The tanks each engine and RCS block draws from, by its part: indices into `tanks`.
    feeds: BTreeMap<PartId, Vec<usize>>,
    /// How the air sees the parts, once worked out.
    shape: OnceLock<Arc<Shape>>,
    /// Sections hollow along the axis of the section they hang from: nothing lies across
    /// it, so that section's exhaust goes straight through (an interstage ring, the
    /// Atlas's booster skirt).
    hollow: BTreeSet<usize>,
}

impl Propulsion {
    pub fn from_craft(def: &CraftDef, catalog: &PartCatalog) -> Result<Self, CraftError> {
        let craft = Craft::build(def, catalog)?;
        let parts = PartTree::from_craft(&craft, catalog)?;
        let mut propulsion = Self::new(parts);
        propulsion.sequences = match &def.sequences {
            Some(list) => {
                let tree = &propulsion.parts;
                let part = |id: &str| craft.find(id).map(|i| tree.parts()[i].id);
                let parse = |kind: &str, id: &str| {
                    let part = part(id).ok_or_else(|| format!("no part `{id}`"))?;
                    propulsion.activations(kind, part)
                };
                Sequences::parse(list, parse).map_err(CraftError::BadSequence)?
            }
            None => Sequences::default_for(&propulsion.sections, |a| propulsion.section_parts(a)),
        };
        Ok(propulsion)
    }

    /// What `<kind>:<part>` in a craft's sequences activates. Naming the part that does it
    /// activates that part: an engine (`engines`), a part hanging from a joint that lets go
    /// (`decoupler`), a fairing panel (`fairing`), an RCS block (`rcs`). Naming another part
    /// of a section activates the section's: all its engines, the joint it hangs from, all
    /// its fairing's panels, its RCS.
    fn activations(&self, kind: &str, part: PartId) -> Result<Vec<Activation>, String> {
        let tree = &self.parts;
        let found = tree.get(part).ok_or("no such part")?;
        let section = found.stage;
        let (own, whole) = match kind {
            "engines" => (
                self.engines
                    .iter()
                    .any(|e| e.part == part)
                    .then_some(Activation::Engine(part)),
                SectionActivation::Engines(section),
            ),
            "decoupler" => (
                (!matches!(found.kind, PartKind::FairingHalf { .. })
                    && tree.joint_capabilities(part).is_some_and(|c| c.release))
                .then_some(Activation::Decoupler(part)),
                SectionActivation::Decoupler(section),
            ),
            "fairing" => (
                matches!(found.kind, PartKind::FairingHalf { .. })
                    .then_some(Activation::Fairing(part)),
                SectionActivation::Fairing(section),
            ),
            "rcs" => (
                self.rcs
                    .iter()
                    .any(|b| b.part == part)
                    .then_some(Activation::Rcs(part)),
                SectionActivation::Rcs(section),
            ),
            other => {
                return Err(format!(
                    "unknown `{other}` (engines, decoupler, fairing or rcs)"
                ));
            }
        };
        let found = own.map_or_else(|| self.section_parts(whole), |a| vec![a]);
        if found.is_empty() {
            return Err("that part's section has nothing to activate there".into());
        }
        Ok(found)
    }

    /// The parts that do `what` to a whole section: its engines, the part of it hanging
    /// from the joint that lets go, its fairing's panels, its first RCS block.
    pub fn section_parts(&self, what: SectionActivation) -> Vec<Activation> {
        let tree = &self.parts;
        let half = |p: &Part| matches!(p.kind, PartKind::FairingHalf { .. });
        match what {
            SectionActivation::Engines(id) => self
                .engines_of(id)
                .map(|e| Activation::Engine(e.part))
                .collect(),
            SectionActivation::Decoupler(id) => (tree.stage(id))
                .find(|p| !half(p) && tree.joint_capabilities(p.id).is_some_and(|c| c.release))
                .map(|p| Activation::Decoupler(p.id))
                .into_iter()
                .collect(),
            SectionActivation::Fairing(id) => (tree.stage(id))
                .filter(|p| half(p))
                .map(|p| Activation::Fairing(p.id))
                .collect(),
            SectionActivation::Rcs(id) => self
                .rcs_of(id)
                .take(1)
                .map(|b| Activation::Rcs(b.part))
                .collect(),
        }
    }

    /// `sequence`'s activations gathered as the staging panel shows them: identical parts
    /// (the same kind of activation by parts of the same name, engines of the same record)
    /// under one icon, in the order each first appears.
    pub fn groups(&self, sequence: &[Activation]) -> Vec<Group> {
        let mut groups: Vec<Group> = Vec::new();
        for (index, &a) in sequence.iter().enumerate() {
            let Some(part) = self.parts.get(a.part()) else {
                continue;
            };
            let look = match a {
                Activation::Engine(p) => (self.engines.iter())
                    .find(|e| e.part == p)
                    .map_or(part.name.clone(), |e| e.def.id.clone()),
                Activation::Rcs(_) => String::new(),
                _ => part.name.clone(),
            };
            let same = |g: &&mut Group| g.kind == a.kind() && g.look == look;
            match groups.iter_mut().find(same) {
                Some(group) => {
                    group.members.push(index);
                    group.parts.push(a.part());
                }
                None => groups.push(Group {
                    kind: a.kind(),
                    look,
                    members: vec![index],
                    parts: vec![a.part()],
                    section: part.stage,
                }),
            }
        }
        groups
    }

    /// How full the tanks feeding `engines` are, 0..1; `None` if none do.
    pub fn feed_fraction(&self, engines: &[PartId]) -> Option<f64> {
        let mut tanks: Vec<usize> = engines
            .iter()
            .flat_map(|p| self.feeds.get(p).into_iter().flatten().copied())
            .collect();
        tanks.sort_unstable();
        tanks.dedup();
        let (mut amount, mut capacity) = (0.0, 0.0);
        for &k in &tanks {
            let tank = &self.tanks[k];
            for (content, &held) in tank.contents.iter().zip(&tank.amounts) {
                amount += held;
                capacity += content.capacity;
            }
        }
        (capacity > 0.0).then(|| amount / capacity)
    }

    /// How long the tanks feeding `engines` last when every engine drawing from them runs at
    /// full thrust, s (a solid's whole burn follows its thrust curve); `None` if none feed
    /// them.
    pub fn burn_time_left(&self, engines: &[PartId]) -> Option<f64> {
        let mut tanks: Vec<usize> = engines
            .iter()
            .flat_map(|p| self.feeds.get(p).into_iter().flatten().copied())
            .collect();
        tanks.sort_unstable();
        tanks.dedup();
        if tanks.is_empty() {
            return None;
        }
        let mass: f64 = tanks.iter().flat_map(|&k| &self.tanks[k].amounts).sum();
        let flow: f64 = (self.engines.iter())
            .filter(|e| (self.feeds.get(&e.part).into_iter().flatten()).any(|k| tanks.contains(k)))
            .map(Engine::mean_mass_flow)
            .sum();
        (flow > 0.0).then(|| mass / flow)
    }

    /// A craft described as a stack of stages, through the stage converter.
    pub fn from_staged(def: &StagedCraftDef, catalog: &PartCatalog) -> Result<Self, CraftError> {
        Self::from_craft(&crate::staged::convert(def, catalog)?, catalog)
    }

    /// The engines, RCS blocks and full tanks of `parts`, nothing armed.
    fn new(parts: PartTree) -> Self {
        let (mut engines, mut rcs, mut tanks) = (Vec::new(), Vec::new(), Vec::new());
        for part in parts.parts() {
            match &part.kind {
                PartKind::Engine(def) => engines.push(Engine {
                    def: (**def).clone(),
                    part: part.id,
                    section: part.stage,
                    running: false,
                    ignitions_left: def.ignitions,
                    mount: part.position,
                    thrust: 0.0,
                    level: 0.0,
                    spooling: None,
                    deflection: DVec2::ZERO,
                }),
                PartKind::Rcs(def) => rcs.push(RcsBlock {
                    def: def.clone(),
                    part: part.id,
                    section: part.stage,
                    mount: part.position.truncate(),
                    firing: [0.0; 4],
                }),
                PartKind::Tank(tank) => tanks.push(Tank {
                    part: part.id,
                    section: part.stage,
                    ..tank.clone()
                }),
                _ => {}
            }
        }
        let mut propulsion = Self {
            parts,
            sections: Vec::new(),
            engines,
            rcs,
            tanks,
            armed: BTreeSet::new(),
            jettisoned: BTreeSet::new(),
            sequences: Sequences::default(),
            throttle: 0.0,
            ignition_pending: false,
            feeds: BTreeMap::new(),
            shape: OnceLock::new(),
            hollow: BTreeSet::new(),
        };
        propulsion.rebuild();
        propulsion
    }

    /// What came loose from a vessel, as one of its own: nothing left to stage, throttle
    /// closed (engines that were running tail off).
    pub fn from_released(released: Released) -> Self {
        let mut propulsion = Self {
            parts: released.parts,
            sections: released.sections,
            engines: released.engines,
            rcs: released.rcs,
            tanks: released.tanks,
            armed: released.armed,
            jettisoned: BTreeSet::new(),
            sequences: Sequences::default(),
            throttle: 0.0,
            ignition_pending: false,
            feeds: BTreeMap::new(),
            shape: OnceLock::new(),
            hollow: BTreeSet::new(),
        };
        propulsion.plumb();
        propulsion.hollow = propulsion.hollow_rings();
        propulsion
    }

    /// The sections that are hollow (see [`Self::hollow`]).
    fn hollow_rings(&self) -> BTreeSet<usize> {
        let tree = &self.parts;
        let mut out = BTreeSet::new();
        for s in self.sections.iter().filter(|s| !s.beside) {
            let Some(above) = self.sections.iter().find(|a| Some(a.id) == s.parent) else {
                continue;
            };
            let axis = above.bottom.truncate();
            let mut triangles = Vec::new();
            for part in tree.stage(s.id) {
                if let Some(built) = &part.built {
                    let place = |p: DVec3| part.position + part.rotation * p;
                    crate::aero::outer_triangles(built, place, &mut triangles);
                }
            }
            let across = triangles.iter().any(|t| {
                let [a, b, c] = t.map(|v| v.truncate() - axis);
                let area = (b - a).perp_dot(c - a);
                let sides = [a.perp_dot(b), b.perp_dot(c), c.perp_dot(a)];
                area.abs() > 1e-9 && sides.iter().all(|&w| w * area >= 0.0)
            });
            if !triangles.is_empty() && !across {
                out.insert(s.id);
            }
        }
        out
    }

    /// Work the sections out again from the parts, after parts came off.
    fn rebuild(&mut self) {
        let ids: BTreeSet<usize> = self.parts.parts().iter().map(|p| p.stage).collect();
        self.sections = ids
            .iter()
            .map(|&id| Section::of(&self.parts, id, self.jettisoned.contains(&id)))
            .collect();
        let engines: BTreeSet<PartId> = self.engines.iter().map(|e| e.part).collect();
        self.armed.retain(|p| engines.contains(p));
        self.plumb();
        self.shape = OnceLock::new();
        self.hollow = self.hollow_rings();
    }

    /// Take the parts of `parts` (just let go of) with their engines, RCS blocks and tanks
    /// off this vessel, as something released `base` up it with `impulse`.
    fn release(&mut self, parts: PartTree, base: f64, impulse: f64) -> Released {
        let gone = |id: PartId| parts.get(id).is_some();
        let (engines, kept) = std::mem::take(&mut self.engines)
            .into_iter()
            .partition(|e| gone(e.part));
        self.engines = kept;
        let (rcs, kept) = std::mem::take(&mut self.rcs)
            .into_iter()
            .partition(|b| gone(b.part));
        self.rcs = kept;
        let (tanks, kept) = std::mem::take(&mut self.tanks)
            .into_iter()
            .partition(|t| gone(t.part));
        self.tanks = kept;
        // Whole sections as they were on the vessel, what they hung from and all; a fairing
        // half as a section of its own.
        let ids: BTreeSet<usize> = parts.parts().iter().map(|p| p.stage).collect();
        let whole = |id: usize| {
            parts
                .stage(id)
                .any(|p| matches!(p.kind, PartKind::Hull { .. }))
        };
        let sections = ids
            .iter()
            .map(|&id| match self.sections.iter().find(|s| s.id == id) {
                Some(section) if whole(id) => section.clone(),
                _ => Section::of(&parts, id, false),
            })
            .collect();
        Released {
            sections,
            armed: (self.armed.iter()).copied().filter(|&p| gone(p)).collect(),
            parts,
            engines,
            rcs,
            tanks,
            base,
            impulse,
        }
    }

    /// Work out where each engine and RCS block draws from: the tanks propellant reaches
    /// it from through joints that carry it (`PartTree::fluid_groups`). A decoupler passes
    /// propellant only with its crossfeed on.
    fn plumb(&mut self) {
        self.feeds.clear();
        for group in self.parts.fluid_groups() {
            let tanks: Vec<usize> = (0..self.tanks.len())
                .filter(|&k| group.contains(&self.tanks[k].part))
                .collect();
            let users = self
                .engines
                .iter()
                .map(|e| e.part)
                .chain(self.rcs.iter().map(|b| b.part));
            for user in users.filter(|u| group.contains(u)) {
                self.feeds.insert(user, tanks.clone());
            }
        }
    }

    /// How much of `resource` the tanks feeding `part` hold, kg.
    pub fn reachable(&self, part: PartId, resource: &str) -> f64 {
        self.feeds
            .get(&part)
            .into_iter()
            .flatten()
            .map(|&k| self.tanks[k].held(resource))
            .sum()
    }

    /// The share of the propellant feeding `engine` when full that its tanks still hold,
    /// by the scarcest kind.
    fn propellant_left(&self, engine: &Engine) -> f64 {
        let feed = self.feeds.get(&engine.part).map_or(&[][..], Vec::as_slice);
        (engine.def.propellants.keys())
            .map(|name| {
                let (held, full) = feed.iter().fold((0.0, 0.0), |(held, full), &k| {
                    let tank = &self.tanks[k];
                    let capacity: f64 = (tank.contents.iter())
                        .filter(|c| &c.resource == name)
                        .map(|c| c.capacity)
                        .sum();
                    (held + tank.held(name), full + capacity)
                });
                if full > 0.0 { held / full } else { 0.0 }
            })
            .fold(1.0, f64::min)
    }

    /// Take `need` (kg of each resource) from the tanks feeding `part`, each tank giving in
    /// proportion to what it holds, so tanks that started alike drain alike. Takes nothing
    /// and returns false if they hold too little of anything.
    pub fn draw(&mut self, part: PartId, need: &[(&str, f64)]) -> bool {
        if need
            .iter()
            .any(|&(resource, kg)| self.reachable(part, resource) < kg)
        {
            return false;
        }
        let feed = self.feeds.get(&part).cloned().unwrap_or_default();
        for &(resource, kg) in need {
            let total = self.reachable(part, resource);
            if total <= 0.0 {
                continue;
            }
            for &k in &feed {
                let tank = &mut self.tanks[k];
                for (content, amount) in tank.contents.iter().zip(&mut tank.amounts) {
                    if content.resource == resource {
                        *amount = (*amount - kg * *amount / total).max(0.0);
                    }
                }
            }
        }
        true
    }

    /// What the vessel is made of.
    pub fn parts(&self) -> &PartTree {
        &self.parts
    }

    /// How the air sees the vessel ([`Shape::new`]), worked out again only when its parts
    /// change.
    pub fn shape(&self) -> Arc<Shape> {
        self.shape
            .get_or_init(|| Arc::new(Shape::new(self)))
            .clone()
    }

    pub fn mass(&self) -> f64 {
        let dry: f64 = self.sections.iter().map(Section::dry_mass).sum();
        dry + self.tanks.iter().flat_map(|t| &t.amounts).sum::<f64>()
    }

    /// Section `id`'s engines, RCS blocks and tanks.
    pub fn engines_of(&self, id: usize) -> impl Iterator<Item = &Engine> {
        self.engines.iter().filter(move |e| e.section == id)
    }

    pub fn rcs_of(&self, id: usize) -> impl Iterator<Item = &RcsBlock> {
        self.rcs.iter().filter(move |b| b.section == id)
    }

    pub fn tanks_of(&self, id: usize) -> impl Iterator<Item = &Tank> {
        self.tanks.iter().filter(move |t| t.section == id)
    }

    /// Whether any of section `id`'s engines is armed.
    pub fn armed(&self, id: usize) -> bool {
        self.engines_of(id).any(|e| self.armed.contains(&e.part))
    }

    /// Whether the engine on `part` is armed.
    pub fn engine_armed(&self, part: PartId) -> bool {
        self.armed.contains(&part)
    }

    /// Arm all of section `id`'s engines.
    pub fn arm(&mut self, id: usize) {
        let engines: Vec<PartId> = self.engines_of(id).map(|e| e.part).collect();
        self.armed.extend(engines);
    }

    /// What section `id`'s tanks hold, by resource.
    pub fn propellants(&self, id: usize) -> Vec<Propellant> {
        let mut out: Vec<Propellant> = Vec::new();
        for tank in self.tanks_of(id) {
            for (content, &amount) in tank.contents.iter().zip(&tank.amounts) {
                match out.iter_mut().find(|p| p.name == content.resource) {
                    Some(p) => {
                        p.amount += amount;
                        p.capacity += content.capacity;
                    }
                    None => out.push(Propellant {
                        name: content.resource.clone(),
                        amount,
                        capacity: content.capacity,
                    }),
                }
            }
        }
        out
    }

    /// What section `id`'s tanks hold, kg.
    pub fn propellant_mass(&self, id: usize) -> f64 {
        self.tanks_of(id).flat_map(|t| &t.amounts).sum()
    }

    pub fn propellant_capacity(&self, id: usize) -> f64 {
        self.tanks_of(id)
            .flat_map(|t| &t.contents)
            .map(|c| c.capacity)
            .sum()
    }

    /// Section `id` with what its tanks hold, kg.
    pub fn section_mass(&self, id: usize) -> f64 {
        let dry = self
            .sections
            .iter()
            .find(|s| s.id == id)
            .map_or(0.0, Section::dry_mass);
        dry + self.propellant_mass(id)
    }

    /// The propellant section `id`'s engines can burn, given their mixture, kg.
    pub fn usable_propellant_mass(&self, id: usize) -> f64 {
        self.engines_of(id)
            .next()
            .map_or(0.0, |e| self.burnable(id, &e.def.propellants))
    }

    /// The propellant section `id`'s RCS blocks can still burn, kg.
    pub fn rcs_propellant_mass_of(&self, id: usize) -> f64 {
        self.rcs_of(id)
            .next()
            .map_or(0.0, |b| self.burnable(id, &b.def.propellants))
    }

    /// How much of section `id`'s propellant a mixture (ratios normalised) can burn, kg.
    fn burnable(&self, id: usize, mixture: &BTreeMap<String, f64>) -> f64 {
        // Limited by whichever propellant runs out first.
        let burnable = mixture
            .iter()
            .map(|(name, ratio)| {
                let available: f64 = self.tanks_of(id).map(|t| t.held(name)).sum();
                available / ratio
            })
            .fold(f64::INFINITY, f64::min);
        if burnable.is_finite() { burnable } else { 0.0 }
    }

    pub fn thrust_vac(&self, id: usize) -> f64 {
        self.engines_of(id).map(|e| e.def.thrust_vac).sum()
    }

    /// Thrust-weighted vacuum Isp of section `id`'s engines.
    pub fn isp_vac(&self, id: usize) -> f64 {
        let flow: f64 = self.engines_of(id).map(Engine::max_mass_flow).sum();
        if flow > 0.0 {
            self.thrust_vac(id) / (flow * G0)
        } else {
            0.0
        }
    }

    /// Fraction of the scarcest propellant left in section `id`, if it carries any. With
    /// engines, only what they burn counts (not RCS propellant).
    pub fn fuel_fraction(&self, id: usize) -> Option<f64> {
        let burns = |name: &str| {
            self.engines_of(id).next().is_none()
                || self
                    .engines_of(id)
                    .any(|e| e.def.propellants.contains_key(name))
        };
        self.fraction_of(id, burns)
    }

    /// Fraction of the scarcest RCS propellant left in section `id`, if it has RCS and
    /// carries any of it.
    pub fn rcs_fraction(&self, id: usize) -> Option<f64> {
        self.fraction_of(id, |name| {
            self.rcs_of(id)
                .any(|b| b.def.propellants.contains_key(name))
        })
    }

    fn fraction_of(&self, id: usize, counts: impl Fn(&str) -> bool) -> Option<f64> {
        self.propellants(id)
            .iter()
            .filter(|p| counts(&p.name))
            .map(|p| p.amount / p.capacity.max(1e-9))
            .reduce(f64::min)
    }

    pub fn throttle(&self) -> f64 {
        self.throttle
    }

    pub fn set_throttle(&mut self, throttle: f64) {
        let throttle = throttle.clamp(0.0, 1.0);
        if self.throttle == 0.0 && throttle > 0.0 {
            self.ignition_pending = true;
        }
        self.throttle = throttle;
    }

    /// Whether the section at `index` is clear to fire: whatever hangs under it lets its
    /// exhaust out. One beside it does not get in the way; one under it does unless it
    /// hangs by a part the exhaust passes through (a truss or vented interstage: hot
    /// staging), or it is hollow along the axis and clear itself (an interstage left on
    /// after the stage under it fell away, a booster skirt round the engine; see
    /// [`Self::hollow`]).
    pub fn clear(&self, index: usize) -> bool {
        self.blocker(index).is_none()
    }

    /// The section under the one at `index` that keeps its exhaust in, if any (see
    /// [`Self::clear`]).
    pub fn blocker(&self, index: usize) -> Option<usize> {
        let id = self.sections[index].id;
        (self.sections.iter().enumerate())
            .filter(|(_, s)| s.parent == Some(id) && !s.beside)
            .find(|&(k, s)| {
                !(self.vented(s.id, id) || (self.hollow.contains(&s.id) && self.clear(k)))
            })
            .map(|(k, _)| k)
    }

    /// Whether section `below` hangs from section `above` by a part that air and exhaust
    /// pass through, on either side of the joint.
    fn vented(&self, below: usize, above: usize) -> bool {
        let tree = &self.parts;
        tree.stage(below).any(|p| {
            tree.parent(p.id)
                .is_some_and(|q| q.stage == above && (p.open || q.open))
        })
    }

    /// The sections nothing hangs under: the bottom of the stack and what hangs beside it,
    /// whose engines burn once armed.
    pub fn bottom(&self) -> impl Iterator<Item = &Section> {
        (0..self.sections.len())
            .filter(|&i| self.clear(i))
            .map(|i| &self.sections[i])
    }

    /// The engines that burn with the throttle: armed, on sections that are clear.
    pub fn firing(&self) -> BTreeSet<PartId> {
        let clear: BTreeSet<usize> = (0..self.sections.len())
            .filter(|&i| self.clear(i))
            .map(|i| self.sections[i].id)
            .collect();
        (self.engines.iter())
            .filter(|e| self.armed.contains(&e.part) && clear.contains(&e.section))
            .map(|e| e.part)
            .collect()
    }

    pub fn any_engine_running(&self) -> bool {
        self.engines.iter().any(|e| e.running)
    }

    /// True when the next physics step may produce thrust, so the vessel cannot coast:
    /// the throttle is open on a section that can fire, or an engine is still tailing off.
    pub fn wants_thrust(&self) -> bool {
        let firing = self.firing();
        (self.throttle > 0.0 && !firing.is_empty()) || self.engines.iter().any(|e| e.level > 0.0)
    }

    /// Thrust-weighted share of full thrust the firing sections' engines are at, 0..1.
    pub fn engine_level(&self) -> f64 {
        let firing = self.firing();
        let engines = || self.engines.iter().filter(|e| firing.contains(&e.part));
        let full: f64 = engines().map(|e| e.def.thrust_vac).sum();
        if full <= 0.0 {
            return 0.0;
        }
        engines().map(|e| e.level * e.def.thrust_vac).sum::<f64>() / full
    }

    /// Impulse the engines would still give if cut now, N·s.
    pub fn tail_off_impulse(&self, pressure: f64) -> f64 {
        self.engines
            .iter()
            .map(|e| e.level * e.max_mass_flow() * e.isp(pressure) * G0 * e.tail_off_time())
            .sum()
    }

    /// Fire the next sequence: do its activations in the order listed. `None` when no
    /// sequence is left.
    pub fn activate_next_sequence(&mut self) -> Option<Staged> {
        let sequence = self.sequences.pop_next()?;
        if sequence.is_empty() {
            let nothing = "Nothing to stage".to_string();
            return Some(Staged {
                messages: vec![nothing],
                ..Staged::default()
            });
        }
        Some(self.run(sequence))
    }

    /// The actions the player can take on `part` now, as the part's window offers them: light
    /// the engine if it is not lit, release the joint it hangs from or the ones a
    /// decoupler on it lets go, jettison a fairing panel, switch its RCS on (`rcs_on`: it is
    /// on already).
    pub fn part_actions(&self, part: PartId, rcs_on: bool) -> Vec<Activation> {
        let tree = &self.parts;
        let Some(found) = tree.get(part) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if self.engines.iter().any(|e| e.part == part) && !self.armed.contains(&part) {
            out.push(Activation::Engine(part));
        }
        if matches!(found.kind, PartKind::FairingHalf { .. }) {
            if tree.joint_capabilities(part).is_some_and(|c| c.release) {
                out.push(Activation::Fairing(part));
            }
        } else {
            let releases = |p: PartId| tree.joint_capabilities(p).is_some_and(|c| c.release);
            // The joint it hangs from, and the joints under it that a decoupler of its
            // own holds.
            if releases(part) {
                out.push(Activation::Decoupler(part));
            }
            let nodes: Vec<&str> = (found.built.iter())
                .flat_map(|b| &b.components)
                .filter_map(|c| match c {
                    Component::Decoupler(d) => Some(d.node.as_str()),
                    _ => None,
                })
                .collect();
            for child in tree.parts() {
                let Some(joint) = child.joint.filter(|j| j.parent == part) else {
                    continue;
                };
                let node = found.connectors.get(joint.parent_connector);
                if node.is_some_and(|c| nodes.contains(&c.name.as_str()))
                    && releases(child.id)
                    && !matches!(child.kind, PartKind::FairingHalf { .. })
                {
                    out.push(Activation::Decoupler(child.id));
                }
            }
        }
        if !rcs_on && self.rcs.iter().any(|b| b.part == part) {
            out.push(Activation::Rcs(part));
        }
        out
    }

    /// Do `activation` now, by hand, if `part_actions` offers it (`rcs_on` as there): it
    /// leaves whatever sequence listed it. `None` if it is not on offer.
    pub fn perform(&mut self, activation: Activation, rcs_on: bool) -> Option<Staged> {
        let offered = self.part_actions(activation.part(), rcs_on);
        if !offered.contains(&activation) {
            return None;
        }
        self.sequences.take(activation);
        Some(self.run(vec![activation]))
    }

    /// Do `activations` in order: release decouplers (the section and everything hanging
    /// from it fall away), light engines (they start with the throttle once no section
    /// hangs under theirs), jettison fairings and switch RCS on.
    fn run(&mut self, activations: Vec<Activation>) -> Staged {
        let mut staged = Staged::default();
        let before = self.parts.base();
        // One message for each section's engines and fairing, however many parts it took.
        let mut said: BTreeSet<(&str, usize)> = BTreeSet::new();
        for activation in activations {
            let Some(part) = self.parts.get(activation.part()) else {
                continue;
            };
            let id = part.stage;
            match activation {
                Activation::Decoupler(decoupler) => {
                    let impulse = self.parts.release_impulse(decoupler);
                    let Some(parts) = self.parts.release(decoupler) else {
                        continue;
                    };
                    let base = parts.base() - before;
                    let released = self.release(parts, base, impulse);
                    self.rebuild();
                    staged.dropped_length = self.parts.base() - before;
                    staged.mass_changed = true;
                    let names: Vec<_> = released.sections.iter().map(|s| s.name.as_str()).collect();
                    staged
                        .messages
                        .push(format!("Separated {}", names.join(", ")));
                    let gone = &released.parts;
                    self.sequences.forget(|p| gone.get(p).is_some());
                    staged.released.push(released);
                    // Engines armed above light as soon as nothing hangs under them.
                    if self.throttle > 0.0 && !self.firing().is_empty() {
                        self.ignition_pending = true;
                    }
                }
                Activation::Engine(engine) => {
                    if !self.engines.iter().any(|e| e.part == engine) {
                        continue;
                    }
                    self.armed.insert(engine);
                    let Some(index) = self.sections.iter().position(|s| s.id == id) else {
                        continue;
                    };
                    let blocker = self.blocker(index);
                    if said.insert(("engines", id)) {
                        let name = &self.sections[index].name;
                        staged.messages.push(format!("Staged: {name}"));
                        if let Some(below) = blocker {
                            let below = &self.sections[below].name;
                            staged.messages.push(format!(
                                "{name} waits for {below} to fall away: nothing under it \
                                 lets its exhaust out"
                            ));
                        }
                    }
                    if blocker.is_none() && self.throttle > 0.0 {
                        self.ignition_pending = true;
                    }
                }
                Activation::Fairing(half) => {
                    let Some(parts) = self.parts.release(half) else {
                        continue;
                    };
                    let base = parts.base() - before;
                    let released = self.release(parts, base, 0.0);
                    staged.released.push(released);
                    // The shells were its nose, and went with it.
                    self.jettisoned.insert(id);
                    self.rebuild();
                    staged.mass_changed = true;
                    if said.insert(("fairing", id)) {
                        staged.messages.push("Fairing jettisoned".into());
                    }
                }
                Activation::Rcs(_) => {
                    staged.rcs_on = true;
                    if said.insert(("rcs", 0)) {
                        staged.messages.push("RCS on".into());
                    }
                }
            }
        }
        staged
    }

    /// Arm the engines of the bottom sections ([`Self::bottom`]) outside the sequences (the
    /// burn executor does), taking their activations out of whichever sequences held them.
    /// Returns a log message if some were not armed yet.
    pub fn arm_bottom(&mut self) -> Option<String> {
        let clear: BTreeSet<usize> = (0..self.sections.len())
            .filter(|&i| self.clear(i))
            .map(|i| self.sections[i].id)
            .collect();
        let idle: Vec<(PartId, usize)> = (self.engines.iter())
            .filter(|e| clear.contains(&e.section) && !self.armed.contains(&e.part))
            .map(|e| (e.part, e.section))
            .collect();
        if idle.is_empty() {
            return None;
        }
        let mut names: Vec<String> = Vec::new();
        for (engine, section) in idle {
            self.armed.insert(engine);
            if let Some(s) = self.sections.iter().find(|s| s.id == section)
                && !names.contains(&s.name)
            {
                names.push(s.name.clone());
            }
            self.sequences.take(Activation::Engine(engine));
        }
        if self.throttle > 0.0 {
            self.ignition_pending = true;
        }
        Some(format!("Staged: {}", names.join(", ")))
    }

    /// Run the engines for `dt` seconds. Handles ignition (consuming ignitions, failing if
    /// propellant is unsettled), spool-up, throttling, tail-off, flameout and propellant
    /// use. Returns thrust in newtons (the mean over the step).
    pub fn burn(&mut self, pressure: f64, dt: f64, settled: bool, log: &mut Vec<String>) -> f64 {
        let throttle = self.throttle;
        let attempt_ignition = std::mem::take(&mut self.ignition_pending);
        let firing = self.firing();
        let mut thrust = 0.0;
        for j in 0..self.engines.len() {
            if !firing.contains(&self.engines[j].part) {
                continue;
            }
            let engine = &mut self.engines[j];
            engine.thrust = 0.0;
            if throttle <= 0.0 {
                engine.running = false;
            } else if !engine.running && attempt_ignition {
                engine.running = ignite(engine, settled, log);
            }
            let target = if engine.running {
                engine.commanded_level(throttle)
            } else {
                0.0
            };
            let before = engine.level;
            engine.respond(target, dt);
            let level = 0.5 * (before + engine.level);
            if level <= 0.0 {
                continue;
            }
            let engine = &self.engines[j];
            let share = match engine.def.thrust_curve {
                Some(_) => engine.curve_share(self.propellant_left(engine)),
                None => 1.0,
            };
            let flow = engine.max_mass_flow() * level * share;
            let isp = engine.isp(pressure);
            let part = engine.part;
            let mixture: Vec<(String, f64)> = engine
                .def
                .propellants
                .iter()
                .map(|(name, ratio)| (name.clone(), flow * ratio * dt))
                .collect();
            let need: Vec<(&str, f64)> = mixture.iter().map(|(n, kg)| (n.as_str(), *kg)).collect();
            let fed = self.draw(part, &need);
            let engine = &mut self.engines[j];
            if !fed {
                if engine.running {
                    log.push(format!("{}: flameout, propellant depleted", engine.def.id));
                }
                engine.running = false;
                engine.level = 0.0;
                engine.spooling = None;
                continue;
            }
            engine.thrust = flow * isp * G0;
            thrust += engine.thrust;
        }
        thrust
    }

    /// Mass of propellant left for the RCS, kg.
    pub fn rcs_propellant_mass(&self) -> f64 {
        self.sections
            .iter()
            .map(|s| self.rcs_propellant_mass_of(s.id))
            .sum()
    }

    /// Delta-v the RCS can still give, pushing the whole vessel along in vacuum, m/s.
    pub fn rcs_delta_v(&self) -> f64 {
        let (mut thrust, mut flow) = (0.0, 0.0);
        for block in self
            .rcs
            .iter()
            .filter(|b| self.rcs_propellant_mass_of(b.section) > 0.0)
        {
            thrust += block.def.thrust;
            flow += block.nozzle_mass_flow();
        }
        let (mass, propellant) = (self.mass(), self.rcs_propellant_mass());
        if flow <= 0.0 || propellant >= mass {
            return 0.0;
        }
        thrust / flow * (mass / (mass - propellant)).ln()
    }

    /// From the base of the lowest section to the top of the highest, m.
    pub fn length(&self) -> f64 {
        let floor = floor(&self.sections);
        self.sections
            .iter()
            .map(|s| s.bottom.z - floor + s.length)
            .fold(0.0, f64::max)
    }

    /// The fins of the section at `index` tore off: they are gone from the parts.
    pub fn tear_off_fins(&mut self, index: usize) {
        let id = self.sections[index].id;
        let fins: Vec<_> = self
            .parts
            .stage(id)
            .filter(|p| matches!(p.kind, PartKind::Fins(_)))
            .map(|p| p.id)
            .collect();
        for fin in fins {
            self.parts.break_off(fin);
        }
        self.rebuild();
    }

    /// Cut the throttle and stop every engine for good (the vessel broke up).
    pub fn shut_down(&mut self) {
        self.throttle = 0.0;
        self.ignition_pending = false;
        for engine in &mut self.engines {
            engine.running = false;
            engine.thrust = 0.0;
            engine.level = 0.0;
            engine.spooling = None;
        }
    }

    /// Delta-v of each remaining section in vacuum, in firing order. A section burns
    /// together with the sections hanging beside it until they run out and fall away, then
    /// on its own: its row counts both, and theirs count no delta-v of their own.
    pub fn stage_info(&self) -> Vec<StageInfo> {
        // Averaged over the burn, for engines whose thrust follows a curve.
        let flow = |id: usize| self.engines_of(id).map(Engine::mean_mass_flow).sum::<f64>();
        let thrust = |id: usize| {
            (self.engines_of(id))
                .map(|e| e.mean_mass_flow() * e.def.isp_vac * G0)
                .sum::<f64>()
        };
        let sections = &self.sections;
        // A section with engines and no propellant of its own that the one it hangs under
        // feeds (the Atlas's booster skirt) burns with it for its engines' rated burn time,
        // or until the propellant is gone; that one burns what is left.
        let mut fed: BTreeMap<usize, (usize, f64)> = BTreeMap::new();
        for s in sections.iter().filter(|s| !s.beside && s.engines > 0) {
            let Some(parent) = s.parent else { continue };
            let feeds_it = self.engines_of(s.id).all(|e| {
                (self.feeds.get(&e.part).into_iter().flatten())
                    .any(|&k| self.tanks[k].section == parent)
            });
            if self.usable_propellant_mass(s.id) > 0.0 || !feeds_it {
                continue;
            }
            let rated = (self.engines_of(s.id))
                .filter_map(|e| e.def.reliability.as_ref()?.rated_burn_time)
                .fold(f64::INFINITY, f64::min);
            let both = flow(s.id) + flow(parent);
            let time = rated.min(self.usable_propellant_mass(parent) / both.max(1e-9));
            fed.insert(s.id, (parent, time));
        }
        let drawn = |id: usize| -> f64 {
            (fed.values())
                .filter(|(p, _)| *p == id)
                .map(|&(p, t)| {
                    (flow(p)
                        + fed
                            .iter()
                            .filter(|(_, v)| v.0 == p)
                            .map(|(&s, _)| flow(s))
                            .sum::<f64>())
                        * t
                })
                .sum()
        };
        sections
            .iter()
            .enumerate()
            .map(|(i, section)| {
                let id = section.id;
                if let Some(&(parent, time)) = fed.get(&id) {
                    let start_mass: f64 =
                        sections[i..].iter().map(|s| self.section_mass(s.id)).sum();
                    let total_flow = flow(id) + flow(parent);
                    let thrust = thrust(id) + thrust(parent);
                    let used = total_flow * time;
                    return StageInfo {
                        name: section.name.clone(),
                        delta_v_vac: thrust / total_flow * (start_mass / (start_mass - used)).ln(),
                        burn_time: time,
                        start_mass,
                    };
                }
                let burned = (self.usable_propellant_mass(id) - drawn(id)).max(0.0);
                let sides: Vec<&Section> = sections
                    .iter()
                    .filter(|s| s.beside && s.parent == Some(id))
                    .collect();
                let start_mass = sections[i..]
                    .iter()
                    .map(|s| self.section_mass(s.id))
                    .sum::<f64>()
                    + sides.iter().map(|s| self.section_mass(s.id)).sum::<f64>();
                let own = flow(id);
                let mut row = StageInfo {
                    name: section.name.clone(),
                    delta_v_vac: 0.0,
                    burn_time: if own > 0.0 { burned / own } else { 0.0 },
                    start_mass,
                };
                if section.beside {
                    return row;
                }
                // Together with the sections beside it, until the first of them runs out.
                let lit: Vec<usize> = sides
                    .iter()
                    .map(|s| s.id)
                    .filter(|&s| flow(s) > 0.0)
                    .collect();
                let together = lit
                    .iter()
                    .map(|&s| self.usable_propellant_mass(s) / flow(s))
                    .fold(f64::INFINITY, f64::min);
                let (mut mass, mut left, mut time) = (start_mass, burned, 0.0);
                if together.is_finite() {
                    let core = (own * together).min(burned);
                    let used = core + lit.iter().map(|&s| flow(s) * together).sum::<f64>();
                    let total_flow = own + lit.iter().map(|&s| flow(s)).sum::<f64>();
                    let total_thrust = thrust(id) + lit.iter().map(|&s| thrust(s)).sum::<f64>();
                    row.delta_v_vac += total_thrust / total_flow * (mass / (mass - used)).ln();
                    mass -= used
                        + lit
                            .iter()
                            .map(|&s| self.section_mass(s) - flow(s) * together)
                            .sum::<f64>();
                    mass -= sides
                        .iter()
                        .filter(|s| flow(s.id) <= 0.0)
                        .map(|s| self.section_mass(s.id))
                        .sum::<f64>();
                    left -= core;
                    time = together;
                }
                let isp = self.isp_vac(id);
                if isp > 0.0 && left > 0.0 {
                    row.delta_v_vac += isp * G0 * (mass / (mass - left)).ln();
                }
                if own > 0.0 {
                    row.burn_time = time + left / own;
                }
                row
            })
            .collect()
    }
}

/// Try to light an engine; returns whether it lit. Uses up an ignition either way, and
/// starts the spool-up.
fn ignite(engine: &mut Engine, settled: bool, log: &mut Vec<String>) -> bool {
    match engine.ignitions_left {
        Some(0) => {
            log.push(format!("{}: no ignitions left", engine.def.id));
            return false;
        }
        Some(ref mut n) => *n -= 1,
        None => {}
    }
    if engine.def.requires_ullage && !settled {
        log.push(format!(
            "{}: ignition failed, propellant unsettled (ullage needed)",
            engine.def.id
        ));
        return false;
    }
    engine.spooling = Some(0.0);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(id: &str) -> Engine {
        let def = crate::test_data::records::<EngineDef>(include_str!(
            "../../../assets/core/engines/stock.json5"
        ))
        .into_iter()
        .find(|e| e.id == id)
        .unwrap();
        Engine {
            ignitions_left: def.ignitions,
            def,
            part: PartId(0),
            section: 0,
            running: true,
            mount: DVec3::ZERO,
            thrust: 0.0,
            level: 0.0,
            spooling: Some(0.0),
            deflection: DVec2::ZERO,
        }
    }

    /// Run `respond` for `seconds` in ticks of `dt`; returns the level after each tick.
    fn run(e: &mut Engine, target: f64, seconds: f64, dt: f64) -> Vec<f64> {
        (0..(seconds / dt).round() as usize)
            .map(|_| {
                e.respond(target, dt);
                e.level
            })
            .collect()
    }

    #[test]
    fn startup_follows_the_curve_to_full_thrust() {
        let mut e = engine("K-1 Booster");
        let spool = e.def.response.spool_up;
        let levels = run(&mut e, 1.0, spool + 0.1, 0.02);
        // Never drops while spooling, and is still well short of full halfway through.
        assert!(levels.windows(2).all(|w| w[1] >= w[0]));
        let half = levels[(spool / 2.0 / 0.02) as usize];
        assert!(half > 0.3 && half < 0.8, "{half}");
        assert_eq!(e.level, 1.0);
        assert!(!e.spooling());
    }

    #[test]
    fn a_solid_follows_its_thrust_curve() {
        use crate::vessel::tests::{catalog, pathfinder};
        let mut p = Propulsion::from_craft(&pathfinder(), &catalog()).unwrap();
        let booster = p.sections[0].id;
        let steady = p.stage_info()[0].clone();
        // Full thrust while the first half of its propellant burns, half for the rest.
        for e in p.engines.iter_mut().filter(|e| e.section == booster) {
            e.def.thrust_curve = Some(vec![[1.0, 1.0], [0.5, 1.0], [0.5, 0.5], [0.0, 0.5]]);
        }
        let curved = p.stage_info()[0].clone();
        // The same delta-v, but the second half takes twice as long.
        assert!((curved.delta_v_vac / steady.delta_v_vac - 1.0).abs() < 1e-9);
        assert!((curved.burn_time / steady.burn_time - 1.5).abs() < 1e-9);
        p.set_throttle(1.0);
        p.activate_next_sequence();
        let full = p.thrust_vac(booster);
        let (mut early, mut late) = (0.0, 0.0);
        for _ in 0..(2.0 * curved.burn_time / 0.1) as usize {
            let thrust = p.burn(0.0, 0.1, true, &mut Vec::new());
            match p.fuel_fraction(booster).unwrap_or(0.0) {
                left if (0.6..0.9).contains(&left) => early = thrust,
                left if (0.1..0.4).contains(&left) => late = thrust,
                _ => {}
            }
        }
        assert!((early / full - 1.0).abs() < 1e-6, "{early} {full}");
        assert!((late / full - 0.5).abs() < 1e-6, "{late} {full}");
    }

    #[test]
    fn an_activation_can_be_done_by_hand_and_leaves_its_sequence() {
        use crate::vessel::tests::{catalog, pathfinder};
        let mut p = Propulsion::from_craft(&pathfinder(), &catalog()).unwrap();
        let listed =
            |p: &Propulsion, a: Activation| p.sequences.list().iter().flatten().any(|&x| x == a);
        let engine = (p.sequences.list().iter().flatten())
            .find_map(|&a| matches!(a, Activation::Engine(_)).then_some(a))
            .unwrap();
        let decoupler = (p.sequences.list().iter().flatten())
            .find_map(|&a| matches!(a, Activation::Decoupler(_)).then_some(a))
            .unwrap();
        // The engine is offered to light, once.
        assert!(p.part_actions(engine.part(), false).contains(&engine));
        assert!(listed(&p, engine));
        let count = p.sequences.list().len();
        assert!(p.perform(engine, false).is_some());
        assert!(p.engine_armed(engine.part()));
        assert!(!listed(&p, engine));
        assert!(!p.part_actions(engine.part(), false).contains(&engine));
        assert!(p.perform(engine, false).is_none());
        // A sequence it was in keeps its place and number.
        assert_eq!(p.sequences.list().len(), count);
        // Nothing but what a part offers can be done to it.
        assert!(p.perform(Activation::Rcs(engine.part()), false).is_none());
        // The decoupler lets go of what hangs from it, ahead of its sequence.
        assert!(p.part_actions(decoupler.part(), false).contains(&decoupler));
        let staged = p.perform(decoupler, false).unwrap();
        assert!(staged.mass_changed && !staged.released.is_empty());
        assert!(!listed(&p, decoupler));
        assert!(p.part_actions(decoupler.part(), false).is_empty());
    }

    #[test]
    fn a_sequence_does_its_activations_in_the_order_listed() {
        use crate::vessel::tests::{catalog, pathfinder};
        let fresh = || Propulsion::from_craft(&pathfinder(), &catalog()).unwrap();
        let pick = |p: &Propulsion, kind: fn(&Activation) -> bool| {
            (p.sequences.list().iter().flatten())
                .copied()
                .find(kind)
                .unwrap()
        };
        let p = fresh();
        let engine = pick(&p, |a| matches!(a, Activation::Engine(_)));
        let decoupler = pick(&p, |a| matches!(a, Activation::Decoupler(_)));
        let first_message = |order: Vec<Activation>| {
            let mut p = fresh();
            p.sequences = Sequences::new(vec![order]);
            p.activate_next_sequence().unwrap().messages.remove(0)
        };
        assert!(first_message(vec![decoupler, engine]).starts_with("Separated"));
        assert!(first_message(vec![engine, decoupler]).starts_with("Staged"));
    }

    #[test]
    fn burn_time_left_counts_every_engine_on_the_tanks() {
        use crate::vessel::tests::{catalog, pathfinder};
        let p = Propulsion::from_craft(&pathfinder(), &catalog()).unwrap();
        let booster = p.sections[0].id;
        let engines: Vec<PartId> = p.engines_of(booster).map(|e| e.part).collect();
        let all = p.burn_time_left(&engines).unwrap();
        // One engine of the cluster still shares the tanks with the rest, so the tanks
        // last as long: the others are drawing from them.
        let one = p.burn_time_left(&engines[..1]).unwrap();
        assert_eq!(all, one);
        // Which is the stage's burn time.
        let burn = p.stage_info()[0].burn_time;
        assert!((all / burn - 1.0).abs() < 0.02, "{all} {burn}");
        assert_eq!(p.burn_time_left(&[]), None);
    }

    #[test]
    fn a_throttled_start_stops_at_the_setting() {
        let mut e = engine("K-1 Booster");
        run(&mut e, 0.6, 3.0, 0.02);
        assert_eq!(e.level, 0.6);
        assert!(!e.spooling());
    }

    #[test]
    fn throttle_settings_below_the_floor_run_at_the_floor() {
        let e = engine("K-1 Booster");
        assert_eq!(e.commanded_level(0.1), e.def.min_throttle);
        assert_eq!(e.commanded_level(0.7), 0.7);
        // Not throttleable: any setting is full thrust.
        assert_eq!(engine("H-10").commanded_level(0.2), 1.0);
    }

    #[test]
    fn throttling_is_rate_limited() {
        let mut e = engine("K-1 Booster");
        run(&mut e, 1.0, 3.0, 0.02);
        let rate = 1.0 / e.def.response.throttle_time;
        run(&mut e, 0.4, 0.5, 0.02);
        assert!((e.level - (1.0 - 0.5 * rate)).abs() < 1e-9, "{}", e.level);
        run(&mut e, 0.4, 5.0, 0.02);
        assert_eq!(e.level, 0.4);
    }

    #[test]
    fn cutoff_tails_off_to_one_percent_after_spool_down() {
        let mut e = engine("K-1 Booster");
        run(&mut e, 1.0, 3.0, 0.02);
        e.running = false;
        let spool_down = e.def.response.spool_down;
        run(&mut e, 0.0, spool_down, 0.02);
        assert!((e.level - 0.01).abs() < 1e-6, "{}", e.level);
        run(&mut e, 0.0, 2.0, 0.02);
        assert_eq!(e.level, 0.0);
    }

    #[test]
    fn response_does_not_depend_on_the_tick_length() {
        let (mut a, mut b) = (engine("K-1 Vacuum"), engine("K-1 Vacuum"));
        run(&mut a, 1.0, 4.0, 0.02);
        run(&mut b, 1.0, 4.0, 0.08);
        assert_eq!(a.level, b.level);
        run(&mut a, 0.5, 0.8, 0.02);
        run(&mut b, 0.5, 0.8, 0.08);
        assert!((a.level - b.level).abs() < 1e-12);
        run(&mut a, 0.0, 0.4, 0.02);
        run(&mut b, 0.0, 0.4, 0.08);
        assert!((a.level - b.level).abs() < 1e-12);
    }

    #[test]
    fn exit_pressure_is_estimated_from_the_isp_loss() {
        let mut e = engine("K-1 Booster").def;
        e.exit_pressure = None;
        assert!(e.exit_pressure() > 50_000.0, "{}", e.exit_pressure());
        let mut e = engine("K-1 Vacuum").def;
        e.exit_pressure = None;
        assert!(e.exit_pressure() < 2_000.0, "{}", e.exit_pressure());
    }
}
