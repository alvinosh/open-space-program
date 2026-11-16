//! Components: what a layout writes on one line and the game builds from several nodes, so every
//! screen of the menu is made the same way. They are expanded when the documents load, after
//! patches, so a patch sees and changes what the layout wrote.
//!
//! | Widget | Written as | Builds |
//! | --- | --- | --- |
//! | `screen` | `title`, `title_if`, `subtitle`, `display: true`, `back: false`, `shade: false`, `children`, `footer` | The left panel: Back, heading (shown while `title_if` holds if there is one), a scrolling body, what just happened (`menu.notice`) and the `footer`'s nodes at the bottom, where they stay however long the body is (a screen's Start or Save), over the whole window so the scene takes no clicks, unless `shade: false` leaves it the panel alone |
//! | `menu_row` | `text`, `action`, `meta`, `meta_class`, `detail`, `detail_if`, `detail_class` | A full-width row to click: its text, a value at the right and a line under it, shown while `detail_if` holds if there is one |
//! | `setting_row` | `label`, `children` | One line of a settings page: a label and what sets it |
//! | `setting_choice` | `label`, `key`, `action`, `options` | A `setting_row` with a `choice`, each option firing `<action>:<value>` and lit while `<key> == <value>` |
//! | `setting_slider` | `label`, `key`, `action`, `min`, `max`, `step`, `unit` | A `setting_row` with a `slider` firing `<action>:<value>` as it is dragged, and `<key>` and `unit` beside it (`%`, say) |
//!
//! An option is a string (shown with a capital), a number, or `{ value, label }`.

use serde_json::Value;

use crate::layout::NodeDef;

/// The nodes the components build, and the control classes a menu may use, are in
/// `themes/default.json5`.
fn node(widget: &str, class: &str) -> NodeDef {
    NodeDef {
        widget: widget.into(),
        class: class.into(),
        ..Default::default()
    }
}

fn label(class: &str, text: String) -> NodeDef {
    NodeDef {
        text: Some(text),
        ..node("label", class)
    }
}

fn with_children(mut parent: NodeDef, children: Vec<NodeDef>) -> NodeDef {
    parent.children = children;
    parent
}

/// A string property of a component, taken out of the node.
fn take_text(node: &mut NodeDef, key: &str) -> Option<String> {
    match node.extra.remove(key) {
        Some(Value::String(text)) => Some(text),
        _ => None,
    }
}

/// `node`'s classes after the component's own, so a layout can add to them.
fn classes(own: &str, node: &NodeDef) -> String {
    format!("{own} {}", node.class).trim().to_owned()
}

/// `node` and everything under it with the components built. What is wrong with one (a
/// `setting_choice` with no options) goes in `problems`.
pub fn expand(mut def: NodeDef, problems: &mut Vec<String>) -> NodeDef {
    def.children = std::mem::take(&mut def.children)
        .into_iter()
        .map(|child| expand(child, problems))
        .collect();
    match def.widget.as_str() {
        "screen" => screen(def, problems),
        "menu_row" => menu_row(def),
        "setting_row" => setting_row(def),
        "setting_choice" => setting_choice(def, problems),
        "setting_slider" => setting_slider(def, problems),
        _ => def,
    }
}

