//! The menu: the screens over the game, one at a time on the left of the window.
//!
//! Without `OSP_SCENARIO` the game opens on it, over the default scenario standing still; Esc
//! (`game.menu`) brings it back in flight. While it is open time stands still, the flight UI is
//! hidden and the pilot's controls do nothing.
//!
//! A screen is a layout `menu_<id>` in the group `menu` (see `layouts/menu/`). The root is `title`
//! with no game in progress and `pause` with one. `menu.open:<id>` goes to a screen on top of the
//! one showing, and `menu.back` or Esc goes back: from `pause` that resumes, from `title` it does
//! nothing. So a new screen is a new file and a row that opens it.
//!
//! `game.new` goes to the editor (see `editor`) to choose what to launch: nothing is lost until the
//! first launch from there, and going back drops it. `game.exit` leaves the game for the title
//! screen. Saves are in `saves`, and asking before anything is lost in `dialog`.

use bevy::prelude::*;
use osp_sim::{
    tutorial::TutorialDef,
    vessel::{Situation, Vessel},
};
use osp_ui::{LayoutVisibility, UiAction, UiData, UiDocuments, UiRecord, record};

use crate::{
    FrameSet,
    camera::CameraRig,
    content::GameContent,
    dialog::Dialog,
    editor::Editor,
    saves::{Saves, ago, now},
    sim::{DEFAULT_SCENARIO, Sim, SimRestarted, TimeWarp},
    ui_data::format_date,
};

/// The group of the menu's layouts.
const GROUP: &str = "menu";
/// The Settings screen's tabs start here (`ui.tab.settings`).
const FIRST_SETTINGS_TAB: &str = "graphics";

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        // Scripted captures, `OSP_SCENARIO` and `OSP_TUTORIAL` go straight into flight.
        let open = ["OSP_SCENARIO", "OSP_TUTORIAL", "OSP_CAPTURE"]
            .iter()
            .all(|var| std::env::var_os(var).is_none());
        app.insert_resource(Menu {
            open: false,
            playing: !open,
            stack: Vec::new(),
            new_game: false,
            notice: String::new(),
        })
        .add_systems(
            Startup,
            move |mut menu: ResMut<Menu>,
                  mut sim: ResMut<Sim>,
                  mut ui: ResMut<LayoutVisibility>,
                  mut data: ResMut<UiData>| {
                data.set("ui.tab.settings", FIRST_SETTINGS_TAB);
                if open {
                    menu.show(true, &mut sim, &mut ui);
                }
            },
        )
        .add_systems(
            Update,
            (
                publish_menu,
                menu_actions.in_set(FrameSet::Input),
                show_screen.after(menu_actions),
            ),
        );
    }
}

#[derive(Resource)]
pub struct Menu {
    pub open: bool,
    /// A game is in progress, so the menu can go back to it.
    pub playing: bool,
    /// The screens on top of the root, the last one showing.
    stack: Vec<String>,
    /// New game was picked: the next launch from the editor starts it, in place of the game in
    /// progress.
    pub new_game: bool,
    /// What just went wrong, for the screen showing.
    pub notice: String,
}

/// The layout of a screen.
fn layout_of(screen: &str) -> String {
    format!("{GROUP}_{screen}")
}

impl Menu {
    /// Open or close the menu: pause or run the simulation, and swap the menu for the
    /// flight UI. The screens stay as they are, for the editor to come back to.
    pub fn show(&mut self, open: bool, sim: &mut Sim, ui: &mut LayoutVisibility) {
        self.open = open;
        sim.paused = open;
        if open {
            ui.hidden_groups.insert("flight".into());
        } else {
            ui.hidden_groups.remove("flight");
        }
    }

    /// The screen showing while the menu is open.
    pub fn screen(&self) -> &str {
        self.stack.last().map_or(self.root(), String::as_str)
    }

