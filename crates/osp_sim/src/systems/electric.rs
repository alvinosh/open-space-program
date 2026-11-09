//! What makes and uses electricity, besides a battery: fuel cells, solar arrays, RTGs and loads.
//! Sized from what they must do by [`crate::sizing`]; the network is [`super::Systems`].

use std::any::Any;

use glam::DVec3;
use serde::{Deserialize, Serialize};

use super::{Power, Reading, System};
use crate::{
    comms::{Radio, S_BAND, dish_gain},
    component::{AvionicsUnitDef, Component, ElectricLoadDef, FuelCellDef, RtgDef, SolarArrayDef},
    environment::Environment,
    parts::Part,
    staging::Action,
};

/// Hydrogen burned per kJ, kg: 0.042 kg per kWh at the 0.9 V a cell gives, with the oxygen eight
/// times its mass, as water's chemistry has it.
const HYDROGEN_PER_KJ: f64 = 0.042 / 3600.0;
const OXYGEN_PER_KJ: f64 = 8.0 * HYDROGEN_PER_KJ;
/// Seconds in a year.
const YEAR: f64 = 365.25 * 86400.0;

macro_rules! boxed {
    () => {
        fn boxed(&self) -> Box<dyn System> {
            Box::new(self.clone())
        }

        fn any(&self) -> &dyn Any {
            self
        }
    };
}

fn reading(id: &'static str, label: &'static str, value: f64, unit: &'static str) -> Reading {
    Reading {
        id,
        label,
        value,
        unit,
    }
}

/// Burns hydrogen and oxygen for as much electricity as the network asks, up to what it is rated
/// for. It makes water, which the crew drink (T049).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FuelCell {
    /// W.
    pub rated: f64,
    /// What it made last, W.
    pub output: f64,
    /// Burned so far, kg.
    pub hydrogen: f64,
    pub oxygen: f64,
    /// Its tanks ran out.
    pub starved: bool,
    /// What it burned since [`System::take_consumption`] last asked, kg.
    owed: (f64, f64),
}

impl FuelCell {
    pub fn new(def: &FuelCellDef) -> Self {
        Self {
            rated: def.power,
            output: 0.0,
            hydrogen: 0.0,
            oxygen: 0.0,
            starved: false,
            owed: (0.0, 0.0),
        }
    }
}

impl System for FuelCell {
    fn kind(&self) -> &'static str {
        "fuel_cell"
    }

    fn parameters(&self) -> Vec<Reading> {
        vec![reading("rated", "Rated", self.rated, "W")]
    }

    fn state(&self) -> Vec<Reading> {
        vec![
            reading("output", "Output", self.output, "W"),
            reading("hydrogen", "Hydrogen burned", self.hydrogen, "kg"),
            reading("oxygen", "Oxygen burned", self.oxygen, "kg"),
        ]
    }

    fn power(&self, _environment: &Environment) -> Option<Power> {
        Some(Power::Dispatchable(if self.starved {
            0.0
        } else {
            self.rated
        }))
    }

    fn deliver(&mut self, kj: f64, dt: f64) {
        self.output = if dt > 0.0 { kj * 1e3 / dt } else { 0.0 };
        let (hydrogen, oxygen) = (kj * HYDROGEN_PER_KJ, kj * OXYGEN_PER_KJ);
        self.hydrogen += hydrogen;
        self.oxygen += oxygen;
        self.owed.0 += hydrogen;
        self.owed.1 += oxygen;
    }

    fn consumes(&self) -> bool {
        true
    }

    fn take_consumption(&mut self) -> Vec<(&'static str, f64)> {
        let (hydrogen, oxygen) = std::mem::take(&mut self.owed);
        vec![("LqdHydrogen", hydrogen), ("LqdOxygen", oxygen)]
    }

    fn consuming(&self) -> Vec<(&'static str, f64)> {
        vec![
            ("LqdHydrogen", self.output / 1e3 * HYDROGEN_PER_KJ),
            ("LqdOxygen", self.output / 1e3 * OXYGEN_PER_KJ),
        ]
    }

    fn fed(&mut self, fed: bool) {
        self.starved = !fed;
        if !fed {
            self.output = 0.0;
        }
    }

    boxed!();
}

/// Makes what the sunlight on it gives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolarArray {
    /// m².
    pub area: f64,
    /// The share of the sunlight it turns into electricity, new.
    pub efficiency: f64,
    /// It turns to face the Sun; or else it lies square to `normal`.
    pub tracking: bool,
    /// Its face, in the vessel's axes.
    pub normal: DVec3,
    /// What it loses a year, as a share.
    pub degradation: f64,
    /// How long it has flown, s.
    pub age: f64,
    /// What it made last, W.
    pub output: f64,
}

