//! Layout, keybinding and document definitions (the moddable UI formats).
//!
//! A layout record (collection `layouts`) is a tree of nodes:
//!
//! ```json5
//! {
//!   id: "orbit_info",
//!   visible: true,
//!   root: {
//!     widget: "window", title: "Orbit", style: { left: 20, top: 300, width: 240 },
//!     children: [
//!       { widget: "label", text: "Apoapsis {orbit.apoapsis|distance}" },
//!       { widget: "bar", value: "vessel.stages[0].fuel" },
//!       { widget: "button", text: "Stage", action: "vessel.stage" },
//!       { widget: "column", repeat: "vessel.stages", children: [
//!         { widget: "label", text: "{item.name}: {item.delta_v|speed}" },
//!       ]},
//!     ],
//!   },
//! }
//! ```

use bevy::prelude::*;
use phymi_mods::{Diagnostic, Registry};
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
    /// Data key controlling visibility; prefix with `!` to negate.
    #[serde(default)]
    pub visible: Option<String>,
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
    /// Copy of this subtree with `item.` bindings pointing at `list[index].`.
    pub fn instantiate(&self, list: &str, index: usize) -> NodeDef {
        let prefix = format!("{list}[{index}].");
        let sub = |s: &Option<String>| s.as_ref().map(|s| s.replace("item.", &prefix));
        NodeDef {
            text: sub(&self.text),
            title: sub(&self.title),
            action: sub(&self.action),
            value: sub(&self.value),
            visible: sub(&self.visible),
            repeat: sub(&self.repeat),
            children: self
                .children
                .iter()
                .map(|c| c.instantiate(list, index))
                .collect(),
            ..self.clone()
        }
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
