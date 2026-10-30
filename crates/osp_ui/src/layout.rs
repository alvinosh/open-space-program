//! Layout, keybinding and document definitions (the moddable UI formats).
//!
//! A layout record (collection `layouts`) is a tree of nodes:
//!
//! ```json5
//! {
//!   id: "orbit_info",
//!   visible: true,
//!   group: "flight",
//!   root: {
//!     widget: "window", title: "Orbit", style: { left: 20, top: 300, width: 240 },
//!     children: [
//!       { widget: "label", text: "Apoapsis {orbit.apoapsis|distance}" },
//!       { widget: "bar", value: "vessel.stages[0].fuel" },
//!       { widget: "button", text: "Stage", action: "vessel.stage" },
//!       { widget: "button", text: "SAS", action: "vessel.sas_toggle", active: "vessel.sas" },
//!       { widget: "column", repeat: "vessel.stages", children: [
//!         { widget: "label", text: "{item.name}: {item.delta_v|speed}" },
//!       ]},
//!     ],
//!   },
//! }
//! ```
//!
//! A layout with `dock: "<id>"` is stacked into the `dock` node with that id instead of
//! being placed on its own: docked windows line up in a column (in `dock_order`) and never
//! overlap. Dragging a docked window by its title pops it out to float; closing it puts it
//! back.
//!
//! A layout with `instanced: true` is a template, opened once per key rather than once:
//! `ui.show:<id>:<key>` opens a copy for `key` (see [`instance_id`]), and `self.` in its
//! bindings reads that copy's data, `<id>.<key>.` (see [`instance_data`]). The part
//! windows are copies of one layout, a copy per part. A layout with `popup: true` closes
//! when the player clicks outside it, unless pinned with the pin button its window gets,
//! and opens beside the pointer; a popup that is not a window (a context menu) works the
//! same, without the pin.

use bevy::prelude::*;
use osp_mods::{Diagnostic, Registry};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::style::{StyleDef, ThemeDef};

pub const LAYOUTS: &str = "layouts";
pub const THEMES: &str = "themes";
pub const KEYBINDINGS: &str = "keybindings";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutDef {
    pub id: String,
    /// Shown at startup. Toggle at runtime with the `ui.toggle:<id>` action.
    #[serde(default = "yes")]
    pub visible: bool,
    /// Layouts in a group can be hidden and shown together (`ui.toggle_group:<group>`)
    /// without forgetting which of them were open.
    #[serde(default)]
    pub group: Option<String>,
    /// Id of a `dock` node to stack this layout into, instead of placing it by its own
    /// position.
    #[serde(default)]
    pub dock: Option<String>,
    /// Place in the dock: lower first.
    #[serde(default)]
    pub dock_order: i32,
    /// A template opened once per key (see the module docs); `visible` does not apply.
    #[serde(default)]
    pub instanced: bool,
    /// Closes on a click outside it unless pinned, and opens beside the pointer. Its window
    /// gets a pin button unless the window says `pin: false`.
    #[serde(default)]
    pub popup: bool,
    pub root: NodeDef,
}

fn yes() -> bool {
    true
}

/// Id of the copy of instanced layout `layout` for `key`. It opens and closes like a
/// layout of that id: `ui.show:<layout>:<key>`, `ui.hide:<layout>:<key>`.
pub fn instance_id(layout: &str, key: &str) -> String {
    format!("{layout}:{key}")
}

/// Where the data of a copy's `self.` bindings lives: `<layout>.<key>.`.
pub fn instance_data(layout: &str, key: &str) -> String {
    format!("{layout}.{key}.")
}

