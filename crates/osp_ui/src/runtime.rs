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
    style::{ColorDef, StyleDef, ThemeDef},
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

/// Which layouts are open (`ui.toggle:<id>`, `ui.show:<id>`, `ui.hide:<id>`) and which
/// groups are hidden (`ui.toggle_group:<group>`, ...). A layout shows when it is open and
/// its group is not hidden.
#[derive(Resource, Default, Debug)]
pub struct LayoutVisibility {
    pub layouts: HashMap<String, bool>,
    pub hidden_groups: HashSet<String>,
}

impl LayoutVisibility {
    /// Whether the layout is open (it may still be hidden with its group).
    pub fn shown(&self, layout: &str) -> bool {
        self.layouts.get(layout).copied().unwrap_or(false)
    }

    pub fn visible(&self, layout: &str, group: Option<&str>) -> bool {
        self.shown(layout) && group.is_none_or(|g| !self.hidden_groups.contains(g))
    }
}

#[derive(Component)]
pub struct LayoutRoot {
    pub id: String,
    pub group: Option<String>,
}

#[derive(Component)]
pub struct BoundText(pub Template);

#[derive(Component)]
pub struct BoundVisible(pub String);

/// A bar's fill: `value` between `min` and `max`, filled from zero (so a −1..1 bar fills
/// left or right of its middle), along the width or, if `vertical`, up from the bottom.
#[derive(Component)]
pub struct BoundBar {
    pub key: String,
    pub min: f64,
    pub max: f64,
    pub vertical: bool,
}

impl BoundBar {
    /// Start and end of the fill along the bar, as fractions 0..1.
    pub fn span(&self, value: f64) -> (f32, f32) {
        let range = self.max - self.min;
        if range.is_nan() || range <= 0.0 || value.is_nan() {
            return (0.0, 0.0);
        }
        let at = |v: f64| ((v - self.min) / range).clamp(0.0, 1.0) as f32;
        let (zero, value) = (at(0.0), at(value));
        (zero.min(value), zero.max(value))
    }
}

#[derive(Component)]
pub struct ActionSource(pub Template);

/// Condition for a node's active look.
#[derive(Component)]
pub struct BoundActive(pub String);

/// Background and border colours of an interactive node: normal and active, plus hover
/// for buttons.
#[derive(Component)]
pub struct StateColors {
    pub background: [Color; 2],
    pub hover: Option<Color>,
    pub border: Option<[Color; 2]>,
}

impl StateColors {
    pub fn from_style(style: &StyleDef) -> Self {
        let color = |c: &Option<ColorDef>| c.as_ref().map(ColorDef::to_color);
        let background = color(&style.background).unwrap_or(Color::NONE);
        let border = color(&style.border_color);
        Self {
            background: [
                background,
                color(&style.active_background).unwrap_or(background),
            ],
            hover: color(&style.hover_background),
            border: border
                .or(color(&style.active_border_color))
                .map(|normal| [normal, color(&style.active_border_color).unwrap_or(normal)]),
        }
    }
}

/// Text colour of a node with an `active` condition: normal and active.
#[derive(Component)]
pub struct ActiveText(pub [Color; 2]);

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

