//! A cabin's own stores: its air and its lockers, which the vessel's account of its tanks
//! ([`crate::propulsion::Propulsion::step_systems`]) reaches for anything on the same part, before
//! its tanks.
//!
//! Its air is gases at partial pressures, from what there is of each in its volume: under the
//! Realistic rules oxygen, nitrogen and what people breathe out. A regulator tops each gas up to
//! the pressure the rules keep it at, from the tanks, no faster than it fills an empty cabin. Air
//! goes out through a hole, or the valve the cabin is let down through, at a share of what there is
//! a second (choked flow to vacuum), which the account of the tanks works out exactly with
//! everything else going in and out. Time warp stops where the air crosses a limit people live by.
//!
//! What the lockers carry is in the part's mass as built, and does not come off as it is used.

use std::any::Any;

use serde::{Deserialize, Serialize};

use super::{Air, CREW_PRIORITY, Exchange, Reading, System};
use crate::{component::Component, data::AirLimitDef, parts::Part, staging::Action};

pub const LET_DOWN: Action = Action::new("cabin_let_down", "Let the cabin down");
pub const FILL: Action = Action::new("cabin_fill", "Fill the cabin");

/// The gas constant, J/(mol K).
const R: f64 = 8.314_462_618;
/// What flows out of a hole is near enough air: its heat capacity ratio and molar mass, kg/mol.
const GAMMA: f64 = 1.4;
const AIR_MOLAR_MASS: f64 = 0.028_97;
/// A sharp-edged hole lets out this share of what an ideal nozzle of its size would.
const DISCHARGE: f64 = 0.8;
/// A step meant to end where the air crosses a limit ends this much after it, s, so that the
/// limit is crossed by the end of it, however the sums round.
const PAST: f64 = 1e-6;

/// One thing a cabin keeps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stowed {
    pub resource: String,
    /// kg.
    pub held: f64,
    pub capacity: f64,
}

/// One gas of a cabin's air, beside its place in `kept`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gas {
    pub name: String,
    /// kg/mol.
    pub molar_mass: f64,
    /// What its regulator tops it up from.
    pub from: Option<String>,
    /// The most its regulator gives, kg/s.
    pub flow: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cabin {
    /// Its air first, then its lockers, full.
    pub kept: Vec<Stowed>,
    /// How many of `kept` are air.
    pub air: usize,
    /// Each gas of its air. A save from before cabins had air of their own has none, and its air
    /// is only what is breathed out, up to what it can take.
    #[serde(default)]
    pub gases: Vec<Gas>,
    /// m³.
    #[serde(default)]
    pub volume: f64,
    /// Of its air, K.
    #[serde(default)]
    pub temperature: f64,
    /// What its air has to be for people to be well.
    #[serde(default)]
    pub limits: Vec<AirLimitDef>,
    /// The hole its air leaks through, m².
    #[serde(default)]
    pub hole: f64,
    /// Its valve, m², open while it is let down.
    #[serde(default)]
    pub vent: f64,
    #[serde(default)]
    pub venting: bool,
    /// The regulator tops its air up.
    #[serde(default = "yes")]
    pub regulating: bool,
    /// The share of full flow each gas the regulator tops up got over the last step.
    #[serde(default)]
    running: Vec<f64>,
    #[serde(default)]
    said: Option<String>,
}

fn yes() -> bool {
    true
}

pub(super) fn of(component: &Component, _part: &Part) -> Option<Box<dyn System>> {
    let Component::LifeSupport(life_support) = component else {
        return None;
    };
    if life_support.cabin.is_none() && life_support.lockers.is_empty() {
        return None;
    }
    let mut cabin = Cabin {
        kept: Vec::new(),
        air: 0,
        gases: Vec::new(),
        volume: 0.0,
        temperature: 0.0,
        limits: Vec::new(),
        hole: 0.0,
        vent: 0.0,
        venting: false,
        regulating: true,
        running: Vec::new(),
        said: None,
    };
    if let Some(built) = &life_support.cabin {
        let air = &built.air;
        (cabin.volume, cabin.temperature) = (built.volume, air.temperature);
        (cabin.vent, cabin.limits) = (air.vent, built.limits.clone());
        for gas in &air.gases {
            // It starts at the pressure the regulator keeps.
            let kg = |pa: f64| pa * built.volume * gas.molar_mass / (R * air.temperature);
            cabin.kept.push(Stowed {
                resource: gas.resource.clone(),
                held: kg(gas.keep),
                capacity: kg(gas.most()),
            });
            cabin.gases.push(Gas {
                name: gas.called().to_owned(),
                molar_mass: gas.molar_mass,
                from: gas.from.clone(),
                flow: kg(gas.keep) / air.refill,
            });
        }
        cabin.air = air.gases.len();
    }
    cabin
        .kept
        .extend(life_support.lockers.iter().map(|(resource, kg)| Stowed {
            resource: resource.clone(),
            held: *kg,
            capacity: *kg,
        }));
    Some(Box::new(cabin))
}