    fn root(&self) -> &'static str {
        if self.playing { "pause" } else { "title" }
    }

    fn push(&mut self, screen: &str) {
        self.stack.push(screen.to_owned());
        self.notice.clear();
    }

    /// Back to the root, with nothing left of what the screens on it were doing.
    pub fn back_to_root(&mut self) {
        self.stack.clear();
        self.new_game = false;
        self.notice.clear();
    }

    /// One screen back. False when there is none to go back to.
    fn pop(&mut self) -> bool {
        self.notice.clear();
        self.stack.pop().is_some()
    }
}

/// Only the screen on top shows, and none while the menu is closed.
fn show_screen(menu: Res<Menu>, docs: Res<UiDocuments>, mut ui: ResMut<LayoutVisibility>) {
    if !menu.is_changed() && !docs.is_changed() {
        return;
    }
    let showing = menu.open.then(|| layout_of(menu.screen()));
    for layout in docs
        .layouts
        .iter()
        .filter(|l| l.group.as_deref() == Some(GROUP))
    {
        let show = showing.as_ref() == Some(&layout.id);
        if ui.shown(&layout.id) != show {
            ui.layouts.insert(layout.id.clone(), show);
        }
    }
}

/// What a vessel in the list says about where it is.
fn whereabouts(vessel: &Vessel, sim: &Sim) -> String {
    let system = &sim.universe.system;
    let body = &system.body(vessel.parent()).name;
    match vessel.situation(system, sim.render_time) {
        Situation::Landed => format!("Landed on {body}"),
        Situation::Flying => format!("Flying over {body}"),
        Situation::SubOrbital => format!("Sub-orbital over {body}"),
        Situation::Orbiting => format!("Orbiting {body}"),
        Situation::Escaping => format!("Leaving {body}"),
    }
}

/// `menu.vessels` (id, name, where, flying, state): what there is to fly in the game in progress.
fn vessel_list(sim: &Sim, menu: &Menu) -> Vec<UiRecord> {
    if !menu.playing {
        return Vec::new();
    }
    (sim.universe.vessels.iter())
        .filter(|vessel| !vessel.is_debris())
        .map(|vessel| {
            let flying = vessel.id() == sim.active_id();
            record([
                ("id", vessel.id().0.into()),
                ("name", vessel.name.as_str().into()),
                ("where", whereabouts(vessel, sim).into()),
                ("flying", flying.into()),
                ("state", if flying { "Flying now" } else { "" }.into()),
            ])
        })
        .collect()
}

/// A save's line in the list: what is flying, when in the game, and how many vessels.
fn save_detail(flying: &str, time: &str, vessels: usize) -> String {
    let vessels = match vessels {
        1 => "1 vessel".to_owned(),
        count => format!("{count} vessels"),
    };
    let parts = [flying, time, vessels.as_str()];
    let shown: Vec<_> = parts.into_iter().filter(|part| !part.is_empty()).collect();
    shown.join(", ")
}

/// `menu.saves` (file, name, saved, time, flying, vessels, problem, detail), newest first.
fn save_list(saves: &Saves) -> Vec<UiRecord> {
    let now = now();
    (saves.list.iter())
        .map(|entry| {
            let header = entry.header.as_ref();
            let name = header.map_or(entry.file.as_str(), |h| h.name.as_str());
            let saved = header.map_or(String::new(), |h| ago(h.saved_at, now));
            let time = header.map_or(String::new(), |h| format_date(h.time));
            let flying = header.map_or("", |h| h.flying.as_str());
            let vessels = header.map_or(0, |h| h.vessels);
            let problem = entry.problem.as_deref().unwrap_or("");
            let detail = match problem {
                "" => save_detail(flying, &time, vessels),
                problem => problem.to_owned(),
            };
            record([
                ("file", entry.file.as_str().into()),
                ("name", name.into()),
                ("saved", saved.into()),
                ("time", time.into()),
                ("flying", flying.into()),
                ("vessels", vessels.into()),
                ("problem", problem.into()),
                ("detail", detail.into()),
            ])
        })
        .collect()
}

