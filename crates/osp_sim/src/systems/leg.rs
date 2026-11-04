//! A landing leg that folds: stowed it stands on nothing and is drawn folded against the vessel; once
//! deployed it is a leg like any other ([`crate::gear`]).

use std::any::Any;

use glam::DVec3;
use serde::{Deserialize, Serialize};

use super::{Reading, System};
use crate::{component::Component, parts::Part, staging::Action};

pub const DEPLOY_LEG: Action = Action::new("deploy_leg", "Deploy leg");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Leg {
    pub deployed: bool,
    /// Where the leg points when stowed, in its own part's frame.
    pub stowed: [f64; 3],
}

impl Leg {
    /// Where the leg points when stowed, in its own part's frame.
    pub fn stowed_direction(&self) -> DVec3 {
        DVec3::from(self.stowed)
    }
}

pub(super) fn of(component: &Component, part: &Part) -> Option<Box<dyn System>> {
    let Component::Gear(def) = component else {
        return None;
    };
    let stowed = DVec3::from(def.stowed?).normalize_or(DVec3::Z);
    Some(Box::new(Leg {
        deployed: false,
        stowed: (part.rotation.inverse() * stowed).into(),
    }))
}

impl System for Leg {
    fn kind(&self) -> &'static str {
        "leg"
    }

    fn parameters(&self) -> Vec<Reading> {
        Vec::new()
    }

    fn state(&self) -> Vec<Reading> {
        vec![Reading {
            id: "deployed",
            label: "Deployed",
            value: f64::from(u8::from(self.deployed)),
            unit: "",
        }]
    }

    fn actions(&self) -> Vec<Action> {
        vec![DEPLOY_LEG]
    }

    fn act(&mut self, action: Action) -> Option<String> {
        (action.kind == DEPLOY_LEG.kind && !self.deployed).then(|| {
            self.deployed = true;
            "Landing leg deployed".to_owned()
        })
    }

    fn boxed(&self) -> Box<dyn System> {
        Box::new(self.clone())
    }

    fn any(&self) -> &dyn Any {
        self
    }
}
