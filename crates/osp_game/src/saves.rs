//! Saved games on disk, and the actions on them: `game.save` writes the game in progress to its
//! save, `game.save_as` to a new one, `game.save_to:<file>` over the save `file`, `game.load:<file>`
//! carries on from one, `game.continue` from the newest, and `game.delete_save:<file>` deletes one
//! (`dialog` is what asks first, before overwriting another game's save too). Saving from the Save
//! screen goes back from it, to Pause, which says what was saved. A save starts
//! with what the list shows, so the list only reads that much of each (see `osp_sim::save`).
//!
//! A save holds quicksaves of its own, in a folder of the same name beside it: `game_1.save` and
//! `game_1/quicksave_<n>.save`, named `<save>/quicksave_<n>` in the actions. `game.quicksave` (F5)
//! adds one to the save the game in progress goes to, starting that save if there is none yet;
//! holding `game.quickload` (F9) for [`QUICKLOAD_HOLD`] carries on from the newest, and the action
//! fired from a button does it at once. Neither touches the save itself, so leaving after either
//! still asks to save the game. Deleting a save deletes its quicksaves with it.
//!
//! A save says which mods it was played with. Carrying on from one loads those, whatever the Mods
//! screen says, and the list says so if one of them is not installed.
//!
//! Saves live in `saves/` in `OSP_USER_DIR`, or beside the settings file. Scripted captures
//! (`OSP_CAPTURE`) only touch them with `OSP_USER_DIR` set, so they stay reproducible and
//! leave the player's games alone.

use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use bevy::{ecs::system::SystemParam, prelude::*};
use osp_sim::save::{self, FORMAT, Header};
use osp_ui::{ActionState, LayoutVisibility, UiAction, UiData};

use crate::{
    FrameSet,
    camera::CameraRig,
    content::{GameContent, ModSwitch},
    editing::{EditorStatus, LastLaunch},
    menu::Menu,
    sim::{Sim, SimRestarted, TimeWarp},
};

const EXTENSION: &str = "save";
/// The longest first line of a save the list will read, bytes.
const LONGEST_HEADER: u64 = 64 * 1024;
/// A save's quicksaves are `quicksave_<n>` in its folder, called `Quicksave <n>`.
const QUICK_FILE: &str = "quicksave_";
const QUICK_NAME: &str = "Quicksave ";
/// How long the quickload key is held before it loads, real seconds: a slip of the finger
/// loses nothing.
pub const QUICKLOAD_HOLD: f64 = 1.0;
/// How long what a quicksave or quickload says stays up, real seconds.
const FLASH_SECONDS: f64 = 3.0;

pub struct SavesPlugin;

impl Plugin for SavesPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Saves::new(saves_dir()))
            .add_systems(Update, follow_mods.before(FrameSet::Input))
            .add_systems(Update, save_actions.in_set(FrameSet::Input))
            .add_systems(Update, publish_flash.in_set(FrameSet::Render));
    }
}

/// Where saves go; `None` when they are off.
fn saves_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("OSP_USER_DIR") {
        return Some(PathBuf::from(dir).join("saves"));
    }
    if std::env::var_os("OSP_CAPTURE").is_some() {
        return None;
    }
    Some(crate::settings::settings_file()?.with_file_name("saves"))
}

/// A save in the list.
pub struct SaveEntry {
    /// Its file name without the extension, after its save's and a `/` for a quicksave.
    pub file: String,
    /// What it says it is; `None` if it is not a save this game can read.
    pub header: Option<Header>,
    /// Why it cannot be loaded.
    pub problem: Option<String>,
    /// A save's own quicksaves, newest first; none in a quicksave.
    pub quicksaves: Vec<SaveEntry>,
}

#[derive(Resource)]
pub struct Saves {
    dir: Option<PathBuf>,
    /// Newest first, as of the last look.
    pub list: Vec<SaveEntry>,
    /// The save the game in progress goes to: its file and name.
    current: Option<(String, String)>,
    /// What that save holds (`save::fingerprint`), to tell if the game has moved on from it.
    saved: Option<u64>,
    /// What just happened, for the menu.
    pub notice: String,
    /// What a quicksave or quickload just said over the flight, and the real time it did.
    flash: Option<(String, f64)>,
    /// The mods loaded now, which a save written now is played with.
    mods: Vec<String>,
    /// The mods there are to load.
    installed: Vec<String>,
}

