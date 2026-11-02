//! Seats, and who sits in them.

use std::any::Any;

use super::{Reading, System};
use crate::{component::Component, parts::Part};

#[derive(Debug, Clone, PartialEq)]
pub struct Crew {
    pub seats: u32,
    /// How many are aboard: the seats are full until the crew are people of their own (T049).
    pub aboard: u32,
}

pub(super) fn of(component: &Component, _part: &Part) -> Option<Box<dyn System>> {
    let Component::Crew { seats } = component else {
        return None;
    };
    Some(Box::new(Crew {
        seats: *seats,
        aboard: *seats,
    }))
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
        vec![Reading {
            id: "aboard",
            label: "Aboard",
            value: f64::from(self.aboard),
            unit: "",
        }]
    }

    fn boxed(&self) -> Box<dyn System> {
        Box::new(self.clone())
    }

    fn any(&self) -> &dyn Any {
        self
    }
}
