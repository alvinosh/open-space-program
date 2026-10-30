//! Records what every craft weighs and how it flies, as the golden figures that
//! `tests/craft_golden.rs` holds the game to: after liftoff and after each sequence fires,
//! the stages' masses and delta-v, the centre of mass and inertia, the outline the air
//! sees, and the air's loads in three flows (see `tests/golden/record.rs`).
//!
//! ```sh
//! cargo run -p osp_sim --example craft_golden --release > crates/osp_sim/tests/golden/craft.json
//! ```
//!
//! Rerun it only when a change is meant to move these figures, and say why in the commit.

use std::path::Path;

use osp_sim::{data::CraftDef, propulsion::Propulsion};
use serde_json::Value;

#[path = "../tests/golden/record.rs"]
mod record;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let catalog = record::catalog(registry);
    let mut craft = serde_json::Map::new();
    let mut ids: Vec<String> = registry.records("craft").map(|r| r.id.clone()).collect();
    ids.sort();
    for id in ids {
        let def: CraftDef = registry
            .get("craft", &id)
            .unwrap_or_else(|e| panic!("{id}: {e}"));
        let propulsion =
            Propulsion::from_craft(&def, &catalog).unwrap_or_else(|e| panic!("{id}: {e}"));
        craft.insert(id, record::record(propulsion, &[]));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&Value::Object(craft)).unwrap()
    );
}
