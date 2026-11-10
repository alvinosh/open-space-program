//! Data-driven mod loading for Open Space Program. Engine-agnostic (no Bevy).
//!
//! The base game is itself a mod (`core`); every other mod layers on top of it.
//!
//! # Layout of a mod
//!
//! ```text
//! mods/my_mod/
//!   mod.json5              manifest: id, name, version, dependencies, load order hints
//!   engines/*.json5        records, grouped by collection = first directory name
//!   bodies/*.json5
//!   layouts/*.json5        UI layouts are records too
//!   patches/*.json5        changes to records from other mods (see [`patch`])
//!   ...any other files     resolved through [`Content::resolve_file`] (textures, etc.)
//! ```
//!
//! A record file holds one object or an array of objects, each with a string `id`.
//! A later mod defining the same `(collection, id)` replaces the record; patches modify
//! records in place, in a deterministic order (pass, then mod load order, then file name).
//!
//! Records stay as lossless JSON values until the game deserializes them into typed
//! structs with [`Registry::get`] / [`Registry::all`], so mods can patch any field of any
//! data type without the loader knowing about it.

pub mod manifest;
pub mod patch;

use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub use manifest::{
    ModChoices, ModManifest, ModSource, choose, exactly, missing_dependency, resolved,
};
pub use patch::Patch;

/// Directory name reserved for patch files.
pub const PATCHES_DIR: &str = "patches";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// A problem (or note) found while loading. Loading never aborts on bad mod content; the
/// offending file or record is skipped and reported here.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub mod_id: Option<String>,
    pub file: Option<PathBuf>,
    pub message: String,
}

impl Diagnostic {
    pub fn new(
        severity: Severity,
        mod_id: Option<&str>,
        file: Option<&Path>,
        message: String,
    ) -> Self {
        Self {
            severity,
            mod_id: mod_id.map(str::to_owned),
            file: file.map(Path::to_path_buf),
            message,
        }
    }

    pub fn info(mod_id: Option<&str>, file: Option<&Path>, message: String) -> Self {
        Self::new(Severity::Info, mod_id, file, message)
    }

    pub fn warning(mod_id: Option<&str>, file: Option<&Path>, message: String) -> Self {
        Self::new(Severity::Warning, mod_id, file, message)
    }

    pub fn error(mod_id: Option<&str>, file: Option<&Path>, message: String) -> Self {
        Self::new(Severity::Error, mod_id, file, message)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.severity {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        write!(f, "{level}")?;
        if let Some(id) = &self.mod_id {
            write!(f, " [{id}]")?;
        }
        if let Some(file) = &self.file {
            write!(f, " {}", file.display())?;
        }
        write!(f, ": {}", self.message)
    }
}

/// Parse a JSON5 file into a lossless JSON value.
pub fn read_json5(path: &Path) -> Result<Value, Diagnostic> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| Diagnostic::error(None, Some(path), e.to_string()))?;
    json5::from_str(&text).map_err(|e| Diagnostic::error(None, Some(path), e.to_string()))
}

/// One data record and where it came from.
#[derive(Debug, Clone)]
pub struct Record {
    pub id: String,
    pub value: Value,
    /// Mod that (last) defined the record.
    pub source: String,
    pub file: PathBuf,
    /// Mods whose patches changed it, in order.
    pub patched_by: Vec<String>,
}

/// All records, by collection and id, in definition order.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    collections: BTreeMap<String, IndexMap<String, Record>>,
}

impl Registry {
    pub fn collections(&self) -> impl Iterator<Item = &str> {
        self.collections.keys().map(String::as_str)
    }

    pub fn records(&self, collection: &str) -> impl Iterator<Item = &Record> {
        self.collections
            .get(collection)
            .into_iter()
            .flat_map(|c| c.values())
    }

    pub fn record(&self, collection: &str, id: &str) -> Option<&Record> {
        self.collections.get(collection)?.get(id)
    }

    /// Insert or replace. Returns the replaced record's source mod, if any.
    pub fn insert(&mut self, collection: &str, record: Record) -> Option<String> {
        self.collections
            .entry(collection.to_owned())
            .or_default()
            .insert(record.id.clone(), record)
            .map(|old| old.source)
    }