impl Saves {
    fn new(dir: Option<PathBuf>) -> Self {
        let mut saves = Self {
            dir,
            list: Vec::new(),
            current: None,
            saved: None,
            notice: String::new(),
            flash: None,
            mods: Vec::new(),
            installed: Vec::new(),
        };
        saves.refresh();
        saves
    }

    /// Look at what is in the folder.
    pub fn refresh(&mut self) {
        self.list = self.dir.as_deref().map(read_list).unwrap_or_default();
        let installed = &self.installed;
        for game in &mut self.list {
            note_missing_mod(game, installed);
            for quick in &mut game.quicksaves {
                note_missing_mod(quick, installed);
            }
        }
    }

    /// The game in progress is a new one, with no save yet.
    pub fn forget_current(&mut self) {
        self.current = None;
        self.saved = None;
    }

    /// The save holds the game exactly as it is now.
    pub fn up_to_date(&self, sim: &Sim) -> bool {
        self.saved.is_some() && save::fingerprint(&sim.universe).ok() == self.saved
    }

    /// The newest save this game can read.
    pub fn newest(&self) -> Option<&SaveEntry> {
        self.list.iter().find(|entry| entry.problem.is_none())
    }

    /// The save or quicksave `file` in the list.
    fn entry(&self, file: &str) -> Option<&SaveEntry> {
        (self.list.iter())
            .flat_map(|entry| std::iter::once(entry).chain(&entry.quicksaves))
            .find(|entry| entry.file == file)
    }

    /// What the save or quicksave `file` is called in the list.
    pub fn name_of<'a>(&'a self, file: &'a str) -> &'a str {
        (self.entry(file))
            .and_then(|entry| entry.header.as_ref())
            .map_or(file, |header| header.name.as_str())
    }

    /// How many quicksaves the save `file` holds.
    pub fn quicksave_count(&self, file: &str) -> usize {
        self.entry(file).map_or(0, |entry| entry.quicksaves.len())
    }

    /// The newest quicksave this game can read in the save the game in progress goes to.
    pub fn newest_quicksave(&self) -> Option<&SaveEntry> {
        let game = self.entry(self.current_file()?)?;
        game.quicksaves.iter().find(|entry| entry.problem.is_none())
    }

    /// The name the game in progress is saved under, if it has been.
    pub fn current_name(&self) -> Option<&str> {
        self.current.as_ref().map(|(_, name)| name.as_str())
    }

    /// The file of the save the game in progress goes to, if it has one.
    pub fn current_file(&self) -> Option<&str> {
        self.current.as_ref().map(|(file, _)| file.as_str())
    }

    /// What a new save would be called.
    pub fn next_name(&self) -> String {
        next_save(&self.list).1
    }

    /// Write the game in progress to its save, starting one if it has none, and say so. False if
    /// it could not be written.
    pub fn save(&mut self, sim: &Sim) -> bool {
        let (file, name) = match self.current.clone() {
            Some(current) => current,
            None => next_save(&self.list),
        };
        match self.write(&file, &name, sim) {
            Ok(()) => {
                info!("saved `{name}`");
                self.notice = format!("Saved as {name}");
                self.current = Some((file, name));
                self.saved = save::fingerprint(&sim.universe).ok();
                self.refresh();
                true
            }
            Err(e) => {
                error!("cannot save `{name}`: {e}");
                self.notice = format!("Cannot save: {e}");
                false
            }
        }
    }

    /// Write it to a new save, which becomes the one it goes to.
    fn save_as_new(&mut self, sim: &Sim) -> bool {
        self.refresh();
        let to = next_save(&self.list);
        self.save_to(to, sim)
    }

    /// Write it over the save `file`, under that save's name, which becomes the one it goes to.
    fn save_over(&mut self, file: &str, sim: &Sim) -> bool {
        let name = self.name_of(file).to_owned();
        self.save_to((file.to_owned(), name), sim)
    }

    /// Write it to the save `to` (file and name), which becomes the one it goes to; if it cannot be
    /// written, the game goes on going to the save it went to.
    fn save_to(&mut self, to: (String, String), sim: &Sim) -> bool {
        let before = (self.current.replace(to), self.saved);
        let saved = self.save(sim);
        if !saved {
            (self.current, self.saved) = before;
        }
        saved
    }

