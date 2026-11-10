//! A part that turns some resources into others, by a record of `processes`: electrolysis, a
//! recycler, a converter. It runs as fast as its inputs, the room for its outputs and its power
//! let it, which the vessel's one account of its tanks works out
//! ([`crate::propulsion::Propulsion::step_systems`]).

use std::any::Any;

use serde::{Deserialize, Serialize};

use super::{Exchange, Power, Reading, System};
use crate::{
    component::Component, data::ProcessDef, environment::Environment, parts::Part, staging::Action,
};

pub const START: Action = Action::new("process_start", "Start");
pub const STOP: Action = Action::new("process_stop", "Stop");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Process {
    pub def: ProcessDef,
    pub scale: f64,
    pub priority: u8,
    pub on: bool,
    /// The share of full speed its tanks allow this step, worked out before the power networks.
    #[serde(default)]
    planned: f64,
    /// Its network gave it the power it asked for.
    #[serde(default)]
    powered: bool,
    /// The share of full speed it ran at over the last step.
    #[serde(default)]
    pub running: f64,
}

pub(super) fn of(component: &Component, _part: &Part) -> Option<Box<dyn System>> {
    let Component::Process(unit) = component else {
        return None;
    };
    Some(Box::new(Process {
        def: unit.def.clone(),
        scale: unit.scale,
        priority: unit.priority,
        on: unit.on,
        planned: 0.0,
        powered: false,
        running: 0.0,
    }))
}

impl Process {
    /// What it takes and makes at full speed over `dt` seconds.
    fn full(&self, dt: f64) -> Exchange {
        let times = |rates: &std::collections::BTreeMap<String, f64>| -> Vec<(String, f64)> {
            (rates.iter())
                .map(|(resource, rate)| (resource.clone(), rate * self.scale * dt))
                .collect()
        };
        Exchange {
            inputs: times(&self.def.inputs),
            outputs: times(&self.def.outputs),
            dump: self.def.dump.clone(),
            priority: self.priority,
        }
    }

    fn needs_power(&self) -> bool {
        self.def.power > 0.0
    }

    /// It can run this step: switched on, and powered if it needs to be.
    fn can_run(&self) -> bool {
        self.on && (self.powered || !self.needs_power())
    }
}

impl System for Process {
    fn kind(&self) -> &'static str {
        "process"
    }

    fn parameters(&self) -> Vec<Reading> {
        vec![Reading {
            id: "power",
            label: "Power at full speed",
            value: self.def.power * self.scale,
            unit: "W",
        }]
    }

    fn state(&self) -> Vec<Reading> {
        vec![Reading {
            id: "running",
            label: "Running at",
            value: 100.0 * self.running,
            unit: "%",
        }]
    }

    fn power(&self, _environment: &Environment) -> Option<Power> {
        self.needs_power().then(|| Power::Load {
            watts: if self.on {
                self.def.power * self.scale * self.planned
            } else {
                0.0
            },
            priority: self.priority,
        })
    }

    fn powered(&mut self, on: bool) {
        self.powered = on;
    }

    fn consumes(&self) -> bool {
        true
    }

    fn wants(&self, dt: f64) -> Vec<Exchange> {
        if self.on {
            vec![self.full(dt)]
        } else {
            Vec::new()
        }
    }

    fn plan(&mut self, shares: &[f64]) {
        self.planned = shares.first().copied().unwrap_or(0.0);
    }

    fn uses(&self, dt: f64) -> Vec<Exchange> {
        if !self.can_run() || self.planned <= 0.0 {
            return Vec::new();
        }
        let mut lot = self.full(dt);
        for (_, kg) in lot.inputs.iter_mut().chain(lot.outputs.iter_mut()) {
            *kg *= self.planned;
        }
        vec![lot]
    }

    fn exchanged(&mut self, shares: &[f64], _dt: f64) {
        self.running = self.planned * shares.first().copied().unwrap_or(0.0);
    }

    fn exchanging(&self) -> Vec<Exchange> {
        if self.running > 0.0 {
            let mut lot = self.full(1.0);
            for (_, kg) in lot.inputs.iter_mut().chain(lot.outputs.iter_mut()) {
                *kg *= self.running;
            }
            vec![lot]
        } else {
            Vec::new()
        }
    }

    fn actions(&self) -> Vec<Action> {
        vec![if self.on { STOP } else { START }]
    }

    fn act(&mut self, action: Action) -> Option<String> {
        let name = if self.def.name.is_empty() {
            &self.def.id
        } else {
            &self.def.name
        };
        if action.kind == START.kind && !self.on {
            self.on = true;
            Some(format!("{name} started"))
        } else if action.kind == STOP.kind && self.on {
            self.on = false;
            self.running = 0.0;
            Some(format!("{name} stopped"))
        } else {
            None
        }
    }

    fn boxed(&self) -> Box<dyn System> {
        Box::new(self.clone())
    }

    fn any(&self) -> &dyn Any {
        self
    }
}
