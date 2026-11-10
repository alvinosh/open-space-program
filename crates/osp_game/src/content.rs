//! Loads the base game and mods, builds the universe and UI documents from them, and hot
//! reloads UI content on the `game.reload` action.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use bevy::{ecs::system::SystemParam, math::DVec3, prelude::*};
use osp_mods::{Content, Diagnostic, ModChoices, ModManifest, Registry, Severity};
use osp_sim::{
    data::{
        AirBreathingEngineDef, AvionicsDef, BatteryDef, BodyDef, CraftDef, EngineDef,
        EngineFamilyDef, GroundStationDef, LaunchSiteDef, LifeSupportProfileDef, LiveryDef,
        MaterialDef, PartCatalog, PodDef, ProcessDef, RcsDef, ReactionWheelDef, ResourceDef,
        ScenarioDef, SolarPanelDef, SystemDef, TankTypeDef, TechNodeDef,
    },
    tech::{TechTree, calendar_year},
    terrain::HeightSource,
    time::SimTime,
    tutorial::{Tutorial, TutorialDef},
    universe::Universe,
};
use osp_terrain::{Dataset, dataset::META_FILE};
use osp_ui::{LayoutVisibility, UiAction, UiDocuments, UiImage, UiSet};
use serde::Deserialize;

use crate::{
    menu::Menu,
    settings::{SettingsSource, chosen_mods, remember_mods},
};

pub struct ContentPlugin;

impl Plugin for ContentPlugin {
    fn build(&self, app: &mut App) {
        let choices = match chosen_mods() {
            Some(saved) if saved.is_empty() => default_choices(),
            Some(saved) => saved,
            None => ModChoices::new(),
        };
        let content = load_content(&choices);
        let loaded = osp_mods::resolved(&content.available, &choices);
        let mut diagnostics = Vec::new();
        let documents = UiDocuments::from_registry(&content.registry, &mut diagnostics);
        let library = Library::load(&content.registry, &mut diagnostics);
        info!("{}", library.summary());
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
                .layouts
                .insert("mods".into(), true);
        }
        app.insert_resource(content)
            .insert_resource(library)
            .insert_resource(documents)
            .insert_resource(PlayerMods {
                choices,
                game: None,
                loaded,
            })
            .add_systems(
                Update,
                (choose_mods, reload_on_request, follow_the_game)
                    .chain()
                    .after(UiSet::Input),
            )
            .add_systems(Update, (load_ui_images, load_ui_fonts).after(UiSet::Update));
    }
}

#[derive(Resource)]
pub struct GameContent(pub Content);

/// The menu screen that says which mods a new game has.
const NEW_GAME_SCREEN: &str = "new_game";

/// A named set of mods, offered for a new game. Anything but the mods it names is off.
#[derive(Debug, Clone, Deserialize)]
pub struct GameModeDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// What it turns on; what those depend on comes too.
    #[serde(default)]
    pub mods: Vec<String>,
    /// What a player who has not chosen yet gets.
    #[serde(default)]
    pub default: bool,
}

/// The mode that turns on what `choices` does, if there is one.
pub fn mode_of<'a>(
    modes: &'a [GameModeDef],
    available: &[ModManifest],
    choices: &ModChoices,
) -> Option<&'a GameModeDef> {
    let on = osp_mods::resolved(available, choices);
    (modes.iter()).find(|mode| osp_mods::exactly(available, &mode.mods) == on)
}

/// Whether every mod of `mode` is installed, so that it can be offered.
pub fn can_offer(mode: &GameModeDef, available: &[ModManifest]) -> bool {
    (mode.mods.iter()).all(|id| available.iter().any(|m| m.id == *id))
}

/// What a player who has not chosen yet starts with: the base game's default mode.
fn default_choices() -> ModChoices {
    let base = osp_mods::load(&asset_root().join("core"), None);
    let modes = (base.registry).all::<GameModeDef>("game_modes", &mut Vec::new());
    match modes.iter().find(|mode| mode.default) {
        Some(mode) => osp_mods::exactly(&osp_mods::installed(&mods_root()), &mode.mods),
        None => ModChoices::new(),
    }
}