impl Cabin {
    /// What its air holds of `resource`, and can take, kg.
    pub fn air_of(&self, resource: &str) -> Option<(f64, f64)> {
        (self.kept[..self.air].iter())
            .find(|k| k.resource == resource)
            .map(|k| (k.held, k.capacity))
    }

    /// What one kg of each gas adds to the pressure, Pa.
    fn pascals_per_kg(&self) -> Vec<f64> {
        (self.gases.iter())
            .map(|gas| R * self.temperature / (gas.molar_mass * self.volume))
            .collect()
    }

    /// Each gas's resource and partial pressure, Pa.
    pub fn partial(&self) -> Vec<(String, f64)> {
        (self.kept.iter().zip(self.pascals_per_kg()))
            .map(|(kept, per_kg)| (kept.resource.clone(), kept.held * per_kg))
            .collect()
    }

    /// Pa.
    pub fn pressure(&self) -> f64 {
        self.partial().iter().map(|(_, p)| p).sum()
    }

    /// A partial pressure, Pa, by what people call the gas.
    pub fn partial_of(&self, name: &str) -> Option<f64> {
        let k = self.gases.iter().position(|gas| gas.name == name)?;
        Some(self.partial()[k].1)
    }

    /// The share of its air that goes out a second, 1/s: choked flow through the hole and an open
    /// valve, to vacuum.
    pub fn leak_rate(&self) -> f64 {
        let area = self.hole + if self.venting { self.vent } else { 0.0 };
        if area <= 0.0 || self.volume <= 0.0 {
            return 0.0;
        }
        let choked = (2.0 / (GAMMA + 1.0)).powf((GAMMA + 1.0) / (2.0 * (GAMMA - 1.0)));
        let sound = (GAMMA * R * self.temperature / AIR_MOLAR_MASS).sqrt();
        DISCHARGE * choked * area * sound / self.volume
    }

    /// The gases the regulator tops up, by their place in `kept`.
    fn regulated(&self) -> impl Iterator<Item = (usize, &Gas, &str)> {
        (self.gases.iter().enumerate())
            .filter_map(|(k, gas)| Some((k, gas, gas.from.as_deref()?)))
            .filter(|(_, gas, _)| gas.flow > 0.0)
    }

    fn say(&mut self, news: String) {
        self.said = Some(match self.said.take() {
            Some(before) => format!("{before}. {news}"),
            None => news,
        });
    }

    /// The regulator's lots over `dt`, at `shares` of full flow.
    fn regulator(&self, dt: f64, shares: Option<&[f64]>) -> Vec<Exchange> {
        if !self.regulating || self.venting {
            return Vec::new();
        }
        (self.regulated().enumerate())
            .map(|(n, (k, gas, from))| {
                let kg = gas.flow * dt * shares.map_or(1.0, |s| s.get(n).copied().unwrap_or(0.0));
                Exchange {
                    inputs: vec![(from.to_owned(), kg)],
                    outputs: vec![(self.kept[k].resource.clone(), kg)],
                    dump: Vec::new(),
                    priority: CREW_PRIORITY,
                }
            })
            .collect()
    }
}

/// When a value, now `now`, that goes up by `rate` a second and down by `k` of itself, crosses
/// `line`, s; a little after, so that it has.
fn crossing(now: f64, rate: f64, k: f64, line: f64) -> Option<f64> {
    let gap = line - now;
    let t = if k > 0.0 {
        // It heads for `rate / k`, by e^-kt.
        let target = rate / k;
        if gap == 0.0 {
            return (target != now).then_some(PAST);
        }
        if gap.signum() != (target - line).signum() || target == line {
            return None;
        }
        ((now - target) / (line - target)).ln() / k
    } else if gap == 0.0 {
        return (rate != 0.0).then_some(PAST);
    } else {
        gap / rate
    };
    (t.is_finite() && t >= 0.0).then_some(t + PAST)
}

/// What a reading of a gas is called, by what people call it.
fn label(name: &str) -> (&'static str, &'static str) {
    match name.to_lowercase().as_str() {
        "oxygen" => ("oxygen", "Oxygen"),
        "nitrogen" => ("nitrogen", "Nitrogen"),
        "co2" | "carbon dioxide" => ("co2", "CO2"),
        _ => ("gas", "Other gas"),
    }
}

