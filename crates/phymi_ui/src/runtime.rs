//! Building layouts into Bevy UI entities and keeping them in sync with [`UiData`].

use std::collections::{HashMap, HashSet};

use bevy::{
    picking::{
        events::{PointerClick, PointerDrag},
        hover::Hovered,
        pointer::PointerButton,
    },
    prelude::*,
};

use crate::{
    data::{UiData, UiValue, record},
    format::{Formatters, Template, plain},
    layout::{LayoutDef, NodeDef, UiDocuments},
    style::{StyleDef, ThemeDef},
    widgets::{WidgetContext, WidgetRegistry},
};

/// An action from a button or key: `name` or `name:argument`.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct UiAction(pub String);

impl UiAction {
    pub fn name(&self) -> &str {
        self.0.split_once(':').map_or(&self.0, |(n, _)| n)
    }

    pub fn arg(&self) -> Option<&str> {
        self.0.split_once(':').map(|(_, a)| a)
    }
}

/// Actions whose key is currently held (keybindings with `hold: true`).
#[derive(Resource, Default, Debug)]
pub struct ActionState {
    held: HashSet<String>,
}

impl ActionState {
    pub fn held(&self, action: &str) -> bool {
        self.held.contains(action)
    }

    /// `+1` if `positive` is held, `-1` if `negative` is, `0` for both or neither.
    pub fn axis(&self, positive: &str, negative: &str) -> f64 {
        f64::from(u8::from(self.held(positive))) - f64::from(u8::from(self.held(negative)))
    }
}

/// Which layouts are shown (toggled by `ui.toggle:<id>`, `ui.show:<id>`, `ui.hide:<id>`).
#[derive(Resource, Default, Debug)]
pub struct LayoutVisibility(pub HashMap<String, bool>);

impl LayoutVisibility {
    pub fn shown(&self, layout: &str) -> bool {
        self.0.get(layout).copied().unwrap_or(false)
    }
}

#[derive(Component)]
pub struct LayoutRoot(pub String);

#[derive(Component)]
pub struct BoundText(pub Template);

#[derive(Component)]
pub struct BoundVisible(pub String);

#[derive(Component)]
pub struct BoundBar(pub String);

#[derive(Component)]
pub struct ActionSource(pub Template);

#[derive(Component)]
pub struct HoverStyle {
    pub normal: Color,
    pub hover: Color,
}

#[derive(Component)]
pub struct WindowDrag {
    pub window: Entity,
}

#[derive(Component)]
pub struct Repeater {
    list: String,
    template: Vec<NodeDef>,
    count: usize,
    layout: String,
}

/// Spawn a bare UI node with layout and box visuals from `style`.
pub fn spawn_box(
    commands: &mut Commands,
    widget: &str,
    style: &StyleDef,
    parent: Option<Entity>,
) -> Entity {
    let mut node = Node {
        flex_direction: if widget == "row" {
            FlexDirection::Row
        } else {
            FlexDirection::Column
        },
        ..default()
    };
    style.apply(&mut node);
    let mut e = commands.spawn(node);
    if let Some(bg) = &style.background {
        e.insert(BackgroundColor(bg.to_color()));
    }
    if let Some(border) = &style.border_color {
        e.insert(BorderColor::all(border.to_color()));
    }
    if let Some(parent) = parent {
        e.insert(ChildOf(parent));
    }
    e.id()
}

/// Spawn a node subtree. Returns the node's entity.
pub fn spawn_node(
    commands: &mut Commands,
    widgets: &WidgetRegistry,
    theme: &ThemeDef,
    layout: &str,
    def: &NodeDef,
    parent: Option<Entity>,
) -> Entity {
    let style = theme.resolve(&def.widget, &def.class, &def.style);
    let entity = spawn_box(commands, &def.widget, &style, parent);
    if let Some(id) = &def.id {
        commands.entity(entity).insert(Name::new(id.clone()));
    }
    if let Some(condition) = &def.visible {
        commands
            .entity(entity)
            .insert(BoundVisible(condition.clone()));
    }
    let builder = widgets.get(&def.widget);
    let children_parent = builder(&mut WidgetContext {
        commands,
        entity,
        def,
        style: &style,
        theme,
        layout,
    });
    match &def.repeat {
        Some(list) => {
            commands.entity(children_parent).insert(Repeater {
                list: list.clone(),
                template: def.children.clone(),
                count: 0,
                layout: layout.to_owned(),
            });
        }
        None => {
            for child in &def.children {
                spawn_node(
                    commands,
                    widgets,
                    theme,
                    layout,
                    child,
                    Some(children_parent),
                );
            }
        }
    }
    entity
}

fn spawn_layout(
    commands: &mut Commands,
    widgets: &WidgetRegistry,
    theme: &ThemeDef,
    index: usize,
    layout: &LayoutDef,
) {
    let root = spawn_node(commands, widgets, theme, &layout.id, &layout.root, None);
    let style = theme.resolve(&layout.root.widget, &layout.root.class, &layout.root.style);
    commands.entity(root).insert((
        LayoutRoot(layout.id.clone()),
        GlobalZIndex(style.z_index.unwrap_or(index as i32)),
        Name::new(format!("layout:{}", layout.id)),
    ));
}

