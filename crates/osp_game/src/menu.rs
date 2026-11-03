//! The main menu: play, pick a scenario to fly, go back to the one in progress, or quit.
//!
//! Without `OSP_SCENARIO` the game opens on it, over the default scenario standing still;
//! Esc (`game.menu`) brings it back in flight. While it is open time stands still, the
//! flight UI is hidden and the pilot's controls do nothing.
//!
//! The Play page goes by steps (`menu.play:<step>`, see [`Play`]). A new game (`game.new`) or the
//! one in progress offers its vessels to fly (`game.fly:<id>`), and a new one to pick from the
//! craft (`editor.open:<id>`, see `editor`), which is how anything gets into the game; a new game
//! has no vessels, and nothing is lost until the first launch from it. Load game lists the saves
//! (see `saves`).
//!
//! A scenario is picked in two steps: the place it starts (`menu.place:<pad|runway|orbit|flight>`,
//! `menu.place` to go back), then the spacecraft there; that starts it afresh in the same solar
//! system (`game.start:<id>`). Its Tutorials page lists the guided runs
//! (`game.start_tutorial:<id>`, which start the tutorial's scenario with its steps), and its
//! Graphics page (`menu.page:graphics`) sets the graphics (see `settings`).

use bevy::prelude::*;
use osp_sim::{
    data::{LaunchSiteDef, ScenarioDef, StartDef},
    tutorial::TutorialDef,
    vessel::{Situation, Vessel, VesselId},
};
use osp_ui::{LayoutVisibility, UiAction, UiData, UiRecord, record};

use crate::{
    FrameSet,
    camera::CameraRig,
    content::GameContent,
    editor::Editor,
    saves::{Saves, ago, now},
    sim::{Sim, SimRestarted, TimeWarp},
    ui_data::format_date,
};

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
            page: "play".into(),
            place: String::new(),
            play: Play::Start,
            new_game: false,
            notice: String::new(),
        })
        .add_systems(
            Startup,
            move |mut menu: ResMut<Menu>,
                  mut sim: ResMut<Sim>,
                  mut ui: ResMut<LayoutVisibility>| {
                if open {
                    menu.show(true, &mut sim, &mut ui);
                }
            },
        )
        .add_systems(
            Update,
            (
                publish_scenarios,
                publish_play,
                menu_actions.in_set(FrameSet::Input),
            ),
        );
    }
}

/// Where the Play page has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Play {
    /// New game or load game, and with a game in progress its vessels and saving it.
    #[default]
    Start,
    /// The saves to load.
    Load,
    /// The vessels in the game to fly, or a new one.
    Vessels,
    /// The craft to pick a new vessel from.
    Craft,
}

impl Play {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "" => Some(Self::Start),
            "load" => Some(Self::Load),
            "vessels" => Some(Self::Vessels),
            "craft" => Some(Self::Craft),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Start => "",
            Self::Load => "load",
            Self::Vessels => "vessels",
            Self::Craft => "craft",
        }
    }
}

#[derive(Resource)]
pub struct Menu {
    pub open: bool,
    /// A scenario is being flown, so the menu can go back to it.
    pub playing: bool,
    /// The page showing: `play`, `scenarios`, `tutorials` or `graphics`.
    page: String,
    /// The place picked on the scenarios page (see `place`), or empty while picking one.
    place: String,
    /// How far the Play page has got.
    pub play: Play,
    /// New game was picked: the next launch starts it, in place of the game in progress.
    pub new_game: bool,
    /// What just went wrong, for the Play page.
    pub notice: String,
}

impl Menu {
    /// Open or close the menu: pause or run the simulation, and swap the menu for the
    /// flight UI.
    pub fn show(&mut self, open: bool, sim: &mut Sim, ui: &mut LayoutVisibility) {
        self.open = open;
        sim.paused = open;
        ui.layouts.insert("main_menu".into(), open);
        if open {
            ui.hidden_groups.insert("flight".into());
        } else {
            ui.hidden_groups.remove("flight");
        }
    }
}

/// Where a scenario starts, the first choice on the scenarios page.
fn place(start: &StartDef) -> &'static str {
    match start {
        StartDef::LaunchSite(_) => "pad",
        StartDef::Runway(_) => "runway",
        StartDef::Orbit(_) => "orbit",
        StartDef::Flight(_) => "flight",
    }
}

