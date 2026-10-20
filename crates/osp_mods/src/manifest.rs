//! Mod manifests, discovery and load order.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{Diagnostic, read_json5};

pub const MANIFEST_FILE: &str = "mod.json5";

/// `mod.json5` at the root of every mod (including the base game, id `core`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModManifest {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub description: String,
    /// Mods that must be present; they always load first.
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Load after these mods if they are installed (soft ordering).
    #[serde(default)]
    pub load_after: Vec<String>,
    /// Load before these mods if they are installed (soft ordering).
    #[serde(default)]
    pub load_before: Vec<String>,
    #[serde(default = "enabled")]
    pub enabled: bool,
}

fn enabled() -> bool {
    true
}

/// A mod found on disk.
#[derive(Debug, Clone)]
pub struct ModSource {
    pub manifest: ModManifest,
    pub root: PathBuf,
}

impl ModSource {
    pub fn id(&self) -> &str {
        &self.manifest.id
    }
}

/// Read one mod directory's manifest.
pub fn read_mod(root: &Path) -> Result<ModSource, Diagnostic> {
    let path = root.join(MANIFEST_FILE);
    let mut manifest: ModManifest = read_json5(&path).and_then(|value| {
        serde_json::from_value(value)
            .map_err(|e| Diagnostic::error(None, Some(&path), e.to_string()))
    })?;
    if manifest.name.is_empty() {
        manifest.name = manifest.id.clone();
    }
    Ok(ModSource {
        manifest,
        root: root.to_path_buf(),
    })
}

/// Every subdirectory of `mods_dir` containing a manifest.
pub fn discover(mods_dir: &Path, diagnostics: &mut Vec<Diagnostic>) -> Vec<ModSource> {
    let Ok(entries) = std::fs::read_dir(mods_dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join(MANIFEST_FILE).is_file())
        .collect();
    dirs.sort();
    let mut mods = Vec::new();
    for dir in dirs {
        match read_mod(&dir) {
            Ok(source) if source.manifest.enabled => mods.push(source),
            Ok(source) => diagnostics.push(Diagnostic::info(
                Some(source.id()),
                None,
                "disabled in its manifest".into(),
            )),
            Err(d) => diagnostics.push(d),
        }
    }
    mods
}

/// Order mods so dependencies and `load_after`/`load_before` constraints hold, with `core`
/// first and ties broken alphabetically (deterministic across machines). Mods with missing
/// dependencies, duplicate ids or ordering cycles are dropped with an error diagnostic.
pub fn resolve_order(
    core: ModSource,
    mods: Vec<ModSource>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<ModSource> {
    let mut by_id: BTreeMap<String, ModSource> = BTreeMap::new();
    for source in mods {
        if source.id() == core.id() || by_id.contains_key(source.id()) {
            diagnostics.push(Diagnostic::error(
                Some(source.id()),
                Some(&source.root),
                "duplicate mod id; this copy is ignored".into(),
            ));
            continue;
        }
        by_id.insert(source.id().to_owned(), source);
    }

    // Drop mods whose dependencies are missing, repeating until stable (a dropped mod may
    // be someone else's dependency).
    loop {
        let missing: Vec<(String, String)> = by_id
            .values()
            .flat_map(|m| {
                m.manifest
                    .dependencies
                    .iter()
                    .filter(|d| *d != core.id() && !by_id.contains_key(*d))
                    .map(|d| (m.id().to_owned(), d.clone()))
            })
            .collect();
        if missing.is_empty() {
            break;
        }
        for (id, dep) in missing {
            if by_id.remove(&id).is_some() {
                diagnostics.push(Diagnostic::error(
                    Some(&id),
                    None,
                    format!("missing dependency `{dep}`; mod not loaded"),
                ));
            }
        }
    }

    // Edges: before -> after.
    let mut incoming: BTreeMap<String, BTreeSet<String>> = by_id
        .keys()
        .map(|id| (id.clone(), BTreeSet::new()))
        .collect();
    for m in by_id.values() {
        let id = m.id().to_owned();
        for before in m.manifest.dependencies.iter().chain(&m.manifest.load_after) {
            if by_id.contains_key(before) {
                incoming.get_mut(&id).unwrap().insert(before.clone());
            }
        }
        for after in &m.manifest.load_before {
            if let Some(set) = incoming.get_mut(after) {
                set.insert(id.clone());
            }
        }
    }

    // Kahn's algorithm; BTreeMap iteration gives alphabetical tie-breaking.
    let mut order = vec![core];
    while !incoming.is_empty() {
        let Some(ready) = incoming
            .iter()
            .find(|(_, deps)| deps.is_empty())
            .map(|(id, _)| id.clone())
        else {
            for id in incoming.keys() {
                diagnostics.push(Diagnostic::error(
                    Some(id),
                    None,
                    "load-order cycle (dependencies / load_after / load_before); mod not loaded"
                        .into(),
                ));
            }
            break;
        };
        incoming.remove(&ready);
        for deps in incoming.values_mut() {
            deps.remove(&ready);
        }
        order.push(by_id.remove(&ready).unwrap());
    }
    order
}