impl System for Cabin {
    fn kind(&self) -> &'static str {
        "cabin"
    }

    fn parameters(&self) -> Vec<Reading> {
        let mut readings = vec![Reading {
            id: "lockers",
            label: "Lockers hold, full",
            value: self.kept[self.air..].iter().map(|k| k.capacity).sum(),
            unit: "kg",
        }];
        if !self.gases.is_empty() {
            readings.push(Reading {
                id: "volume",
                label: "Cabin",
                value: self.volume,
                unit: "m³",
            });
        }
        readings
    }

    fn state(&self) -> Vec<Reading> {
        let mut readings = vec![Reading {
            id: "lockers_held",
            label: "Lockers hold",
            value: self.kept[self.air..].iter().map(|k| k.held).sum(),
            unit: "kg",
        }];
        if self.gases.is_empty() {
            let (foul, most) = (self.kept[..self.air].iter())
                .fold((0.0, 0.0), |(held, most), k| {
                    (held + k.held, most + k.capacity)
                });
            readings.push(Reading {
                id: "air_fouled",
                label: "Air fouled",
                value: if most > 0.0 { 100.0 * foul / most } else { 0.0 },
                unit: "%",
            });
            return readings;
        }
        readings.push(Reading {
            id: "pressure",
            label: "Pressure",
            value: self.pressure() / 1e3,
            unit: "kPa",
        });
        for (gas, (_, pa)) in self.gases.iter().zip(self.partial()) {
            let (id, label) = label(&gas.name);
            readings.push(Reading {
                id,
                label,
                value: pa / 1e3,
                unit: "kPa",
            });
        }
        if self.hole > 0.0 {
            readings.push(Reading {
                id: "hole",
                label: "Leaking through",
                value: self.hole * 1e4,
                unit: "cm²",
            });
        }
        readings
    }

    fn consumes(&self) -> bool {
        self.regulated().next().is_some()
    }

    fn wants(&self, dt: f64) -> Vec<Exchange> {
        self.regulator(dt, None)
    }

    fn exchanged(&mut self, shares: &[f64], _dt: f64) {
        self.running = shares.to_vec();
    }

    fn exchanging(&self) -> Vec<Exchange> {
        self.regulator(1.0, Some(&self.running))
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

    fn decay(&self) -> Vec<f64> {
        let k = if self.gases.is_empty() {
            0.0
        } else {
            self.leak_rate()
        };
        (0..self.kept.len())
            .map(|i| if i < self.air { k } else { 0.0 })
            .collect()
    }

    fn next_event_at(&self, inflows: &[f64]) -> Option<f64> {
        if self.gases.is_empty() {
            return None;
        }
        let k = self.leak_rate();
        let per_kg = self.pascals_per_kg();
        let mut soonest: Option<f64> = None;
        for limit in &self.limits {
            // The limit's pressure now, and what goes into it a second besides the leak.
            let (mut now, mut rate) = (0.0, 0.0);
            for (i, (kept, per_kg)) in self.kept[..self.air].iter().zip(&per_kg).enumerate() {
                if limit.gas.as_ref().is_none_or(|g| *g == kept.resource) {
                    now += per_kg * kept.held;
                    rate += per_kg * inflows.get(i).copied().unwrap_or(0.0);
                }
            }
            for line in [limit.least, limit.most].into_iter().flatten() {
                if let Some(t) = crossing(now, rate, k, line) {
                    soonest = Some(soonest.map_or(t, |s| s.min(t)));
                }
            }
        }
        soonest
    }

    fn air(&self) -> Option<Air> {
        (!self.gases.is_empty()).then(|| Air {
            pressure: self.pressure(),
            partial: self.partial(),
        })
    }

    fn breach(&mut self, area: f64) {
        if area > 0.0 && !self.gases.is_empty() {
            self.hole += area;
            self.say("The cabin is leaking".to_owned());
        }
    }

    fn actions(&self) -> Vec<Action> {
        match (self.gases.is_empty(), self.venting) {
            (true, _) => Vec::new(),
            (false, true) => vec![FILL],
            (false, false) => vec![LET_DOWN],
        }
    }

    fn act(&mut self, action: Action) -> Option<String> {
        if action.kind == LET_DOWN.kind && !self.venting && !self.gases.is_empty() {
            (self.venting, self.regulating) = (true, false);
            Some("Letting the cabin down".to_owned())
        } else if action.kind == FILL.kind && self.venting {
            (self.venting, self.regulating) = (false, true);
            Some("Filling the cabin".to_owned())
        } else {
            None
        }
    }

    fn announce(&mut self) -> Option<String> {
        self.said.take()
    }

    fn boxed(&self) -> Box<dyn System> {
        Box::new(self.clone())
    }

    fn any(&self) -> &dyn Any {
        self
    }
}
