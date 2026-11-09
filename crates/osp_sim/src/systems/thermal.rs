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
/// How hot a part can get before it fails, K, when what it is made of does not say.
const TEMPERATURE_LIMIT: f64 = 1200.0;
/// The cool wall that a stagnation heating figure is quoted for, K.
const COLD_WALL: f64 = 300.0;
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
        let shield = (built.components.iter()).any(|c| matches!(c, Component::HeatShield(_)));
        Some(Self {
            temperature: AMBIENT,
            peak: AMBIENT,
            capacity: mass.max(LEAST_MASS) * built.specific_heat.unwrap_or(SPECIFIC_HEAT),
            // A rough ball of it.
            area: 6.0 * (mass.max(LEAST_MASS) / 300.0).powf(2.0 / 3.0),
            limit: match built.max_temperature {
                // What takes the heat is the ablator, not the frame it is on.
                _ if shield && !crew => ABLATOR_SURFACE,
                Some(limit) => limit,
                None => TEMPERATURE_LIMIT,
            },
            critical: crew || part.body.is_some(),
            heating: 0.0,
        })
    }

    /// Whether this part has failed the vessel.
    pub fn failed(&self) -> bool {
        self.critical && self.temperature > self.limit
    }

    /// Whether this part has got too hot to stay on, the vessel going on without it.
    pub fn burned_through(&self) -> bool {
        !self.critical && self.temperature > self.limit
    }

    /// How far it is from where it was comfortable to its limit: 0 cool, 1 failing.
    pub fn load(&self) -> f64 {
        ((self.temperature - AMBIENT) / (self.limit - AMBIENT)).max(0.0)
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

    fn fly(&mut self, flight: &FlightNow, heat: &mut PartHeat, dt: f64) {
        // The nearer it is to as hot as the air can make it, the less the air gives it: nothing
        // slow gets hotter than its air, however long it flies.
        let air = flight.recovery_temperature;
        let drive = ((air - self.temperature) / (air - COLD_WALL).max(1.0)).clamp(0.0, 1.0);
        self.heating = heat.bare * drive;
        let reached = self.temperature + self.heating * dt / self.capacity;
        self.temperature = reached.min(air.max(self.temperature));
        self.peak = self.peak.max(self.temperature);
    }

    fn heat(&self) -> Option<(f64, f64)> {
        Some((self.temperature, self.capacity))
    }

    fn warm(&mut self, joules: f64) {
        self.temperature += joules / self.capacity;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parts::PartId, systems::Systems};

    fn lump(temperature: f64) -> Thermal {
        Thermal {
            temperature,
            peak: temperature,
            capacity: 45_000.0,
            area: 1.0,
            limit: 800.0,
            critical: false,
            heating: 0.0,
        }
    }

    #[test]
    fn slow_air_cannot_make_a_part_hotter_than_itself() {
        let mut part = lump(AMBIENT);
        let flight = FlightNow {
            recovery_temperature: 500.0,
            ..FlightNow::default()
        };
        for _ in 0..100_000 {
            let mut heat = PartHeat {
                bare: 1e6,
                ..PartHeat::default()
            };
            part.fly(&flight, &mut heat, 0.02);
        }
        assert!(part.temperature <= 500.0 + 1e-6, "{}", part.temperature);
        assert!(part.temperature > 490.0, "{}", part.temperature);
        assert!(!part.burned_through());
    }

    #[test]
    fn fast_air_heats_a_cold_wall_at_its_full_rate() {
        let mut part = lump(COLD_WALL);
        let flight = FlightNow {
            recovery_temperature: 30_000.0,
            ..FlightNow::default()
        };
        let mut heat = PartHeat {
            bare: 45_000.0,
            ..PartHeat::default()
        };
        part.fly(&flight, &mut heat, 1.0);
        assert!((part.temperature - COLD_WALL - 1.0).abs() < 1e-9);
    }

    #[test]
    fn past_its_limit_a_part_comes_off_and_only_a_critical_one_ends_the_vessel() {
        let mut part = lump(801.0);
        assert!(part.burned_through() && !part.failed());
        part.critical = true;
        assert!(part.failed() && !part.burned_through());
    }

    #[test]
    fn heat_crosses_a_joint_to_level_and_is_kept() {
        let (hot, cold) = (PartId(1), PartId(2));
        let mut big = lump(1000.0);
        big.capacity *= 3.0;
        let mut systems = Systems::default();
        systems.parts.insert(hot, vec![Box::new(big)]);
        systems.parts.insert(cold, vec![Box::new(lump(200.0))]);
        systems.joints = vec![(hot, cold)];
        // A step as long as time warp likes does not overshoot.
        systems.conduct(1e9);
        let (a, b) = (
            systems.heat_of(hot).unwrap(),
            systems.heat_of(cold).unwrap(),
        );
        assert!((a.0 - 800.0).abs() < 1e-6, "{a:?}");
        assert!((b.0 - 800.0).abs() < 1e-6, "{b:?}");
    }
}
