//! A part that folds out once: it is drawn as built until it is told to unfold, then turns about its
//! origin. Only how it is drawn changes.

use std::any::Any;

use glam::{DQuat, DVec3};
use serde::{Deserialize, Serialize};

use super::{Reading, System};
use crate::{component::Component, parts::Part, staging::Action};

pub const UNFOLD: Action = Action::new("unfold", "Unfold");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Swing {
    /// In the part's own frame.
    pub axis: [f64; 3],
    /// How far it turns, degrees.
    pub angle: f64,
    /// How long that takes, s.
    pub seconds: f64,
    pub unfolding: bool,
    /// From 0, as built, to 1.
    pub done: f64,
}

impl Swing {
    /// How it is turned now, in its own frame.
    pub fn turn(&self) -> DQuat {
        let eased = self.done * self.done * (3.0 - 2.0 * self.done);
        let axis = DVec3::from(self.axis).normalize_or(DVec3::X);
        DQuat::from_axis_angle(axis, (eased * self.angle).to_radians())
    }
}

pub(super) fn of(component: &Component, _part: &Part) -> Option<Box<dyn System>> {
    let Component::Swing(def) = component else {
        return None;
    };
    Some(Box::new(Swing {
        axis: def.axis,
        angle: def.angle,
        seconds: def.seconds,
        unfolding: false,
        done: 0.0,
    }))
}

impl System for Swing {
    fn kind(&self) -> &'static str {
        "swing"
    }

    fn parameters(&self) -> Vec<Reading> {
        Vec::new()
    }

    fn state(&self) -> Vec<Reading> {
        vec![Reading {
            id: "unfolded",
            label: "Unfolded",
            value: self.done,
            unit: "",
        }]
    }

    fn actions(&self) -> Vec<Action> {
        vec![UNFOLD]
    }

    fn act(&mut self, action: Action) -> Option<String> {
        (action.kind == UNFOLD.kind && !self.unfolding).then(|| {
            self.unfolding = true;
            "Unfolding".to_owned()
        })
    }

    fn step(&mut self, dt: f64) {
        if self.unfolding {
            self.done = (self.done + dt / self.seconds.max(1e-3)).min(1.0);
        }
    }

    fn boxed(&self) -> Box<dyn System> {
        Box::new(self.clone())
    }

    fn any(&self) -> &dyn Any {
        self
    }
}