fn screen(mut def: NodeDef, problems: &mut Vec<String>) -> NodeDef {
    let footer: Vec<NodeDef> = match def.extra.remove("footer") {
        Some(Value::Array(nodes)) => (nodes.into_iter())
            .filter_map(|n| match serde_json::from_value(n) {
                Ok(n) => Some(expand(n, problems)),
                Err(e) => {
                    problems.push(format!("a screen's footer: {e}"));
                    None
                }
            })
            .collect(),
        Some(other) => {
            problems.push(format!("a screen's footer is a list of nodes, not {other}"));
            Vec::new()
        }
        None => Vec::new(),
    };
    let mut rail = Vec::new();
    if def.extra.remove("back") != Some(Value::Bool(false)) {
        rail.push(NodeDef {
            text: Some("‹ Back".into()),
            action: Some("menu.back".into()),
            ..node("button", "back")
        });
    }
    let display = def.extra.remove("display") == Some(Value::Bool(true));
    if let Some(title) = def.title.take() {
        let class = if display {
            "menu_display"
        } else {
            "menu_heading"
        };
        rail.push(NodeDef {
            visible: take_text(&mut def, "title_if"),
            ..label(class, title)
        });
    }
    if let Some(subtitle) = take_text(&mut def, "subtitle") {
        rail.push(label("menu_subtitle", subtitle));
    }
    // A list gets a column of its own: filling one empties it, and the body keeps its scroll thumb.
    let mut body = std::mem::take(&mut def.children);
    if let Some(list) = def.repeat.take() {
        body = vec![NodeDef {
            repeat: Some(list),
            children: body,
            ..node("column", "")
        }];
    }
    rail.push(with_children(node("column", "menu_body"), body));
    rail.push(NodeDef {
        visible: Some("menu.notice".into()),
        ..label("menu_notice", "{menu.notice}".into())
    });
    if !footer.is_empty() {
        rail.push(with_children(node("column", "menu_footer"), footer));
    }
    if def.extra.remove("shade") == Some(Value::Bool(false)) {
        return NodeDef {
            widget: "column".into(),
            class: classes("menu_rail", &def),
            children: rail,
            ..def
        };
    }
    NodeDef {
        widget: "column".into(),
        class: classes("menu_shade", &def),
        children: vec![with_children(node("column", "menu_rail"), rail)],
        ..def
    }
}

fn menu_row(mut def: NodeDef) -> NodeDef {
    let mut head = vec![label("menu_row_text", def.text.take().unwrap_or_default())];
    if let Some(meta) = take_text(&mut def, "meta") {
        let class = take_text(&mut def, "meta_class");
        head.push(node("spacer", ""));
        head.push(label(class.as_deref().unwrap_or("menu_row_meta"), meta));
    }
    let mut body = vec![with_children(node("row", "menu_row_head"), head)];
    if let Some(detail) = take_text(&mut def, "detail") {
        let class = take_text(&mut def, "detail_class");
        body.push(NodeDef {
            visible: take_text(&mut def, "detail_if"),
            ..label(class.as_deref().unwrap_or("menu_row_detail"), detail)
        });
    }
    NodeDef {
        widget: "button".into(),
        class: classes("menu_row", &def),
        children: body,
        ..def
    }
}

fn setting_row(mut def: NodeDef) -> NodeDef {
    let name = take_text(&mut def, "label").unwrap_or_default();
    let mut children = vec![label("setting_label", name)];
    children.append(&mut def.children);
    NodeDef {
        widget: "row".into(),
        class: classes("setting_row", &def),
        children,
        ..def
    }
}

fn setting_choice(mut def: NodeDef, problems: &mut Vec<String>) -> NodeDef {
    let key = take_text(&mut def, "key");
    let action = def.action.take();
    let options = def.extra.remove("options");
    let name = def
        .extra
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let items = match (key, action, options) {
        (Some(key), Some(action), Some(Value::Array(options))) if !options.is_empty() => options
            .iter()
            .filter_map(|option| {
                let parsed = parse_option(option);
                if parsed.is_none() {
                    problems.push(format!("setting `{name}`: cannot read option {option}"));
                }
                parsed
            })
            .map(|(value, shown)| NodeDef {
                text: Some(shown),
                action: Some(format!("{action}:{value}")),
                active: Some(format!("{key} == {value}")),
                ..node("button", "choice_item")
            })
            .collect(),
        _ => {
            problems.push(format!(
                "setting `{name}` needs `key`, `action` and `options`"
            ));
            Vec::new()
        }
    };
    def.children = vec![with_children(node("row", "choice"), items)];
    setting_row(def)
}

