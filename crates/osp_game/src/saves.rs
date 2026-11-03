//! Saved games on disk, and the actions on them: `game.save` writes the game in progress to its
//! save, `game.save_as` to a new one, `game.load:<file>` carries on from one, and
//! `game.delete_save:<file>` asks once and deletes one on the second go. A save starts with what
//! the list shows, so the list only reads that much of each (see `osp_sim::save`).
//!
//! Saves live in `saves/` in `OSP_USER_DIR`, or beside the settings file. Scripted captures
//! (`OSP_CAPTURE`) only touch them with `OSP_USER_DIR` set, so they stay reproducible and
//! leave the player's games alone.

use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use bevy::prelude::*;
use osp_sim::save::{self, FORMAT, Header};
use osp_ui::{LayoutVisibility, UiAction};

use crate::{
    FrameSet,
    camera::CameraRig,
    menu::{Menu, Play},
    sim::{Sim, SimRestarted, TimeWarp},
};

const EXTENSION: &str = "save";
/// The longest first line of a save the list will read, bytes.
const LONGEST_HEADER: u64 = 64 * 1024;

pub struct SavesPlugin;

impl Plugin for SavesPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Saves::new(saves_dir()))
            .add_systems(Update, save_actions.in_set(FrameSet::Input));
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
    /// Its file name without the extension.
    pub file: String,
    /// What it says it is; `None` if it is not a save this game can read.
    pub header: Option<Header>,
    /// Why it cannot be loaded.
    pub problem: Option<String>,
}

#[derive(Resource)]
pub struct Saves {
    dir: Option<PathBuf>,
    /// Newest first, as of the last look.
    pub list: Vec<SaveEntry>,
    /// The save the game in progress goes to: its file and name.
    current: Option<(String, String)>,
    /// The save that a second go at deleting deletes.
    pub deleting: Option<String>,
    /// What just happened, for the menu.
    pub notice: String,
}

impl Saves {
    fn new(dir: Option<PathBuf>) -> Self {
        let mut saves = Self {
            dir,
            list: Vec::new(),
            current: None,
            deleting: None,
            notice: String::new(),
        };
        saves.refresh();
        saves
    }

    /// Look at what is in the folder.
    pub fn refresh(&mut self) {
        self.list = self.dir.as_deref().map(read_list).unwrap_or_default();
        self.deleting = None;
    }

    /// The game in progress is a new one, with no save yet.
    pub fn forget_current(&mut self) {
        self.current = None;
    }

    /// The name the game in progress is saved under, if it has been.
    pub fn current_name(&self) -> Option<&str> {
        self.current.as_ref().map(|(_, name)| name.as_str())
    }

    /// Write the game in progress to its save, starting one if it has none, and say so.
    pub fn save(&mut self, sim: &Sim) {
        let (file, name) = match self.current.clone() {
            Some(current) => current,
            None => next_save(&self.list),
        };
        match self.write(&file, &name, sim) {
            Ok(()) => {
                info!("saved `{name}`");
                self.notice = format!("Saved as {name}");
                self.current = Some((file, name));
                self.refresh();
            }
            Err(e) => {
                error!("cannot save `{name}`: {e}");
                self.notice = format!("Cannot save: {e}");
            }
        }
    }

    /// Write it to a new save, which becomes the one it goes to.
    fn save_as_new(&mut self, sim: &Sim) {
        self.refresh();
        self.current = None;
        self.save(sim);
    }

