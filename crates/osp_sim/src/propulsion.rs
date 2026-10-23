//! Stages and RO-style engines: pressure-dependent Isp, limited ignitions, ullage
//! requirements, throttle limits and thrust that spools up after ignition, follows the
//! throttle at a limited rate and tails off after cutoff. Tanks, fins, RCS blocks and
//! fairings are part of their stage too; sizing tanks and fins is in [`crate::structure`],
//! firing RCS in [`crate::rcs`]. What the stage key does is a list of activation sequences
//! ([`crate::staging`]), carried out here.

use std::collections::BTreeMap;

use glam::DVec2;
use thiserror::Error;

use crate::{
    constants::{ATM, G0},
    data::{
        CraftDef, EngineDef, GearDef, PartCatalog, PodDef, RcsDef, StageAppearance, TankShape,
        WingDef,
    },
    staging::{Activation, Sequences},
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
    pub running: bool,
    pub ignitions_left: Option<u32>,
    /// Where the engine is mounted, seen from behind: offset from the vessel axis in vessel
    /// X/Y, m. The gimbal pivot is at the base of the stage.
    pub mount: DVec2,
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

#[derive(Debug, Clone)]
pub struct RcsBlock {
    pub def: RcsDef,
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

#[derive(Debug, Clone)]
pub struct Stage {
    /// Index in the craft definition; stays the same as stages below fall away.
    pub id: usize,
    pub name: String,
    /// Engines armed: they light with the throttle once this is the bottom stage.
    pub active: bool,
    pub engines: Vec<Engine>,
    pub tanks: Vec<Tank>,
    /// Empty tanks, kg: thin shells along the stage.
    pub tank_mass: f64,
    pub fins: Option<FinSet>,
    /// Wings, tailplanes and tail fins.
    pub wings: Vec<WingDef>,
    /// Landing gear legs.
    pub gear: Vec<GearDef>,
    /// Structure, avionics, payload, crew pod, reaction wheels and RCS, kg: spread through
    /// the stage.
    pub structure_mass: f64,
    /// Reaction wheel torque about each axis, N·m.
    pub wheel_torque: f64,
    pub rcs: Vec<RcsBlock>,
    pub pod: Option<PodDef>,
    pub fairing: Option<Fairing>,
    /// What the tanks hold, pooled: engines and RCS draw from here and the tanks drain
    /// together.
    pub propellants: Vec<Propellant>,
    pub diameter: f64,
    pub length: f64,
    /// How it is drawn.
    pub appearance: StageAppearance,
}

impl Stage {
    pub fn propellant_mass(&self) -> f64 {
        self.propellants.iter().map(|p| p.amount).sum()
    }

    pub fn propellant_capacity(&self) -> f64 {
        self.propellants.iter().map(|p| p.capacity).sum()
    }

    pub fn engine_mass(&self) -> f64 {
        self.engines.iter().map(|e| e.def.mass).sum()
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

    /// Everything except propellant, kg.
    pub fn dry_mass(&self) -> f64 {
        self.engine_mass()
            + self.tank_mass
            + self.fin_mass()
            + self.wing_mass()
            + self.gear_mass()
            + self.structure_mass
            + self.fairing_mass()
    }

    pub fn mass(&self) -> f64 {
        self.dry_mass() + self.propellant_mass()
    }

    /// Mass of the propellants this stage's engines can actually burn, given mixture ratios.
    pub fn usable_propellant_mass(&self) -> f64 {
        self.engines
            .first()
            .map_or(0.0, |e| self.burnable(&e.def.propellants))
    }

    /// Mass of the propellants this stage's RCS blocks can still burn, kg.
    pub fn rcs_propellant_mass(&self) -> f64 {
        self.rcs
            .first()
            .map_or(0.0, |b| self.burnable(&b.def.propellants))
    }

    /// How much of the stage's propellant a mixture (ratios normalised) can burn, kg.
    fn burnable(&self, mixture: &BTreeMap<String, f64>) -> f64 {
        // Limited by whichever propellant runs out first.
        let burnable = mixture
            .iter()
            .map(|(name, ratio)| {
                let available = self
                    .propellants
                    .iter()
                    .find(|p| &p.name == name)
                    .map_or(0.0, |p| p.amount);
                available / ratio
            })
            .fold(f64::INFINITY, f64::min);
        if burnable.is_finite() { burnable } else { 0.0 }
    }

    pub fn thrust_vac(&self) -> f64 {
        self.engines.iter().map(|e| e.def.thrust_vac).sum()
    }

    /// Thrust-weighted vacuum Isp.
    pub fn isp_vac(&self) -> f64 {
        let flow: f64 = self.engines.iter().map(Engine::max_mass_flow).sum();
        if flow > 0.0 {
            self.thrust_vac() / (flow * G0)
        } else {
            0.0
        }
    }

    /// Fraction of the scarcest propellant remaining, if the stage carries any. With
    /// engines, only what they burn counts (not RCS propellant).
    pub fn fuel_fraction(&self) -> Option<f64> {
        self.fraction_of(|name| {
            self.engines.is_empty()
                || self
                    .engines
                    .iter()
                    .any(|e| e.def.propellants.contains_key(name))
        })
    }

    /// Fraction of the scarcest RCS propellant remaining, if the stage has RCS and carries
    /// any of it.
    pub fn rcs_fraction(&self) -> Option<f64> {
        self.fraction_of(|name| {
            self.rcs
                .iter()
                .any(|b| b.def.propellants.contains_key(name))
        })
    }

    fn fraction_of(&self, counts: impl Fn(&str) -> bool) -> Option<f64> {
        self.propellants
            .iter()
            .filter(|p| counts(&p.name))
            .map(|p| p.amount / p.capacity.max(1e-9))
            .reduce(f64::min)
    }
}

/// A payload fairing around a stage.
#[derive(Debug, Clone, PartialEq)]
pub struct Fairing {
    /// kg.
    pub mass: f64,
    /// Not jettisoned yet.
    pub attached: bool,
    /// Only one half of it, flying on its own after a jettison: the angle about the stage
    /// axis that the middle of the half faces (0 along vessel +X), rad.
    pub half: Option<f64>,
}

/// Angles the two halves of a fairing face; their seams are a quarter turn either side.
pub const FAIRING_HALVES: [f64; 2] = [0.0, std::f64::consts::PI];

/// Something a sequence released, to fly on as debris.
#[derive(Debug, Clone)]
pub struct Released {
    /// Bottom first, as they were on the vessel.
    pub stages: Vec<Stage>,
    /// Height of its base above the vessel's base before the sequence fired, m.
    pub base: f64,
}

/// What firing a sequence did.
#[derive(Debug, Clone, Default)]
pub struct Staged {
    /// For the flight log, one per activation.
    pub messages: Vec<String>,
    /// Length of the stages that fell away, m.
    pub dropped_length: f64,
    /// Mass left the vessel (stages fell away or a fairing was jettisoned).
    pub mass_changed: bool,
    /// An activation switched RCS on.
    pub rcs_on: bool,
    /// Stages that fell away and fairing halves that were jettisoned.
    pub released: Vec<Released>,
}

/// Per-stage delta-v readout.
#[derive(Debug, Clone)]
pub struct StageInfo {
    pub name: String,
    pub delta_v_vac: f64,
    pub burn_time: f64,
    pub start_mass: f64,
}

/// The stack of stages, the activation sequences, and throttle and ignition state.
#[derive(Debug, Clone)]
pub struct Propulsion {
    /// Stages from the bottom of the stack; index 0 is the current bottom stage.
    pub stages: Vec<Stage>,
    /// What the stage key does next; the player can rearrange them.
    pub sequences: Sequences,
    throttle: f64,
    ignition_pending: bool,
}

impl Propulsion {
    pub fn from_craft(craft: &CraftDef, catalog: &PartCatalog) -> Result<Self, CraftError> {
        if craft.stages.is_empty() {
            return Err(CraftError::NoStages);
        }
        let stages = craft
            .stages
            .iter()
            .enumerate()
            .map(|(id, def)| {
                let mut tank_mass = 0.0;
                let mut structure_mass = def.structure_mass;
                let mut wheel_torque = 0.0;
                let mut engines = Vec::new();
                for name in &def.engines {
                    let mut engine_def = catalog
                        .engines
                        .iter()
                        .find(|e| &e.id == name)
                        .cloned()
                        .ok_or_else(|| CraftError::UnknownEngine(name.clone()))?;
                    let total: f64 = engine_def.propellants.values().sum();
                    for ratio in engine_def.propellants.values_mut() {
                        *ratio /= total;
                    }
                    engines.push(Engine {
                        ignitions_left: engine_def.ignitions,
                        def: engine_def,
                        running: false,
                        mount: DVec2::ZERO,
                        thrust: 0.0,
                        level: 0.0,
                        spooling: None,
                        deflection: DVec2::ZERO,
                    });
                }
                let mounts = engine_layout(engines.len(), def.diameter / 2.0);
                for (engine, mount) in engines.iter_mut().zip(mounts) {
                    engine.mount = mount;
                }
                for name in &def.reaction_wheels {
                    let wheel = catalog
                        .reaction_wheels
                        .iter()
                        .find(|w| &w.id == name)
                        .ok_or_else(|| CraftError::UnknownReactionWheel(name.clone()))?;
                    structure_mass += wheel.mass;
                    wheel_torque += wheel.torque;
                }
                let mut rcs = Vec::new();
                for (i, name) in def.rcs.iter().enumerate() {
                    let mut block_def = catalog
                        .rcs
                        .iter()
                        .find(|b| &b.id == name)
                        .cloned()
                        .ok_or_else(|| CraftError::UnknownRcs(name.clone()))?;
                    let total: f64 = block_def.propellants.values().sum();
                    for ratio in block_def.propellants.values_mut() {
                        *ratio /= total;
                    }
                    structure_mass += block_def.mass;
                    let angle = std::f64::consts::TAU * i as f64 / def.rcs.len() as f64;
                    rcs.push(RcsBlock {
                        def: block_def,
                        mount: DVec2::from_angle(angle) * def.diameter / 2.0,
                        firing: [0.0; 4],
                    });
                }
                let bad = |problem: String| CraftError::BadStage {
                    stage: def.name.clone(),
                    problem,
                };
                let pod = def
                    .pod
                    .as_ref()
                    .map(|name| {
                        catalog
                            .pods
                            .iter()
                            .find(|p| &p.id == name)
                            .cloned()
                            .ok_or_else(|| CraftError::UnknownPod(name.clone()))
                    })
                    .transpose()?;
                if let Some(pod) = &pod {
                    if pod.height > def.length {
                        return Err(bad(format!(
                            "the {} pod is {} m tall, the stage only {} m",
                            pod.id, pod.height, def.length
                        )));
                    }
                    structure_mass += pod.mass;
                }
                let fins = def
                    .fins
                    .as_ref()
                    .map(|f| FinSet::new(f, catalog))
                    .transpose()?;
                for wing in &def.wings {
                    check_wing(wing)?;
                }
                for leg in &def.gear {
                    if leg.length <= 0.0 || leg.wheel_radius <= 0.0 || leg.stroke <= 0.0 {
                        return Err(bad(format!(
                            "gear `{}`: length, wheel radius and stroke must be positive",
                            leg.name
                        )));
                    }
                    if leg.load <= 0.0 || leg.mass < 0.0 {
                        return Err(bad(format!(
                            "gear `{}`: it needs a load to be sprung for, and no negative mass",
                            leg.name
                        )));
                    }
                }
                // Cylinders stack up from the stage base; spheres sit inside, at the base.
                let mut tanks = Vec::new();
                let mut stacked = 0.0;
                for tank_def in &def.tanks {
                    let cylinder = tank_def.shape == TankShape::Cylinder;
                    let base = if cylinder { stacked } else { 0.0 };
                    let tank = Tank::new(tank_def, def.diameter, base, catalog)?;
                    if 2.0 * tank.radius > def.diameter + 1e-9 {
                        return Err(bad(format!(
                            "a {} m tank does not fit its {} m diameter",
                            2.0 * tank.radius,
                            def.diameter
                        )));
                    }
                    if cylinder {
                        stacked += tank.length;
                    }
                    tank_mass += tank.dry_mass;
                    tanks.push(tank);
                }
                if stacked > def.length + 1e-9 {
                    return Err(bad(format!(
                        "its tanks are {stacked} m long, the stage only {} m",
                        def.length
                    )));
                }
                let mut propellants: Vec<Propellant> = Vec::new();
                for content in tanks.iter().flat_map(|t| &t.contents) {
                    match propellants.iter_mut().find(|p| p.name == content.resource) {
                        Some(p) => {
                            p.amount += content.capacity;
                            p.capacity += content.capacity;
                        }
                        None => propellants.push(Propellant {
                            name: content.resource.clone(),
                            amount: content.capacity,
                            capacity: content.capacity,
                        }),
                    }
                }
                Ok(Stage {
                    id,
                    name: def.name.clone(),
                    active: false,
                    engines,
                    tanks,
                    tank_mass,
                    fins,
                    wings: def.wings.clone(),
                    gear: def.gear.clone(),
                    structure_mass,
                    wheel_torque,
                    rcs,
                    pod,
                    fairing: def.fairing.as_ref().map(|f| Fairing {
                        mass: f.mass,
                        attached: true,
                        half: None,
                    }),
                    propellants,
                    diameter: def.diameter,
                    length: def.length,
                    appearance: def.appearance.clone(),
                })
            })
            .collect::<Result<Vec<_>, CraftError>>()?;
        let sequences = match &craft.sequences {
            Some(def) => Sequences::parse(def, &stages).map_err(CraftError::BadSequence)?,
            None => Sequences::default_for(&stages),
        };
        Ok(Self {
            stages,
            sequences,
            throttle: 0.0,
            ignition_pending: false,
        })
    }

    /// Stages that came loose from a vessel, as one of their own: nothing left to stage,
    /// throttle closed (engines that were running tail off).
    pub fn from_stages(stages: Vec<Stage>) -> Self {
        Self {
            stages,
            sequences: Sequences::default(),
            throttle: 0.0,
            ignition_pending: false,
        }
    }

    pub fn mass(&self) -> f64 {
        self.stages.iter().map(Stage::mass).sum()
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

    pub fn any_engine_running(&self) -> bool {
        self.stages
            .first()
            .is_some_and(|s| s.engines.iter().any(|e| e.running))
    }

    /// True when the next physics step may produce thrust, so the vessel cannot coast:
    /// the throttle is open on an armed stage, or an engine is still tailing off.
    pub fn wants_thrust(&self) -> bool {
        self.stages.first().is_some_and(|s| {
            (self.throttle > 0.0 && s.active && !s.engines.is_empty())
                || s.engines.iter().any(|e| e.level > 0.0)
        })
    }

    /// Thrust-weighted share of full thrust the bottom stage's engines are at, 0..1.
    pub fn engine_level(&self) -> f64 {
        let Some(stage) = self.stages.first() else {
            return 0.0;
        };
        let full: f64 = stage.engines.iter().map(|e| e.def.thrust_vac).sum();
        if full <= 0.0 {
            return 0.0;
        }
        stage
            .engines
            .iter()
            .map(|e| e.level * e.def.thrust_vac)
            .sum::<f64>()
            / full
    }

    /// Impulse the bottom stage's engines would still give if cut now, N·s.
    pub fn tail_off_impulse(&self, pressure: f64) -> f64 {
        self.stages.first().map_or(0.0, |s| {
            s.engines
                .iter()
                .map(|e| e.level * e.max_mass_flow() * e.isp(pressure) * G0 * e.tail_off_time())
                .sum()
        })
    }

    /// Fire the next sequence: release its decouplers (everything below falls away), light
    /// its engines (they start with the throttle once their stage is at the bottom), jettison
    /// its fairings and switch RCS on. `None` when no sequence is left.
    pub fn activate_next_sequence(&mut self) -> Option<Staged> {
        let sequence = self.sequences.pop_next()?;
        let mut staged = Staged::default();
        if sequence.is_empty() {
            staged.messages.push("Nothing to stage".into());
        }
        for activation in sequence {
            let Some(index) = self.stages.iter().position(|s| s.id == activation.stage()) else {
                continue;
            };
            match activation {
                Activation::Decoupler(_) => {
                    if index + 1 >= self.stages.len() {
                        continue;
                    }
                    let spent: Vec<Stage> = self.stages.drain(..=index).collect();
                    let base = staged.dropped_length;
                    staged.dropped_length += spent.iter().map(|s| s.length).sum::<f64>();
                    staged.mass_changed = true;
                    let names: Vec<_> = spent.iter().map(|s| s.name.as_str()).collect();
                    staged
                        .messages
                        .push(format!("Separated {}", names.join(", ")));
                    let gone: Vec<usize> = spent.iter().map(|s| s.id).collect();
                    self.sequences.forget(&gone);
                    staged.released.push(Released {
                        stages: spent,
                        base,
                    });
                    // Engines armed above light as soon as they are at the bottom.
                    if self.throttle > 0.0 && self.stages[0].active {
                        self.ignition_pending = true;
                    }
                }
                Activation::Engines(_) => {
                    let stage = &mut self.stages[index];
                    stage.active = true;
                    staged.messages.push(format!("Staged: {}", stage.name));
                    if index == 0 && self.throttle > 0.0 {
                        self.ignition_pending = true;
                    }
                }
                Activation::Fairing(_) => {
                    let stage = &self.stages[index];
                    let Some(mass) = stage
                        .fairing
                        .as_ref()
                        .filter(|f| f.attached)
                        .map(|f| f.mass)
                    else {
                        continue;
                    };
                    let base = staged.dropped_length
                        + self.stages[..index].iter().map(|s| s.length).sum::<f64>();
                    for angle in FAIRING_HALVES {
                        staged.released.push(Released {
                            stages: vec![fairing_half(stage, mass / 2.0, angle)],
                            base,
                        });
                    }
                    if let Some(fairing) = &mut self.stages[index].fairing {
                        fairing.attached = false;
                    }
                    staged.mass_changed = true;
                    staged.messages.push("Fairing jettisoned".into());
                }
                Activation::Rcs(_) => {
                    staged.rcs_on = true;
                    staged.messages.push("RCS on".into());
                }
            }
        }
        Some(staged)
    }

    /// Arm the bottom stage's engines outside the sequences (the burn executor does), taking
    /// their activation out of whichever sequence held it. Returns a log message if they
    /// were not armed yet.
    pub fn arm_bottom(&mut self) -> Option<String> {
        let stage = self
            .stages
            .first_mut()
            .filter(|s| !s.active && !s.engines.is_empty())?;
        stage.active = true;
        let message = format!("Staged: {}", stage.name);
        let id = stage.id;
        self.sequences.take(Activation::Engines(id));
        if self.throttle > 0.0 {
            self.ignition_pending = true;
        }
        Some(message)
    }

    /// Run the engines for `dt` seconds. Handles ignition (consuming ignitions, failing if
    /// propellant is unsettled), spool-up, throttling, tail-off, flameout and propellant
    /// use. Returns thrust in newtons (the mean over the step).
    pub fn burn(&mut self, pressure: f64, dt: f64, settled: bool, log: &mut Vec<String>) -> f64 {
        let throttle = self.throttle;
        let attempt_ignition = std::mem::take(&mut self.ignition_pending);
        let Some(stage) = self.stages.first_mut().filter(|s| s.active) else {
            return 0.0;
        };
        let mut thrust = 0.0;
        for i in 0..stage.engines.len() {
            let engine = &mut stage.engines[i];
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

            let flow = engine.max_mass_flow() * level;
            let isp = engine.isp(pressure);
            let starved = engine.def.propellants.iter().any(|(name, ratio)| {
                stage
                    .propellants
                    .iter()
                    .find(|p| &p.name == name)
                    .is_none_or(|p| p.amount < flow * ratio * dt)
            });
            if starved {
                let engine = &mut stage.engines[i];
                if engine.running {
                    log.push(format!("{}: flameout, propellant depleted", engine.def.id));
                }
                engine.running = false;
                engine.level = 0.0;
                engine.spooling = None;
                continue;
            }
            for (name, ratio) in stage.engines[i].def.propellants.clone() {
                if let Some(p) = stage.propellants.iter_mut().find(|p| p.name == name) {
                    p.amount -= flow * ratio * dt;
                }
            }
            stage.engines[i].thrust = flow * isp * G0;
            thrust += stage.engines[i].thrust;
        }
        thrust
    }

    /// Delta-v of each remaining stage in vacuum, in firing order.
    pub fn stage_info(&self) -> Vec<StageInfo> {
        (0..self.stages.len())
            .map(|i| {
                let stage = &self.stages[i];
                let start_mass: f64 = self.stages[i..].iter().map(Stage::mass).sum();
                let burned = stage.usable_propellant_mass();
                let isp = stage.isp_vac();
                let flow: f64 = stage.engines.iter().map(Engine::max_mass_flow).sum();
                StageInfo {
                    name: stage.name.clone(),
                    delta_v_vac: if isp > 0.0 {
                        isp * G0 * (start_mass / (start_mass - burned)).ln()
                    } else {
                        0.0
                    },
                    burn_time: if flow > 0.0 { burned / flow } else { 0.0 },
                    start_mass,
                }
            })
            .collect()
    }

    /// Mass of propellant left for the RCS, kg.
    pub fn rcs_propellant_mass(&self) -> f64 {
        self.stages.iter().map(Stage::rcs_propellant_mass).sum()
    }

    /// Delta-v the RCS can still give, pushing the whole vessel along in vacuum, m/s.
    pub fn rcs_delta_v(&self) -> f64 {
        let (mut thrust, mut flow) = (0.0, 0.0);
        for stage in self.stages.iter().filter(|s| s.rcs_propellant_mass() > 0.0) {
            for block in &stage.rcs {
                thrust += block.def.thrust;
                flow += block.nozzle_mass_flow();
            }
        }
        let (mass, propellant) = (self.mass(), self.rcs_propellant_mass());
        if flow <= 0.0 || propellant >= mass {
            return 0.0;
        }
        thrust / flow * (mass / (mass - propellant)).ln()
    }

    pub fn length(&self) -> f64 {
        self.stages.iter().map(|s| s.length).sum()
    }

    /// Cut the throttle and stop every engine for good (the vessel broke up).
    pub fn shut_down(&mut self) {
        self.throttle = 0.0;
        self.ignition_pending = false;
        for engine in self.stages.iter_mut().flat_map(|s| &mut s.engines) {
            engine.running = false;
            engine.thrust = 0.0;
            engine.level = 0.0;
            engine.spooling = None;
        }
    }
}

/// One half of `stage`'s fairing, of `mass`, as a stage of its own: the shell around the
/// stage (and its nose), facing `angle`.
fn fairing_half(stage: &Stage, mass: f64, angle: f64) -> Stage {
    Stage {
        id: stage.id,
        name: "Fairing".into(),
        active: false,
        engines: Vec::new(),
        tanks: Vec::new(),
        tank_mass: 0.0,
        fins: None,
        wings: Vec::new(),
        gear: Vec::new(),
        structure_mass: 0.0,
        wheel_torque: 0.0,
        rcs: Vec::new(),
        pod: None,
        fairing: Some(Fairing {
            mass,
            attached: true,
            half: Some(angle),
        }),
        propellants: Vec::new(),
        diameter: stage.diameter,
        length: stage.length,
        appearance: stage.appearance.clone(),
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

/// Engine positions for a stage of the given radius, seen from behind: a single engine on
/// the axis, small clusters on a ring, and seven or more as a ring around a central engine
/// (like a Falcon 9 octaweb). Off-axis engines can roll the vessel by gimballing sideways.
fn engine_layout(count: usize, radius: f64) -> Vec<DVec2> {
    let ring = |n: usize, r: f64| {
        (0..n).map(move |i| DVec2::from_angle(std::f64::consts::TAU * i as f64 / n as f64) * r)
    };
    match count {
        0 => Vec::new(),
        1 => vec![DVec2::ZERO],
        2..=6 => ring(count, 0.5 * radius).collect(),
        _ => std::iter::once(DVec2::ZERO)
            .chain(ring(count - 1, 0.65 * radius))
            .collect(),
    }
}

/// A wing must have a shape, a thickness that is an airfoil's, and its control surface
/// must fit on it.
fn check_wing(wing: &WingDef) -> Result<(), CraftError> {
    let bad = |problem: &str| {
        Err(CraftError::BadWing {
            wing: wing.name.clone(),
            problem: problem.into(),
        })
    };
    if wing.root_chord <= 0.0 || wing.tip_chord < 0.0 || wing.span <= 0.0 {
        return bad("chords and span must be positive");
    }
    if !(0.0..=0.5).contains(&wing.thickness) || wing.thickness == 0.0 {
        return bad("thickness is a share of the chord, above 0 and at most 0.5");
    }
    if wing.mass < 0.0 {
        return bad("mass cannot be negative");
    }
    if let Some(c) = &wing.control {
        if !(c.chord > 0.0 && c.chord <= 1.0) {
            return bad("the control surface's chord is a share of the wing's, up to 1");
        }
        if !(0.0 <= c.span[0] && c.span[0] < c.span[1] && c.span[1] <= 1.0) {
            return bad(
                "the control surface's span runs from a share to a larger one, within 0..1",
            );
        }
        if !(c.deflection > 0.0 && c.deflection <= 60.0) {
            return bad("the control surface deflects more than 0 and at most 60 degrees");
        }
    }
    Ok(())
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
            running: true,
            mount: DVec2::ZERO,
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