/// Which mods the game uses: a new game has what was chosen on the New game screen, and a game
/// from a save keeps the mods it was saved with. Choices wait for the player to start the game or
/// leave the screen: loading every record takes a moment, and one click often leads to another.
#[derive(Resource)]
pub struct PlayerMods {
    /// What new games start with, kept in the settings.
    pub choices: ModChoices,
    /// The mods of the game in progress, if it came from a save; otherwise it has `choices`.
    game: Option<ModChoices>,
    /// Whether each installed mod is on in what is loaded now.
    loaded: ModChoices,
}

impl PlayerMods {
    /// The game in progress is a new one, with the mods new games start with.
    pub fn new_game_starts(&mut self) {
        self.game = None;
    }
}

/// What changing the loaded mods changes.
#[derive(SystemParam)]
pub struct ModSwitch<'w> {
    content: ResMut<'w, GameContent>,
    library: ResMut<'w, Library>,
    documents: ResMut<'w, UiDocuments>,
    mods: ResMut<'w, PlayerMods>,
}

impl ModSwitch<'_> {
    /// Reload every mod from disk and rebuild the UI. Physics data already in use (bodies, parts in
    /// flight) is not swapped mid-flight.
    fn reload(&mut self, choices: &ModChoices) {
        let mut fresh = load_content(choices);
        let mut diagnostics = Vec::new();
        *self.documents = UiDocuments::from_registry(&fresh.registry, &mut diagnostics);
        *self.library = Library::load(&fresh.registry, &mut diagnostics);
        fresh.diagnostics.extend(diagnostics);
        info!(
            "reloaded {} mods ({} errors)",
            fresh.mods.len(),
            fresh.error_count()
        );
        self.mods.loaded = osp_mods::resolved(&fresh.available, choices);
        self.content.0 = fresh;
    }

    /// Load `wanted`, unless those are the mods loaded already.
    fn switch_to(&mut self, wanted: &ModChoices) {
        if osp_mods::resolved(&self.content.0.available, wanted) != self.mods.loaded {
            self.reload(wanted);
        }
    }

    /// The game in progress now comes from a save with the mods `ids`, or, from a save too old to
    /// say, with whatever is loaded.
    pub fn carry_on_with(&mut self, ids: Option<&[String]>) {
        let wanted = match ids {
            Some(ids) => osp_mods::exactly(&self.content.0.available, ids),
            None => self.mods.loaded.clone(),
        };
        self.switch_to(&wanted);
        self.mods.game = Some(wanted);
    }
}

/// The catalogue records the physics doesn't use yet (batteries, avionics, solar cells, jet
/// engines, engine families) and the tech tree that dates everything. Loading them here
/// checks every record against its type, so broken mod data shows in the Mods window.
#[derive(Resource, Default)]
pub struct Library {
    pub tech: TechTree,
    pub engine_families: Vec<EngineFamilyDef>,
    pub air_breathing_engines: Vec<AirBreathingEngineDef>,
    pub batteries: Vec<BatteryDef>,
    pub avionics: Vec<AvionicsDef>,
    pub solar_panels: Vec<SolarPanelDef>,
    /// What a new game can be.
    pub modes: Vec<GameModeDef>,
}

impl Library {
    pub fn load(registry: &Registry, diagnostics: &mut Vec<Diagnostic>) -> Self {
        Self {
            tech: TechTree::new(registry.all::<TechNodeDef>("tech_nodes", diagnostics)),
            engine_families: registry.all("engine_families", diagnostics),
            air_breathing_engines: registry.all("air_breathing_engines", diagnostics),
            batteries: registry.all("batteries", diagnostics),
            avionics: registry.all("avionics", diagnostics),
            solar_panels: registry.all("solar_panels", diagnostics),
            modes: registry.all("game_modes", diagnostics),
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "library: {} tech nodes, {} engine families, {} jet engines, {} batteries, \
             {} avionics levels, {} solar cell levels",
            self.tech.len(),
            self.engine_families.len(),
            self.air_breathing_engines.len(),
            self.batteries.len(),
            self.avionics.len(),
            self.solar_panels.len()
        )
    }
}

