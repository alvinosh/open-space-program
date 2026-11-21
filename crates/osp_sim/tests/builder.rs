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

#[test]
fn every_size_of_every_shipped_part_can_be_changed() {
    use osp_sim::{craft::Craft, shaping};
    let (catalog, craft) = content();
    let mut failed = Vec::new();
    let mut changed = 0;
    for def in &craft {
        for (index, part) in def.parts.iter().enumerate() {
            for size in shaping::part_sizes(part) {
                for scale in [0.9, 1.1] {
                    let mut copy = def.clone();
                    let value = size.value * scale + if size.value == 0.0 { 0.1 } else { 0.0 };
                    let built = shaping::resize(&mut copy.parts[index], size.name, value)
                        .and_then(|()| Craft::build(&copy, &catalog).map_err(|e| e.to_string()));
                    match built {
                        Ok(_) => changed += 1,
                        Err(why) => {
                            failed.push(format!("{} {} {}: {why}", def.id, part.id, size.name))
                        }
                    }
                }
            }
        }
    }
    assert!(changed > 2000, "only {changed} sizes changed");
    assert!(
        failed.is_empty(),
        "{} failed\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// The shipped craft called `id`.
fn shipped(id: &str) -> (PartCatalog, CraftDef) {
    let (catalog, craft) = content();
    let def = craft.into_iter().find(|c| c.id == id).unwrap();
    (catalog, def)
}

#[test]
fn a_resize_says_what_it_changed_and_is_undone_and_done_again() {
    use osp_sim::{builder::Builder, shaping};
    let (catalog, def) = shipped("saturn_v");
    let mut builder = Builder::of(def.clone());
    let length = |b: &Builder| {
        let part = b.draft().parts.iter().find(|p| p.id == "s_ic").unwrap();
        shaping::part_sizes(part)[0].value
    };
    let before = length(&builder);
    let change = builder
        .resize(&catalog, "s_ic", &[("length", before + 2.0)])
        .unwrap();
    assert_eq!(change.changed, ["s_ic"]);
    assert!(change.added.is_empty() && change.removed.is_empty());
    assert!((length(&builder) - before - 2.0).abs() < 1e-9);
    assert!(builder.undo());
    assert!(builder.draft().parts == def.parts);
    assert!(builder.redo());
    assert!((length(&builder) - before - 2.0).abs() < 1e-9);
    // Setting what it already is changes nothing, and leaves nothing to undo.
    let mut same = Builder::of(def);
    let change = same
        .resize(&catalog, "s_ic", &[("length", before)])
        .unwrap();
    assert!(change.changed.is_empty() && !same.can_undo());
}

#[test]
fn a_resize_that_runs_a_part_into_another_is_refused() {
    use osp_sim::builder::{Builder, Rule};
    let (catalog, def) = shipped("comet");
    let mut builder = Builder::of(def.clone());
    // The Comet's wing, its root made long enough to run back into the tailplane.
    let refused = builder.resize(&catalog, "wing", &[("root_chord", 4.0)]);
    assert_eq!(refused.map_err(|r| r.rule), Err(Rule::Overlap));
    assert!(builder.draft().parts == def.parts && !builder.can_undo());
}
