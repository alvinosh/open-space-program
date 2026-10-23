//! Building layouts into Bevy UI entities and keeping them in sync with [`UiData`].

use std::collections::{HashMap, HashSet};

use bevy::{
    input::mouse::MouseScrollPixelsPerLine,
    picking::{
        Pickable,
        events::{
            PointerClick, PointerDrag, PointerDragDrop, PointerDragEnd, PointerDragStart,
            PointerPress, PointerScroll,
        },
        hover::{HoverMap, Hovered},
        pointer::PointerButton,
    },
    prelude::*,
    ui::{ComputedUiRenderTargetInfo, UiGlobalTransform},
};

use crate::{
    data::{UiData, UiValue, record},
    format::{Formatters, Template, plain},
    layout::{LayoutDef, NodeDef, UiDocuments},
    style::{ColorDef, Length, StyleDef, ThemeDef},
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

/// A node that layouts with a matching `dock` stack into (the `dock` widget with an id).
#[derive(Component)]
pub struct Dock(pub String);

/// A layout root that belongs in a dock.
#[derive(Component, Debug)]
pub struct DockMember {
    pub dock: String,
    pub order: i32,
    pub state: DockState,
    /// The layout's own width; in the dock it is a minimum, so the column lines up.
    width: Val,
    /// Stacking order while docked.
    z: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockState {
    /// To be stacked into its dock.
    Pending,
    Docked,
    /// Dragged out of the dock; goes back when closed.
    Floating,
    /// Its dock does not exist; placed by its own position.
    Missing,
}

/// Window layouts come to the front when opened or clicked.
#[derive(Component)]
pub struct Raise;

/// Stacking order handed to the window raised last.
#[derive(Resource)]
pub struct WindowStack {
    next: i32,
}

impl Default for WindowStack {
    fn default() -> Self {
        // Above every layout's own index.
        Self { next: 1000 }
    }
}

/// Whether the pointer is over a UI node, so the game can leave mouse input (camera zoom,
/// map clicks) to the UI.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub struct PointerOverUi(pub bool);

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
    /// Data key of the fill colour (`"#rrggbb"`); empty or missing keeps the theme's.
    pub color: Option<String>,
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

/// A node that can be dragged onto `drop` nodes; what it carries (`drag`).
#[derive(Component)]
pub struct DragPayload(pub Template);

/// A node things can be dropped on: the action to fire, with `$drag` standing for what the
/// dropped node carries.
#[derive(Component)]
pub struct DropAction(pub Template);

/// Text shown beside a node while the pointer is over it (`tooltip`, a text template).
#[derive(Component)]
pub struct Tooltip(pub String);

/// An image from a mod (`image`, relative to the mod root), tinted with the style's text
/// colour. The game resolves the path through its mods and adds the `ImageNode`.
#[derive(Component, Debug, Clone)]
pub struct UiImage {
    pub path: String,
    pub tint: Color,
}

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

/// Tint of an `image` node with an `active` condition: normal and active.
#[derive(Component)]
pub struct ActiveTint(pub [Color; 2]);

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
    let entity = e.id();
    if style.scroll == Some(true) {
        e.insert((Scrollable, ScrollPosition::default()))
            .observe(scroll_wheel);
        commands.spawn((
            ScrollThumb,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(2.0),
                width: Val::Px(4.0),
                display: Display::None,
                border_radius: BorderRadius::all(Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.37, 0.83, 0.54, 0.7)),
            // Above docked windows, below windows raised over the dock.
            GlobalZIndex(999),
            Pickable::IGNORE,
            ChildOf(entity),
        ));
    }
    entity
}

/// A `scroll` node.
#[derive(Component)]
pub struct Scrollable;

/// The scroll position indicator of a `scroll` node, shown while its content overflows.
#[derive(Component)]
pub struct ScrollThumb;