impl SolarArray {
    fn make(&self, environment: &Environment) -> f64 {
        let facing = if self.tracking {
            1.0
        } else {
            self.normal.dot(environment.sun_in_vessel).max(0.0)
        };
        let worn = (1.0 - self.degradation).powf(self.age / YEAR);
        environment.solar_flux * self.area * self.efficiency * worn * facing
    }
}

impl System for SolarArray {
    fn kind(&self) -> &'static str {
        "solar_array"
    }

    fn parameters(&self) -> Vec<Reading> {
        vec![
            reading("area", "Area", self.area, "m²"),
            reading("efficiency", "Efficiency", self.efficiency * 100.0, "%"),
        ]
    }

    fn state(&self) -> Vec<Reading> {
        vec![reading("output", "Output", self.output, "W")]
    }

    fn power(&self, environment: &Environment) -> Option<Power> {
        Some(Power::Source(self.make(environment)))
    }

    fn step(&mut self, dt: f64) {
        self.age += dt;
    }

    fn uses_light(&self) -> bool {
        true
    }

    fn deliver(&mut self, kj: f64, dt: f64) {
        self.output = if dt > 0.0 { kj * 1e3 / dt } else { 0.0 };
    }

    boxed!();
}

/// Plutonium's heat turned into electricity, falling away as it decays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rtg {
    /// New, W.
    pub power: f64,
    /// Years.
    pub half_life: f64,
    /// How long since it was new, s.
    pub age: f64,
}

impl Rtg {
    pub fn output(&self) -> f64 {
        self.power * 0.5_f64.powf(self.age / YEAR / self.half_life)
    }
}

impl System for Rtg {
    fn kind(&self) -> &'static str {
        "rtg"
    }

    fn parameters(&self) -> Vec<Reading> {
        vec![reading("power", "New", self.power, "W")]
    }

    fn state(&self) -> Vec<Reading> {
        vec![reading("output", "Output", self.output(), "W")]
    }

    fn power(&self, _environment: &Environment) -> Option<Power> {
        Some(Power::Source(self.output()))
    }

    fn step(&mut self, dt: f64) {
        self.age += dt;
    }

    boxed!();
}

/// Uses electricity: avionics, a transmitter, a cabin, heaters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Load {
    #[serde(deserialize_with = "crate::save::interned")]
    pub label: crate::save::Kept,
    /// W.
    pub watts: f64,
    /// The lowest on a network is shed first.
    pub priority: u8,
    /// Without it the vessel has no control and lights no engine.
    pub essential: bool,
    /// It had power on the last step.
    pub on: bool,
}

impl System for Load {
    fn kind(&self) -> &'static str {
        "load"
    }

    fn parameters(&self) -> Vec<Reading> {
        vec![reading("watts", "Draws", self.watts, "W")]
    }

    fn state(&self) -> Vec<Reading> {
        vec![reading(
            "on",
            "Powered",
            if self.on { 1.0 } else { 0.0 },
            "",
        )]
    }

    fn power(&self, _environment: &Environment) -> Option<Power> {
        Some(Power::Load {
            watts: self.watts,
            priority: self.priority,
        })
    }

    fn powered(&mut self, on: bool) {
        self.on = on;
    }

    fn browned_out(&self) -> bool {
        self.essential && !self.on
    }

    fn shed(&self) -> bool {
        !self.on
    }

    boxed!();
}

/// Guidance and control electronics: with power, it controls a vessel of up to so much mass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Avionics {
    /// kg.
    pub controlled_mass: f64,
    /// W.
    pub watts: f64,
    pub interplanetary: bool,
    /// A probe core: in a Realistic game it answers only over a link.
    pub needs_link: bool,
    /// It had power on the last step.
    pub on: bool,
}

impl System for Avionics {
    fn kind(&self) -> &'static str {
        "avionics"
    }

    fn parameters(&self) -> Vec<Reading> {
        vec![
            reading(
                "controlled_mass",
                "Controls up to",
                self.controlled_mass,
                "kg",
            ),
            reading("watts", "Draws", self.watts, "W"),
        ]
    }

    fn state(&self) -> Vec<Reading> {
        vec![reading(
            "on",
            "Powered",
            if self.on { 1.0 } else { 0.0 },
            "",
        )]
    }

    fn power(&self, _environment: &Environment) -> Option<Power> {
        (self.watts > 0.0).then_some(Power::Load {
            watts: self.watts,
            priority: 8,
        })
    }

    fn powered(&mut self, on: bool) {
        self.on = on;
    }

    fn shed(&self) -> bool {
        !self.on
    }

    boxed!();
}

/// Turn an antenna off, so it draws nothing and gives no link, and back on.
pub const ANTENNA_OFF: Action = Action::new("antenna_off", "Shut down");
pub const ANTENNA_ON: Action = Action::new("antenna_on", "Turn on");

