//! The widget registry and built-in widgets.
//!
//! Any widget name not in the registry is a plain container: `column`, `row`, `panel`,
//! `spacer` and friends differ only by their default theme class. Built-ins with behaviour:
//! `label`, `button`, `bar`, `window` and `dock` (a container that lets clicks through to
//! the scene; with an id, layouts with that `dock` stack into it). Rust code can register
//! more widgets with [`WidgetRegistry::register`]; data-only mods compose these.

use std::collections::HashMap;

use bevy::{
    picking::{Pickable, hover::Hovered},
    prelude::*,
};

use crate::{
    format::Template,
    layout::NodeDef,
    runtime::{
        ActionSource, ActiveText, BoundActive, BoundBar, BoundText, Dock, WindowDrag, click_action,
        drag_window, insert_state_colors, spawn_box,
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

    /// Make `entity` a text node showing `template`. Follows the node's `active` condition
    /// if its style has an `active_color`.
    pub fn text(&mut self, entity: Entity, template: &str, style: &StyleDef) {
        let template = Template::parse(template);
        let mut e = self.commands.entity(entity);
        e.insert((
            Text::new(String::new()),
            TextFont::from_font_size(style.font_size()),
            TextColor(style.text_color()),
        ));
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
        registry.register("window", window);
        registry.register("dock", dock);
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
/// zero the fill grows either way from zero) and `vertical` (fill upwards).
fn bar(ctx: &mut WidgetContext) -> Entity {
    let number = |key: &str| ctx.def.extra.get(key).and_then(|v| v.as_f64());
    let bound = ctx.def.value.as_ref().map(|key| BoundBar {
        key: key.clone(),
        min: number("min").unwrap_or(0.0),
        max: number("max").unwrap_or(1.0),
        vertical: ctx.def.extra.get("vertical").and_then(|v| v.as_bool()) == Some(true),
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

fn window(ctx: &mut WidgetContext) -> Entity {
    let window = ctx.entity;
    let (title_bar, _) = ctx.child(window, "window_title", "");
    ctx.commands
        .entity(title_bar)
        .insert(WindowDrag { window })
        .observe(drag_window);
    let (title, title_style) = ctx.child(title_bar, "window_title_text", "");
    let title_text = ctx.def.title.clone().unwrap_or_default();
    ctx.text(title, &title_text, &title_style);
    ctx.child(title_bar, "spacer", "");

    let close_def = NodeDef {
        widget: "button".into(),
        class: "window_close".into(),
        text: Some("x".into()),
        action: Some(format!("ui.hide:{}", ctx.layout)),
        ..Default::default()
    };
    let close_style = ctx
        .theme
        .resolve("button", &close_def.class, &close_def.style);
    let close = spawn_box(ctx.commands, "button", &close_style, Some(title_bar));
    insert_state_colors(ctx.commands, close, &close_style);
    let layout = ctx.layout;
    button(&mut WidgetContext {
        commands: ctx.commands,
        entity: close,
        def: &close_def,
        style: &close_style,
        theme: ctx.theme,
        layout,
    });

    let (content, _) = ctx.child(window, "window_content", "");
    content
}