/// The line under a spacecraft saying where exactly it starts.
fn start_detail(start: &StartDef, sites: &[LaunchSiteDef]) -> String {
    let site = |id: &str| {
        sites
            .iter()
            .find(|s| s.id == id)
            .map_or(id.to_owned(), |s| s.display_name().to_owned())
    };
    match start {
        StartDef::LaunchSite(s) | StartDef::Runway(s) => site(&s.site),
        StartDef::Orbit(o) => {
            let (pe, ap) = (o.periapsis_altitude / 1e3, o.apoapsis_altitude / 1e3);
            if (ap - pe).abs() < 1.0 {
                format!("{pe:.0} km around {}", o.body)
            } else {
                format!("{pe:.0} × {ap:.0} km around {}", o.body)
            }
        }
        StartDef::Flight(f) => format!("{:.0} m over {}, {:.0} m/s", f.altitude, f.body, f.speed),
    }
}

/// `menu.count.<place>` (scenarios starting there), `menu.scenarios` (those at
/// `menu.place`, in the order the records load: id, craft, name, description, start),
/// `menu.tutorials` (id, name, description, steps), `menu.playing` and `menu.page`. A scenario's name is left out when it is the craft's.
fn publish_scenarios(content: Res<GameContent>, menu: Res<Menu>, mut data: ResMut<UiData>) {
    if content.is_changed() || menu.is_changed() {
        let registry = &content.0.registry;
        let all = registry.all::<ScenarioDef>("scenarios", &mut Vec::new());
        let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
        for id in ["pad", "runway", "orbit", "flight"] {
            let count = all.iter().filter(|s| place(&s.start) == id).count();
            data.set(format!("menu.count.{id}"), count);
        }
        let scenarios: Vec<_> = all
            .iter()
            .filter(|s| place(&s.start) == menu.place)
            .map(|s| {
                let craft = registry
                    .record("craft", &s.craft)
                    .and_then(|r| r.value.get("name")?.as_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or(&s.craft);
                let name = if s.name == craft { "" } else { s.name.as_str() };
                record([
                    ("id", s.id.as_str().into()),
                    ("craft", craft.into()),
                    ("name", name.into()),
                    ("description", s.description.as_str().into()),
                    ("start", start_detail(&s.start, &sites).into()),
                ])
            })
            .collect();
        data.set("menu.scenarios", scenarios);
        let tutorials: Vec<_> = (registry.all::<TutorialDef>("tutorials", &mut Vec::new()))
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
        data.set("menu.playing", menu.playing);
        data.set("menu.page", menu.page.as_str());
        data.set("menu.place", menu.place.as_str());
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

/// `menu.vessels` (id, name, where, flying): what there is to fly in the game in progress, or
/// nothing for a new one.
fn vessel_list(sim: &Sim, menu: &Menu) -> Vec<UiRecord> {
    if menu.new_game || !menu.playing {
        return Vec::new();
    }
    (sim.universe.vessels.iter())
        .filter(|vessel| !vessel.is_debris())
        .map(|vessel| {
            record([
                ("id", vessel.id().0.into()),
                ("name", vessel.name.as_str().into()),
                ("where", whereabouts(vessel, sim).into()),
                ("flying", (vessel.id() == sim.active_id()).into()),
            ])
        })
        .collect()
}

/// `menu.craft` (id, name, parts), by name: every craft there is to fly.
fn craft_list(content: &GameContent) -> Vec<UiRecord> {
    let mut craft: Vec<_> = (content.0.registry.records("craft"))
        .map(|r| {
            let name = (r.value.get("name").and_then(|n| n.as_str()))
                .filter(|name| !name.is_empty())
                .unwrap_or(&r.id);
            let parts = r.value.get("parts").and_then(|p| p.as_array());
            (name.to_owned(), r.id.clone(), parts.map_or(0, Vec::len))
        })
        .collect();
    craft.sort();
    craft
        .into_iter()
        .map(|(name, id, parts)| {
            record([
                ("id", id.into()),
                ("name", name.into()),
                ("parts", parts.into()),
            ])
        })
        .collect()
}

/// `menu.saves` (file, name, saved, time, flying, vessels, problem, deleting), newest first.
fn save_list(saves: &Saves) -> Vec<UiRecord> {
    let now = now();
    (saves.list.iter())
        .map(|entry| {
            let header = entry.header.as_ref();
            let name = header.map_or(entry.file.as_str(), |h| h.name.as_str());
            let saved = header.map_or(String::new(), |h| ago(h.saved_at, now));
            let time = header.map_or(String::new(), |h| format_date(h.time));
            let flying = header.map_or("", |h| h.flying.as_str());
            record([
                ("file", entry.file.as_str().into()),
                ("name", name.into()),
                ("saved", saved.into()),
                ("time", time.into()),
                ("flying", flying.into()),
                ("vessels", header.map_or(0, |h| h.vessels).into()),
                ("problem", entry.problem.as_deref().unwrap_or("").into()),
                (
                    "deleting",
                    (saves.deleting.as_ref() == Some(&entry.file)).into(),
                ),
            ])
        })
        .collect()
}

/// `menu.play`, `menu.new_game`, `menu.vessels`, `menu.craft`, `menu.saves`, `menu.has_saves`,
/// `menu.save_name` and `menu.notice`. See `docs/modding.md`.
fn publish_play(
    content: Res<GameContent>,
    menu: Res<Menu>,
    saves: Res<Saves>,
    sim: Res<Sim>,
    mut data: ResMut<UiData>,
) {
    if menu.is_changed() {
        data.set("menu.play", menu.play.id());
        data.set("menu.new_game", menu.new_game);
        data.set("menu.vessels", vessel_list(&sim, &menu));
    }
    if content.is_changed() {
        data.set("menu.craft", craft_list(&content));
    }
    if menu.is_changed() || saves.is_changed() {
        data.set("menu.saves", save_list(&saves));
        data.set("menu.has_saves", !saves.list.is_empty());
        data.set("menu.save_name", saves.current_name().unwrap_or(""));
        let notice = if menu.notice.is_empty() {
            saves.notice.as_str()
        } else {
            menu.notice.as_str()
        };
        data.set("menu.notice", notice);
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
    mut ui: ResMut<LayoutVisibility>,
    mut saves: ResMut<Saves>,
    editor: Res<Editor>,
    mut restarted: MessageWriter<SimRestarted>,
    mut exit: MessageWriter<AppExit>,
) {
    for action in actions.read() {
        match action.name() {
            // The editor sees to Esc while it is open.
            "game.menu" if editor.is_open() => {}
            "game.menu" | "game.resume" if menu.open && menu.playing => {
                menu.new_game = false;
                menu.play = Play::Start;
                menu.notice.clear();
                saves.notice.clear();
                menu.show(false, &mut sim, &mut ui);
            }
            "game.menu" if !menu.open => menu.show(true, &mut sim, &mut ui),
            "menu.page" => {
                if let Some(page) = action.arg() {
                    menu.page = page.to_owned();
                }
            }
            "menu.place" => menu.place = action.arg().unwrap_or_default().to_owned(),
            "menu.play" => {
                let Some(step) = Play::parse(action.arg().unwrap_or_default()) else {
                    warn!("no step `{}` on the Play page", action.0);
                    continue;
                };
                menu.play = step;
                menu.notice.clear();
                saves.notice.clear();
                match step {
                    // Going back past the vessels drops a new game that has not started.
                    Play::Start => menu.new_game = false,
                    Play::Load => saves.refresh(),
                    Play::Vessels | Play::Craft => {}
                }
            }
            "game.new" => {
                menu.new_game = true;
                menu.play = Play::Vessels;
                menu.notice.clear();
                saves.notice.clear();
            }
            "game.fly" if menu.playing => {
                let id = action.arg().and_then(|id| id.parse().ok()).map(VesselId);
                let Some(id) = id.filter(|&id| sim.universe.vessel(id).is_some()) else {
                    warn!("no vessel to fly in `{}`", action.0);
                    continue;
                };
                sim.fly(id);
                rig.map_view = false;
                rig.follow = None;
                rig.frame(sim.active_vessel().propulsion.length());
                menu.play = Play::Start;
                menu.show(false, &mut sim, &mut ui);
            }
            name @ ("game.start" | "game.start_tutorial") => {
                let Some(id) = action.arg() else { continue };
                let started = match name {
                    "game.start" => crate::content::restart(&content.0, &mut sim.universe, id),
                    _ => crate::content::restart_tutorial(&content.0, &mut sim.universe, id),
                };
                match started {
                    Ok(()) => {
                        info!("started `{id}`");
                        sim.restarted();
                        *warp = TimeWarp::default();
                        rig.map_view = false;
                        restarted.write(SimRestarted);
                        saves.forget_current();
                        menu.new_game = false;
                        menu.play = Play::Start;
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

    #[test]
    fn the_steps_of_the_play_page_read_back_as_they_are_written() {
        for step in [Play::Start, Play::Load, Play::Vessels, Play::Craft] {
            assert_eq!(Play::parse(step.id()), Some(step));
        }
        assert_eq!(Play::parse("nowhere"), None);
    }
}