/// `menu.screen`, `menu.playing`, `menu.new_game`, `menu.tutorials`, `menu.vessels`,
/// `menu.vessel_line`, `menu.saves`, `menu.has_saves`, `menu.can_continue`, `menu.continue_line`,
/// `menu.save_name`, `menu.game_line` and `menu.notice`. See `docs/modding.md`.
fn publish_menu(
    content: Res<GameContent>,
    menu: Res<Menu>,
    saves: Res<Saves>,
    sim: Res<Sim>,
    mut data: ResMut<UiData>,
) {
    if content.is_changed() {
        let tutorials: Vec<_> = (content
            .0
            .registry
            .all::<TutorialDef>("tutorials", &mut Vec::new()))
        .iter()
        .map(|t| {
            let name = if t.name.is_empty() { &t.id } else { &t.name };
            record([
                ("id", t.id.as_str().into()),
                ("name", name.as_str().into()),
                ("description", t.description.as_str().into()),
                ("steps", t.steps.len().into()),
            ])
        })
        .collect();
        data.set("menu.tutorials", tutorials);
    }
    if menu.is_changed() {
        data.set("menu.screen", menu.screen());
        data.set("menu.playing", menu.playing);
        data.set("menu.new_game", menu.new_game);
        let vessels = vessel_list(&sim, &menu);
        let line = match vessels.len() {
            1 => "1 vessel".to_owned(),
            count => format!("{count} vessels"),
        };
        data.set("menu.vessel_line", line);
        data.set("menu.vessels", vessels);
    }
    if menu.is_changed() || saves.is_changed() {
        data.set("menu.saves", save_list(&saves));
        data.set("menu.has_saves", !saves.list.is_empty());
        data.set("menu.can_continue", saves.newest().is_some());
        let header = saves.newest().and_then(|entry| entry.header.as_ref());
        let line = header.map_or(String::new(), |h| match h.flying.as_str() {
            "" => h.name.clone(),
            flying => format!("{}, {flying}", h.name),
        });
        data.set("menu.continue_line", line);
        data.set("menu.save_name", saves.current_name().unwrap_or(""));
        let flying = &sim.active_vessel().name;
        let line = match saves.current_name() {
            Some(save) => format!("{save}, flying {flying}"),
            None => format!("Flying {flying}"),
        };
        data.set("menu.game_line", line);
        let notice = if menu.notice.is_empty() {
            saves.notice.as_str()
        } else {
            menu.notice.as_str()
        };
        data.set("menu.notice", notice);
    }
}