/// (Re)build every layout when the documents change (startup, hot reload).
pub fn rebuild_ui(
    mut commands: Commands,
    docs: Res<UiDocuments>,
    widgets: Res<WidgetRegistry>,
    roots: Query<Entity, With<LayoutRoot>>,
    mut visibility: ResMut<LayoutVisibility>,
) {
    for root in &roots {
        commands.entity(root).despawn();
    }
    for (index, layout) in docs.layouts.iter().enumerate() {
        // Keep the player's toggles across reloads.
        visibility
            .0
            .entry(layout.id.clone())
            .or_insert(layout.visible);
        spawn_layout(&mut commands, &widgets, &docs.theme, index, layout);
    }
}

pub fn update_repeaters(
    mut commands: Commands,
    data: Res<UiData>,
    docs: Res<UiDocuments>,
    widgets: Res<WidgetRegistry>,
    mut repeaters: Query<(Entity, &mut Repeater)>,
) {
    for (entity, mut repeater) in &mut repeaters {
        let count = data.list_len(&repeater.list);
        if count == repeater.count {
            continue;
        }
        repeater.count = count;
        commands.entity(entity).despawn_children();
        for index in 0..count {
            for template in &repeater.template {
                let instance = template.instantiate(&repeater.list, index);
                spawn_node(
                    &mut commands,
                    &widgets,
                    &docs.theme,
                    &repeater.layout,
                    &instance,
                    Some(entity),
                );
            }
        }
    }
}

pub fn update_text(
    data: Res<UiData>,
    formatters: Res<Formatters>,
    mut texts: Query<(&BoundText, &mut Text)>,
) {
    for (bound, mut text) in &mut texts {
        let rendered = bound.0.render(&data, &formatters);
        if text.0 != rendered {
            text.0 = rendered;
        }
    }
}

pub fn update_visibility(
    data: Res<UiData>,
    shown: Res<LayoutVisibility>,
    mut nodes: Query<(&mut Node, Option<&BoundVisible>, Option<&LayoutRoot>)>,
) {
    for (mut node, condition, root) in &mut nodes {
        if condition.is_none() && root.is_none() {
            continue;
        }
        let visible = condition.is_none_or(|c| data.condition(&c.0))
            && root.is_none_or(|r| shown.shown(&r.0));
        let display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
    }
}

pub fn update_bars(data: Res<UiData>, mut bars: Query<(&BoundBar, &mut Node)>) {
    for (bar, mut node) in &mut bars {
        let fraction = data.get(&bar.0).and_then(UiValue::as_number).unwrap_or(0.0);
        let width = Val::Percent((fraction.clamp(0.0, 1.0) * 100.0) as f32);
        if node.width != width {
            node.width = width;
        }
    }
}

pub fn update_hover(mut buttons: Query<(&HoverStyle, &Hovered, &mut BackgroundColor)>) {
    for (style, hovered, mut background) in &mut buttons {
        let color = if hovered.get() {
            style.hover
        } else {
            style.normal
        };
        if background.0 != color {
            background.0 = color;
        }
    }
}

pub fn click_action(
    mut event: On<PointerClick>,
    sources: Query<&ActionSource>,
    data: Res<UiData>,
    formatters: Res<Formatters>,
    mut actions: MessageWriter<UiAction>,
) {
    if event.button != PointerButton::Primary {
        return;
    }
    if let Ok(source) = sources.get(event.entity) {
        event.propagate(false);
        let action = source.0.render(&data, &formatters);
        if !action.is_empty() {
            actions.write(UiAction(action));
        }
    }
}

pub fn drag_window(
    event: On<PointerDrag>,
    handles: Query<&WindowDrag>,
    mut nodes: Query<&mut Node>,
) {
    let Ok(handle) = handles.get(event.entity) else {
        return;
    };
    if let Ok(mut node) = nodes.get_mut(handle.window) {
        let px = |v: Val| if let Val::Px(x) = v { x } else { 0.0 };
        node.left = Val::Px(px(node.left) + event.delta.x);
        node.top = Val::Px(px(node.top) + event.delta.y);
        node.right = Val::Auto;
        node.bottom = Val::Auto;
    }
}

pub fn read_keybindings(
    keys: Res<ButtonInput<KeyCode>>,
    docs: Res<UiDocuments>,
    mut state: ResMut<ActionState>,
    mut actions: MessageWriter<UiAction>,
) {
    state.held.clear();
    for binding in &docs.bindings {
        if binding.hold {
            if keys.pressed(binding.key) {
                state.held.insert(binding.action.clone());
            }
        } else if keys.just_pressed(binding.key) {
            actions.write(UiAction(binding.action.clone()));
        }
    }
}

pub fn builtin_actions(mut actions: MessageReader<UiAction>, mut shown: ResMut<LayoutVisibility>) {
    for action in actions.read() {
        let Some(layout) = action.arg() else {
            continue;
        };
        let current = shown.shown(layout);
        let next = match action.name() {
            "ui.toggle" => !current,
            "ui.show" => true,
            "ui.hide" => false,
            _ => continue,
        };
        shown.0.insert(layout.to_owned(), next);
    }
}

/// Publish every data key as `ui.inspector` while the `data_inspector` layout is open, so
/// modders can discover what they can bind to.
pub fn publish_inspector(shown: Res<LayoutVisibility>, mut data: ResMut<UiData>) {
    if !shown.shown("data_inspector") {
        return;
    }
    let rows: Vec<_> = data
        .keys()
        .into_iter()
        .filter(|k| *k != "ui.inspector")
        .map(|k| {
            let value = data.get(k).map(plain).unwrap_or_default();
            record([("key", k.into()), ("value", value.into())])
        })
        .collect();
    data.set("ui.inspector", rows);
}