fn setting_slider(mut def: NodeDef, problems: &mut Vec<String>) -> NodeDef {
    let key = take_text(&mut def, "key");
    let unit = take_text(&mut def, "unit").unwrap_or_default();
    let name = (def.extra.get("label").and_then(Value::as_str)).unwrap_or("?");
    let (Some(key), Some(action)) = (key, def.action.take()) else {
        problems.push(format!("setting `{name}` needs `key` and `action`"));
        return setting_row(def);
    };
    let mut slider = NodeDef {
        value: Some(key.clone()),
        action: Some(action),
        ..node("slider", "")
    };
    for bound in ["min", "max", "step"] {
        if let Some(value) = def.extra.remove(bound) {
            slider.extra.insert(bound.into(), value);
        }
    }
    let value = label("slider_value", format!("{{{key}}}{unit}"));
    let control = with_children(node("row", "slider_row"), vec![slider, value]);
    def.children = vec![control];
    setting_row(def)
}

/// An option's value and what it shows.
fn parse_option(option: &Value) -> Option<(String, String)> {
    match option {
        Value::String(value) => Some((value.clone(), capitalised(value))),
        Value::Number(value) => Some((value.to_string(), value.to_string())),
        Value::Object(fields) => {
            let value = match fields.get("value")? {
                Value::String(value) => value.clone(),
                Value::Number(value) => value.to_string(),
                _ => return None,
            };
            let shown = fields.get("label").and_then(Value::as_str);
            Some((value.clone(), shown.map_or(value, str::to_owned)))
        }
        _ => None,
    }
}

