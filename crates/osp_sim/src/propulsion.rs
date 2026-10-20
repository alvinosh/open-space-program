//! Stages and RO-style engines: pressure-dependent Isp, limited ignitions, ullage
//! requirements and throttle limits. Tanks, fins and RCS blocks are part of their stage too;
//! sizing tanks and fins is in [`crate::structure`], firing RCS in [`crate::rcs`].

use glam::DVec2;
use thiserror::Error;

use crate::{
    constants::{ATM, G0},
    data::{CraftDef, EngineDef, PartCatalog, PodDef, RcsDef, TankShape},
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
    #[error("stage `{stage}`: {problem}")]
    BadStage { stage: String, problem: String },
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

    /// Map player throttle (0..1) onto the engine's throttle range.
    pub fn effective_throttle(&self, throttle: f64) -> f64 {
        let min = self.def.min_throttle.clamp(0.0, 1.0);
        min + (1.0 - min) * throttle.clamp(0.0, 1.0)
    }
}

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
    pub name: String,
    /// Staged (engines armed). Only the bottom stage can be active.
    pub active: bool,
    pub engines: Vec<Engine>,
    pub tanks: Vec<Tank>,
    /// Empty tanks, kg: thin shells along the stage.
    pub tank_mass: f64,
    pub fins: Option<FinSet>,
    /// Structure, avionics, payload, crew pod, reaction wheels and RCS, kg: spread through
    /// the stage.
    pub structure_mass: f64,
    /// Reaction wheel torque about each axis, N·m.
    pub wheel_torque: f64,
    pub rcs: Vec<RcsBlock>,
    pub pod: Option<PodDef>,
    /// What the tanks hold, pooled: engines and RCS draw from here and the tanks drain
    /// together.
    pub propellants: Vec<Propellant>,
    pub diameter: f64,
    pub length: f64,
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

    /// Everything except propellant, kg.
    pub fn dry_mass(&self) -> f64 {
        self.engine_mass() + self.tank_mass + self.fin_mass() + self.structure_mass
    }

    pub fn mass(&self) -> f64 {
        self.dry_mass() + self.propellant_mass()
    }

    /// Mass of the propellants this stage's engines can actually burn, given mixture ratios.
    pub fn usable_propellant_mass(&self) -> f64 {
        let Some(engine) = self.engines.first() else {
            return 0.0;
        };
        // Limited by whichever propellant runs out first.
        let burnable = engine
            .def
            .propellants
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

/// Per-stage delta-v readout.
#[derive(Debug, Clone)]
pub struct StageInfo {
    pub name: String,
    pub delta_v_vac: f64,
    pub burn_time: f64,
    pub start_mass: f64,
}

/// The stack of stages plus throttle and ignition state.
#[derive(Debug, Clone)]
pub struct Propulsion {
    /// Stages in firing order; index 0 is the current bottom stage.
    pub stages: Vec<Stage>,
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
            .map(|def| {
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
                    name: def.name.clone(),
                    active: false,
                    engines,
                    tanks,
                    tank_mass,
                    fins,
                    structure_mass,
                    wheel_torque,
                    rcs,
                    pod,
                    propellants,
                    diameter: def.diameter,
                    length: def.length,
                })
            })
            .collect::<Result<Vec<_>, CraftError>>()?;
        Ok(Self {
            stages,
            throttle: 0.0,
            ignition_pending: false,
        })
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

    /// True when the next physics step may produce thrust, so the vessel cannot coast.
    pub fn wants_thrust(&self) -> bool {
        self.throttle > 0.0
            && self
                .stages
                .first()
                .is_some_and(|s| s.active && !s.engines.is_empty())
    }

    /// Arm the bottom stage, or jettison it and arm the next one. Returns a log message.
    pub fn activate_next_stage(&mut self) -> Option<String> {
        let first = self.stages.first_mut()?;
        let message = if !first.active {
            first.active = true;
            format!("Staged: {}", first.name)
        } else if self.stages.len() > 1 {
            // TODO: spawn the spent stage as a debris vessel.
            let spent = self.stages.remove(0);
            self.stages[0].active = true;
            format!("Jettisoned {}; staged {}", spent.name, self.stages[0].name)
        } else {
            return None;
        };
        if self.throttle > 0.0 {
            self.ignition_pending = true;
        }
        Some(message)
    }

    /// Run the engines for `dt` seconds. Handles ignition (consuming ignitions, failing if
    /// propellant is unsettled), flameout and propellant use. Returns thrust in newtons.
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
                continue;
            }
            if !engine.running && attempt_ignition {
                match engine.ignitions_left {
                    Some(0) => {
                        log.push(format!("{}: no ignitions left", engine.def.id));
                        continue;
                    }
                    Some(ref mut n) => *n -= 1,
                    None => {}
                }
                if engine.def.requires_ullage && !settled {
                    log.push(format!(
                        "{}: ignition failed, propellant unsettled (ullage needed)",
                        engine.def.id
                    ));
                    continue;
                }
                engine.running = true;
            }
            if !engine.running {
                continue;
            }

            let flow = engine.max_mass_flow() * engine.effective_throttle(throttle);
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
                engine.running = false;
                log.push(format!("{}: flameout, propellant depleted", engine.def.id));
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

    /// Frontal area for drag, m².
    pub fn reference_area(&self) -> f64 {
        let d = self.stages.iter().map(|s| s.diameter).fold(0.0, f64::max);
        std::f64::consts::PI * d * d / 4.0
    }

    /// Drag area (C_D · A) of the fins, m², on top of the body's.
    pub fn fin_drag_area(&self) -> f64 {
        self.stages
            .iter()
            .filter_map(|s| s.fins.as_ref())
            .map(FinSet::drag_area)
            .sum()
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
        }
    }
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
