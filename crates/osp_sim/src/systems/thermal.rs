//! Heat. Every part with any mass is one lump at one temperature, heated by the air on the surfaces
//! that face it ([`crate::heating`]) and cooling by radiating towards where its surroundings would
//! hold it. A heat shield takes what falls on the surfaces it covers: it radiates what it can from a
//! surface that glows, and the rest goes into its ablator until there is none left.

use std::any::Any;

use serde::{Deserialize, Serialize};

use super::{FlightNow, PartHeat, Reading, System};
use crate::{
    component::Component,
    parts::{Part, PartKind},
};

/// What a lump's temperature comes back to with nothing heating it: thermal control, K.
const AMBIENT: f64 = 290.0;
/// J/kg K, for a part's walls and what is in it.
const SPECIFIC_HEAT: f64 = 900.0;
/// The least mass a part counts as for heating, kg: small things are bolted to big ones.
const LEAST_MASS: f64 = 50.0;
/// Hull emissivity, and Stefan and Boltzmann's constant.
const EMISSIVITY: f64 = 0.8;
const STEFAN_BOLTZMANN: f64 = 5.670_374e-8;
/// How hot a part can get before it fails, K.
const TEMPERATURE_LIMIT: f64 = 1200.0;
/// How hot an ablator's surface runs, K, which is how much it radiates away.
const ABLATOR_SURFACE: f64 = 2200.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thermal {
    /// K.
    pub temperature: f64,
    /// The hottest it has been, K.
    pub peak: f64,
    /// J/K.
    capacity: f64,
    /// Radiating, m².
    area: f64,
    /// K.
    pub limit: f64,
    /// The vessel cannot go on without it: a stage's body or a crew's cabin.
    critical: bool,
    /// W, on the last tick.
    pub heating: f64,
}

impl Thermal {
    /// The lump for `part`, if it weighs anything.
    pub(super) fn of_part(part: &Part) -> Option<Self> {
        let built = part.built.as_ref()?;
        let mass = built.mass.mass;
        if mass <= 0.0 {
            return None;
        }
        let crew = (built.components.iter()).any(|c| matches!(c, Component::Crew { .. }))
            || part.is(|k| matches!(k, PartKind::Pod(_)));
        Some(Self {
            temperature: AMBIENT,
            peak: AMBIENT,
            capacity: mass.max(LEAST_MASS) * SPECIFIC_HEAT,
            // A rough ball of it.
            area: 6.0 * (mass.max(LEAST_MASS) / 300.0).powf(2.0 / 3.0),
            limit: TEMPERATURE_LIMIT,
            critical: crew || part.body.is_some(),
            heating: 0.0,
        })
    }

    /// Whether this part has failed the vessel.
    pub fn failed(&self) -> bool {
        self.critical && self.temperature > self.limit
    }
}

impl System for Thermal {
    fn kind(&self) -> &'static str {
        "thermal"
    }

    fn parameters(&self) -> Vec<Reading> {
        vec![Reading {
            id: "limit",
            label: "Temperature limit",
            value: self.limit,
            unit: "K",
        }]
    }

    fn state(&self) -> Vec<Reading> {
        vec![Reading {
            id: "temperature",
            label: "Temperature",
            value: self.temperature,
            unit: "K",
        }]
    }

    fn step(&mut self, dt: f64) {
        // Exact for any step, as time warp takes long ones.
        let rate = 4.0 * EMISSIVITY * STEFAN_BOLTZMANN * self.area * self.temperature.powi(3)
            / self.capacity;
        self.temperature = AMBIENT + (self.temperature - AMBIENT) * (-rate * dt).exp();
    }

    fn fly(&mut self, _flight: &FlightNow, heat: &mut PartHeat, dt: f64) {
        self.heating = heat.bare;
        self.temperature += heat.bare * dt / self.capacity;
        self.peak = self.peak.max(self.temperature);
    }

    fn boxed(&self) -> Box<dyn System> {
        Box::new(self.clone())
    }

    fn any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeatShield {
    /// kg left.
    pub ablator: f64,
    /// kg it began with.
    pub full: f64,
    /// What burning a kilogram takes away, J.
    pub heat_of_ablation: f64,
    /// W/m², at its surface, on the last tick.
    pub flux: f64,
    /// The most it has taken, W/m².
    pub peak_flux: f64,
}

pub(super) fn heat_shield(component: &Component, _part: &Part) -> Option<Box<dyn System>> {
    let Component::HeatShield(def) = component else {
        return None;
    };
    Some(Box::new(HeatShield {
        ablator: def.ablator,
        full: def.ablator,
        heat_of_ablation: def.heat_of_ablation,
        flux: 0.0,
        peak_flux: 0.0,
    }))
}

impl System for HeatShield {
    fn kind(&self) -> &'static str {
        "heat_shield"
    }

    fn parameters(&self) -> Vec<Reading> {
        vec![Reading {
            id: "ablator_full",
            label: "Ablator",
            value: self.full,
            unit: "kg",
        }]
    }

    fn state(&self) -> Vec<Reading> {
        vec![Reading {
            id: "ablator",
            label: "Ablator left",
            value: self.ablator,
            unit: "kg",
        }]
    }

    fn fly(&mut self, flight: &FlightNow, heat: &mut PartHeat, dt: f64) {
        let area = heat.shield_area;
        self.flux = flight.stagnation_heating * (area > 0.0) as u8 as f64;
        self.peak_flux = self.peak_flux.max(self.flux);
        if heat.shielded <= 0.0 {
            return;
        }
        if self.ablator <= 0.0 {
            // Burned through: what it was taking goes into the part behind it.
            heat.bare += heat.shielded;
            heat.shielded = 0.0;
            return;
        }
        let radiated = area * EMISSIVITY * STEFAN_BOLTZMANN * ABLATOR_SURFACE.powi(4);
        let left = (heat.shielded - radiated).max(0.0);
        self.ablator = (self.ablator - left * dt / self.heat_of_ablation).max(0.0);
        heat.shielded = 0.0;
    }

    fn boxed(&self) -> Box<dyn System> {
        Box::new(self.clone())
    }

    fn any(&self) -> &dyn Any {
        self
    }
}
