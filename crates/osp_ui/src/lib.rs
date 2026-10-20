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
//!
//! Built-in actions: `ui.toggle:<layout>`, `ui.show:<layout>`, `ui.hide:<layout>`, and
//! `ui.toggle_group:<group>`, `ui.show_group:<group>`, `ui.hide_group:<group>` for every
//! layout in a group at once. `ui.shown.<layout>` tells whether a layout is showing.
//! The `data_inspector` layout, when shown, lists every bindable key.

pub mod data;
pub mod format;
pub mod layout;
pub mod runtime;
pub mod style;
pub mod widgets;

use bevy::prelude::*;

pub use data::{UiData, UiRecord, UiValue, record};
pub use format::{Formatter, Formatters, Template};
pub use layout::{LayoutDef, NodeDef, UiDocuments};
pub use runtime::{ActionState, LayoutVisibility, UiAction};
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
        app.init_resource::<UiData>()
            .init_resource::<Formatters>()
            .init_resource::<WidgetRegistry>()
            .init_resource::<UiDocuments>()
            .init_resource::<LayoutVisibility>()
            .init_resource::<ActionState>()
            .add_message::<UiAction>()
            .configure_sets(Update, (UiSet::Input, UiSet::Build, UiSet::Update).chain())
            .add_systems(Update, runtime::read_keybindings.in_set(UiSet::Input))
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
                    runtime::publish_layouts,
                    runtime::publish_inspector,
                    runtime::update_repeaters,
                    runtime::update_text,
                    runtime::update_visibility,
                    runtime::update_bars,
                    runtime::update_states,
                )
                    .chain()
                    .in_set(UiSet::Update),
            );
    }
}