    /// In one step as far as the folder goes, so a crash leaves the old save or the new one.
    fn write(&self, file: &str, name: &str, sim: &Sim) -> Result<(), String> {
        let dir = self.dir.as_ref().ok_or("saving is off")?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH);
        let header = Header::new(name, now.map_or(0, |t| t.as_secs()), &sim.universe);
        let bytes = save::write(&header, &sim.universe)?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("{file}.{EXTENSION}"));
        let writing = path.with_extension("saving");
        std::fs::write(&writing, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&writing, &path).map_err(|e| e.to_string())
    }

    /// Carry on from the save `file`; nothing changes if it cannot be loaded.
    fn load(&mut self, file: &str, sim: &mut Sim) -> Result<(), String> {
        let dir = self.dir.as_ref().ok_or("saving is off")?;
        let path = dir.join(format!("{file}.{EXTENSION}"));
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let header = save::read_header(&bytes)?;
        sim.universe.restore(save::read(&bytes)?)?;
        self.current = Some((file.to_owned(), header.name));
        Ok(())
    }

    fn delete(&mut self, file: &str) -> Result<(), String> {
        let dir = self.dir.as_ref().ok_or("saving is off")?;
        let path = dir.join(format!("{file}.{EXTENSION}"));
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        // The game in progress goes on, and starts a new save if it is saved again.
        if self
            .current
            .as_ref()
            .is_some_and(|(current, _)| current == file)
        {
            self.current = None;
        }
        Ok(())
    }
}

/// What the list says about the first line of a save.
fn read_entry(path: &Path) -> Option<SaveEntry> {
    let file = path.file_stem()?.to_string_lossy().into_owned();
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
    })
}

/// The saves in `dir`, newest first.
fn read_list(dir: &Path) -> Vec<SaveEntry> {
    let Ok(files) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut list: Vec<_> = files
        .filter_map(|file| Some(file.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|e| e == EXTENSION))
        .filter_map(|path| read_entry(&path))
        .collect();
    let saved_at = |entry: &SaveEntry| entry.header.as_ref().map_or(0, |h| h.saved_at);
    list.sort_by_key(|entry| std::cmp::Reverse(saved_at(entry)));
    list
}

/// The file and name of a new save: the first `Game <n>` after any there are.
fn next_save(list: &[SaveEntry]) -> (String, String) {
    let number_in = |text: &str, prefix: &str| text.strip_prefix(prefix)?.parse::<u32>().ok();
    let highest = list.iter().filter_map(|entry| {
        let name = entry.header.as_ref().map(|h| h.name.as_str());
        let file = number_in(&entry.file, "game_");
        file.max(name.and_then(|name| number_in(name, "Game ")))
    });
    let next = highest.max().unwrap_or(0) + 1;
    (format!("game_{next}"), format!("Game {next}"))
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

#[allow(clippy::too_many_arguments)]
fn save_actions(
    mut actions: MessageReader<UiAction>,
    mut saves: ResMut<Saves>,
    mut menu: ResMut<Menu>,
    mut sim: ResMut<Sim>,
    mut warp: ResMut<TimeWarp>,
    mut rig: ResMut<CameraRig>,
    mut ui: ResMut<LayoutVisibility>,
    mut restarted: MessageWriter<SimRestarted>,
) {
    for action in actions.read() {
        match action.name() {
            "game.save" if menu.playing => saves.save(&sim),
            "game.save_as" if menu.playing => saves.save_as_new(&sim),
            "game.load" => {
                let Some(file) = action.arg() else { continue };
                match saves.load(file, &mut sim) {
                    Ok(()) => {
                        info!("loaded `{file}`");
                        saves.notice.clear();
                        sim.restarted();
                        *warp = TimeWarp::default();
                        rig.map_view = false;
                        rig.follow = None;
                        rig.frame(sim.active_vessel().propulsion.length());
                        restarted.write(SimRestarted);
                        menu.playing = true;
                        menu.new_game = false;
                        menu.play = Play::Start;
                        menu.show(false, &mut sim, &mut ui);
                    }
                    Err(e) => {
                        error!("cannot load `{file}`: {e}");
                        saves.notice = format!("Cannot load it: {e}");
                    }
                }
            }
            "game.delete_save" => {
                let Some(file) = action.arg() else { continue };
                if saves.deleting.as_deref() == Some(file) {
                    if let Err(e) = saves.delete(file) {
                        error!("cannot delete `{file}`: {e}");
                        saves.notice = format!("Cannot delete it: {e}");
                    }
                    saves.refresh();
                } else {
                    saves.deleting = Some(file.to_owned());
                }
            }
            _ => {}
        }
    }
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
            }),
            problem: None,
        }
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
    fn a_missing_folder_is_just_no_saves() {
        assert!(read_list(Path::new("/nowhere/at/all")).is_empty());
    }
}
