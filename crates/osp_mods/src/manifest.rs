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
    /// Whether it is on until the player says otherwise.
    #[serde(default = "enabled")]
    pub enabled: bool,
}

fn enabled() -> bool {
    true
}

impl ModManifest {
    /// Whether the mod is on, given what the player has chosen.
    pub fn is_on(&self, choices: &ModChoices) -> bool {
        choices.get(&self.id).copied().unwrap_or(self.enabled)
    }
}

/// The mod id of the base game, which every mod may depend on and nobody can turn off.
const CORE_ID: &str = "core";

/// What the player has turned on or off, by mod id. A mod it does not name follows its
/// manifest's `enabled`.
pub type ModChoices = BTreeMap<String, bool>;

/// Whether each mod in `available` is on, given `choices`: two sets of choices that give the same
/// answer load the same mods. A mod the choices do not name is on by its manifest only while what it
/// depends on is not turned off: one installed after the choices were made, which needs a mod they
/// turned off, stays off rather than fail to load. A mod chosen on is on, and says what it lacks.
pub fn resolved(available: &[ModManifest], choices: &ModChoices) -> ModChoices {
    let mut on: ModChoices = (available.iter())
        .map(|m| (m.id.clone(), m.is_on(choices)))
        .collect();
    loop {
        let stranded: Vec<String> = (available.iter())
            .filter(|m| on[&m.id] && !choices.contains_key(&m.id))
            .filter(|m| {
                // Installed and off: one not installed at all is the loader's to report.
                (m.dependencies.iter()).any(|d| on.get(d) == Some(&false))
            })
            .map(|m| m.id.clone())
            .collect();
        if stranded.is_empty() {
            return on;
        }
        for id in stranded {
            on.insert(id, false);
        }
    }
}

/// The choices that turn on the mods `ids` and what they depend on, which may be more than when
/// the list was made, and no others.
pub fn exactly(available: &[ModManifest], ids: &[String]) -> ModChoices {
    let needed = ids.iter().flat_map(|id| requirements(available, id));
    let on: BTreeSet<String> = needed.flatten().collect();
    let is_listed = |m: &ModManifest| (m.id.clone(), on.contains(&m.id));
    available.iter().map(is_listed).collect()
}

/// The mod `id` and all it depends on, at any depth, or the first of those not installed.
fn requirements(available: &[ModManifest], id: &str) -> Result<BTreeSet<String>, String> {
    let mut found = BTreeSet::new();
    let mut queue = vec![id.to_owned()];
    while let Some(next) = queue.pop() {
        if next == CORE_ID || found.contains(&next) {
            continue;
        }
        let Some(manifest) = available.iter().find(|m| m.id == next) else {
            return Err(next);
        };
        queue.extend(manifest.dependencies.iter().cloned());
        found.insert(next);
    }
    Ok(found)
}

/// The mod `id` and all that depend on it, at any depth.
fn dependents(available: &[ModManifest], id: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut queue = vec![id.to_owned()];
    while let Some(next) = queue.pop() {
        if found.contains(&next) {
            continue;
        }
        let users = available.iter().filter(|m| m.dependencies.contains(&next));
        queue.extend(users.map(|m| m.id.clone()));
        found.insert(next);
    }
    found
}

/// What `id` needs that is not installed, if anything: a mod that cannot be turned on.
pub fn missing_dependency(available: &[ModManifest], id: &str) -> Option<String> {
    requirements(available, id).err()
}

/// `choices` with the mod `id` turned on or off, and whatever has to go with it: what it depends
/// on when turned on, what depends on it when turned off. Unchanged if the mod is not installed
/// or needs one that is not.
pub fn choose(available: &[ModManifest], choices: &ModChoices, id: &str, on: bool) -> ModChoices {
    let mut next = choices.clone();
    if !available.iter().any(|m| m.id == id) {
        return next;
    }
    let changing = if on {
        match requirements(available, id) {
            Ok(mods) => mods,
            Err(_) => return next,
        }
    } else {
        dependents(available, id)
    };
    next.extend(changing.into_iter().map(|id| (id, on)));
    next
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

/// Every subdirectory of `mods_dir` containing a manifest, on or off.
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
            Ok(source) => mods.push(source),
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