fn capitalised(text: &str) -> String {
    let mut letters = text.chars();
    letters.next().map_or(String::new(), |first| {
        first.to_uppercase().chain(letters).collect()
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{
        layout::{LayoutDef, UiDocuments},
        style::ThemeDef,
    };

    /// The four type sizes of the menus.
    const TYPE_SCALE: [f32; 4] = [28.0, 20.0, 15.0, 12.0];
    /// What can be clicked in a menu.
    const CONTROLS: [&str; 5] = ["back", "menu_row", "btn", "choice_item", "tab"];

    /// The layouts that may show pictures: the editor, whose parts list and bar are icons and
    /// pictures of parts where words did not work (T212, T215). The menus are words.
    const PICTURED: [&str; 1] = ["editor"];

    /// What a menu layout does against the rules in T185.
    fn broken_rules(layout: &LayoutDef, theme: &ThemeDef) -> Vec<String> {
        let mut problems = Vec::new();
        check_node(&layout.id, &layout.root, theme, &mut problems);
        problems
    }

    fn check_node(layout: &str, node: &NodeDef, theme: &ThemeDef, problems: &mut Vec<String>) {
        let at = format!("{layout}/{}", node.widget);
        let picture = node.widget == "image" && PICTURED.contains(&layout);
        if matches!(node.widget.as_str(), "image" | "panel" | "window") && !picture {
            problems.push(format!("{at}: no {} in a menu", node.widget));
        }
        let s = &node.style;
        if s.font_size.is_some() || s.font_weight.is_some() || s.color.is_some() {
            problems.push(format!("{at}: sets its own type"));
        }
        if s.background.is_some()
            || s.hover_background.is_some()
            || s.gradient.is_some()
            || s.border.is_some()
            || s.border_color.is_some()
            || s.padding.is_some()
            || s.radius.is_some()
        {
            problems.push(format!("{at}: sets its own look"));
        }
        if node.widget == "button" {
            let listed = node.class.split_whitespace().any(|c| CONTROLS.contains(&c));
            if !listed {
                problems.push(format!("{at}: `{}` is not a control", node.class));
            }
        }
        let has_text = node.widget == "label" || (node.widget == "button" && node.text.is_some());
        if has_text {
            let size = theme
                .resolve(&node.widget, &node.class, &node.style)
                .font_size();
            if !TYPE_SCALE.contains(&size) {
                problems.push(format!("{at}: type size {size} is not on the scale"));
            }
        }
        for child in &node.children {
            check_node(layout, child, theme, problems);
        }
    }

    fn core() -> UiDocuments {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/core");
        let registry = osp_mods::load(&root, None).registry;
        let mut diagnostics = Vec::new();
        let documents = UiDocuments::from_registry(&registry, &mut diagnostics);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        documents
    }

    fn layout(root: serde_json::Value) -> LayoutDef {
        let mut problems = Vec::new();
        let root = expand(serde_json::from_value(root).unwrap(), &mut problems);
        LayoutDef {
            id: "test".into(),
            visible: false,
            group: Some("menu".into()),
            dock: None,
            dock_order: 0,
            instanced: false,
            popup: false,
            root,
        }
    }

    #[test]
    fn a_setting_choice_missing_something_is_reported() {
        let mut problems = Vec::new();
        let node = serde_json::from_value(json!({ "widget": "setting_choice", "label": "Bloom" }));
        expand(node.unwrap(), &mut problems);
        assert_eq!(
            problems,
            ["setting `Bloom` needs `key`, `action` and `options`"]
        );
    }

    #[test]
    fn the_rules_catch_a_stray_size_icon_look_and_control() {
        let theme = core().theme;
        for (node, wanted) in [
            (
                json!({ "widget": "label", "text": "x" }),
                "not on the scale",
            ),
            (
                json!({ "widget": "label", "class": "menu_text", "text": "x", "style": { "font_size": 13 } }),
                "sets its own type",
            ),
            (
                json!({ "widget": "image", "image": "icons/play.svg" }),
                "no image",
            ),
            (json!({ "widget": "panel" }), "no panel"),
            (
                json!({ "widget": "column", "style": { "background": "#123" } }),
                "sets its own look",
            ),
            (
                json!({ "widget": "column", "style": { "border": 1 } }),
                "sets its own look",
            ),
            (
                json!({ "widget": "button", "class": "step_button", "text": "x" }),
                "not a control",
            ),
        ] {
            let screen = layout(json!({ "widget": "screen", "children": [node] }));
            let problems = broken_rules(&screen, &theme);
            assert!(
                problems.iter().any(|p| p.contains(wanted)),
                "{wanted}: {problems:?}"
            );
        }
    }

    #[test]
    fn every_menu_screen_in_the_game_follows_the_rules() {
        let documents = core();
        let menus = (documents.layouts.iter())
            .filter(|l| l.group.as_deref() == Some("menu") || l.id == "editor" || l.id == "dialog");
        for layout in menus {
            let problems = broken_rules(layout, &documents.theme);
            assert!(problems.is_empty(), "{problems:#?}");
        }
    }

    /// Every `menu.open:<screen>` under `node`.
    fn screens_opened(node: &NodeDef, found: &mut Vec<String>) {
        let action = node.action.as_deref();
        if let Some(screen) = action.and_then(|a| a.strip_prefix("menu.open:")) {
            found.push(screen.to_owned());
        }
        for child in &node.children {
            screens_opened(child, found);
        }
    }

    fn has_back(node: &NodeDef) -> bool {
        node.class == "back" || node.children.iter().any(has_back)
    }

    #[test]
    fn every_screen_can_be_reached_and_left() {
        let documents = core();
        let menus: Vec<_> = (documents.layouts.iter())
            .filter(|l| l.group.as_deref() == Some("menu"))
            .collect();
        let exists = |id: &str| menus.iter().any(|l| l.id == id);
        assert!(exists("menu_title") && exists("menu_pause"));
        let mut opened = Vec::new();
        for layout in &menus {
            assert!(layout.id.starts_with("menu_"), "{}", layout.id);
            assert!(!layout.visible, "{} shows without the menu", layout.id);
            screens_opened(&layout.root, &mut opened);
            let root = matches!(layout.id.as_str(), "menu_title" | "menu_pause");
            assert_eq!(has_back(&layout.root), !root, "{}", layout.id);
        }
        for screen in &opened {
            assert!(exists(&format!("menu_{screen}")), "no screen `{screen}`");
        }
        for layout in &menus {
            let id = layout.id.trim_start_matches("menu_");
            assert!(
                matches!(id, "title" | "pause") || opened.iter().any(|s| s == id),
                "nothing opens `{id}`"
            );
        }
    }
}
