//! A cabin's own stores: its air, which takes what people breathe out until it is too foul to
//! breathe, and its lockers, full of what its crew and its fitted processes need. The vessel's
//! account of its tanks ([`crate::propulsion::Propulsion::step_systems`]) reaches them for anything
//! on the same part, before its tanks. What the lockers carry is in the part's mass as built, and
//! does not come off as it is used.

use std::any::Any;

use serde::{Deserialize, Serialize};

use super::{Reading, System};
use crate::{component::Component, parts::Part};

/// One thing a cabin keeps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stowed {
    pub resource: String,
    /// kg.
    pub held: f64,
    pub capacity: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cabin {
    /// Its air first, empty at the start, then its lockers, full.
    pub kept: Vec<Stowed>,
    /// How many of `kept` are air.
    pub air: usize,
}

pub(super) fn of(component: &Component, _part: &Part) -> Option<Box<dyn System>> {
    let Component::LifeSupport(life_support) = component else {
        return None;
    };
    if life_support.air.is_empty() && life_support.lockers.is_empty() {
        return None;
    }
    let air = life_support.air.iter().map(|(resource, capacity)| Stowed {
        resource: resource.clone(),
        held: 0.0,
        capacity: *capacity,
    });
    let lockers = life_support.lockers.iter().map(|(resource, kg)| Stowed {
        resource: resource.clone(),
        held: *kg,
        capacity: *kg,
    });
    Some(Box::new(Cabin {
        kept: air.chain(lockers).collect(),
        air: life_support.air.len(),
    }))
}

impl Cabin {
    /// What its air holds of `resource`, and can take, kg.
    pub fn air_of(&self, resource: &str) -> Option<(f64, f64)> {
        (self.kept[..self.air].iter())
            .find(|k| k.resource == resource)
            .map(|k| (k.held, k.capacity))
    }
}

impl System for Cabin {
    fn kind(&self) -> &'static str {
        "cabin"
    }

    fn parameters(&self) -> Vec<Reading> {
        vec![Reading {
            id: "lockers",
            label: "Lockers hold, full",
            value: self.kept[self.air..].iter().map(|k| k.capacity).sum(),
            unit: "kg",
        }]
    }

    fn state(&self) -> Vec<Reading> {
        let (foul, most) = (self.kept[..self.air].iter()).fold((0.0, 0.0), |(held, most), k| {
            (held + k.held, most + k.capacity)
        });
        vec![
            Reading {
                id: "lockers_held",
                label: "Lockers hold",
                value: self.kept[self.air..].iter().map(|k| k.held).sum(),
                unit: "kg",
            },
            Reading {
                id: "air_fouled",
                label: "Air fouled",
                value: if most > 0.0 { 100.0 * foul / most } else { 0.0 },
                unit: "%",
            },
        ]
    }

    fn stored(&self) -> Vec<(String, f64, f64)> {
        (self.kept.iter())
            .map(|k| (k.resource.clone(), k.held, k.capacity))
            .collect()
    }

    fn set_stored(&mut self, amounts: &[f64]) {
        for (kept, amount) in self.kept.iter_mut().zip(amounts) {
            kept.held = amount.clamp(0.0, kept.capacity);
        }
    }

    fn boxed(&self) -> Box<dyn System> {
        Box::new(self.clone())
    }

    fn any(&self) -> &dyn Any {
        self
    }
}