    /// Write the game in progress to a new quicksave in the save it goes to, starting that save if
    /// it has none. The quicksave's name, or why it could not be written.
    fn quicksave(&mut self, sim: &Sim) -> Result<String, String> {
        if self.current.is_none() && !self.save(sim) {
            return Err(self.notice.clone());
        }
        let game = self
            .current_file()
            .ok_or("the game has no save")?
            .to_owned();
        // From the folder itself: the list may not have it, if its save went from under the game.
        let folder = self.path_of(&game)?.with_extension("");
        let there = read_folder(&folder, &format!("{game}/"));
        let (file, name) = next_numbered(&there, &format!("{game}/{QUICK_FILE}"), QUICK_NAME);
        self.write(&file, &name, sim)?;
        info!("quicksaved `{file}`");
        self.refresh();
        Ok(name)
    }

    /// Where the save or quicksave `file` is. Only names the list could give are taken, so an
    /// action cannot reach outside the saves.
    fn path_of(&self, file: &str) -> Result<PathBuf, String> {
        let dir = self.dir.as_ref().ok_or("saving is off")?;
        let fine =
            |part: &str| !part.is_empty() && part != "." && part != ".." && !part.contains('\\');
        let parts: Vec<_> = file.split('/').collect();
        if parts.len() > 2 || !parts.iter().all(|part| fine(part)) {
            return Err(format!("`{file}` is not a save"));
        }
        Ok(dir.join(format!("{file}.{EXTENSION}")))
    }

    /// In one step as far as the folder goes, so a crash leaves the old save or the new one.
    fn write(&self, file: &str, name: &str, sim: &Sim) -> Result<(), String> {
        let path = self.path_of(file)?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH);
        let mut header = Header::new(name, now.map_or(0, |t| t.as_secs()), &sim.universe);
        header.mods = Some(self.mods.clone());
        let bytes = save::write(&header, &sim.universe)?;
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        }
        let writing = path.with_extension("saving");
        std::fs::write(&writing, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&writing, &path).map_err(|e| e.to_string())
    }

    /// Carry on from the save or quicksave `file`; nothing changes if it cannot be loaded. After a
    /// quicksave the game goes to the save it is in, which does not hold what was loaded. The mods
    /// it was played with, if it says.
    fn load(&mut self, file: &str, sim: &mut Sim) -> Result<Option<Vec<String>>, String> {
        let path = self.path_of(file)?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let header = save::read_header(&bytes)?;
        if let Some(id) = missing_mod(&header, &self.installed) {
            return Err(format!("it needs the mod `{id}`, which is not installed"));
        }
        sim.universe.restore(save::read(&bytes)?)?;
        let mods = header.mods.clone();
        match file.split_once('/') {
            Some((game, _)) => {
                self.current = Some((game.to_owned(), self.name_of(game).to_owned()));
                self.saved = None;
            }
            None => {
                self.current = Some((file.to_owned(), header.name));
                self.saved = save::fingerprint(&sim.universe).ok();
            }
        }
        Ok(mods)
    }

    /// Delete the save or quicksave `file`; a save's quicksaves go with it.
    fn delete(&mut self, file: &str) -> Result<(), String> {
        let path = self.path_of(file)?;
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        if file.contains('/') {
            // The folder goes with its last quicksave; one with others in stays.
            if let Some(folder) = path.parent() {
                let _ = std::fs::remove_dir(folder);
            }
            return Ok(());
        }
        let folder = path.with_extension("");
        if folder.is_dir() {
            std::fs::remove_dir_all(&folder).map_err(|e| e.to_string())?;
        }
        // The game in progress goes on, and starts a new save if it is saved again.
        if self
            .current
            .as_ref()
            .is_some_and(|(current, _)| current == file)
        {
            self.forget_current();
        }
        Ok(())
    }

    /// Say this over the flight for a few seconds, from the real time `now`.
    fn flash(&mut self, text: String, now: f64) {
        self.flash = Some((text, now));
    }
}

/// The first mod a save was played with that is not installed.
fn missing_mod<'a>(header: &'a Header, installed: &[String]) -> Option<&'a str> {
    let mods = header.mods.as_deref().unwrap_or_default();
    let missing = mods.iter().find(|id| !installed.contains(id));
    missing.map(String::as_str)
}

/// Say so in the list if a save cannot be loaded for want of a mod.
fn note_missing_mod(entry: &mut SaveEntry, installed: &[String]) {
    if entry.problem.is_none() {
        let header = entry.header.as_ref();
        entry.problem = header
            .and_then(|header| missing_mod(header, installed))
            .map(|id| format!("Needs the mod {id}"));
    }
}

