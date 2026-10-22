//! The main menu: pick a scenario to fly, go back to the one in progress, or quit.
//!
//! Without `OSP_SCENARIO` the game opens on it, over the default scenario standing still;
//! Esc (`game.menu`) brings it back in flight. While it is open time stands still, the
//! flight UI is hidden and the pilot's controls do nothing. Picking a scenario starts it
//! afresh in the same solar system (`game.start:<id>`).

use bevy::prelude::*;
use osp_sim::data::ScenarioDef;
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

/// `menu.scenarios` (id, name, description, in the order the records load) and
/// `menu.playing`.
fn publish_scenarios(content: Res<GameContent>, menu: Res<Menu>, mut data: ResMut<UiData>) {
    if content.is_changed() {
        let scenarios: Vec<_> = content
            .0
            .registry
            .all::<ScenarioDef>("scenarios", &mut Vec::new())
            .into_iter()
            .map(|s| {
                let name = if s.name.is_empty() { &s.id } else { &s.name };
                record([
                    ("id", s.id.as_str().into()),
                    ("name", name.as_str().into()),
                    ("description", s.description.as_str().into()),
                ])
            })
            .collect();
        data.set("menu.scenarios", scenarios);
    }
    if menu.is_changed() {
        data.set("menu.playing", menu.playing);
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
