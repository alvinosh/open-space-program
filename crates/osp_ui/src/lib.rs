//! Open Space Program's data-driven, moddable UI on top of `bevy_ui`.
//!
//! - **Layouts** (`layouts` collection): trees of widgets in JSON5, see [`layout`].
//! - **Themes** (`themes` collection): named style classes merged across mods, see [`style`].
//! - **Data bindings**: the game publishes values into [`UiData`]; layouts show them with
//!   text templates (`"{vessel.altitude|distance}"`), conditions, bars and repeated lists.
//! - **Actions**: buttons and keybindings (`keybindings` collection) emit [`UiAction`]
//!   messages by name; held keys show up in [`ActionState`]. The game decides what each
//!   action does, so mods can add buttons and rebind keys without code.
//! - **Hot reload**: replace the [`UiDocuments`] resource and the UI rebuilds.
//! - **Art**: `image` nodes and window icons take PNG or SVG files from the mods (SVGs are
//!   rasterised with mipmaps, see [`svg`]); the theme names font files to load (`fonts`).
//!
//! Built-in actions: `ui.toggle:<layout>`, `ui.show:<layout>`, `ui.hide:<layout>`, and
//! `ui.toggle_group:<group>`, `ui.show_group:<group>`, `ui.hide_group:<group>` for every
//! layout in a group at once, and `ui.pin:<layout>` to pin or unpin a popup.
//! `ui.shown.<layout>` tells whether a layout is showing, `ui.pinned.<layout>` whether a
//! popup is pinned. The `data_inspector` layout, when shown, lists every bindable key.
//!
//! Windows can stack into docks (see [`layout`]) and come to the front when opened or
//! clicked; [`PointerOverUi`] tells the game when the mouse belongs to the UI. Instanced
//! layouts open a copy per key (a window per part), and popups close on a click outside
//! them unless pinned.

pub mod data;
pub mod format;
pub mod layout;
pub mod runtime;
pub mod style;
pub mod svg;
pub mod widgets;

use bevy::prelude::*;

pub use data::{UiData, UiRecord, UiValue, record};
pub use format::{Formatter, Formatters, Template};
pub use layout::{LayoutDef, NodeDef, UiDocuments, instance_data, instance_id};
pub use runtime::{
    ActionState, CLICK_SLOP, LayoutVisibility, PointerOverUi, Tooltip, UiAction, UiImage,
    ViewOverlay,
};
pub use widgets::{WidgetBuilder, WidgetContext, WidgetRegistry};

/// Ordering for UI work within `Update`. Publish data before [`UiSet::Update`] and read
/// [`UiAction`]s after [`UiSet::Input`].
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum UiSet {
    Input,
    Build,
    Update,
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        // Headless apps (tests) have no asset server.
        if app.is_plugin_added::<AssetPlugin>() {
            app.init_asset_loader::<svg::SvgLoader>();
        }
        app.init_resource::<UiData>()
            .init_resource::<Formatters>()
            .init_resource::<WidgetRegistry>()
            .init_resource::<UiDocuments>()
            .init_resource::<LayoutVisibility>()
            .init_resource::<ActionState>()
            .init_resource::<runtime::WindowStack>()
            .init_resource::<PointerOverUi>()
            .add_message::<UiAction>()
            .configure_sets(Update, (UiSet::Input, UiSet::Build, UiSet::Update).chain())
            .add_systems(
                Update,
                (runtime::read_keybindings, runtime::track_pointer_over_ui).in_set(UiSet::Input),
            )
            .add_systems(
                Update,
                runtime::rebuild_ui
                    .run_if(resource_changed::<UiDocuments>)
                    .in_set(UiSet::Build),
            )
            .add_systems(
                Update,
                (
                    runtime::builtin_actions,
                    runtime::close_popups,
                    runtime::update_instances,
                    runtime::redock_closed,
                    runtime::attach_docked,
                    runtime::sort_docks,
                    runtime::raise_opened,
                    runtime::keep_on_screen,
                    runtime::publish_layouts,
                    runtime::publish_inspector,
                    runtime::update_repeaters,
                    runtime::update_text,
                    runtime::update_visibility,
                    runtime::update_bars,
                    runtime::update_accents,
                    runtime::update_gauges,
                    runtime::update_states,
                    runtime::update_tooltip,
                    runtime::update_scroll_thumbs,
                )
                    .chain()
                    .in_set(UiSet::Update),
            );
    }
}