/// What the list says about the first line of a save, called `<prefix><its file name>`.
fn read_entry(path: &Path, prefix: &str) -> Option<SaveEntry> {
    let file = format!("{prefix}{}", path.file_stem()?.to_string_lossy());
    let line = std::fs::File::open(path).and_then(|opened| {
        let mut line = Vec::new();
        let mut reader = BufReader::new(std::io::Read::take(opened, LONGEST_HEADER));
        reader.read_until(b'\n', &mut line)?;
        Ok(line)
    });
    let header = line
        .map_err(|e| e.to_string())
        .and_then(|line| save::read_header(&line));
    let (header, problem) = match header {
        Ok(header) if header.format == FORMAT => (Some(header), None),
        Ok(header) => (Some(header), Some("Saved by another version".to_owned())),
        Err(_) => (None, Some("Not a save".to_owned())),
    };
    Some(SaveEntry {
        file,
        header,
        problem,
        quicksaves: Vec::new(),
    })
}

/// The saves in `dir`, newest first, each with its quicksaves.
fn read_list(dir: &Path) -> Vec<SaveEntry> {
    let mut list = read_folder(dir, "");
    for entry in &mut list {
        entry.quicksaves = read_folder(&dir.join(&entry.file), &format!("{}/", entry.file));
    }
    list
}

/// The saves in `folder`, newest first, called `<prefix><file name>`. Two saved in the same
/// second go by the number they end in, the higher first.
fn read_folder(folder: &Path, prefix: &str) -> Vec<SaveEntry> {
    let Ok(files) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut list: Vec<_> = files
        .filter_map(|file| Some(file.ok()?.path()))
        .filter(|path| path.is_file() && path.extension().is_some_and(|e| e == EXTENSION))
        .filter_map(|path| read_entry(&path, prefix))
        .collect();
    let order = |entry: &SaveEntry| {
        let saved_at = entry.header.as_ref().map_or(0, |h| h.saved_at);
        let number = entry.file.rsplit('_').next().and_then(|n| n.parse().ok());
        (saved_at, number.unwrap_or(0u32))
    };
    list.sort_by_key(|entry| std::cmp::Reverse(order(entry)));
    list
}

/// The file and name of a new save: the first `Game <n>` after any there are.
fn next_save(list: &[SaveEntry]) -> (String, String) {
    next_numbered(list, "game_", "Game ")
}

/// The first `<file><n>`, `<name><n>` after any in `list`, by file or by name.
fn next_numbered(list: &[SaveEntry], file: &str, name: &str) -> (String, String) {
    let number_in = |text: &str, prefix: &str| text.strip_prefix(prefix)?.parse::<u32>().ok();
    let highest = list.iter().filter_map(|entry| {
        let named = entry.header.as_ref().map(|h| h.name.as_str());
        let filed = number_in(&entry.file, file);
        filed.max(named.and_then(|named| number_in(named, name)))
    });
    let next = highest.max().unwrap_or(0) + 1;
    (format!("{file}{next}"), format!("{name}{next}"))
}

/// How long ago `then` was, seconds since the Unix epoch, as the list says it.
pub fn ago(then: u64, now: u64) -> String {
    let seconds = now.saturating_sub(then);
    let count = |n: u64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match seconds {
        0..60 => "Just now".to_owned(),
        60..3_600 => count(seconds / 60, "minute"),
        3_600..86_400 => count(seconds / 3_600, "hour"),
        _ => count(seconds / 86_400, "day"),
    }
}

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH);
    now.map_or(0, |t| t.as_secs())
}

/// What carrying on from a save changes, besides the universe.
#[derive(SystemParam)]
struct Carry<'w> {
    menu: ResMut<'w, Menu>,
    warp: ResMut<'w, TimeWarp>,
    rig: ResMut<'w, CameraRig>,
    ui: ResMut<'w, LayoutVisibility>,
    last_launch: ResMut<'w, LastLaunch>,
    restarted: MessageWriter<'w, SimRestarted>,
    mods: ModSwitch<'w>,
}

