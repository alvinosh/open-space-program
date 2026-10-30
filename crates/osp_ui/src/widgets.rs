//! The widget registry and built-in widgets.
//!
//! Any widget name not in the registry is a plain container: `column`, `row`, `panel`,
//! `spacer` and friends differ only by their default theme class. Built-ins with behaviour:
//! `label`, `button`, `bar`, `gauge` (an arc), `window`, `dock` (a container that lets clicks
//! through to the scene; with an id, layouts with that `dock` stack into it) and `image` (a
//! picture from a mod, PNG or SVG: `image: "icons/engine.svg"`). Rust code can register
//! more widgets with [`WidgetRegistry::register`]; data-only mods compose these.
//!
//! A `window` has a title bar to drag it by and a close button; a popup's window also has
//! a pin button (see [`crate::layout`]).

use std::collections::HashMap;

use bevy::{
    picking::{Pickable, hover::Hovered},
    prelude::*,
    text::LetterSpacing,
};

use crate::{
    format::Template,
    layout::NodeDef,
    runtime::{
        ActionSource, ActiveText, ActiveTint, BoundActive, BoundBar, BoundGauge, BoundText, Dock,
        UiImage, Uppercase, WindowDrag, click_action, drag_window, insert_state_colors, spawn_box,
    },
    style::{StyleDef, ThemeDef},
};

/// Passed to widget builders.
pub struct WidgetContext<'a, 'w, 's> {
    pub commands: &'a mut Commands<'w, 's>,
    /// The widget's node, already spawned with its resolved layout style.
    pub entity: Entity,
    pub def: &'a NodeDef,
    pub style: &'a StyleDef,
    pub theme: &'a ThemeDef,
    /// Id of the layout being built.
    pub layout: &'a str,
}

impl WidgetContext<'_, '_, '_> {
    /// Spawn a themed child node (`widget` picks the default class).
    pub fn child(&mut self, parent: Entity, widget: &str, class: &str) -> (Entity, StyleDef) {
        let style = self.theme.resolve(widget, class, &StyleDef::default());
        (
            spawn_box(self.commands, widget, &style, Some(parent)),
            style,
        )
    }

    /// Make `entity` a text node showing `template`, in the font of the theme's `text`
    /// class unless `style` sets its own. Follows the node's `active` condition if its
    /// style has an `active_color`.
    pub fn text(&mut self, entity: Entity, template: &str, style: &StyleDef) {
        let template = Template::parse(template);
        let style = &self.theme.text_style(style);
        let mut e = self.commands.entity(entity);
        e.insert((
            Text::new(String::new()),
            style.text_font(),
            TextColor(style.text_color()),
        ));
        if let Some(spacing) = style.letter_spacing {
            e.insert(LetterSpacing::Px(spacing));
        }
        if style.uppercase == Some(true) {
            e.insert(Uppercase);
        }
        e.insert(BoundText(template));
        if let (Some(condition), Some(active)) = (&self.def.active, &style.active_color) {
            e.insert((
                BoundActive(condition.clone()),
                ActiveText([style.text_color(), active.to_color()]),
            ));
        }
    }
}

/// Builds a widget. Returns the entity that the node's children should be attached to.
pub type WidgetBuilder = fn(&mut WidgetContext) -> Entity;

#[derive(Resource)]
pub struct WidgetRegistry {
    builders: HashMap<String, WidgetBuilder>,
}

impl Default for WidgetRegistry {
    fn default() -> Self {
        let mut registry = Self {
            builders: HashMap::new(),
        };
        registry.register("label", label);
        registry.register("button", button);
        registry.register("bar", bar);
        registry.register("gauge", gauge);
        registry.register("window", window);
        registry.register("dock", dock);
        registry.register("image", image);
        registry
    }
}

impl WidgetRegistry {
    pub fn register(&mut self, name: &str, builder: WidgetBuilder) {
        self.builders.insert(name.to_owned(), builder);
    }

