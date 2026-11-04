//! A parachute: stowed until armed, then it opens by itself when the vessel is low enough. It pulls
//! along the air at the part it hangs from ([`crate::vessel`] adds the force). Opened in too much
//! air it tears. Drogues and mains are the same part with different numbers.

use std::any::Any;

use serde::{Deserialize, Serialize};

use super::{FlightNow, PartHeat, Reading, System};
use crate::{component::Component, parts::Part, staging::Action};

pub const ARM: Action = Action::new("arm", "Arm");
pub const DEPLOY: Action = Action::new("deploy", "Deploy");
pub const CUT: Action = Action::new("cut", "Cut");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChuteState {
    Stowed,
    /// It opens when the vessel gets low enough.
    Armed,
    /// Open, or opening.
    Open,
    Cut,
    Torn,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parachute {
    /// m.
    pub diameter: f64,
    /// Drag coefficient.
    pub drag: f64,
    /// It opens below this, m.
    pub deploy_altitude: f64,
    /// It lets go below this, for the next one to take over, m; 0 to stay on.
    pub cut_altitude: f64,
    /// It tears in more dynamic pressure than this, Pa.
    pub max_pressure: f64,
    /// How long it takes to fill, s.
    pub opening_time: f64,
    pub state: ChuteState,
    /// How far open, 0..1.
    pub opened: f64,
    /// What to tell the pilot.
    #[serde(default)]
    said: Option<String>,
}

pub(super) fn of(component: &Component, _part: &Part) -> Option<Box<dyn System>> {
    let Component::Parachute(def) = component else {
        return None;
    };
    Some(Box::new(Parachute {
        diameter: def.diameter,
        drag: def.drag,
        deploy_altitude: def.deploy_altitude,
        cut_altitude: def.cut_altitude,
        max_pressure: def.max_pressure,
        opening_time: def.opening_time,
        state: ChuteState::Stowed,
        opened: 0.0,
        said: None,
    }))
}

impl Parachute {
    /// Its drag coefficient times its area as far open as it is, m².
    pub fn drag_area(&self) -> f64 {
        if self.state != ChuteState::Open {
            return 0.0;
        }
        // The area grows as the square of how far the mouth has opened.
        let radius = self.diameter / 2.0;
        self.drag * std::f64::consts::PI * radius * radius * self.opened * self.opened
    }

    /// Whether there is a canopy to see, and how big it is now, m.
    pub fn canopy(&self) -> Option<f64> {
        (self.state == ChuteState::Open && self.opened > 0.0)
            .then_some(self.diameter * self.opened.sqrt())
    }
}

impl System for Parachute {
    fn kind(&self) -> &'static str {
        "parachute"
    }

    fn parameters(&self) -> Vec<Reading> {
        vec![
            Reading {
                id: "diameter",
                label: "Diameter",
                value: self.diameter,
                unit: "m",
            },
            Reading {
                id: "deploy_altitude",
                label: "Opens below",
                value: self.deploy_altitude,
                unit: "m",
            },
        ]
    }

    fn state(&self) -> Vec<Reading> {
        vec![Reading {
            id: "opened",
            label: "Open",
            value: self.opened * 100.0,
            unit: "%",
        }]
    }

    fn fly(&mut self, flight: &FlightNow, _heat: &mut PartHeat, dt: f64) {
        match self.state {
            ChuteState::Armed
                if flight.density > 0.0 && flight.altitude <= self.deploy_altitude =>
            {
                self.state = ChuteState::Open;
                self.said = Some(format!("{:.0} m parachute opening", self.diameter));
            }
            ChuteState::Open => {
                self.opened = (self.opened + dt / self.opening_time.max(0.01)).min(1.0);
                if flight.dynamic_pressure > self.max_pressure {
                    self.state = ChuteState::Torn;
                    self.opened = 0.0;
                    self.said = Some(format!(
                        "{:.0} m parachute torn off at {:.0} kPa",
                        self.diameter,
                        flight.dynamic_pressure / 1e3
                    ));
                } else if (self.cut_altitude > 0.0 && flight.altitude <= self.cut_altitude)
                    || flight.on_the_ground
                {
                    self.state = ChuteState::Cut;
                    self.opened = 0.0;
                }
            }
            _ => {}
        }
    }

    fn announce(&mut self) -> Option<String> {
        self.said.take()
    }

    fn actions(&self) -> Vec<Action> {
        vec![ARM, DEPLOY, CUT]
    }

    fn act(&mut self, action: Action) -> Option<String> {
        match action.kind {
            "arm" if self.state == ChuteState::Stowed => {
                self.state = ChuteState::Armed;
                None
            }
            "deploy" if matches!(self.state, ChuteState::Stowed | ChuteState::Armed) => {
                self.state = ChuteState::Open;
                Some("Parachute deployed".to_owned())
            }
            "cut" if self.state == ChuteState::Open => {
                self.state = ChuteState::Cut;
                self.opened = 0.0;
                Some("Parachute cut".to_owned())
            }
            _ => None,
        }
    }

    fn boxed(&self) -> Box<dyn System> {
        Box::new(self.clone())
    }

    fn any(&self) -> &dyn Any {
        self
    }
}