/// Root containing `assets/` and `mods/`: the working directory or the executable's directory
/// for shipped builds, the workspace when run through cargo. Absolute, because Bevy resolves
/// a relative asset root against the crate directory when run through cargo.
pub fn game_root() -> PathBuf {
    if Path::new("assets/core").is_dir()
        && let Ok(dir) = std::env::current_dir()
    {
        return dir;
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
        && dir.join("assets/core").is_dir()
    {
        return dir.to_path_buf();
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn asset_root() -> PathBuf {
    game_root().join("assets")
}

/// Where mods live; Bevy loads their files through the `mods://` asset source.
pub fn mods_root() -> PathBuf {
    game_root().join("mods")
}

fn load_content(choices: &ModChoices) -> Content {
    osp_mods::load_chosen(&asset_root().join("core"), Some(&mods_root()), choices)
}

/// Bevy asset path of a file that a mod ships (a model, say), from its path relative to the
/// mod root. The highest-priority mod's copy wins, as for records.
pub fn mod_asset_path(content: &Content, relative: &str) -> Option<String> {
    let file = content.resolve_file(relative)?;
    let slashes = |p: &Path| p.to_string_lossy().replace('\\', "/");
    if let Ok(rest) = file.strip_prefix(asset_root()) {
        return Some(slashes(rest));
    }
    let rest = file.strip_prefix(mods_root()).ok()?;
    Some(format!("mods://{}", slashes(rest)))
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
                     Bake it with `cargo run -p osp_bake --release`.",
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
    let catalog = catalog(registry, &mut diagnostics);
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut diagnostics);
    let scenario = scenario_def(registry, scenario)?;
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
    let craft_of = |id: &str| craft(registry, id);
    let mut universe = Universe::new_game(&system, heights, &sites, &catalog, craft_of, &scenario)
        .map_err(|e| e.to_string())?;
    let stations = registry.all::<GroundStationDef>("ground_stations", &mut diagnostics);
    universe.set_stations(&stations);
    // What vessels collide with is what is drawn, models included.
    universe
        .colliders
        .set_files(Arc::new(content.file_reader()));
    let tech = TechTree::new(registry.all::<TechNodeDef>("tech_nodes", &mut diagnostics));
    check_dates(&tech, &catalog, registry, &scenario, &mut diagnostics);
    Ok(NewGame {
        universe,
        datasets,
        sites,
        diagnostics,
    })
}

/// Start the scenario `id` afresh in `universe` (same solar system and terrain), from the
/// records loaded now. On an error nothing changes.
pub fn restart(content: &Content, universe: &mut Universe, id: &str) -> Result<(), String> {
    let registry = &content.registry;
    let mut diagnostics = Vec::new();
    let catalog = catalog(registry, &mut diagnostics);
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut diagnostics);
    let scenario = scenario_def(registry, id)?;
    universe
        .restart(&sites, &catalog, |id| craft(registry, id), &scenario)
        .map_err(|e| e.to_string())
}

/// A tutorial record.
pub fn tutorial_def(registry: &Registry, id: &str) -> Result<TutorialDef, String> {
    registry
        .get::<TutorialDef>("tutorials", id)
        .map_err(|d| d.to_string())
}

/// The tutorial `id` ready to play in `universe`'s solar system, and the scenario it is
/// played over. Fails if a step names something that is not there.
fn build_tutorial(
    registry: &Registry,
    universe: &Universe,
    id: &str,
) -> Result<(Tutorial, ScenarioDef), String> {
    let def = tutorial_def(registry, id)?;
    let scenario = scenario_def(registry, &def.scenario)?;
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
    let time = SimTime::from_seconds(scenario.start_time);
    let tutorial = Tutorial::new(&def, &scenario, &sites, &universe.system, time)?;
    Ok((tutorial, scenario))
}

/// Play the tutorial `id` in a game that has just started its scenario (`OSP_TUTORIAL`).
pub fn start_tutorial(content: &Content, universe: &mut Universe, id: &str) -> Result<(), String> {
    let (tutorial, _) = build_tutorial(&content.registry, universe, id)?;
    universe.tutorial = Some(tutorial);
    Ok(())
}

/// Start the tutorial `id` afresh in `universe`: its scenario, and the guided steps over it.
/// On an error nothing changes.
pub fn restart_tutorial(
    content: &Content,
    universe: &mut Universe,
    id: &str,
) -> Result<(), String> {
    let registry = &content.registry;
    let (tutorial, scenario) = build_tutorial(registry, universe, id)?;
    restart(content, universe, &scenario.id)?;
    universe.tutorial = Some(tutorial);
    Ok(())
}

pub fn catalog(registry: &Registry, diagnostics: &mut Vec<Diagnostic>) -> PartCatalog {
    PartCatalog {
        rules: registry.get("rules", "game").unwrap_or_default(),
        engines: registry.all::<EngineDef>("engines", diagnostics),
        resources: registry.all::<ResourceDef>("resources", diagnostics),
        materials: registry.all::<MaterialDef>("materials", diagnostics),
        tank_types: registry.all::<TankTypeDef>("tank_types", diagnostics),
        reaction_wheels: registry.all::<ReactionWheelDef>("reaction_wheels", diagnostics),
        rcs: registry.all::<RcsDef>("rcs", diagnostics),
        pods: registry.all::<PodDef>("pods", diagnostics),
        batteries: registry.all::<BatteryDef>("batteries", diagnostics),
        avionics: registry.all::<AvionicsDef>("avionics", diagnostics),
        liveries: registry.all::<LiveryDef>("liveries", diagnostics),
        life_support: registry.all::<LifeSupportProfileDef>("life_support", diagnostics),
        processes: registry.all::<ProcessDef>("processes", diagnostics),
        crew_names: registry.all("crew_names", diagnostics),
        engine_families: registry.all("engine_families", diagnostics),
    }
}

/// Warn about parts the scenario's craft use before they existed (by the tech tree's dates).
/// It is a warning, not an error: alternate histories are allowed.
fn check_dates(
    tech: &TechTree,
    catalog: &PartCatalog,
    registry: &Registry,
    scenario: &ScenarioDef,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let year = calendar_year(SimTime::from_seconds(scenario.start_time));
    let crafts = std::iter::once(&scenario.craft).chain(scenario.others.iter().map(|o| &o.craft));
    for id in crafts {
        let Ok(def) = craft(registry, id) else {
            continue;
        };
        for a in tech.anachronisms(catalog, &def, year) {
            diagnostics.push(Diagnostic::warning(
                None,
                None,
                format!(
                    "{}: {} `{}` is from {}, after the scenario's {year}",
                    scenario.id, a.kind, a.part, a.year
                ),
            ));
        }
    }
}

fn craft(registry: &Registry, id: &str) -> Result<CraftDef, String> {
    registry
        .get::<CraftDef>("craft", id)
        .map_err(|d| d.to_string())
}

/// A scenario record. `OSP_START_TIME=<seconds since J2000>` moves its start (debugging
/// lighting).
fn scenario_def(registry: &Registry, id: &str) -> Result<ScenarioDef, String> {
    let mut scenario: ScenarioDef = registry.get("scenarios", id).map_err(|d| d.to_string())?;
    if let Some(time) = std::env::var("OSP_START_TIME")
        .ok()
        .and_then(|t| t.parse().ok())
    {
        scenario.start_time = time;
    }
    Ok(scenario)
}

/// Load the pictures of UI `image` nodes from the mods (the highest-priority mod's file).
/// Repeated lists rebuild their nodes as the data changes, so a node may be gone by the
/// time the command runs; that is fine.
fn load_ui_images(
    mut commands: Commands,
    content: Res<GameContent>,
    assets: Res<AssetServer>,
    images: Query<(Entity, &UiImage), Without<ImageNode>>,
) {
    for (entity, image) in &images {
        let node = match mod_asset_path(&content.0, &image.path) {
            Some(path) => ImageNode::new(assets.load(path)).with_color(image.tint),
            None => {
                warn!("UI image `{}` is not in any mod", image.path);
                ImageNode::default().with_color(Color::NONE)
            }
        };
        commands.entity(entity).try_insert(node);
    }
}

/// The theme's font files, kept loaded while the theme names them; text finds them by
/// family name once they are in.
#[derive(Default)]
struct UiFonts(Vec<Handle<Font>>);

fn load_ui_fonts(
    content: Res<GameContent>,
    documents: Res<UiDocuments>,
    assets: Res<AssetServer>,
    mut fonts: Local<UiFonts>,
) {
    if !documents.is_changed() {
        return;
    }
    fonts.0 = documents
        .theme
        .fonts
        .iter()
        .filter_map(|file| match mod_asset_path(&content.0, file) {
            Some(path) => Some(assets.load(path)),
            None => {
                warn!("UI font `{file}` is not in any mod");
                None
            }
        })
        .collect();
}

/// `mods.toggle:<id>` and `mods.mode:<id>`, on the New game screen only: what a new game will have.
fn choose_mods(
    mut actions: MessageReader<UiAction>,
    content: Res<GameContent>,
    library: Res<Library>,
    menu: Res<Menu>,
    mut mods: ResMut<PlayerMods>,
    mut source: ResMut<SettingsSource>,
) {
    for action in actions.read() {
        let Some(id) = action.arg().filter(|_| menu.screen() == NEW_GAME_SCREEN) else {
            continue;
        };
        let available = &content.0.available;
        let next = match action.name() {
            "mods.toggle" => {
                let on = available.iter().find(|m| m.id == id);
                let on = !on.is_some_and(|m| m.is_on(&mods.choices));
                osp_mods::choose(available, &mods.choices, id, on)
            }
            "mods.mode" => match library.modes.iter().find(|mode| mode.id == id) {
                Some(mode) if can_offer(mode, available) => {
                    osp_mods::exactly(available, &mode.mods)
                }
                _ => continue,
            },
            _ => continue,
        };
        if next != mods.choices {
            remember_mods(&mut source, &next);
            mods.choices = next;
        }
    }
}

/// `game.reload`: reload the mods in use from disk.
fn reload_on_request(mut actions: MessageReader<UiAction>, mut switch: ModSwitch) {
    if actions.read().any(|a| a.name() == "game.reload") {
        let loaded = switch.mods.loaded.clone();
        switch.reload(&loaded);
    }
}

/// Keep the loaded mods those of what the player is doing: the game in progress's while in it, and
/// what a new game will have on the first screen and in the editor of a new game.
fn follow_the_game(menu: Res<Menu>, mut switch: ModSwitch) {
    if menu.screen() == NEW_GAME_SCREEN && !menu.new_game {
        return;
    }
    if !menu.playing && switch.mods.game.is_some() {
        switch.mods.game = None;
    }
    let game = (menu.playing && !menu.new_game)
        .then(|| switch.mods.game.clone())
        .flatten();
    let wanted = game.unwrap_or_else(|| switch.mods.choices.clone());
    switch.switch_to(&wanted);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> (Vec<GameModeDef>, Vec<ModManifest>) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
        let modes = (content.registry).all::<GameModeDef>("game_modes", &mut Vec::new());
        (modes, content.available)
    }

    #[test]
    fn a_new_player_gets_casual_and_realistic_is_all_there() {
        let (modes, available) = shipped();
        let mode = |id: &str| modes.iter().find(|m| m.id == id).unwrap();
        let defaults: Vec<_> = modes.iter().filter(|m| m.default).collect();
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0].id, "casual");
        assert!(mode("casual").mods.is_empty());
        assert!(can_offer(mode("casual"), &available));
        assert!(can_offer(mode("realistic"), &available));
        for id in [
            "realistic_engines",
            "realistic_fuels",
            "realistic_avionics",
            "realistic_life_support",
            "realism",
        ] {
            assert!(mode("realistic").mods.iter().any(|m| m == id), "{id}");
        }
    }

    #[test]
    fn choices_are_a_mode_only_if_they_turn_on_just_what_it_does() {
        let (modes, available) = shipped();
        let name =
            |choices: &ModChoices| mode_of(&modes, &available, choices).map(|m| m.id.as_str());
        let casual = osp_mods::exactly(&available, &[]);
        assert_eq!(name(&casual), Some("casual"));
        let realistic = (modes.iter().find(|m| m.id == "realistic").unwrap().mods).clone();
        assert_eq!(
            name(&osp_mods::exactly(&available, &realistic)),
            Some("realistic")
        );
        // The real parts bring what they depend on, but not the ship systems' mods (T250).
        let realism = ["realism".to_owned()];
        assert_eq!(name(&osp_mods::exactly(&available, &realism)), None);
        let engines = ["realistic_engines".to_owned()];
        assert_eq!(name(&osp_mods::exactly(&available, &engines)), None);
    }
}
