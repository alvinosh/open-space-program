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
//! - **Rotation** ([`attitude`]): torque-free spin and tumbling are exact like coasting,
//!   for any body ([`elliptic`]), torque from gimbal, reaction wheels, RCS ([`rcs`]) and
//!   the air is integrated on the tick, and SAS holds a steady attitude exactly on rails.
//! - **Aerodynamics** ([`aero`]): FAR-style, the vessel cut into sections along its axis,
//!   each pushed by the air it meets, so lift, drag, stability and damping follow the shape.
//! - **Parts** ([`parts`]): a vessel is a tree of parts joined by connectors, which splits
//!   where a decoupler lets go. Each part is a simple shape whose mass and inertia add up
//!   to the vessel's ([`mass`]).
//! - **Geometry** ([`geometry`]): one description of a part's shape (revolved walls and
//!   regions, lofts, lattices, models) that its mass, volumes, mounting surfaces and liquid
//!   levels are measured from, by integration rather than per-shape formulas.
//! - **Assembly** ([`assembly`]): parts joined node to node or on each other's surfaces,
//!   into a tree that places every part and splits where a joint opens.
//! - **Components** ([`component`]): what a part does on top of its shape (storage with
//!   walls sized for their pressure, engines, decouplers, lifting surfaces, containers,
//!   ...), and templates that make parts of catalogue records.
//! - **Craft** ([`craft`]): craft files list plain parts and the joints between them;
//!   stacks of stages are a shorthand the stage converter ([`staged`]) writes parts from.
//! - **History** ([`tech`]): parts carry the year they first flew and the tech tree node
//!   that unlocks them, so the game's date decides what is available.
//! - **Planning**: burns ([`plan`]) are predicted as impulses with the same event code, and
//!   flown as finite burns by the executor ([`autopilot`]). Alarms ([`alarm`]) say when to stop.
//! - **Saves** ([`save`]): the state of a universe to bytes and back, bit for bit, so a game
//!   carries on from a save exactly as it would have without one.

pub mod aero;
pub mod alarm;
pub mod assembly;
pub mod atmosphere;
pub mod attitude;
pub mod autopilot;
pub mod body;
pub mod builder;
pub mod career;
pub mod clouds;
pub mod comms;
pub mod collision;
pub mod component;
pub mod contact;
pub mod constants;
pub mod craft;
pub mod data;
pub mod docking;
pub mod elliptic;
pub mod environment;
pub mod events;
pub mod gear;
pub mod guidance;
pub mod heating;
pub mod geometry;
pub mod integrator;
pub mod kepler;
pub mod mass;
pub mod mission;
pub mod ocean;
pub mod orbit;
pub mod palette;
pub mod parts;
pub mod plan;
pub mod plasma;
pub mod prediction;
pub mod properties;
pub mod propulsion;
pub mod radiation;
pub mod rcs;
pub mod rendezvous;
pub mod save;
pub mod scattering;
pub mod science;
pub mod search;
pub mod shaping;
pub mod shell;
pub mod sizing;
mod skip;
pub mod staged;
pub mod staging;
pub mod structure;
pub mod system;
pub mod systems;
pub mod tech;
pub mod terrain;
pub mod time;
pub mod transfer;
pub mod tutorial;
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

    /// The base game's records of `collection`, from `text`, as the Realistic Engines and
    /// Realistic Fuels mods have them: most tests are of the game with both on.
    pub fn realistic<T: serde::de::DeserializeOwned>(text: &str, collection: &str) -> Vec<T> {
        let mut items: Vec<serde_json::Value> = records(text);
        let mut patches: Vec<osp_mods::Patch> = records(include_str!(
            "../../../mods/realistic_engines/patches/engines.json5"
        ));
        patches.extend(records::<osp_mods::Patch>(include_str!(
            "../../../mods/realistic_fuels/patches/fuels.json5"
        )));
        for patch in patches.iter().filter(|p| p.collection == collection) {
            if let Some(item) = items.iter_mut().find(|e| e["id"] == patch.target) {
                osp_mods::patch::apply_to_record(item, patch).unwrap();
            }
        }
        let typed = items.into_iter().map(serde_json::from_value);
        typed.collect::<Result<_, _>>().unwrap()
    }

    /// The stock engines with both mods on, for the tests of start-ups, throttle floors, ignitions
    /// and ullage.
    pub fn realistic_engines() -> Vec<crate::data::EngineDef> {
        realistic(
            include_str!("../../../assets/core/engines/stock.json5"),
            "engines",
        )
    }

    /// The base game's tank types, the special ones as Realistic Fuels has them.
    pub fn realistic_tank_types() -> Vec<crate::data::TankTypeDef> {
        let mut all: Vec<crate::data::TankTypeDef> =
            records(include_str!("../../../assets/core/tank_types/stock.json5"));
        let theirs: Vec<crate::data::TankTypeDef> =
            records(include_str!("../../../mods/realistic_fuels/tank_types/fuels.json5"));
        all.retain(|kind| !theirs.iter().any(|t| t.id == kind.id));
        all.extend(theirs);
        all
    }

    /// The base game's resources and the real propellants.
    pub fn realistic_resources() -> Vec<crate::data::ResourceDef> {
        let mut all: Vec<crate::data::ResourceDef> =
            records(include_str!("../../../assets/core/resources/stock.json5"));
        all.extend(records::<crate::data::ResourceDef>(include_str!(
            "../../../mods/realistic_fuels/resources/fuels.json5"
        )));
        all
    }
}
