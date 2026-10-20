//! Stages, tanks and RO-style engines: pressure-dependent Isp, limited ignitions, ullage
//! requirements and throttle limits.

use thiserror::Error;

use crate::{
    constants::{ATM, G0},
    data::{CraftDef, EngineDef, PartCatalog},
};

#[derive(Debug, Error)]
pub enum CraftError {
    #[error("unknown engine `{0}`")]
    UnknownEngine(String),
    #[error("unknown tank `{0}`")]
    UnknownTank(String),
    #[error("craft has no stages")]
    NoStages,
}

#[derive(Debug, Clone)]
pub struct Engine {
    pub def: EngineDef,
    pub running: bool,
    pub ignitions_left: Option<u32>,
}

impl Engine {
    /// Full-throttle propellant mass flow, kg/s.
    pub fn max_mass_flow(&self) -> f64 {
        self.def.thrust_vac / (self.def.isp_vac * G0)
    }

    /// Specific impulse at a given ambient pressure (linear between vacuum and 1 atm).
    pub fn isp(&self, pressure: f64) -> f64 {
        let x = pressure / ATM;
        (self.def.isp_vac + (self.def.isp_sl - self.def.isp_vac) * x).max(1.0)
    }

    /// Map player throttle (0..1) onto the engine's throttle range.
    pub fn effective_throttle(&self, throttle: f64) -> f64 {
        let min = self.def.min_throttle.clamp(0.0, 1.0);
        min + (1.0 - min) * throttle.clamp(0.0, 1.0)
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
    /// Everything except propellant, kg.
    pub dry_mass: f64,
    pub propellants: Vec<Propellant>,
    pub diameter: f64,
    pub length: f64,
}

impl Stage {
    pub fn propellant_mass(&self) -> f64 {
        self.propellants.iter().map(|p| p.amount).sum()
    }

    pub fn mass(&self) -> f64 {
        self.dry_mass + self.propellant_mass()
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

    /// Fraction of the scarcest propellant remaining, if the stage carries any.
    pub fn fuel_fraction(&self) -> Option<f64> {
        self.propellants
            .iter()
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
                let mut dry_mass = def.structure_mass;
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
                    dry_mass += engine_def.mass;
                    engines.push(Engine {
                        ignitions_left: engine_def.ignitions,
                        def: engine_def,
                        running: false,
                    });
                }
                let mut propellants: Vec<Propellant> = Vec::new();
                for name in &def.tanks {
                    let tank = catalog
                        .tanks
                        .iter()
                        .find(|t| &t.id == name)
                        .ok_or_else(|| CraftError::UnknownTank(name.clone()))?;
                    dry_mass += tank.dry_mass;
                    for (resource, amount) in &tank.contents {
                        match propellants.iter_mut().find(|p| &p.name == resource) {
                            Some(p) => {
                                p.amount += *amount;
                                p.capacity += *amount;
                            }
                            None => propellants.push(Propellant {
                                name: resource.clone(),
                                amount: *amount,
                                capacity: *amount,
                            }),
                        }
                    }
                }
                Ok(Stage {
                    name: def.name.clone(),
                    active: false,
                    engines,
                    dry_mass,
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
            thrust += flow * isp * G0;
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

    pub fn length(&self) -> f64 {
        self.stages.iter().map(|s| s.length).sum()
    }
}