    pub fn get(&self, name: &str) -> WidgetBuilder {
        self.builders.get(name).copied().unwrap_or(container)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.builders.keys().map(String::as_str)
    }
}

fn container(ctx: &mut WidgetContext) -> Entity {
    ctx.entity
}

/// The picture is tinted with the style's `color`, and `active_color` while the node's
/// `active` condition holds.
fn image(ctx: &mut WidgetContext) -> Entity {
    if let Some(path) = ctx.def.extra.get("image").and_then(|v| v.as_str()) {
        let tint = ctx.style.text_color();
        let mut e = ctx.commands.entity(ctx.entity);
        e.insert(UiImage {
            path: path.to_owned(),
            tint,
        });
        if let (Some(_), Some(active)) = (&ctx.def.active, &ctx.style.active_color) {
            e.insert(ActiveTint([tint, active.to_color()]));
        }
    }
    ctx.entity
}

fn dock(ctx: &mut WidgetContext) -> Entity {
    let mut e = ctx.commands.entity(ctx.entity);
    // Empty dock space is not UI: the scene underneath stays clickable.
    e.insert(Pickable::IGNORE);
    if let Some(id) = &ctx.def.id {
        e.insert(Dock(id.clone()));
    }
    ctx.entity
}

fn label(ctx: &mut WidgetContext) -> Entity {
    let text = ctx.def.text.clone().unwrap_or_default();
    let style = ctx.style.clone();
    ctx.text(ctx.entity, &text, &style);
    ctx.entity
}

fn button(ctx: &mut WidgetContext) -> Entity {
    ctx.commands.entity(ctx.entity).insert((
        ActionSource(Template::parse(
            ctx.def.action.as_deref().unwrap_or_default(),
        )),
        Hovered::default(),
    ));
    ctx.commands.entity(ctx.entity).observe(click_action);
    if let Some(text) = ctx.def.text.clone() {
        let (label, _) = ctx.child(ctx.entity, "button_text", "");
        let style = ctx.style.clone();
        ctx.text(label, &text, &style);
    }
    ctx.entity
}

/// A filled bar. Extra properties: `min` and `max` (default 0 and 1; with `min` below
/// zero the fill grows either way from zero), `vertical` (fill upwards) and `color` (a data
/// key holding the fill colour, instead of the theme's).
fn bar(ctx: &mut WidgetContext) -> Entity {
    let number = |key: &str| ctx.def.extra.get(key).and_then(|v| v.as_f64());
    let bound = ctx.def.value.as_ref().map(|key| BoundBar {
        key: key.clone(),
        min: number("min").unwrap_or(0.0),
        max: number("max").unwrap_or(1.0),
        vertical: ctx.def.extra.get("vertical").and_then(|v| v.as_bool()) == Some(true),
        color: ctx
            .def
            .extra
            .get("color")
            .and_then(|v| v.as_str())
            .map(str::to_owned),
    });
    let (fill, _) = ctx.child(ctx.entity, "bar_fill", "");
    let mut e = ctx.commands.entity(fill);
    e.entry::<Node>().and_modify(|mut node| {
        node.position_type = PositionType::Absolute;
        node.width = Val::Percent(0.0);
        node.height = Val::Percent(0.0);
    });
    if let Some(bound) = bound {
        e.insert(bound);
    }
    ctx.entity
}