    pub fn remove(&mut self, collection: &str, id: &str) -> Option<Record> {
        self.collections.get_mut(collection)?.shift_remove(id)
    }

    /// Deserialize one record into `T`.
    pub fn get<T: DeserializeOwned>(&self, collection: &str, id: &str) -> Result<T, Diagnostic> {
        let record = self.record(collection, id).ok_or_else(|| {
            Diagnostic::error(
                None,
                None,
                format!("no `{collection}` record with id `{id}`"),
            )
        })?;
        typed(record)
    }

    /// Deserialize every record in a collection. Broken records are skipped and reported.
    pub fn all<T: DeserializeOwned>(
        &self,
        collection: &str,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Vec<T> {
        self.records(collection)
            .filter_map(|record| typed(record).map_err(|d| diagnostics.push(d)).ok())
            .collect()
    }
}

fn typed<T: DeserializeOwned>(record: &Record) -> Result<T, Diagnostic> {
    serde_json::from_value(record.value.clone()).map_err(|e| {
        let patched = if record.patched_by.is_empty() {
            String::new()
        } else {
            format!(" (patched by {})", record.patched_by.join(", "))
        };
        Diagnostic::error(
            Some(&record.source),
            Some(&record.file),
            format!("record `{}`{patched}: {e}", record.id),
        )
    })
}

/// Everything loaded: mods in load order, the merged registry, and diagnostics.
#[derive(Debug, Clone, Default)]
pub struct Content {
    pub mods: Vec<ModSource>,
    /// Every mod installed, on or off (not the base game): what a player can choose from.
    pub available: Vec<ModManifest>,
    pub registry: Registry,
    pub diagnostics: Vec<Diagnostic>,
}

impl Content {
    /// The highest-priority mod's copy of a non-record file (e.g. a texture), by path
    /// relative to the mod root.
    pub fn resolve_file(&self, relative: &str) -> Option<PathBuf> {
        self.mods
            .iter()
            .rev()
            .map(|m| m.root.join(relative))
            .find(|p| p.is_file())
    }

    /// How to read the mods' files from anywhere, later: their contents by relative path.
    pub fn file_reader(&self) -> impl Fn(&str) -> Option<Vec<u8>> + Send + Sync + 'static {
        let roots: Vec<PathBuf> = self.mods.iter().rev().map(|m| m.root.clone()).collect();
        move |relative| {
            roots
                .iter()
                .map(|root| root.join(relative))
                .find(|path| path.is_file())
                .and_then(|path| std::fs::read(path).ok())
        }
    }

    /// The mods loaded, the base game aside, which is always first and always there.
    pub fn added_mods(&self) -> Vec<String> {
        let added = self.mods.iter().skip(1);
        added.map(|m| m.id().to_owned()).collect()
    }

    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }
}

/// Every mod in `mods_dir`, on or off.
pub fn installed(mods_dir: &Path) -> Vec<ModManifest> {
    let found = manifest::discover(mods_dir, &mut Vec::new());
    found.into_iter().map(|source| source.manifest).collect()
}

/// Load the base game from `core_root` plus every mod in `mods_dir` that its manifest leaves on.
pub fn load(core_root: &Path, mods_dir: Option<&Path>) -> Content {
    load_chosen(core_root, mods_dir, &ModChoices::new())
}

