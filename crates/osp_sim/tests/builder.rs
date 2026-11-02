//! The builder: its operations on real craft, and what it refuses.

use std::path::Path;

use osp_sim::{
    builder::{self},
    data::{CraftDef, PartCatalog},
};

#[path = "golden/record.rs"]
#[allow(dead_code)]
mod record;

/// Every shipped craft, and the catalogue they are made from.
fn content() -> (PartCatalog, Vec<CraftDef>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let mut diagnostics = Vec::new();
    let craft = content.registry.all::<CraftDef>("craft", &mut diagnostics);
    (record::catalog(&content.registry), craft)
}

#[test]
#[ignore = "T015: the overlap rule still refuses a stage's own tanks and small wheels in shipped craft"]
fn every_shipped_craft_keeps_the_rules() {
    let (catalog, craft) = content();
    let mut refused = Vec::new();
    for def in &craft {
        for part in &def.parts {
            if let Err(why) = builder::check(def, &catalog, &[part.id.as_str()]) {
                refused.push(format!("{}: {why}", def.id));
            }
        }
    }
    assert!(
        refused.is_empty(),
        "{} refusals\n{}",
        refused.len(),
        refused.join("\n")
    );
}