/// Give a node hover and active colours from its style.
pub fn insert_state_colors(commands: &mut Commands, entity: Entity, style: &StyleDef) {
    let colors = StateColors::from_style(style);
    let mut e = commands.entity(entity);
    e.insert((BackgroundColor(colors.background[0]), Hovered::default()));
    if let Some([border, _]) = colors.border {
        e.insert(BorderColor::all(border));
    }
    e.insert(colors);
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
    if let Some(condition) = &def.active {
        commands
            .entity(entity)
            .insert(BoundActive(condition.clone()));
    }
    if def.active.is_some() || style.hover_background.is_some() {
        insert_state_colors(commands, entity, &style);
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
        LayoutRoot {
            id: layout.id.clone(),
            group: layout.group.clone(),
        },
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
            .layouts
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
            && root.is_none_or(|r| shown.visible(&r.id, r.group.as_deref()));
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
        let value = data
            .get(&bar.key)
            .and_then(UiValue::as_number)
            .unwrap_or(0.0);
        let (start, end) = bar.span(value);
        let (offset, size) = (
            Val::Percent(start * 100.0),
            Val::Percent((end - start) * 100.0),
        );
        let full = Val::Percent(100.0);
        let (left, bottom, width, height) = if bar.vertical {
            (Val::Px(0.0), offset, full, size)
        } else {
            (offset, Val::Px(0.0), size, full)
        };
        if node.left != left
            || node.bottom != bottom
            || node.width != width
            || node.height != height
        {
            node.left = left;
            node.bottom = bottom;
            node.width = width;
            node.height = height;
        }
    }
}

/// Apply hover and active colours.
pub fn update_states(
    data: Res<UiData>,
    mut nodes: Query<(
        &StateColors,
        Option<&BoundActive>,
        Option<&Hovered>,
        &mut BackgroundColor,
        Option<&mut BorderColor>,
    )>,
    mut texts: Query<(&ActiveText, &BoundActive, &mut TextColor)>,
) {
    for (colors, active, hovered, mut background, border) in &mut nodes {
        let active = usize::from(active.is_some_and(|a| data.condition(&a.0)));
        let hover = colors.hover.filter(|_| hovered.is_some_and(|h| h.get()));
        background.set_if_neq(BackgroundColor(hover.unwrap_or(colors.background[active])));
        if let (Some(mut border), Some(pair)) = (border, colors.border) {
            border.set_if_neq(BorderColor::all(pair[active]));
        }
    }
    for (colors, active, mut text) in &mut texts {
        let color = colors.0[usize::from(data.condition(&active.0))];
        text.set_if_neq(TextColor(color));
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
        let Some(target) = action.arg() else {
            continue;
        };
        let (layout, group) = (shown.shown(target), !shown.hidden_groups.contains(target));
        let (open, is_group) = match action.name() {
            "ui.toggle" => (!layout, false),
            "ui.show" => (true, false),
            "ui.hide" => (false, false),
            "ui.toggle_group" => (!group, true),
            "ui.show_group" => (true, true),
            "ui.hide_group" => (false, true),
            _ => continue,
        };
        if !is_group {
            shown.layouts.insert(target.to_owned(), open);
        } else if open {
            shown.hidden_groups.remove(target);
        } else {
            shown.hidden_groups.insert(target.to_owned());
        }
    }
}

/// Publish `ui.shown.<layout>` (open and not hidden with its group), so buttons can show
/// which windows are open.
pub fn publish_layouts(
    docs: Res<UiDocuments>,
    shown: Res<LayoutVisibility>,
    mut data: ResMut<UiData>,
) {
    if !docs.is_changed() && !shown.is_changed() {
        return;
    }
    for layout in &docs.layouts {
        let visible = shown.visible(&layout.id, layout.group.as_deref());
        data.set(format!("ui.shown.{}", layout.id), visible);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(min: f64, max: f64) -> BoundBar {
        BoundBar {
            key: String::new(),
            min,
            max,
            vertical: false,
        }
    }

    #[test]
    fn bars_fill_from_zero() {
        assert_eq!(bar(0.0, 1.0).span(0.25), (0.0, 0.25));
        assert_eq!(bar(0.0, 1.0).span(3.0), (0.0, 1.0));
        // Bipolar: from the middle, either way.
        assert_eq!(bar(-1.0, 1.0).span(0.5), (0.5, 0.75));
        assert_eq!(bar(-1.0, 1.0).span(-1.0), (0.0, 0.5));
        assert_eq!(bar(-1.0, 1.0).span(f64::NAN), (0.0, 0.0));
        // A range above zero fills from its start.
        assert_eq!(bar(10.0, 20.0).span(15.0), (0.0, 0.5));
    }
}
