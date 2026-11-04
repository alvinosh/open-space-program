//! Seats, and who sits in them. With a `life_support` on the part they breathe, drink and eat from
//! the tanks it can reach, and are lost if those run out. Any crew can be lost to acceleration.

use std::any::Any;

use serde::{Deserialize, Serialize};

use super::{FlightNow, PartHeat, Reading, System};
use crate::{component::Component, parts::Part, staging::Action};

pub const BOARD: Action = Action::new("board", "Move someone here");

/// Standard gravity, m/s².
const G: f64 = 9.80665;
/// Below this nobody comes to harm however long it lasts, in g.
const G_SAFE: f64 = 5.0;
/// How long a person lasts at `G_REFERENCE`, s; the time goes as the inverse fourth power of g.
const G_REFERENCE: f64 = 15.0;
const G_ENDURANCE: f64 = 30.0;
/// How long people last with nothing to breathe, s.
const AIRLESS: f64 = 300.0;
/// A day, s.
const DAY: f64 = 86_400.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Crew {
    pub seats: u32,
    /// How many are aboard, alive.
    pub aboard: u32,
    /// How many have died.
    #[serde(default)]
    pub lost: u32,
    /// What each uses a day, kg, if the part has life support.
    #[serde(default)]
    pub daily: Option<[f64; 3]>,
    /// Burned and not yet taken out of the tanks, kg.
    #[serde(default)]
    pending: [f64; 3],
    /// How long since the tanks last gave what was needed, s.
    #[serde(default)]
    short_for: f64,
    /// How much of what a body can take of acceleration it has taken, 0..1.
    #[serde(default)]
    strain: f64,
    /// The most acceleration they have felt, g.
    #[serde(default)]
    pub peak_g: f64,
    #[serde(default)]
    said: Option<String>,
}

const RESOURCES: [&str; 3] = ["LqdOxygen", "Water", "Food"];

pub(super) fn of(component: &Component, part: &Part) -> Option<Box<dyn System>> {
    let Component::Crew { seats, aboard } = component else {
        return None;
    };
    let daily = (part.built.iter().flat_map(|built| &built.components)).find_map(|c| match c {
        Component::LifeSupport(def) => Some([def.oxygen, def.water, def.food]),
        _ => None,
    });
    Some(Box::new(Crew {
        seats: *seats,
        aboard: aboard.unwrap_or(*seats).min(*seats),
        lost: 0,
        daily,
        pending: [0.0; 3],
        short_for: 0.0,
        strain: 0.0,
        peak_g: 0.0,
        said: None,
    }))
}

impl Crew {
    fn lose_everyone(&mut self, why: &str) {
        if self.aboard > 0 {
            self.said = Some(format!("Crew lost: {why}"));
            self.lost += self.aboard;
            self.aboard = 0;
        }
    }

    fn rates(&self) -> Option<[f64; 3]> {
        self.daily
            .filter(|_| self.aboard > 0)
            .map(|daily| daily.map(|kg| kg * f64::from(self.aboard) / DAY))
    }
}

impl System for Crew {
    fn kind(&self) -> &'static str {
        "crew"
    }

    fn parameters(&self) -> Vec<Reading> {
        vec![Reading {
            id: "seats",
            label: "Seats",
            value: f64::from(self.seats),
            unit: "",
        }]
    }

    fn state(&self) -> Vec<Reading> {
        vec![
            Reading {
                id: "aboard",
                label: "Aboard",
                value: f64::from(self.aboard),
                unit: "",
            },
            Reading {
                id: "lost",
                label: "Lost",
                value: f64::from(self.lost),
                unit: "",
            },
        ]
    }

    fn step(&mut self, dt: f64) {
        if let Some(rates) = self.rates() {
            for (pending, rate) in self.pending.iter_mut().zip(rates) {
                *pending += rate * dt;
            }
        }
    }

    fn consumes(&self) -> bool {
        self.rates().is_some()
    }

    fn take_consumption(&mut self) -> Vec<(&'static str, f64)> {
        let pending = std::mem::take(&mut self.pending);
        RESOURCES.into_iter().zip(pending).collect()
    }

    fn consuming(&self) -> Vec<(&'static str, f64)> {
        self.rates().map_or(Vec::new(), |rates| {
            RESOURCES.into_iter().zip(rates).collect()
        })
    }

    fn can_go_without(&self, resource: &str) -> bool {
        // Days, against the minutes they last with nothing to breathe.
        resource != RESOURCES[0]
    }

    fn fed_for(&mut self, fed: bool, dt: f64) {
        if fed {
            self.short_for = 0.0;
        } else {
            self.short_for += dt;
            if self.short_for > AIRLESS {
                self.lose_everyone("life support ran out");
            }
        }
    }

    fn fly(&mut self, flight: &FlightNow, _heat: &mut PartHeat, dt: f64) {
        let g = flight.acceleration / G;
        self.peak_g = self.peak_g.max(g);
        if g > G_SAFE {
            let lasts = G_ENDURANCE * (G_REFERENCE / g).powi(4);
            self.strain += dt / lasts;
            if self.strain >= 1.0 {
                self.lose_everyone(&format!("{g:.0} g was too much"));
            }
        } else {
            self.strain = (self.strain - dt / G_ENDURANCE).max(0.0);
        }
    }

    fn announce(&mut self) -> Option<String> {
        self.said.take()
    }

    fn actions(&self) -> Vec<Action> {
        vec![BOARD]
    }

    fn take_person(&mut self) -> bool {
        let can = self.aboard > 0;
        if can {
            self.aboard -= 1;
        }
        can
    }

    fn seat_person(&mut self) -> bool {
        let can = self.aboard < self.seats;
        if can {
            self.aboard += 1;
        }
        can
    }

    fn boxed(&self) -> Box<dyn System> {
        Box::new(self.clone())
    }

    fn any(&self) -> &dyn Any {
        self
    }
}