/// What a dish draws to hold it on the ground, for each metre across, W.
const DISH_DRAW_PER_M: f64 = 4.0;

/// What talks to the ground. An antenna draws about four times what it sends, and a dish more the
/// bigger it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Antenna {
    pub radio: Radio,
    /// It had power on the last step.
    pub on: bool,
    /// It passes on other vessels' signals.
    #[serde(default)]
    pub relay: bool,
    /// The player has shut it down.
    #[serde(default)]
    pub shut: bool,
    /// What it draws besides sending, W.
    #[serde(default)]
    pub holding: f64,
}

impl Antenna {
    /// What it draws while on, W.
    pub fn watts(&self) -> f64 {
        4.0 * self.radio.power + self.holding
    }
}

impl System for Antenna {
    fn kind(&self) -> &'static str {
        "antenna"
    }

    fn parameters(&self) -> Vec<Reading> {
        vec![
            reading("power", "Sends", self.radio.power, "W"),
            reading("draws", "Draws", self.watts(), "W"),
            reading("rate", "Data rate", self.radio.data_rate, "bit/s"),
        ]
    }

    fn state(&self) -> Vec<Reading> {
        vec![reading(
            "on",
            "Powered",
            if self.on { 1.0 } else { 0.0 },
            "",
        )]
    }

    fn power(&self, _environment: &Environment) -> Option<Power> {
        (!self.shut).then(|| Power::Load {
            watts: self.watts(),
            priority: 6,
        })
    }

    fn powered(&mut self, on: bool) {
        self.on = on && !self.shut;
    }

    fn shed(&self) -> bool {
        !self.on && !self.shut
    }

    fn actions(&self) -> Vec<Action> {
        vec![if self.shut { ANTENNA_ON } else { ANTENNA_OFF }]
    }

    fn act(&mut self, action: Action) -> Option<String> {
        if action.kind == ANTENNA_OFF.kind && !self.shut {
            self.shut = true;
            self.on = false;
            Some("Antenna shut down".to_owned())
        } else if action.kind == ANTENNA_ON.kind && self.shut {
            self.shut = false;
            Some("Antenna on".to_owned())
        } else {
            None
        }
    }

    boxed!();
}

pub(super) fn antenna(component: &Component, _part: &Part) -> Option<Box<dyn System>> {
    let Component::Antenna(def) = component else {
        return None;
    };
    let gain = if def.diameter > 0.0 {
        dish_gain(def.diameter, S_BAND)
    } else {
        1.0
    };
    Some(Box::new(Antenna {
        radio: Radio {
            power: def.power,
            gain,
            data_rate: def.data_rate,
        },
        on: true,
        relay: def.relay,
        shut: false,
        holding: def.diameter * DISH_DRAW_PER_M,
    }))
}

pub(super) fn avionics(component: &Component, _part: &Part) -> Option<Box<dyn System>> {
    let Component::Avionics(AvionicsUnitDef {
        controlled_mass,
        power,
        interplanetary,
        needs_link,
    }) = component
    else {
        return None;
    };
    Some(Box::new(Avionics {
        controlled_mass: *controlled_mass,
        watts: *power,
        interplanetary: *interplanetary,
        needs_link: *needs_link,
        on: true,
    }))
}

pub(super) fn fuel_cell(component: &Component, _part: &Part) -> Option<Box<dyn System>> {
    let Component::FuelCell(def) = component else {
        return None;
    };
    Some(Box::new(FuelCell::new(def)))
}

pub(super) fn solar_array(component: &Component, part: &Part) -> Option<Box<dyn System>> {
    let Component::SolarArray(SolarArrayDef {
        area,
        efficiency,
        tracking,
        degradation,
    }) = component
    else {
        return None;
    };
    Some(Box::new(SolarArray {
        area: *area,
        efficiency: *efficiency,
        tracking: *tracking,
        normal: part.rotation * DVec3::X,
        degradation: *degradation,
        age: 0.0,
        output: 0.0,
    }))
}

pub(super) fn rtg(component: &Component, _part: &Part) -> Option<Box<dyn System>> {
    let Component::Rtg(RtgDef { power, half_life }) = component else {
        return None;
    };
    Some(Box::new(Rtg {
        power: *power,
        half_life: *half_life,
        age: 0.0,
    }))
}

pub(super) fn load(component: &Component, part: &Part) -> Option<Box<dyn System>> {
    let Component::ElectricLoad(ElectricLoadDef {
        watts,
        priority,
        essential,
    }) = component
    else {
        return None;
    };
    // A name the window and the log can keep: the part's, for as long as the game runs.
    let label: &'static str = Box::leak(part.name.clone().into_boxed_str());
    Some(Box::new(Load {
        label,
        watts: *watts,
        priority: *priority,
        essential: *essential,
        on: true,
    }))
}