/// Load the base game from `core_root` plus the mods in `mods_dir` that are on, given what the
/// player has chosen.
pub fn load_chosen(core_root: &Path, mods_dir: Option<&Path>, choices: &ModChoices) -> Content {
    let mut diagnostics = Vec::new();
    let core = match manifest::read_mod(core_root) {
        Ok(core) => core,
        Err(d) => {
            diagnostics.push(d);
            ModSource {
                manifest: ModManifest {
                    id: "core".into(),
                    name: "Core".into(),
                    version: String::new(),
                    authors: Vec::new(),
                    description: String::new(),
                    dependencies: Vec::new(),
                    load_after: Vec::new(),
                    load_before: Vec::new(),
                    enabled: true,
                },
                root: core_root.to_path_buf(),
            }
        }
    };
    let found = mods_dir
        .map(|dir| manifest::discover(dir, &mut diagnostics))
        .unwrap_or_default();
    let mut available: Vec<ModManifest> = Vec::new();
    for source in &found {
        if source.id() != core.id() && !available.iter().any(|m| m.id == source.id()) {
            available.push(source.manifest.clone());
        }
    }
    let wanted = resolved(&available, choices);
    let is_on = |m: &ModSource| wanted.get(m.id()).copied().unwrap_or(m.manifest.is_on(choices));
    let (on, off): (Vec<_>, Vec<_>) = found.into_iter().partition(is_on);
    for source in &off {
        diagnostics.push(Diagnostic::info(
            Some(source.id()),
            None,
            "turned off".into(),
        ));
    }
    let mods = manifest::resolve_order(core, on, &mut diagnostics);

    let mut registry = Registry::default();
    let mut patches: Vec<(String, PathBuf, Patch)> = Vec::new();
    for source in &mods {
        for file in json_files(&source.root) {
            let relative = file.strip_prefix(&source.root).unwrap_or(&file);
            let Some(collection) = relative
                .components()
                .next()
                .and_then(|c| c.as_os_str().to_str())
                .filter(|_| relative.components().count() > 1)
            else {
                continue; // files at the mod root (the manifest) are not records
            };
            let value = match read_json5(&file) {
                Ok(v) => v,
                Err(mut d) => {
                    d.mod_id = Some(source.id().to_owned());
                    diagnostics.push(d);
                    continue;
                }
            };
            let items = match value {
                Value::Array(items) => items,
                single => vec![single],
            };
            for item in items {
                if collection == PATCHES_DIR {
                    match serde_json::from_value::<Patch>(item) {
                        Ok(p) => patches.push((source.id().to_owned(), file.clone(), p)),
                        Err(e) => diagnostics.push(Diagnostic::error(
                            Some(source.id()),
                            Some(&file),
                            format!("invalid patch: {e}"),
                        )),
                    }
                    continue;
                }
                let Some(id) = item.get("id").and_then(Value::as_str).map(str::to_owned) else {
                    diagnostics.push(Diagnostic::error(
                        Some(source.id()),
                        Some(&file),
                        "record without a string `id`; skipped".into(),
                    ));
                    continue;
                };
                let replaced = registry.insert(
                    collection,
                    Record {
                        id: id.clone(),
                        value: item,
                        source: source.id().to_owned(),
                        file: file.clone(),
                        patched_by: Vec::new(),
                    },
                );
                if let Some(previous) = replaced {
                    diagnostics.push(Diagnostic::info(
                        Some(source.id()),
                        Some(&file),
                        format!("replaces `{collection}/{id}` from `{previous}`"),
                    ));
                }
            }
        }
    }

    for pass in patch::Pass::ALL {
        for (mod_id, file, p) in patches.iter().filter(|(_, _, p)| p.pass == pass) {
            apply_patch(&mut registry, mod_id, file, p, &mut diagnostics);
        }
    }

    Content {
        mods,
        available,
        registry,
        diagnostics,
    }
}

fn apply_patch(
    registry: &mut Registry,
    mod_id: &str,
    file: &Path,
    p: &Patch,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let targets: Vec<String> = registry
        .records(&p.collection)
        .filter(|r| patch::glob_match(&p.target, &r.id))
        .filter(|r| {
            p.filter
                .as_ref()
                .is_none_or(|f| patch::matches_filter(&r.value, f))
        })
        .map(|r| r.id.clone())
        .collect();
    if targets.is_empty() {
        if !p.optional {
            diagnostics.push(Diagnostic::warning(
                Some(mod_id),
                Some(file),
                format!(
                    "{:?} patch matched nothing in `{}` (target `{}`)",
                    p.op, p.collection, p.target
                ),
            ));
        }
        return;
    }
    for id in targets {
        let result = match p.op {
            patch::Op::Remove if p.path.is_empty() => {
                registry.remove(&p.collection, &id);
                Ok(())
            }
            patch::Op::Copy => copy_record(registry, mod_id, file, p, &id),
            _ => {
                let collection = registry.collections.get_mut(&p.collection).unwrap();
                let record = collection.get_mut(&id).unwrap();
                patch::apply_to_record(&mut record.value, p).map(|()| {
                    record.patched_by.push(mod_id.to_owned());
                })
            }
        };
        if let Err(message) = result {
            diagnostics.push(Diagnostic::warning(
                Some(mod_id),
                Some(file),
                format!("{:?} `{}/{id}`: {message}", p.op, p.collection),
            ));
        }
    }
}

