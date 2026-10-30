//! Which flight windows are open: the ones the player opened stay open between runs, and
//! the Orbit window opens by itself on leaving the atmosphere and closes on re-entering it.
//!
//! A launch starts with only what flying needs (the altimeter, the navball, the stage
//! stack and the clock); Flight, Resources and the rest are one click away on the launcher
//! (`layouts` with `visible: false`). The choices are saved in `windows.json` beside the
//! settings file; scripted captures (`OSP_CAPTURE`) neither read nor write it.

use std::{collections::BTreeMap, path::PathBuf};

use bevy::prelude::*;
use osp_ui::{LayoutVisibility, UiData, UiDocuments, UiSet};

pub struct WindowsPlugin;

impl Plugin for WindowsPlugin {
    fn build(&self, app: &mut App) {
        let file = windows_file();
        let windows = file.as_ref().map(load).unwrap_or_default();
        app.insert_resource(Remembered { file, windows })
            .add_systems(Startup, restore)
            .add_systems(
                Update,
                (
                    orbit_window,
                    remember.run_if(resource_changed::<LayoutVisibility>),
                )
                    .chain()
                    .after(UiSet::Update),
            );
    }
}

/// The window that opens and closes with the vessel's situation, so it is not remembered.
const ORBIT_WINDOW: &str = "orbit_info";

#[derive(Resource)]
struct Remembered {
    file: Option<PathBuf>,
    windows: BTreeMap<String, bool>,
}

fn windows_file() -> Option<PathBuf> {
    if std::env::var_os("OSP_CAPTURE").is_some() {
        return None;
    }
    Some(crate::settings::settings_file()?.with_file_name("windows.json"))
}

fn load(file: &PathBuf) -> BTreeMap<String, bool> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return BTreeMap::new();
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        warn!("ignoring {}: {e}", file.display());
        BTreeMap::new()
    })
}

/// Open and close the windows as they were. Before the layouts are built, which keep what
/// is there instead of their own `visible`.
fn restore(remembered: Res<Remembered>, mut ui: ResMut<LayoutVisibility>) {
    for (id, open) in &remembered.windows {
        ui.layouts.insert(id.clone(), *open);
    }
}

/// What to do with the Orbit window after the vessel's situation is sampled: `Some(open)`
/// to set it. A flight that starts in space opens it; one on the ground leaves it closed.
fn orbit_window_after(before: Option<bool>, in_space: bool) -> Option<bool> {
    match (before, in_space) {
        (Some(false) | None, true) => Some(true),
        (Some(true) | None, false) => Some(false),
        _ => None,
    }
}

fn orbit_window(
    data: Res<UiData>,
    mut ui: ResMut<LayoutVisibility>,
    mut before: Local<Option<bool>>,
) {
    let Some(in_space) = data.get("vessel.in_space").map(|v| v.truthy()) else {
        return;
    };
    if let Some(open) = orbit_window_after(*before, in_space) {
        ui.layouts.insert(ORBIT_WINDOW.into(), open);
    }
    *before = Some(in_space);
}

/// Save the state of the flight windows when it changes.
fn remember(ui: Res<LayoutVisibility>, docs: Res<UiDocuments>, mut remembered: ResMut<Remembered>) {
    let Some(file) = remembered.file.clone() else {
        return;
    };
    let windows: BTreeMap<String, bool> = docs
        .layouts
        .iter()
        .filter(|l| l.group.as_deref() == Some("flight") && !l.instanced && !l.popup)
        .filter(|l| l.id != ORBIT_WINDOW)
        .filter_map(|l| Some((l.id.clone(), *ui.layouts.get(&l.id)?)))
        .collect();
    if windows.is_empty() || windows == remembered.windows {
        return;
    }
    let result = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            let text = serde_json::to_string_pretty(&windows).map_err(std::io::Error::other)?;
            std::fs::write(&file, text)
        });
    if let Err(e) = result {
        warn!("cannot save the open windows to {}: {e}", file.display());
    }
    remembered.windows = windows;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_orbit_window_follows_the_atmosphere() {
        // A launch from the pad: closed, and stays so while flying.
        assert_eq!(orbit_window_after(None, false), Some(false));
        assert_eq!(orbit_window_after(Some(false), false), None);
        // Out of the atmosphere it opens; the player may close it, and it stays closed
        // until the next crossing.
        assert_eq!(orbit_window_after(Some(false), true), Some(true));
        assert_eq!(orbit_window_after(Some(true), true), None);
        assert_eq!(orbit_window_after(Some(true), false), Some(false));
        // Starting in orbit.
        assert_eq!(orbit_window_after(None, true), Some(true));
    }

    fn layout(id: &str, group: &str) -> osp_ui::LayoutDef {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "group": group,
            "root": { "widget": "column" },
        }))
        .unwrap()
    }

    fn app(file: Option<PathBuf>, saved: BTreeMap<String, bool>) -> App {
        let mut app = App::new();
        app.init_resource::<LayoutVisibility>()
            .insert_resource(UiDocuments {
                layouts: vec![
                    layout("flight_info", "flight"),
                    layout("orbit_info", "flight"),
                    layout("help", "menu"),
                ],
                ..default()
            })
            .insert_resource(Remembered {
                file,
                windows: saved,
            })
            .add_systems(Startup, restore)
            .add_systems(
                Update,
                remember.run_if(resource_changed::<LayoutVisibility>),
            );
        app
    }

    #[test]
    fn open_flight_windows_are_saved_and_restored() {
        let file = std::env::temp_dir().join(format!("osp_windows_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&file);

        // The player opens Flight and Help and the orbit window opens by itself.
        let mut first = app(Some(file.clone()), BTreeMap::new());
        first.update();
        {
            let mut ui = first.world_mut().resource_mut::<LayoutVisibility>();
            ui.layouts.insert("flight_info".into(), true);
            ui.layouts.insert("orbit_info".into(), true);
            ui.layouts.insert("help".into(), true);
        }
        first.update();
        // Only the flight windows the player chose are written.
        let saved = load(&file);
        assert_eq!(saved, BTreeMap::from([("flight_info".to_owned(), true)]));

        // A new run starts with them open.
        let mut second = app(Some(file.clone()), saved);
        second.update();
        assert!(
            second
                .world()
                .resource::<LayoutVisibility>()
                .shown("flight_info")
        );
        assert!(!second.world().resource::<LayoutVisibility>().shown("help"));
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn captures_do_not_touch_the_saved_windows() {
        let mut capture = app(None, BTreeMap::new());
        capture.update();
        capture
            .world_mut()
            .resource_mut::<LayoutVisibility>()
            .layouts
            .insert("flight_info".into(), true);
        capture.update();
        assert!(capture.world().resource::<Remembered>().windows.is_empty());
    }
}
