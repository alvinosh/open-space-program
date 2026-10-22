//! Engine-agnostic simulation core for Open Space Program.
//!
//! Everything in here is double precision and SI units (metres, seconds, kilograms,
//! newtons, pascals, kelvin). Nothing depends on Bevy, so the physics can be unit-tested
//! headless and reused by tools (trajectory planners, servers, ...).
//!
//! # Correctness at every scale
//!
//! - **Time** is integer microseconds ([`time::SimTime`]): no drift, exact differences.
//! - **Frames**: every state is relative to a body ("patched conics"), in inertial J2000
//!   ecliptic axes. Frame conversions go through the lowest common ancestor
//!   ([`system::SolarSystem::relative_state`]), never through heliocentric coordinates.
//! - **Coasting** is an exact conic in universal variables ([`kepler`], [`orbit`]), valid
//!   for elliptic, parabolic, hyperbolic and radial motion.
//! - **Events** (SOI changes, atmosphere entry) are solved to the exact tick, independent of
//!   step size ([`events`]), so time warp never changes the outcome.
//! - **Powered flight** uses Encke's method ([`integrator`]): precision scales with the
//!   deviation from a conic, not the distance from the origin.
//! - **Rotation** ([`attitude`]): torque-free spin is exact like coasting, torque from
//!   gimbal, reaction wheels, RCS ([`rcs`]) and the air is integrated on the tick, and SAS
//!   holds a steady attitude exactly on rails.
//! - **Aerodynamics** ([`aero`]): FAR-style, the vessel cut into sections along its axis,
//!   each pushed by the air it meets, so lift, drag, stability and damping follow the shape.
//! - **Planning**: burns ([`plan`]) are predicted as impulses with the same event code, and
//!   flown as finite burns by the executor ([`autopilot`]).

pub mod aero;
pub mod atmosphere;
pub mod attitude;
pub mod autopilot;
pub mod body;
pub mod constants;
pub mod data;
pub mod events;
pub mod gear;
pub mod integrator;
pub mod kepler;
pub mod orbit;
pub mod plan;
pub mod prediction;
pub mod propulsion;
pub mod rcs;
pub mod rendezvous;
pub mod search;
pub mod staging;
pub mod structure;
pub mod system;
pub mod terrain;
pub mod time;
pub mod transfer;
pub mod universe;
pub mod vessel;

pub use glam::{DQuat, DVec3};

#[cfg(test)]
pub(crate) mod test_data {
    /// Parse a JSON5 record file (one object or an array) into typed records.
    pub fn records<T: serde::de::DeserializeOwned>(text: &str) -> Vec<T> {
        let value: serde_json::Value = json5::from_str(text).unwrap();
        let items = match value {
            serde_json::Value::Array(items) => items,
            single => vec![single],
        };
        items
            .into_iter()
            .map(|v| serde_json::from_value(v).unwrap())
            .collect()
    }
}
