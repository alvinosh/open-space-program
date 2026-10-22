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
    pub root: NodeDef,
}

fn yes() -> bool {
    true
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
        let prefix = format!("{list}[{index}].");
        let sub = |s: &Option<String>| s.as_ref().map(|s| s.replace("item.", &prefix));
        let extra = self
            .extra
            .iter()
            .map(|(k, v)| {
                let v = match v {
                    Value::String(s) => Value::String(s.replace("item.", &prefix)),
                    other => other.clone(),
                };
                (k.clone(), v)
            })
            .collect();
        let children = if self.repeat.is_some() {
            self.children.clone()
        } else {
            self.children
                .iter()
                .map(|c| c.instantiate(list, index))
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
