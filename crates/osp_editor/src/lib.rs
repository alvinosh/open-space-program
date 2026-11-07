//! The editor: the hangar where a vessel is chosen, built and launched. It is the game's
//! (`osp_game`) and reads it, never the other way round: the rest of the game knows only what
//! `osp_game::editing` holds, whether the editor is open and the last launch, which the editor
//! keeps. The `osp` binary adds [`EditorPlugin`] after the game's plugin.
//!
//! - [`hangar`]: opening and closing the editor, the hall drawn round what is on show, its camera,
//!   the list of vessels, launching and reverting to a launch.
//! - [`parts_list`]: the parts list, a grid of pictures of parts by tab, filter and search.
//! - [`building`]: taking a part in hand and putting it on.
//! - [`shaping`]: picking a part, and the handles that size it.
//! - [`moving`]: the move and turn tools.
//! - [`part_settings`]: everything that can be set on the picked part, in the rail.
//! - [`capture`]: the steps of the game's scripted captures that look inside the editor.

mod building;
mod capture;
mod hangar;
mod moving;
mod part_settings;
mod parts_list;
mod shaping;

use bevy::prelude::*;

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            hangar::HangarPlugin,
            shaping::ShapingPlugin,
            building::BuildingPlugin,
            parts_list::PartsListPlugin,
            moving::MovingPlugin,
            part_settings::PartSettingsPlugin,
            capture::CapturePlugin,
        ));
    }
}
