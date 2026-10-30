//! The main menu: pick a scenario to fly, go back to the one in progress, or quit.
//!
//! Without `OSP_SCENARIO` the game opens on it, over the default scenario standing still;
//! Esc (`game.menu`) brings it back in flight. While it is open time stands still, the
//! flight UI is hidden and the pilot's controls do nothing. A scenario is picked in two
//! steps: the place it starts (`menu.place:<pad|runway|orbit|flight>`, `menu.place` to go
//! back), then the spacecraft there; that starts it afresh in the same solar system
//! (`game.start:<id>`). Its Graphics page (`menu.page:graphics`) sets the graphics (see
//! `settings`).

use bevy::prelude::*;
use osp_sim::data::{LaunchSiteDef, ScenarioDef, StartDef};
use osp_ui::{LayoutVisibility, UiAction, UiData, record};

use crate::{
    FrameSet,
    camera::CameraRig,
    content::GameContent,
    sim::{Sim, SimRestarted, TimeWarp},
};

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        // Scripted captures and `OSP_SCENARIO` go straight into flight.
        let open =
            std::env::var_os("OSP_SCENARIO").is_none() && std::env::var_os("OSP_CAPTURE").is_none();
        app.insert_resource(Menu {
            open: false,
            playing: !open,
            page: "scenarios".into(),
            place: String::new(),
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
            (publish_scenarios, menu_actions.in_set(FrameSet::Input)),
        );
    }
}

#[derive(Resource)]
pub struct Menu {
    pub open: bool,
    /// A scenario is being flown, so the menu can go back to it.
    pub playing: bool,
    /// The page showing: `scenarios` or `graphics`.
    page: String,
    /// The place picked on the scenarios page (see `place`), or empty while picking one.
    place: String,
}

impl Menu {
    /// Open or close the menu: pause or run the simulation, and swap the menu for the
    /// flight UI.
    fn show(&mut self, open: bool, sim: &mut Sim, ui: &mut LayoutVisibility) {
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
/// `menu.playing` and `menu.page`. A scenario's name is left out when it is the craft's.
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
    }
    if menu.is_changed() {
        data.set("menu.playing", menu.playing);
        data.set("menu.page", menu.page.as_str());
        data.set("menu.place", menu.place.as_str());
    }
}

#[allow(clippy::too_many_arguments)]
fn menu_actions(
    mut actions: MessageReader<UiAction>,
    mut menu: ResMut<Menu>,
    mut sim: ResMut<Sim>,
    mut warp: ResMut<TimeWarp>,
    mut rig: ResMut<CameraRig>,
    content: Res<GameContent>,
    mut ui: ResMut<LayoutVisibility>,
    mut restarted: MessageWriter<SimRestarted>,
    mut exit: MessageWriter<AppExit>,
) {
    for action in actions.read() {
        match action.name() {
            "game.menu" if menu.open && menu.playing => menu.show(false, &mut sim, &mut ui),
            "game.menu" if !menu.open => menu.show(true, &mut sim, &mut ui),
            "game.resume" if menu.playing => menu.show(false, &mut sim, &mut ui),
            "menu.page" => {
                if let Some(page) = action.arg() {
                    menu.page = page.to_owned();
                }
            }
            "menu.place" => menu.place = action.arg().unwrap_or_default().to_owned(),
            "game.start" => {
                let Some(id) = action.arg() else { continue };
                match crate::content::restart(&content.0, &mut sim.universe, id) {
                    Ok(()) => {
                        info!("started scenario `{id}`");
                        sim.restarted();
                        *warp = TimeWarp::default();
                        rig.map_view = false;
                        restarted.write(SimRestarted);
                        menu.playing = true;
                        menu.show(false, &mut sim, &mut ui);
                    }
                    Err(e) => error!("cannot start scenario `{id}`: {e}"),
                }
            }
            "game.quit" => {
                exit.write(AppExit::Success);
            }
            _ => {}
        }
    }
}
