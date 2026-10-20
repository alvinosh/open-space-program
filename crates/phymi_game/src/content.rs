//! Loads the base game and mods, builds the universe and UI documents from them, and hot
//! reloads UI content on the `game.reload` action.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use bevy::{math::DVec3, prelude::*};
use phymi_mods::{Content, Diagnostic, Severity};
use phymi_sim::{
    data::{
        BodyDef, CraftDef, EngineDef, LaunchSiteDef, PartCatalog, ScenarioDef, SystemDef, TankDef,
    },
    terrain::HeightSource,
    universe::Universe,
};
use phymi_terrain::{Dataset, dataset::META_FILE};
use phymi_ui::{LayoutVisibility, UiAction, UiDocuments, UiSet};

pub struct ContentPlugin;

impl Plugin for ContentPlugin {
    fn build(&self, app: &mut App) {
        let content = load_content();
        let mut diagnostics = Vec::new();
        let documents = UiDocuments::from_registry(&content.registry, &mut diagnostics);
        let mut content = GameContent(content);
        content.0.diagnostics.extend(diagnostics);
        for d in &content.0.diagnostics {
            match d.severity {
                Severity::Error => error!("{d}"),
                Severity::Warning => warn!("{d}"),
                Severity::Info => info!("{d}"),
            }
        }
        if content.0.error_count() > 0 {
            // Surface broken mods immediately instead of failing silently.
            app.world_mut()
                .resource_mut::<LayoutVisibility>()
                .0
                .insert("mods".into(), true);
        }
        app.insert_resource(content)
            .insert_resource(documents)
            .add_systems(Update, reload_on_request.after(UiSet::Input));
    }
}

#[derive(Resource)]
pub struct GameContent(pub Content);

/// Root containing `assets/` and `mods/`: the working directory for shipped builds, the
/// workspace when run through cargo.
pub fn game_root() -> PathBuf {
    if Path::new("assets/core").is_dir() {
        return PathBuf::from(".");
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn asset_root() -> PathBuf {
    game_root().join("assets")
}

fn load_content() -> Content {
    let root = game_root();
    phymi_mods::load(&root.join("assets/core"), Some(&root.join("mods")))
}

/// A new game: the universe plus what rendering needs beyond it.
pub struct NewGame {
    pub universe: Universe,
    /// Baked surface data by body id.
    pub datasets: Vec<(String, Arc<Dataset>)>,
    pub sites: Vec<LaunchSiteDef>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Elevation from a baked dataset, for the simulation's terrain.
struct DatasetHeights(Arc<Dataset>);

impl HeightSource for DatasetHeights {
    fn height(&self, dir: DVec3, max_level: u8) -> f64 {
        self.0.height(dir, max_level).unwrap_or(0.0)
    }
}

/// Open the terrain datasets that bodies name (`terrain: "terrain/earth"`), from the
/// highest-priority mod that has them.
fn load_datasets(
    content: &Content,
    bodies: &[BodyDef],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<(String, Arc<Dataset>)> {
    let mut datasets = Vec::new();
    for body in bodies {
        let Some(dir) = &body.terrain else { continue };
        let Some(meta) = content.resolve_file(&format!("{dir}/{META_FILE}")) else {
            diagnostics.push(Diagnostic::warning(
                None,
                None,
                format!(
                    "{}: no terrain data at `{dir}`; the surface will be a smooth sphere. \
                     Bake it with `cargo run -p phymi_bake --release`.",
                    body.id
                ),
            ));
            continue;
        };
        let dir_path = meta.parent().expect("a file has a parent directory");
        match Dataset::open(dir_path) {
            Ok(dataset) => {
                info!("{}: terrain {}", body.id, dataset.meta.description);
                datasets.push((body.id.clone(), Arc::new(dataset)));
            }
            Err(e) => diagnostics.push(Diagnostic::error(
                None,
                Some(dir_path),
                format!("{}: cannot open terrain data: {e}", body.id),
            )),
        }
    }
    datasets
}

/// Build the starting universe for a scenario id from the loaded records.
pub fn new_game(content: &Content, scenario: &str) -> Result<NewGame, String> {
    let registry = &content.registry;
    let mut diagnostics = Vec::new();
    let system = SystemDef {
        bodies: registry.all::<BodyDef>("bodies", &mut diagnostics),
    };
    let catalog = PartCatalog {
        engines: registry.all::<EngineDef>("engines", &mut diagnostics),
        tanks: registry.all::<TankDef>("tanks", &mut diagnostics),
    };
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut diagnostics);
    let mut scenario: ScenarioDef = registry
        .get("scenarios", scenario)
        .map_err(|d| d.to_string())?;
    // Debugging lighting: `PHYMI_START_TIME=<seconds since J2000>`.
    if let Some(time) = std::env::var("PHYMI_START_TIME")
        .ok()
        .and_then(|t| t.parse().ok())
    {
        scenario.start_time = time;
    }
    let craft: CraftDef = registry
        .get("craft", &scenario.craft)
        .map_err(|d| d.to_string())?;
    let datasets = load_datasets(content, &system.bodies, &mut diagnostics);
    let heights = datasets
        .iter()
        .map(|(body, data)| {
            (
                body.clone(),
                Arc::new(DatasetHeights(data.clone())) as Arc<dyn HeightSource>,
            )
        })
        .collect();
    let universe = Universe::new_game(&system, heights, &sites, &catalog, &craft, &scenario)
        .map_err(|e| e.to_string())?;
    Ok(NewGame {
        universe,
        datasets,
        sites,
        diagnostics,
    })
}

/// `game.reload`: reload every mod from disk and rebuild the UI. Physics data already in
/// use (bodies, parts in flight) is not swapped mid-flight.
fn reload_on_request(
    mut actions: MessageReader<UiAction>,
    mut content: ResMut<GameContent>,
    mut documents: ResMut<UiDocuments>,
) {
    if !actions.read().any(|a| a.name() == "game.reload") {
        return;
    }
    let mut fresh = load_content();
    let mut diagnostics = Vec::new();
    *documents = UiDocuments::from_registry(&fresh.registry, &mut diagnostics);
    fresh.diagnostics.extend(diagnostics);
    info!(
        "reloaded {} mods ({} errors)",
        fresh.mods.len(),
        fresh.error_count()
    );
    content.0 = fresh;
}