/// An arc gauge: a ring segment from `start` to `end` degrees (0 at the top, clockwise;
/// an `end` below `start` runs anticlockwise), filled from `start` by `value` between `min`
/// and `max` (default 0 and 1). The node is the ring's square: its `border` is the
/// thickness, `border_color` the unfilled track and `color` the fill.
fn gauge(ctx: &mut WidgetContext) -> Entity {
    let number = |key: &str, default: f64| {
        ctx.def
            .extra
            .get(key)
            .and_then(|v| v.as_f64())
            .unwrap_or(default)
    };
    let (start, end) = (number("start", 225.0), number("end", 315.0));
    let track = ctx
        .style
        .border_color
        .as_ref()
        .map_or(Color::NONE, |c| c.to_color());
    let mut e = ctx.commands.entity(ctx.entity);
    // The arc is the border's gradient; a plain border colour would draw the whole ring.
    e.remove::<BorderColor>();
    e.entry::<Node>().and_modify(|mut node| {
        node.border_radius = BorderRadius::MAX;
    });
    e.insert(BoundGauge {
        key: ctx.def.value.clone().unwrap_or_default(),
        min: number("min", 0.0),
        max: number("max", 1.0),
        start: (start as f32).to_radians(),
        end: (end as f32).to_radians(),
        fill: ctx.style.text_color(),
        track,
        shown: None,
    });
    ctx.entity
}

/// The close and pin buttons' glyphs, relative to the mod root.
const CLOSE_ICON: &str = "icons/close.svg";
const PIN_ICON: &str = "icons/pin.svg";

/// A button in a window's title bar (class `class`) that fires `action`, lit while
/// `active` holds, with `icon` for a glyph (class `<class>_icon`).
fn title_button(
    ctx: &mut WidgetContext,
    bar: Entity,
    class: &str,
    action: String,
    active: Option<String>,
    icon: &str,
) {
    let def = NodeDef {
        widget: "button".into(),
        class: class.into(),
        action: Some(action),
        active: active.clone(),
        ..Default::default()
    };
    let style = ctx.theme.resolve("button", &def.class, &def.style);
    let entity = spawn_box(ctx.commands, "button", &style, Some(bar));
    insert_state_colors(ctx.commands, entity, &style);
    if let Some(condition) = &active {
        ctx.commands
            .entity(entity)
            .insert(BoundActive(condition.clone()));
    }
    let layout = ctx.layout;
    button(&mut WidgetContext {
        commands: ctx.commands,
        entity,
        def: &def,
        style: &style,
        theme: ctx.theme,
        layout,
    });
    let (glyph, glyph_style) = ctx.child(entity, &format!("{class}_icon"), "");
    let tint = glyph_style.text_color();
    let mut e = ctx.commands.entity(glyph);
    e.insert(UiImage {
        path: icon.to_owned(),
        tint,
    });
    if let (Some(condition), Some(lit)) = (active, &glyph_style.active_color) {
        e.insert((BoundActive(condition), ActiveTint([tint, lit.to_color()])));
    }
}

/// The title bar shows the `icon` image before the title. A `pin: true` window (popups
/// have it) gets a pin button that keeps it open (`ui.pin:<layout>`).
fn window(ctx: &mut WidgetContext) -> Entity {
    let window = ctx.entity;
    let (title_bar, _) = ctx.child(window, "window_title", "");
    ctx.commands
        .entity(title_bar)
        .insert(WindowDrag { window })
        .observe(drag_window);
    if let Some(icon) = ctx.def.extra.get("icon").and_then(|v| v.as_str()) {
        let (node, style) = ctx.child(title_bar, "window_icon", "");
        ctx.commands.entity(node).insert(UiImage {
            path: icon.to_owned(),
            tint: style.text_color(),
        });
    }
    let (title, title_style) = ctx.child(title_bar, "window_title_text", "");
    let title_text = ctx.def.title.clone().unwrap_or_default();
    ctx.text(title, &title_text, &title_style);
    ctx.child(title_bar, "spacer", "");

    let layout = ctx.layout.to_owned();
    if ctx.def.extra.get("pin").and_then(|v| v.as_bool()) == Some(true) {
        let (action, pinned) = (format!("ui.pin:{layout}"), format!("ui.pinned.{layout}"));
        title_button(ctx, title_bar, "window_pin", action, Some(pinned), PIN_ICON);
    }
    let close = format!("ui.hide:{layout}");
    title_button(ctx, title_bar, "window_close", close, None, CLOSE_ICON);

    let (content, _) = ctx.child(window, "window_content", "");
    content
}