impl Carry<'_> {
    /// Carry on flying from the save or quicksave `file`, out of the menu; if it cannot be loaded
    /// nothing changes and the error says why.
    fn load(&mut self, saves: &mut Saves, file: &str, sim: &mut Sim) -> Result<(), String> {
        let mods = match saves.load(file, sim) {
            Ok(mods) => mods,
            Err(e) => {
                error!("cannot load `{file}`: {e}");
                return Err(e);
            }
        };
        info!("loaded `{file}`");
        // Before anything is drawn, so that the vessels find their models.
        self.mods.carry_on_with(mods.as_deref());
        saves.notice.clear();
        self.last_launch.forget();
        sim.restarted();
        *self.warp = TimeWarp::default();
        self.rig.map_view = false;
        self.rig.follow = None;
        self.rig.frame(sim.active_vessel().propulsion.length());
        self.restarted.write(SimRestarted);
        self.menu.playing = true;
        self.menu.back_to_root();
        self.menu.show(false, sim, &mut self.ui);
        Ok(())
    }
}

/// Keep up with which mods are loaded and installed.
fn follow_mods(content: Res<GameContent>, mut saves: ResMut<Saves>) {
    if content.is_changed() {
        saves.mods = content.0.added_mods();
        saves.installed = (content.0.available.iter()).map(|m| m.id.clone()).collect();
        saves.refresh();
    }
}