/// Leave the game for the title screen, with the default scenario standing still behind it. If that
/// cannot start nothing changes, and the menu says why.
#[allow(clippy::too_many_arguments)]
pub(crate) fn go_to_title(
    menu: &mut Menu,
    sim: &mut Sim,
    warp: &mut TimeWarp,
    rig: &mut CameraRig,
    ui: &mut LayoutVisibility,
    saves: &mut Saves,
    content: &GameContent,
    restarted: &mut MessageWriter<SimRestarted>,
) {
    match crate::content::restart(&content.0, &mut sim.universe, DEFAULT_SCENARIO) {
        Ok(()) => {
            sim.restarted();
            *warp = TimeWarp::default();
            rig.map_view = false;
            rig.follow = None;
            restarted.write(SimRestarted);
            saves.forget_current();
            saves.notice.clear();
            menu.playing = false;
            menu.back_to_root();
            menu.show(true, sim, ui);
        }
        Err(e) => {
            error!("cannot leave the game: {e}");
            menu.notice = format!("Cannot leave the game: {e}");
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn menu_actions(
    mut actions: MessageReader<UiAction>,
    mut menu: ResMut<Menu>,
    mut sim: ResMut<Sim>,
    mut warp: ResMut<TimeWarp>,
    mut rig: ResMut<CameraRig>,
    content: Res<GameContent>,
    docs: Res<UiDocuments>,
    mut ui: ResMut<LayoutVisibility>,
    mut saves: ResMut<Saves>,
    editor: Res<Editor>,
    dialog: Res<Dialog>,
    mut restarted: MessageWriter<SimRestarted>,
    mut exit: MessageWriter<AppExit>,
) {
    for action in actions.read() {
        match action.name() {
            // Esc and Back close the question being asked, and nothing more.
            "game.menu" | "menu.back" if dialog.is_open() => {}
            // The editor sees to Esc and Back while it is open.
            "game.menu" | "menu.back" if editor.is_open() => {}
            // Esc closes what is open over the view before it opens the menu.
            "game.menu" if !menu.open && ui.dismiss_popups(&docs) => {}
            "game.menu" if !menu.open => {
                menu.back_to_root();
                saves.notice.clear();
                menu.show(true, &mut sim, &mut ui);
            }
            // Esc and the Back button are the same thing.
            "game.menu" | "menu.back" => {
                saves.notice.clear();
                if !menu.pop() && menu.playing {
                    menu.show(false, &mut sim, &mut ui);
                }
            }
            "game.resume" if menu.open && menu.playing => {
                menu.back_to_root();
                saves.notice.clear();
                menu.show(false, &mut sim, &mut ui);
            }
            "menu.open" => {
                let Some(screen) = action.arg() else { continue };
                let layout = layout_of(screen);
                if !docs.layouts.iter().any(|l| l.id == layout) {
                    warn!("no menu screen `{screen}`: no layout `{layout}`");
                    continue;
                }
                saves.notice.clear();
                if screen == "load" {
                    saves.refresh();
                }
                menu.push(screen);
            }
            // The editor opens on it too.
            "game.new" if !editor.is_open() => {
                saves.notice.clear();
                menu.notice.clear();
                menu.new_game = true;
            }
            // `targeting` does the flying.
            "vessel.fly" if menu.open => {
                menu.back_to_root();
                menu.show(false, &mut sim, &mut ui);
            }
            "game.exit" if menu.playing => {
                let (sim, warp, rig, ui) = (&mut *sim, &mut *warp, &mut *rig, &mut *ui);
                let restarted = &mut restarted;
                go_to_title(
                    &mut menu, sim, warp, rig, ui, &mut saves, &content, restarted,
                );
            }
            "game.start_tutorial" => {
                let Some(id) = action.arg() else { continue };
                match crate::content::restart_tutorial(&content.0, &mut sim.universe, id) {
                    Ok(()) => {
                        info!("started `{id}`");
                        sim.restarted();
                        *warp = TimeWarp::default();
                        rig.map_view = false;
                        restarted.write(SimRestarted);
                        saves.forget_current();
                        menu.back_to_root();
                        menu.playing = true;
                        menu.show(false, &mut sim, &mut ui);
                    }
                    Err(e) => error!("cannot start `{id}`: {e}"),
                }
            }
            "game.quit" => {
                exit.write(AppExit::Success);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(playing: bool) -> Menu {
        Menu {
            open: true,
            playing,
            stack: Vec::new(),
            new_game: false,
            notice: String::new(),
        }
    }

    #[test]
    fn the_root_is_the_pause_menu_with_a_game_and_the_title_without() {
        assert_eq!(menu(true).screen(), "pause");
        assert_eq!(menu(false).screen(), "title");
        assert_eq!(layout_of("pause"), "menu_pause");
    }

    #[test]
    fn going_back_walks_down_the_screens_and_then_says_there_are_no_more() {
        let mut menu = menu(true);
        menu.push("settings");
        menu.push("keys");
        assert_eq!(menu.screen(), "keys");
        assert!(menu.pop());
        assert_eq!(menu.screen(), "settings");
        assert!(menu.pop());
        assert_eq!(menu.screen(), "pause");
        assert!(!menu.pop());
        assert_eq!(menu.screen(), "pause");
    }

    #[test]
    fn going_to_the_root_forgets_the_screens_and_what_was_said() {
        let mut menu = menu(true);
        menu.push("load");
        menu.new_game = true;
        menu.notice = "Cannot load it".into();
        menu.back_to_root();
        assert_eq!(menu.screen(), "pause");
        assert!(!menu.new_game);
        assert!(menu.notice.is_empty());
    }

    #[test]
    fn a_save_says_what_it_knows() {
        assert_eq!(
            save_detail("Saturn V", "2000-01-01 18:00:12", 2),
            "Saturn V, 2000-01-01 18:00:12, 2 vessels"
        );
        assert_eq!(save_detail("", "2000-01-01", 1), "2000-01-01, 1 vessel");
    }
}