fn column() -> String {
    "column".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeDef {
    /// Optional id, for patching (`#id` paths) and debugging.
    #[serde(default)]
    pub id: Option<String>,
    /// Widget type from the widget registry.
    #[serde(default = "column")]
    pub widget: String,
    /// Space-separated theme classes.
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub style: StyleDef,
    /// Text template (labels, buttons).
    #[serde(default)]
    pub text: Option<String>,
    /// Window title template.
    #[serde(default)]
    pub title: Option<String>,
    /// Action fired on click; may contain `{bindings}`.
    #[serde(default)]
    pub action: Option<String>,
    /// Data key for a 0..1 value (bars).
    #[serde(default)]
    pub value: Option<String>,
    /// Condition controlling visibility (see [`crate::UiData::condition`]).
    #[serde(default)]
    pub visible: Option<String>,
    /// Condition for the active look (`active_background`, `active_color`,
    /// `active_border_color`): selected modes, toggles that are on.
    #[serde(default)]
    pub active: Option<String>,
    /// List key: children are instantiated once per item, with `item.` referring to it.
    #[serde(default)]
    pub repeat: Option<String>,
    #[serde(default)]
    pub children: Vec<NodeDef>,
    /// Extra properties for custom widgets.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl NodeDef {
    /// Copy of this subtree with `item.` bindings pointing at `list[index].`, in its fields
    /// and in the string properties of custom widgets (`drag`, `drop`, ...). A nested
    /// `repeat` takes its list from here, but its children are its own template: their
    /// `item.` is an item of the nested list.
    pub fn instantiate(&self, list: &str, index: usize) -> NodeDef {
        self.rebind("item.", &format!("{list}[{index}]."), false)
    }

    /// Copy of this subtree with `self.` bindings pointing at `prefix`, a layout copy's data
    /// (see [`instance_data`]). `self.` means the same inside lists, so their templates
    /// are bound too.
    pub fn bind_self(&self, prefix: &str) -> NodeDef {
        self.rebind("self.", prefix, true)
    }

    /// `name` replaced by `prefix` where it starts a key. A nested `repeat`'s children are
    /// its own template, left alone unless `into_lists`.
    fn rebind(&self, name: &str, prefix: &str, into_lists: bool) -> NodeDef {
        let sub = |s: &Option<String>| s.as_ref().map(|s| replace_key(s, name, prefix));
        let extra = self
            .extra
            .iter()
            .map(|(k, v)| {
                let v = match v {
                    Value::String(s) => Value::String(replace_key(s, name, prefix)),
                    other => other.clone(),
                };
                (k.clone(), v)
            })
            .collect();
        let children = if self.repeat.is_some() && !into_lists {
            self.children.clone()
        } else {
            self.children
                .iter()
                .map(|c| c.rebind(name, prefix, into_lists))
                .collect()
        };
        NodeDef {
            text: sub(&self.text),
            title: sub(&self.title),
            action: sub(&self.action),
            value: sub(&self.value),
            visible: sub(&self.visible),
            active: sub(&self.active),
            repeat: sub(&self.repeat),
            children,
            extra,
            ..self.clone()
        }
    }
}

/// `text` with `name` (`item.`, `self.`) replaced by `prefix` where it starts a key, not
/// inside a longer word or path (`itself.`, `stage.item.`).
fn replace_key(text: &str, name: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (at, _) in text.match_indices(name) {
        let inside = text[..at]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.');
        if !inside {
            out.push_str(&text[copied..at]);
            out.push_str(prefix);
            copied = at + name.len();
        }
    }
    out.push_str(&text[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn nested_repeats_keep_their_own_items() {
        let node: NodeDef = serde_json::from_value(json!({
            "repeat": "seqs",
            "children": [{
                "text": "{item.number}",
                "drop": "move:$drag:{item.index}",
                "children": [{
                    "repeat": "item.items",
                    "children": [{ "text": "{item.kind}" }],
                }],
            }],
        }))
        .unwrap();
        let row = node.children[0].instantiate("seqs", 1);
        assert_eq!(row.text.as_deref(), Some("{seqs[1].number}"));
        assert_eq!(row.extra["drop"], json!("move:$drag:{seqs[1].index}"));
        let inner = &row.children[0];
        assert_eq!(inner.repeat.as_deref(), Some("seqs[1].items"));
        // The inner template still refers to its own items.
        assert_eq!(inner.children[0].text.as_deref(), Some("{item.kind}"));
        let icon = inner.children[0].instantiate("seqs[1].items", 2);
        assert_eq!(icon.text.as_deref(), Some("{seqs[1].items[2].kind}"));
    }

    #[test]
    fn self_binds_everywhere_but_only_at_the_start_of_a_key() {
        let node: NodeDef = serde_json::from_value(json!({
            "widget": "window",
            "title": "{self.name}",
            "children": [
                { "text": "Holds itself. {self.amount|mass}", "visible": "!self.empty" },
                {
                    "repeat": "self.resources",
                    "children": [{ "text": "{item.name} of {self.name}" }],
                },
            ],
        }))
        .unwrap();
        let bound = node.bind_self("w.3-1.");
        assert_eq!(bound.title.as_deref(), Some("{w.3-1.name}"));
        let label = &bound.children[0];
        assert_eq!(label.text.as_deref(), Some("Holds itself. {w.3-1.amount|mass}"));
        assert_eq!(label.visible.as_deref(), Some("!w.3-1.empty"));
        let list = &bound.children[1];
        assert_eq!(list.repeat.as_deref(), Some("w.3-1.resources"));
        assert_eq!(
            list.children[0].text.as_deref(),
            Some("{item.name} of {w.3-1.name}")
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyBindingDef {
    /// Bevy `KeyCode` name, e.g. `"Space"`, `"KeyM"`, `"ShiftLeft"`, `"F5"`.
    pub key: KeyCode,
    pub action: String,
    /// Held actions are active every frame the key is down (throttle, steering);
    /// others fire once per press.
    #[serde(default)]
    pub hold: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyBindingsDef {
    pub id: String,
    pub bindings: Vec<KeyBindingDef>,
}

/// Everything the UI is built from. Replace this resource to rebuild the UI (hot reload).
#[derive(Resource, Debug, Clone, Default)]
pub struct UiDocuments {
    /// In definition order.
    pub layouts: Vec<LayoutDef>,
    pub theme: ThemeDef,
    pub bindings: Vec<KeyBindingDef>,
}

impl UiDocuments {
    /// The theme's text font at `size` px, for text the game draws outside layouts (map
    /// labels).
    pub fn text_font(&self, size: f32) -> TextFont {
        let style = StyleDef {
            font_size: Some(size),
            ..default()
        };
        self.theme.text_style(&style).text_font()
    }

    pub fn from_registry(registry: &Registry, diagnostics: &mut Vec<Diagnostic>) -> Self {
        let mut theme = ThemeDef::default();
        for t in registry.all::<ThemeDef>(THEMES, diagnostics) {
            theme.merge(&t);
        }
        Self {
            layouts: registry.all::<LayoutDef>(LAYOUTS, diagnostics),
            theme,
            bindings: registry
                .all::<KeyBindingsDef>(KEYBINDINGS, diagnostics)
                .into_iter()
                .flat_map(|k| k.bindings)
                .collect(),
        }
    }
}