/// Where holding the quickload key has got to.
#[derive(Default)]
struct Hold {
    /// The real time it went down.
    since: Option<f64>,
    /// It has loaded, and does nothing more until it is let go.
    done: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum HoldStep {
    Nothing,
    /// Just pressed: say what holding it will do.
    Began,
    /// Held long enough: load.
    Due,
}

impl Hold {
    fn step(&mut self, held: bool, now: f64) -> HoldStep {
        if !held {
            *self = Self::default();
            return HoldStep::Nothing;
        }
        match self.since {
            None => {
                self.since = Some(now);
                HoldStep::Began
            }
            Some(since) if !self.done && now - since >= QUICKLOAD_HOLD => {
                self.done = true;
                HoldStep::Due
            }
            Some(_) => HoldStep::Nothing,
        }
    }
}

/// Why quicksaves cannot be made or loaded now, if they cannot: only in flight, in a game.
fn not_quick(menu: &Menu, editor: &EditorStatus, sim: &Sim) -> Option<&'static str> {
    let tutorial = (sim.universe.tutorial.as_ref()).is_some_and(|tutorial| tutorial.is_running());
    if !menu.playing || menu.open || editor.is_open() {
        Some("")
    } else if tutorial {
        Some("in a tutorial")
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn save_actions(
    mut actions: MessageReader<UiAction>,
    mut saves: ResMut<Saves>,
    mut sim: ResMut<Sim>,
    mut carry: Carry,
    editor: Res<EditorStatus>,
    keys: Res<ActionState>,
    real: Res<Time<Real>>,
    mut hold: Local<Hold>,
) {
    let now = real.elapsed_secs_f64();
    for action in actions.read() {
        match action.name() {
            "game.save" if carry.menu.playing => {
                saves.save(&sim);
            }
            "game.save_as" | "game.save_to" if carry.menu.playing => {
                let saved = match action.arg() {
                    Some(file) if action.name() == "game.save_to" => saves.save_over(file, &sim),
                    _ => saves.save_as_new(&sim),
                };
                if saved && carry.menu.screen() == "save" {
                    carry.menu.pop();
                }
            }
            "game.load" | "game.continue" => {
                let file = match action.name() {
                    "game.load" => action.arg().map(str::to_owned),
                    _ => saves.newest().map(|entry| entry.file.clone()),
                };
                let Some(file) = file else { continue };
                if let Err(e) = carry.load(&mut saves, &file, &mut sim) {
                    saves.notice = format!("Cannot load it: {e}");
                }
            }
            "game.quicksave" => match not_quick(&carry.menu, &editor, &sim) {
                Some("") => {}
                Some(why) => saves.flash(format!("Cannot quicksave {why}"), now),
                None => {
                    let said = match saves.quicksave(&sim) {
                        Ok(name) => format!("Saved as {name}"),
                        Err(e) => format!("Cannot quicksave: {e}"),
                    };
                    saves.flash(said, now);
                }
            },
            // From a button or a script; the key has to be held (below).
            "game.quickload" => quickload(&mut saves, &mut carry, &editor, &mut sim, now),
            "game.delete_save" => {
                let Some(file) = action.arg() else { continue };
                if let Err(e) = saves.delete(file) {
                    error!("cannot delete `{file}`: {e}");
                    saves.notice = format!("Cannot delete it: {e}");
                }
                saves.refresh();
            }
            _ => {}
        }
    }

    match hold.step(keys.held("game.quickload"), now) {
        HoldStep::Nothing => {}
        HoldStep::Began => match not_quick(&carry.menu, &editor, &sim) {
            Some("") => {}
            Some(why) => saves.flash(format!("Cannot quickload {why}"), now),
            None => {
                saves.refresh();
                let said = match saves.newest_quicksave() {
                    Some(quick) => format!("Keep holding to load {}", saves.name_of(&quick.file)),
                    None => "No quicksave in this game yet".to_owned(),
                };
                saves.flash(said, now);
            }
        },
        HoldStep::Due => quickload(&mut saves, &mut carry, &editor, &mut sim, now),
    }
}

/// Carry on from the newest quicksave of the game in progress, saying so over the flight.
fn quickload(saves: &mut Saves, carry: &mut Carry, editor: &EditorStatus, sim: &mut Sim, now: f64) {
    match not_quick(&carry.menu, editor, sim) {
        Some("") => return,
        Some(why) => return saves.flash(format!("Cannot quickload {why}"), now),
        None => {}
    }
    saves.refresh();
    let Some(quick) = saves.newest_quicksave() else {
        return saves.flash("No quicksave in this game yet".to_owned(), now);
    };
    let (file, name) = (quick.file.clone(), saves.name_of(&quick.file).to_owned());
    let said = match carry.load(saves, &file, sim) {
        Ok(()) => format!("Loaded {name}"),
        Err(e) => format!("Cannot load {name}: {e}"),
    };
    saves.flash(said, now);
}

/// `saves.flash`: what a quicksave or quickload just said, for a few seconds (`messages.json5`).
fn publish_flash(saves: Res<Saves>, real: Res<Time<Real>>, mut data: ResMut<UiData>) {
    let now = real.elapsed_secs_f64();
    let flash = (saves.flash.as_ref())
        .filter(|(_, at)| now - at < FLASH_SECONDS)
        .map_or("", |(text, _)| text.as_str());
    data.set("saves.flash", flash);
}

#[cfg(test)]
mod tests {
    use osp_sim::time::SimTime;

    use super::*;

    fn entry(file: &str, name: &str) -> SaveEntry {
        SaveEntry {
            file: file.to_owned(),
            header: Some(Header {
                format: FORMAT,
                name: name.to_owned(),
                saved_at: 0,
                time: SimTime::J2000,
                flying: String::new(),
                vessels: 0,
                mods: None,
            }),
            problem: None,
            quicksaves: Vec::new(),
        }
    }

    /// A save's first line in `dir/<file>.save`, saved at `saved_at`.
    fn write_header(dir: &Path, file: &str, name: &str, saved_at: u64) {
        let mut header = entry("", name).header.unwrap();
        header.saved_at = saved_at;
        let mut bytes = serde_json::to_vec(&header).unwrap();
        bytes.push(b'\n');
        let path = dir.join(format!("{file}.{EXTENSION}"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn empty_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("osp_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_new_save_comes_after_those_there_are() {
        assert_eq!(next_save(&[]), ("game_1".to_owned(), "Game 1".to_owned()));
        let list = [entry("game_2", "Game 2"), entry("game_7", "Game 7")];
        assert_eq!(next_save(&list), ("game_8".to_owned(), "Game 8".to_owned()));
        // A file renamed by hand is not overwritten, whatever it says inside.
        let list = [entry("game_4", "Moon base")];
        assert_eq!(next_save(&list).0, "game_5");
        let list = [entry("moon", "Game 9")];
        assert_eq!(next_save(&list).0, "game_10");
    }

    #[test]
    fn how_long_ago_reads_naturally() {
        assert_eq!(ago(1_000, 1_030), "Just now");
        assert_eq!(ago(0, 60), "1 minute ago");
        assert_eq!(ago(0, 7_200), "2 hours ago");
        assert_eq!(ago(0, 86_400 * 3), "3 days ago");
        // A clock put back does not make a save from the future.
        assert_eq!(ago(100, 50), "Just now");
    }

    #[test]
    fn what_is_in_the_folder_is_listed_newest_first_and_what_is_not_a_save_says_so() {
        let dir = std::env::temp_dir().join(format!("osp_saves_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |file: &str, header: &mut Header, tail: &[u8]| {
            let mut bytes = serde_json::to_vec(header).unwrap();
            bytes.push(b'\n');
            bytes.extend(tail);
            std::fs::write(dir.join(file), bytes).unwrap();
        };
        let mut old = entry("", "Old").header.unwrap();
        old.saved_at = 100;
        write("old.save", &mut old, b"whatever comes next is not read");
        let mut new = entry("", "New").header.unwrap();
        new.saved_at = 200;
        write("new.save", &mut new, b"");
        let mut future = entry("", "Future").header.unwrap();
        future.format = FORMAT + 1;
        write("future.save", &mut future, b"");
        std::fs::write(dir.join("broken.save"), b"not json at all").unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a save and not listed").unwrap();

        let list = read_list(&dir);
        let names: Vec<_> = list.iter().map(|e| e.file.as_str()).collect();
        assert_eq!(names.len(), 4);
        assert_eq!(&names[..2], ["new", "old"]);
        let of = |file: &str| list.iter().find(|e| e.file == file).unwrap();
        assert!(of("new").problem.is_none());
        assert_eq!(
            of("future").problem.as_deref(),
            Some("Saved by another version")
        );
        assert_eq!(of("broken").problem.as_deref(), Some("Not a save"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn continuing_takes_the_newest_this_game_can_read() {
        let mut saves = Saves::new(None);
        assert!(saves.newest().is_none());
        let mut broken = entry("broken", "Broken");
        broken.problem = Some("Not a save".to_owned());
        saves.list = vec![broken, entry("game_2", "Game 2"), entry("game_1", "Game 1")];
        assert_eq!(saves.newest().map(|e| e.file.as_str()), Some("game_2"));
        assert_eq!(saves.name_of("game_1"), "Game 1");
        assert_eq!(saves.name_of("gone"), "gone");
    }

    #[test]
    fn a_save_lists_its_own_quicksaves_newest_first() {
        let dir = empty_dir("quicksaves");
        write_header(&dir, "game_1", "Game 1", 100);
        write_header(&dir, "game_1/quicksave_1", "Quicksave 1", 300);
        // In the same second: the higher number is the newer.
        write_header(&dir, "game_1/quicksave_2", "Quicksave 2", 300);
        write_header(&dir, "game_1/quicksave_3", "Quicksave 3", 200);
        std::fs::write(dir.join("game_1/notes.txt"), b"not listed").unwrap();
        // Quicksaves with no save of their own are not a game.
        write_header(&dir, "gone/quicksave_1", "Quicksave 1", 400);
        write_header(&dir, "game_2", "Game 2", 50);

        let mut saves = Saves::new(Some(dir.clone()));
        let files: Vec<_> = saves.list.iter().map(|e| e.file.as_str()).collect();
        assert_eq!(files, ["game_1", "game_2"]);
        let quick: Vec<_> = (saves.list[0].quicksaves.iter())
            .map(|e| e.file.as_str())
            .collect();
        assert_eq!(
            quick,
            [
                "game_1/quicksave_2",
                "game_1/quicksave_1",
                "game_1/quicksave_3"
            ]
        );
        assert!(saves.list[1].quicksaves.is_empty());
        assert_eq!(saves.quicksave_count("game_1"), 3);
        assert_eq!(saves.name_of("game_1/quicksave_3"), "Quicksave 3");

        // Quickloading takes the newest of the game in progress, and none without a save.
        assert!(saves.newest_quicksave().is_none());
        saves.current = Some(("game_2".into(), "Game 2".into()));
        assert!(saves.newest_quicksave().is_none());
        saves.current = Some(("game_1".into(), "Game 1".into()));
        let newest = saves.newest_quicksave().map(|e| e.file.as_str());
        assert_eq!(newest, Some("game_1/quicksave_2"));
        let next = next_numbered(&saves.list[0].quicksaves, "game_1/quicksave_", QUICK_NAME);
        assert_eq!(next, ("game_1/quicksave_4".into(), "Quicksave 4".into()));
        // Continuing is from the newest save, not a quicksave in it.
        assert_eq!(saves.newest().map(|e| e.file.as_str()), Some("game_1"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn deleting_a_save_takes_its_quicksaves_and_a_quicksave_only_itself() {
        let dir = empty_dir("delete_quicksaves");
        write_header(&dir, "game_1", "Game 1", 100);
        write_header(&dir, "game_1/quicksave_1", "Quicksave 1", 200);
        write_header(&dir, "game_1/quicksave_2", "Quicksave 2", 300);
        write_header(&dir, "game_2", "Game 2", 100);
        write_header(&dir, "game_2/quicksave_1", "Quicksave 1", 200);
        let mut saves = Saves::new(Some(dir.clone()));
        saves.current = Some(("game_1".into(), "Game 1".into()));

        saves.delete("game_1/quicksave_2").unwrap();
        saves.refresh();
        assert_eq!(saves.quicksave_count("game_1"), 1);
        assert_eq!(
            saves.current_file(),
            Some("game_1"),
            "the game still goes to its save"
        );
        saves.delete("game_1/quicksave_1").unwrap();
        assert!(!dir.join("game_1").exists(), "an empty folder goes too");
        assert!(dir.join("game_1.save").exists());

        saves.delete("game_2").unwrap();
        assert!(!dir.join("game_2").exists());
        assert!(!dir.join("game_2.save").exists());
        saves.delete("game_1").unwrap();
        assert_eq!(saves.current_file(), None);
        saves.refresh();
        assert!(saves.list.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_a_save_or_a_quicksave_in_one_can_be_named() {
        let saves = Saves::new(Some(PathBuf::from("/saves")));
        assert!(saves.path_of("game_1").is_ok());
        assert_eq!(
            saves.path_of("game_1/quicksave_2").unwrap(),
            Path::new("/saves/game_1/quicksave_2.save")
        );
        for file in [
            "",
            "../settings",
            "game_1/..",
            "a/b/c",
            "game_1/",
            "/etc/x",
            "a\\b",
            ".",
        ] {
            assert!(saves.path_of(file).is_err(), "{file}");
        }
        assert!(Saves::new(None).path_of("game_1").is_err());
    }

    #[test]
    fn quickloading_waits_for_the_key_to_be_held_and_loads_once() {
        let mut hold = Hold::default();
        assert_eq!(hold.step(false, 0.0), HoldStep::Nothing);
        assert_eq!(hold.step(true, 1.0), HoldStep::Began);
        assert_eq!(hold.step(true, 1.5), HoldStep::Nothing);
        // Let go too soon: nothing, and the next press starts again.
        assert_eq!(hold.step(false, 1.6), HoldStep::Nothing);
        assert_eq!(hold.step(true, 2.0), HoldStep::Began);
        assert_eq!(
            hold.step(true, 2.0 + QUICKLOAD_HOLD - 0.01),
            HoldStep::Nothing
        );
        assert_eq!(hold.step(true, 2.0 + QUICKLOAD_HOLD), HoldStep::Due);
        // Held on, it does not load again.
        assert_eq!(hold.step(true, 10.0), HoldStep::Nothing);
        assert_eq!(hold.step(false, 10.1), HoldStep::Nothing);
        assert_eq!(hold.step(true, 10.2), HoldStep::Began);
    }

    #[test]
    fn a_save_needing_a_mod_that_is_not_installed_says_which() {
        let dir = empty_dir("missing_mod");
        // Saved before saves said which mods they had: it is taken as it comes.
        write_header(&dir, "old", "Old", 400);
        let with_mod = |file: &str, name: &str, saved_at: u64, mods: &[&str]| {
            let mut header = entry("", name).header.unwrap();
            (header.saved_at, header.mods) = (
                saved_at,
                Some(mods.iter().map(|id| id.to_string()).collect()),
            );
            let mut bytes = serde_json::to_vec(&header).unwrap();
            bytes.push(b'\n');
            std::fs::create_dir_all(dir.join("gone")).unwrap();
            std::fs::write(dir.join(format!("{file}.{EXTENSION}")), bytes).unwrap();
        };
        with_mod("gone", "Gone", 300, &["realism", "deleted_mod"]);
        with_mod("gone/quicksave_1", "Quicksave 1", 300, &["deleted_mod"]);
        with_mod("fine", "Fine", 200, &["realism"]);

        let mut saves = Saves::new(Some(dir.clone()));
        saves.installed = vec!["realism".to_owned()];
        saves.refresh();
        let of = |file: &str| saves.entry(file).unwrap().problem.as_deref();
        assert_eq!(of("gone"), Some("Needs the mod deleted_mod"));
        assert_eq!(of("gone/quicksave_1"), Some("Needs the mod deleted_mod"));
        assert_eq!(of("fine"), None);
        assert_eq!(of("old"), None);

        // Continuing passes over it, and the mods say nothing about a save that is not one.
        saves.list.retain(|entry| entry.file != "old");
        assert_eq!(saves.newest().map(|e| e.file.as_str()), Some("fine"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_folder_is_just_no_saves() {
        assert!(read_list(Path::new("/nowhere/at/all")).is_empty());
    }
}
