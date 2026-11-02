//! Stored electricity, on the power network of its part.

use std::any::Any;

use super::{Power, Reading, System};
use crate::{component::Component, environment::Environment, parts::Part};

#[derive(Debug, Clone, PartialEq)]
pub struct Battery {
    /// When full, kJ.
    pub capacity: f64,
    /// What it holds now, kJ.
    pub charge: f64,
}

pub(super) fn of(component: &Component, _part: &Part) -> Option<Box<dyn System>> {
    let Component::Battery(built) = component else {
        return None;
    };
    Some(Box::new(Battery {
        capacity: built.charge,
        charge: built.charge,
    }))
}

impl System for Battery {
    fn kind(&self) -> &'static str {
        "battery"
    }

    fn parameters(&self) -> Vec<Reading> {
        vec![Reading {
            id: "capacity",
            label: "Capacity",
            value: self.capacity,
            unit: "kJ",
        }]
    }

    fn state(&self) -> Vec<Reading> {
        vec![Reading {
            id: "charge",
            label: "Charge",
            value: self.charge,
            unit: "kJ",
        }]
    }

    fn power(&self, _environment: &Environment) -> Option<Power> {
        Some(Power::Store {
            charge: self.charge,
            capacity: self.capacity,
        })
    }

    fn charge(&mut self, kj: f64) {
        self.charge = (self.charge + kj).clamp(0.0, self.capacity);
    }

    fn boxed(&self) -> Box<dyn System> {
        Box::new(self.clone())
    }

    fn any(&self) -> &dyn Any {
        self
    }
}