pub fn update_scroll_thumbs(
    areas: Query<(&ComputedNode, &ScrollPosition, &Children), With<Scrollable>>,
    mut thumbs: Query<&mut Node, With<ScrollThumb>>,
) {
    for (computed, position, children) in &areas {
        let scale = computed.inverse_scale_factor;
        let (view, content) = (computed.size().y * scale, computed.content_size().y * scale);
        for child in children.iter() {
            let Ok(mut thumb) = thumbs.get_mut(child) else {
                continue;
            };
            let overflow = content > view + 0.5 && view > 0.0;
            let display = if overflow {
                Display::Flex
            } else {
                Display::None
            };
            if thumb.display != display {
                thumb.display = display;
            }
            if overflow {
                // In content coordinates, which scroll with the content.
                let top = Val::Px(position.y + position.y * view / content);
                let height = Val::Px(view * view / content);
                if thumb.top != top || thumb.height != height {
                    thumb.top = top;
                    thumb.height = height;
                }
            }
        }
    }
}

/// Scroll a `scroll` node with the mouse wheel while the pointer is over it or its
/// children, as far as its content reaches.
pub fn scroll_wheel(
    mut scroll: On<PointerScroll>,
    mut areas: Query<(&ComputedNode, &mut ScrollPosition), With<Scrollable>>,
    lines: Option<Res<MouseScrollPixelsPerLine>>,
) {
    let Ok((computed, mut position)) = areas.get_mut(scroll.entity) else {
        return;
    };
    let scale = computed.inverse_scale_factor;
    let range = ((computed.content_size().y - computed.size().y) * scale).max(0.0);
    if range <= 0.0 {
        return;
    }
    let ratio = lines.map_or(MouseScrollPixelsPerLine::default(), |r| *r);
    let delta = scroll.to_pixels(&ratio).y;
    position.y = (position.y - delta).clamp(0.0, range);
    scroll.propagate(false);
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
    let text = |key: &str| def.extra.get(key).and_then(|v| v.as_str());
    if let Some(payload) = text("drag") {
        commands
            .entity(entity)
            .insert(DragPayload(Template::parse(payload)))
            .observe(drag_start)
            .observe(drag_move)
            .observe(drag_end);
    }
    if let Some(action) = text("drop") {
        commands
            .entity(entity)
            .insert(DropAction(Template::parse(action)))
            .observe(drop_on);
    }
    if let Some(tip) = text("tooltip") {
        commands
            .entity(entity)
            .insert((Tooltip(tip.to_owned()), Hovered::default()));
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
    let z = style.z_index.unwrap_or(index as i32);
    let mut e = commands.entity(root);
    e.insert((
        LayoutRoot {
            id: layout.id.clone(),
            group: layout.group.clone(),
        },
        GlobalZIndex(z),
        Name::new(format!("layout:{}", layout.id)),
    ));
    if let Some(dock) = &layout.dock {
        e.insert(DockMember {
            dock: dock.clone(),
            order: layout.dock_order,
            state: DockState::Pending,
            width: style.width.as_ref().map_or(Val::Auto, Length::to_val),
            z,
        });
    }
    if layout.root.widget == "window" {
        e.insert(Raise).observe(raise_on_press);
    }
}

/// (Re)build every layout when the documents change (startup, hot reload).
pub fn rebuild_ui(
    mut commands: Commands,
    docs: Res<UiDocuments>,
    widgets: Res<WidgetRegistry>,
    // Docked layouts go with their dock.
    roots: Query<Entity, (With<LayoutRoot>, Without<ChildOf>)>,
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

/// Stack layouts waiting for their dock into it.
pub fn attach_docked(
    mut commands: Commands,
    docks: Query<(Entity, &Dock)>,
    mut members: Query<(
        Entity,
        &LayoutRoot,
        &mut DockMember,
        &mut Node,
        &mut GlobalZIndex,
    )>,
) {
    for (entity, root, mut member, mut node, mut z) in &mut members {
        if member.state != DockState::Pending {
            continue;
        }
        let Some((dock, _)) = docks.iter().find(|(_, d)| d.0 == member.dock) else {
            warn!(
                "layout `{}`: no dock `{}`; placing it on its own",
                root.id, member.dock
            );
            member.state = DockState::Missing;
            continue;
        };
        member.state = DockState::Docked;
        node.position_type = PositionType::Relative;
        (node.left, node.right, node.top, node.bottom) =
            (Val::Auto, Val::Auto, Val::Auto, Val::Auto);
        node.width = Val::Auto;
        node.min_width = member.width;
        node.flex_shrink = 0.0;
        z.0 = member.z;
        commands.entity(entity).insert(ChildOf(dock));
    }
}

/// Keep each dock's layouts in `dock_order`.
pub fn sort_docks(
    mut docks: Query<&mut Children, (With<Dock>, Changed<Children>)>,
    members: Query<&DockMember>,
) {
    let order = |e: &Entity| members.get(*e).map_or(i32::MAX, |m| m.order);
    for mut children in &mut docks {
        if !children.is_sorted_by_key(order) {
            children.sort_by_key(order);
        }
    }
}

/// Put windows dragged out of their dock back when they are closed.
pub fn redock_closed(
    shown: Res<LayoutVisibility>,
    mut members: Query<(&LayoutRoot, &mut DockMember)>,
) {
    for (root, mut member) in &mut members {
        if member.state == DockState::Floating && !shown.shown(&root.id) {
            member.state = DockState::Pending;
        }
    }
}

/// Whether a window floats (raising a docked one would put it over the dock's other parts).
fn floats(member: Option<&DockMember>) -> bool {
    member.is_none_or(|m| matches!(m.state, DockState::Floating | DockState::Missing))
}

/// Bring floating windows to the front as they open.
pub fn raise_opened(
    shown: Res<LayoutVisibility>,
    mut open: Local<HashSet<String>>,
    mut stack: ResMut<WindowStack>,
    mut windows: Query<(&LayoutRoot, &mut GlobalZIndex, Option<&DockMember>), With<Raise>>,
) {
    for (root, mut z, member) in &mut windows {
        let visible = shown.visible(&root.id, root.group.as_deref());
        if visible && open.insert(root.id.clone()) && floats(member) {
            stack.next += 1;
            z.0 = stack.next;
        } else if !visible {
            open.remove(&root.id);
        }
    }
}

fn raise_on_press(
    press: On<PointerPress>,
    mut stack: ResMut<WindowStack>,
    mut windows: Query<(&mut GlobalZIndex, Option<&DockMember>), With<Raise>>,
) {
    if let Ok((mut z, member)) = windows.get_mut(press.entity)
        && floats(member)
        && z.0 != stack.next
    {
        stack.next += 1;
        z.0 = stack.next;
    }
}

pub fn track_pointer_over_ui(
    hover: Option<Res<HoverMap>>,
    nodes: Query<(), With<Node>>,
    mut over: ResMut<PointerOverUi>,
) {
    let any = hover.is_some_and(|hover| {
        hover
            .values()
            .any(|hits| hits.keys().any(|e| nodes.contains(*e)))
    });
    over.set_if_neq(PointerOverUi(any));
}

/// Rebuild repeated lists whose length changed. A list inside another one that is rebuilt
/// now goes with it (its new copy fills itself next frame), so it is left alone.
pub fn update_repeaters(
    mut commands: Commands,
    data: Res<UiData>,
    docs: Res<UiDocuments>,
    widgets: Res<WidgetRegistry>,
    mut repeaters: Query<(Entity, &mut Repeater)>,
    parents: Query<&ChildOf>,
) {
    let changed: Vec<Entity> = repeaters
        .iter()
        .filter(|(_, r)| data.list_len(&r.list) != r.count)
        .map(|(e, _)| e)
        .collect();
    for &entity in &changed {
        if parents.iter_ancestors(entity).any(|a| changed.contains(&a)) {
            continue;
        }
        let Ok((_, mut repeater)) = repeaters.get_mut(entity) else {
            continue;
        };
        let count = data.list_len(&repeater.list);
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

pub fn update_bars(
    data: Res<UiData>,
    mut bars: Query<(&BoundBar, &mut Node, Option<&mut BackgroundColor>)>,
) {
    for (bar, mut node, background) in &mut bars {
        if let (Some(key), Some(mut background)) = (&bar.color, background)
            && let Some(UiValue::Text(color)) = data.get(key)
            && !color.is_empty()
        {
            background.set_if_neq(BackgroundColor(ColorDef::Hex(color.clone()).to_color()));
        }
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
    mut images: Query<(&ActiveTint, &BoundActive, &mut ImageNode)>,
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
    for (tints, active, mut image) in &mut images {
        let color = tints.0[usize::from(data.condition(&active.0))];
        if image.color != color {
            image.color = color;
        }
    }
}

/// Gap between a node and its tooltip, px.
const TOOLTIP_GAP: f32 = 6.0;

/// Show the tooltip of the node under the pointer beside it, on the side towards the middle
/// of the screen and centred on it (class `tooltip`); it follows the node if the layout
/// moves it. `shown` is the node and its tooltip.
pub fn update_tooltip(
    mut commands: Commands,
    widgets: Res<WidgetRegistry>,
    docs: Res<UiDocuments>,
    sources: Query<(
        Entity,
        &Tooltip,
        &Hovered,
        &ComputedNode,
        &UiGlobalTransform,
        &ComputedUiRenderTargetInfo,
    )>,
    mut rows: Query<&mut Node>,
    mut shown: Local<Option<(Entity, Entity)>>,
) {
    // Not before the node has been laid out.
    let hovered = sources
        .iter()
        .find(|(_, _, hovered, computed, ..)| hovered.get() && computed.size().x > 0.0);
    if let Some((source, tip)) = *shown
        && hovered.is_none_or(|(e, ..)| e != source)
    {
        commands.entity(tip).try_despawn();
        *shown = None;
    }
    let Some((source, tooltip, _, computed, transform, target)) = hovered else {
        return;
    };
    let scale = computed.inverse_scale_factor;
    let (centre, size) = (transform.translation * scale, computed.size() * scale);
    let screen = target.physical_size().as_vec2() * scale;
    // A zero-height row through the node's middle, so the label centres on it.
    let (left, right) = if centre.x > screen.x / 2.0 {
        let right = screen.x - (centre.x - size.x / 2.0) + TOOLTIP_GAP;
        (Val::Auto, Val::Px(right))
    } else {
        (Val::Px(centre.x + size.x / 2.0 + TOOLTIP_GAP), Val::Auto)
    };
    let top = Val::Px(centre.y);
    if let Some((_, tip)) = *shown {
        if let Ok(mut row) = rows.get_mut(tip)
            && (row.left, row.right, row.top) != (left, right, top)
        {
            (row.left, row.right, row.top) = (left, right, top);
        }
        return;
    }
    let label = NodeDef {
        widget: "label".into(),
        class: "tooltip".into(),
        text: Some(tooltip.0.clone()),
        ..default()
    };
    let text = spawn_node(&mut commands, &widgets, &docs.theme, "", &label, None);
    commands.entity(text).insert(Pickable::IGNORE);
    let row = Node {
        position_type: PositionType::Absolute,
        left,
        right,
        top,
        height: Val::Px(0.0),
        align_items: AlignItems::Center,
        ..default()
    };
    let tip = commands
        .spawn((row, GlobalZIndex(i32::MAX - 1), Pickable::IGNORE))
        .add_child(text)
        .id();
    *shown = Some((source, tip));
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

/// Pick up a `drag` node: it follows the pointer, in front of everything, and lets the
/// pointer see through it to where it will be dropped.
fn drag_start(
    mut event: On<PointerDragStart>,
    mut commands: Commands,
    payloads: Query<(), With<DragPayload>>,
    children: Query<&Children>,
) {
    if event.button != PointerButton::Primary || !payloads.contains(event.entity) {
        return;
    }
    event.propagate(false);
    // A list may rebuild its nodes mid-drag, so they may be gone when these run.
    let dragged = event.entity;
    for e in std::iter::once(dragged).chain(children.iter_descendants(dragged)) {
        commands.entity(e).try_insert(Pickable::IGNORE);
    }
    commands
        .entity(dragged)
        .try_insert(GlobalZIndex(i32::MAX - 1));
}

fn drag_move(mut event: On<PointerDrag>, mut nodes: Query<&mut Node, With<DragPayload>>) {
    let Ok(mut node) = nodes.get_mut(event.entity) else {
        return;
    };
    event.propagate(false);
    node.left = Val::Px(event.distance.x);
    node.top = Val::Px(event.distance.y);
}

/// Put a dragged node back where it was (a drop, if any, has fired its action).
fn drag_end(
    mut event: On<PointerDragEnd>,
    mut commands: Commands,
    mut nodes: Query<&mut Node, With<DragPayload>>,
    children: Query<&Children>,
) {
    let dragged = event.entity;
    let Ok(mut node) = nodes.get_mut(dragged) else {
        return;
    };
    event.propagate(false);
    (node.left, node.top) = (Val::Auto, Val::Auto);
    for e in std::iter::once(dragged).chain(children.iter_descendants(dragged)) {
        commands.entity(e).try_remove::<Pickable>();
    }
    commands.entity(dragged).try_remove::<GlobalZIndex>();
}

/// Something was dropped on a `drop` node: fire its action with what it carried. What the
/// pointer grabbed may be a child of the `drag` node (its icon, say), so the payload is the
/// nearest one up from there.
fn drop_on(
    mut event: On<PointerDragDrop>,
    drops: Query<&DropAction>,
    payloads: Query<&DragPayload>,
    parents: Query<&ChildOf>,
    data: Res<UiData>,
    formatters: Res<Formatters>,
    mut actions: MessageWriter<UiAction>,
) {
    let Ok(drop) = drops.get(event.entity) else {
        return;
    };
    let dropped = event.dropped;
    let Some(payload) = std::iter::once(dropped)
        .chain(parents.iter_ancestors(dropped))
        .find_map(|e| payloads.get(e).ok())
    else {
        return;
    };
    event.propagate(false);
    let action = drop_action(&drop.0, &payload.0, &data, &formatters);
    if !action.is_empty() {
        actions.write(UiAction(action));
    }
}

/// The action a drop fires: the drop node's template with `$drag` replaced by what the
/// dragged node carries.
fn drop_action(drop: &Template, payload: &Template, data: &UiData, f: &Formatters) -> String {
    let payload = payload.render(data, f);
    drop.render(data, f).replace("$drag", &payload)
}

/// Move a window by its title bar, from where it is on screen (however it was placed). A
/// docked window pops out of its dock and floats from there.
pub fn drag_window(
    event: On<PointerDrag>,
    mut commands: Commands,
    handles: Query<&WindowDrag>,
    mut windows: Query<(
        &mut Node,
        Option<&mut DockMember>,
        &ComputedNode,
        &UiGlobalTransform,
        &mut GlobalZIndex,
    )>,
    mut stack: ResMut<WindowStack>,
) {
    let Ok(handle) = handles.get(event.entity) else {
        return;
    };
    let Ok((mut node, member, computed, transform, mut z)) = windows.get_mut(handle.window) else {
        return;
    };
    let scale = computed.inverse_scale_factor;
    let size = computed.size() * scale;
    let top_left = transform.translation * scale - size / 2.0;
    if let Some(mut member) = member.filter(|m| m.state == DockState::Docked) {
        member.state = DockState::Floating;
        commands.entity(handle.window).try_remove::<ChildOf>();
        node.position_type = PositionType::Absolute;
        node.width = Val::Px(size.x);
        stack.next += 1;
        z.0 = stack.next;
    }
    node.left = Val::Px(top_left.x + event.delta.x);
    node.top = Val::Px(top_left.y + event.delta.y);
    node.right = Val::Auto;
    node.bottom = Val::Auto;
    node.margin = UiRect::ZERO;
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
            color: None,
        }
    }

    #[test]
    fn drops_carry_what_was_dragged() {
        let mut data = UiData::default();
        data.set("row", 2);
        data.set("seq", 1);
        let action = drop_action(
            &Template::parse("staging.drop:$drag:{row}"),
            &Template::parse("item:{seq}:0"),
            &data,
            &Formatters::default(),
        );
        assert_eq!(action, "staging.drop:item:1:0:2");
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

#[cfg(test)]
mod dock_tests {
    use bevy::input::InputPlugin;
    use serde_json::json;

    use super::*;
    use crate::UiPlugin;

    fn layout(value: serde_json::Value) -> LayoutDef {
        serde_json::from_value(value).unwrap()
    }

    fn window(id: &str, dock: Option<&str>, order: i32) -> LayoutDef {
        layout(json!({
            "id": id,
            "dock": dock,
            "dock_order": order,
            "root": { "widget": "window", "title": id, "style": { "left": 40, "top": 50, "width": 300 } },
        }))
    }

    fn app(layouts: Vec<LayoutDef>) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin, UiPlugin));
        let dock = layout(json!({
            "id": "docks",
            "root": { "widget": "dock", "children": [{ "id": "side", "widget": "dock" }] },
        }));
        app.insert_resource(UiDocuments {
            layouts: std::iter::once(dock).chain(layouts).collect(),
            ..default()
        });
        for _ in 0..3 {
            app.update();
        }
        app
    }

    fn root(app: &mut App, id: &str) -> Entity {
        app.world_mut()
            .query::<(Entity, &LayoutRoot)>()
            .iter(app.world())
            .find(|(_, r)| r.id == id)
            .unwrap()
            .0
    }

    fn dock(app: &mut App) -> Entity {
        app.world_mut()
            .query::<(Entity, &Dock)>()
            .iter(app.world())
            .next()
            .unwrap()
            .0
    }

    fn docked(app: &mut App) -> Vec<String> {
        let dock = dock(app);
        let children: Vec<Entity> = app
            .world()
            .get::<Children>(dock)
            .map_or(Vec::new(), |c| c.to_vec());
        children
            .into_iter()
            .filter_map(|e| app.world().get::<LayoutRoot>(e).map(|r| r.id.clone()))
            .collect()
    }

    #[test]
    fn docked_windows_stack_in_dock_order_and_lose_their_own_position() {
        let mut app = app(vec![
            window("second", Some("side"), 20),
            window("first", Some("side"), 10),
            window("free", None, 0),
        ]);
        assert_eq!(docked(&mut app), ["first", "second"]);
        let first = root(&mut app, "first");
        let node = app.world().get::<Node>(first).unwrap();
        assert_eq!(node.position_type, PositionType::Relative);
        assert_eq!((node.left, node.top), (Val::Auto, Val::Auto));
        // Its width becomes a minimum, so the column lines up on the widest window.
        assert_eq!((node.width, node.min_width), (Val::Auto, Val::Px(300.0)));
        // Undocked layouts keep their place.
        let free = root(&mut app, "free");
        assert!(app.world().get::<ChildOf>(free).is_none());
        assert_eq!(app.world().get::<Node>(free).unwrap().left, Val::Px(40.0));
    }

    #[test]
    fn a_missing_dock_leaves_the_layout_on_its_own() {
        let mut app = app(vec![window("lost", Some("nowhere"), 0)]);
        let lost = root(&mut app, "lost");
        assert!(app.world().get::<ChildOf>(lost).is_none());
        let member = app.world().get::<DockMember>(lost).unwrap();
        assert_eq!(member.state, DockState::Missing);
    }

    #[test]
    fn a_window_dragged_out_goes_back_when_closed() {
        let mut app = app(vec![window("w", Some("side"), 0)]);
        let w = root(&mut app, "w");
        // As a drag does: out of the dock, floating.
        app.world_mut().entity_mut(w).remove::<ChildOf>();
        app.world_mut().get_mut::<DockMember>(w).unwrap().state = DockState::Floating;
        app.update();
        assert!(docked(&mut app).is_empty());
        app.world_mut().write_message(UiAction("ui.hide:w".into()));
        app.update();
        app.update();
        assert_eq!(docked(&mut app), ["w"]);
    }

    fn nested_lists(app: &mut App, rows: &[usize]) {
        let rows: Vec<_> = rows
            .iter()
            .map(|&n| {
                let items: Vec<_> = (0..n).map(|i| record([("i", i.into())])).collect();
                record([("items", items.into())])
            })
            .collect();
        app.world_mut().resource_mut::<UiData>().set("rows", rows);
    }

    #[test]
    fn a_list_and_the_lists_in_it_can_change_in_the_same_frame() {
        let mut app = app(vec![layout(json!({
            "id": "lists",
            "root": {
                "repeat": "rows",
                "children": [{
                    "repeat": "item.items",
                    "children": [{ "widget": "label", "text": "{item.i}" }],
                }],
            },
        }))]);
        nested_lists(&mut app, &[1]);
        app.update();
        app.update();
        // More rows, and more items in the first: the inner list goes with its row.
        nested_lists(&mut app, &[3, 2]);
        app.update();
        app.update();
        let texts = app
            .world_mut()
            .query::<&BoundText>()
            .iter(app.world())
            .count();
        assert_eq!(texts, 5);
    }

    #[test]
    fn reloading_rebuilds_docks_and_their_windows() {
        let mut app = app(vec![window("w", Some("side"), 0)]);
        let docs = app.world().resource::<UiDocuments>().clone();
        app.insert_resource(docs);
        for _ in 0..3 {
            app.update();
        }
        let roots = app
            .world_mut()
            .query::<&LayoutRoot>()
            .iter(app.world())
            .count();
        assert_eq!(roots, 2);
        assert_eq!(docked(&mut app), ["w"]);
    }
}