fn copy_record(
    registry: &mut Registry,
    mod_id: &str,
    file: &Path,
    p: &Patch,
    id: &str,
) -> Result<(), String> {
    let new_id = p.new_id.clone().ok_or("copy needs `new_id`")?;
    let mut value = registry.record(&p.collection, id).unwrap().value.clone();
    if let Some(changes) = &p.value {
        patch::deep_merge(&mut value, changes);
    }
    value["id"] = Value::String(new_id.clone());
    registry.insert(
        &p.collection,
        Record {
            id: new_id,
            value,
            source: mod_id.to_owned(),
            file: file.to_path_buf(),
            patched_by: Vec::new(),
        },
    );
    Ok(())
}

/// All `.json5` / `.json` files below `root`, sorted for deterministic load order.
fn json_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e == "json5" || e == "json")
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde::Deserialize;

    use super::*;

    /// Build a throwaway mod tree under the system temp dir.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("osp_mods_test_{name}_{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Tree(dir)
        }

        fn file(&self, relative: &str, text: &str) -> &Self {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
            self
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Engine {
        id: String,
        thrust: f64,
        #[serde(default)]
        ignitions: Option<u32>,
    }

    fn base(tree: &Tree) {
        tree.file("core/mod.json5", "{ id: 'core', name: 'Core' }")
            .file(
                "core/engines/all.json5",
                "[ { id: 'A', thrust: 1.5e6 }, { id: 'B', thrust: 2e5, ignitions: 1 }, ]",
            );
    }

    #[test]
    fn layering_patching_and_ordering() {
        let tree = Tree::new("layering");
        base(&tree);
        tree.file(
            "mods/zeta/mod.json5",
            "{ id: 'zeta', dependencies: ['alpha'] }",
        )
        .file(
            "mods/zeta/patches/p.json5",
            // Runs after alpha's default-pass patch because of the dependency.
            "[ { op: 'merge', collection: 'engines', target: '*', where: { ignitions: 3 }, value: { ignitions: 4 } } ]",
        )
        .file("mods/alpha/mod.json5", "{ id: 'alpha' }")
        .file(
            "mods/alpha/engines/c.json5",
            "{ id: 'C', thrust: 3 }",
        )
        .file(
            "mods/alpha/patches/p.json5",
            "[ { op: 'set', collection: 'engines', target: 'B', path: 'ignitions', value: 3 },
               { op: 'copy', collection: 'engines', target: 'A', new_id: 'A2', value: { thrust: 9 } },
               { op: 'merge', collection: 'engines', target: 'A', value: { thrust: 1 }, pass: 'final' },
               { op: 'remove', collection: 'engines', target: 'nothing*', optional: true } ]",
        );

        let content = load(&tree.0.join("core"), Some(&tree.0.join("mods")));
        let order: Vec<&str> = content.mods.iter().map(ModSource::id).collect();
        assert_eq!(order, ["core", "alpha", "zeta"]);
        assert!(
            content
                .diagnostics
                .iter()
                .all(|d| d.severity != Severity::Error),
            "{:?}",
            content.diagnostics
        );

        let mut diags = Vec::new();
        let engines: Vec<Engine> = content.registry.all("engines", &mut diags);
        let ids: Vec<&str> = engines.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["A", "B", "C", "A2"]);
        let get = |id: &str| content.registry.get::<Engine>("engines", id).unwrap();
        assert_eq!(get("A").thrust, 1.0); // final pass
        assert_eq!(get("A2").thrust, 9.0); // copied before the final pass
        assert_eq!(get("B").ignitions, Some(4)); // alpha set 3, then zeta matched it
        assert_eq!(
            content.registry.record("engines", "B").unwrap().patched_by,
            ["alpha", "zeta"]
        );
    }

    #[test]
    fn broken_content_is_reported_not_fatal() {
        let tree = Tree::new("broken");
        base(&tree);
        tree.file(
            "mods/bad/mod.json5",
            "{ id: 'bad', dependencies: ['missing'] }",
        )
        .file("mods/ok/mod.json5", "{ id: 'ok' }")
        .file("mods/ok/engines/x.json5", "{ thrust: 1 }") // no id
        .file("mods/ok/engines/y.json5", "{ id: 'Y', thrust: 'fast' }") // wrong type
        .file("mods/ok/engines/z.json5", "{ id: 'Z', thrust: ") // syntax error
        .file(
            "mods/ok/patches/p.json5",
            "[ { op: 'merge', collection: 'engines', target: 'ghost', value: {} } ]",
        )
        .file(
            "mods/cyc1/mod.json5",
            "{ id: 'cyc1', load_after: ['cyc2'] }",
        )
        .file(
            "mods/cyc2/mod.json5",
            "{ id: 'cyc2', load_after: ['cyc1'] }",
        );

        let content = load(&tree.0.join("core"), Some(&tree.0.join("mods")));
        let order: Vec<&str> = content.mods.iter().map(ModSource::id).collect();
        assert_eq!(order, ["core", "ok"]);
        let text: Vec<String> = content.diagnostics.iter().map(|d| d.to_string()).collect();
        let has = |s: &str| text.iter().any(|t| t.contains(s));
        assert!(has("missing dependency `missing`"), "{text:#?}");
        assert!(has("without a string `id`"));
        assert!(has("cycle"));
        assert!(has("matched nothing"));
        let mut diags = Vec::new();
        let engines: Vec<Engine> = content.registry.all("engines", &mut diags);
        assert_eq!(engines.len(), 2); // A, B; Y fails to deserialize
        assert!(diags[0].to_string().contains("record `Y`"));
    }

    fn choosing_tree(name: &str) -> Tree {
        let tree = Tree::new(name);
        base(&tree);
        tree.file("mods/fuels/mod.json5", "{ id: 'fuels' }")
            .file("mods/fuels/engines/f.json5", "{ id: 'F', thrust: 1 }")
            .file(
                "mods/parts/mod.json5",
                "{ id: 'parts', dependencies: ['core', 'fuels'] }",
            )
            .file("mods/parts/engines/p.json5", "{ id: 'P', thrust: 1 }")
            .file("mods/spare/mod.json5", "{ id: 'spare', enabled: false }")
            .file("mods/spare/engines/s.json5", "{ id: 'S', thrust: 1 }")
            .file(
                "mods/broken/mod.json5",
                "{ id: 'broken', dependencies: ['gone'], enabled: false }",
            );
        tree
    }

    fn loaded(content: &Content) -> Vec<&str> {
        content.mods.iter().map(ModSource::id).collect()
    }

    #[test]
    fn the_players_choices_override_the_manifests() {
        let tree = choosing_tree("choices");
        let (core, mods) = (tree.0.join("core"), tree.0.join("mods"));

        let content = load(&core, Some(&mods));
        assert_eq!(loaded(&content), ["core", "fuels", "parts"]);
        let mut every: Vec<_> = content.available.iter().map(|m| m.id.as_str()).collect();
        every.sort();
        assert_eq!(every, ["broken", "fuels", "parts", "spare"]);

        let choices = ModChoices::from([("parts".into(), false), ("spare".into(), true)]);
        let content = load_chosen(&core, Some(&mods), &choices);
        assert_eq!(loaded(&content), ["core", "fuels", "spare"]);
        assert!(content.registry.record("engines", "P").is_none());
        assert!(content.registry.record("engines", "S").is_some());
        assert_eq!(content.available.len(), 4, "the list keeps what is off");
    }

    /// Choices made before a mod was installed do not name it: on by its manifest, it stays off if
    /// they turned off what it needs, rather than fail to load. Chosen on, it says what it lacks.
    #[test]
    fn a_new_mod_needing_one_turned_off_stays_off() {
        let tree = choosing_tree("stranded");
        let (core, mods) = (tree.0.join("core"), tree.0.join("mods"));
        let choices = ModChoices::from([("fuels".into(), false)]);
        let content = load_chosen(&core, Some(&mods), &choices);
        assert_eq!(loaded(&content), ["core"]);
        assert_eq!(content.error_count(), 0, "{:#?}", content.diagnostics);
        assert_eq!(resolved(&content.available, &choices)["parts"], false);

        let choices = ModChoices::from([("fuels".into(), false), ("parts".into(), true)]);
        let content = load_chosen(&core, Some(&mods), &choices);
        assert_eq!(content.error_count(), 1);
    }

    #[test]
    fn a_saved_list_of_mods_loads_those_and_no_others() {
        let tree = choosing_tree("exactly");
        let (core, mods) = (tree.0.join("core"), tree.0.join("mods"));
        let content = load(&core, Some(&mods));
        assert_eq!(content.added_mods(), ["fuels", "parts"]);

        // A mod turned on since the save was made stays off for it, and one left on comes back.
        let saved = ["spare".to_owned()];
        let choices = exactly(&content.available, &saved);
        let again = load_chosen(&core, Some(&mods), &choices);
        assert_eq!(again.added_mods(), saved);

        // A list from before a mod needed another brings that one too.
        let older = ["parts".to_owned()];
        let choices = exactly(&content.available, &older);
        let again = load_chosen(&core, Some(&mods), &choices);
        assert_eq!(again.added_mods(), ["fuels", "parts"]);
        let choices = exactly(&content.available, &saved);

        // Choices that say the same thing in different words are the same mods.
        let none = ModChoices::new();
        let spelled_out = resolved(&content.available, &none);
        assert_eq!(resolved(&content.available, &spelled_out), spelled_out);
        assert_ne!(resolved(&content.available, &choices), spelled_out);
    }

    #[test]
    fn choosing_a_mod_brings_what_goes_with_it() {
        let tree = choosing_tree("cascade");
        let available = load(&tree.0.join("core"), Some(&tree.0.join("mods"))).available;

        // Off takes what depends on it; the mod it depends on stays.
        let choices = choose(&available, &ModChoices::new(), "fuels", false);
        assert_eq!(
            choices,
            ModChoices::from([("fuels".into(), false), ("parts".into(), false)])
        );
        let choices = choose(&available, &ModChoices::new(), "parts", false);
        assert_eq!(choices, ModChoices::from([("parts".into(), false)]));

        // On brings what it depends on.
        let off = ModChoices::from([("fuels".into(), false), ("parts".into(), false)]);
        let choices = choose(&available, &off, "parts", true);
        assert_eq!(
            choices,
            ModChoices::from([("fuels".into(), true), ("parts".into(), true)])
        );

        // What cannot load cannot be turned on, and what is not there is no choice at all.
        assert_eq!(
            missing_dependency(&available, "broken").as_deref(),
            Some("gone")
        );
        assert_eq!(missing_dependency(&available, "parts"), None);
        let none = ModChoices::new();
        assert_eq!(choose(&available, &none, "broken", true), none);
        assert_eq!(choose(&available, &none, "nonesuch", false), none);
    }

    #[test]
    fn later_mod_replaces_records_and_files() {
        let tree = Tree::new("replace");
        base(&tree);
        tree.file("core/textures/a.txt", "core")
            .file("mods/m/mod.json5", "{ id: 'm' }")
            .file("mods/m/engines/a.json5", "{ id: 'A', thrust: 7 }")
            .file("mods/m/textures/a.txt", "mod");
        let content = load(&tree.0.join("core"), Some(&tree.0.join("mods")));
        assert_eq!(
            content
                .registry
                .get::<Engine>("engines", "A")
                .unwrap()
                .thrust,
            7.0
        );
        let file = content.resolve_file("textures/a.txt").unwrap();
        assert_eq!(fs::read_to_string(file).unwrap(), "mod");
    }
}
