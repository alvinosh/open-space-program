//! Under Realistic, the parts list shows each engine once, however many configurations it has.

use std::{collections::BTreeSet, path::Path};

use osp_sim::palette::{self, Group};

#[path = "golden/record.rs"]
#[allow(dead_code)]
mod record;

#[test]
fn realistic_lists_one_engine_a_family() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let catalog = record::catalog(&content.registry);
    // The engine records' entries: not the solid booster, which is a part of its own.
    let engines: Vec<_> = (palette::entries(&catalog).into_iter())
        .filter(|e| e.group == Group::Engines && e.id.starts_with("engine:"))
        .collect();
    let families: BTreeSet<&str> = (catalog.engines.iter())
        .filter_map(|e| catalog.family_of(&e.id).map(|f| f.id.as_str()))
        .collect();
    let alone = (catalog.engines.iter())
        .filter(|e| catalog.family_of(&e.id).is_none())
        .count();
    println!(
        "{} engine records: {} families and {} engines of none, {} in the parts list",
        catalog.engines.len(),
        families.len(),
        alone,
        engines.len()
    );
    assert_eq!(engines.len(), families.len() + alone);
    assert!(engines.len() * 2 < catalog.engines.len());
    // Each family once.
    let listed: BTreeSet<Option<&str>> = (engines.iter())
        .map(|e| {
            e.part
                .template
                .as_deref()
                .and_then(|t| catalog.family_of(t))
                .map(|f| f.id.as_str())
        })
        .filter(Option::is_some)
        .collect();
    assert_eq!(listed.len(), families.len());
}
