//! The part window in the editor, as KSP's part menu with its procedural parts: a click on a part
//! opens it beside the pointer (and a part just put on opens it), with everything that can be set
//! on that part (`osp_sim::properties`): its shape and every size, what its tank holds, its tank
//! type and utilization, its material, a sizer's requirement. Numbers are typed (`editor.set:<field>:
//! <text>`) or stepped (`editor.step:<field>:<-1|1>`), choices drop down (`editor.expand:<field>`)
//! and are picked with `editor.set`. What the part comes to (volume, propellant, dry and full mass)
//! is under them. Each change goes through the builder, so it is checked and can be undone; a
//! refusal says why in the window. The handles on the part stay, for dragging a size by eye.
//!
//! A click elsewhere closes the window unless it is pinned; the part stays picked. Publishes
//! `editor.part.picked`, `editor.part.name`, `editor.part.rows` and `editor.part.problem` (see
//! the `editor_part` layout).

use std::sync::Arc;

use bevy::prelude::*;
use osp_sim::{
    data::{CraftPartDef, PartCatalog},
    properties::{self, Unit, Value},
};
use osp_ui::{LayoutVisibility, UiAction, UiData, UiRecord, UiValue, record};

use osp_game::{
    FrameSet,
    content::{GameContent, catalog},
    sim::Sim,
};

use crate::{
    hangar::Editor,
    shaping::{Shaping, nice_step, shape_parts},
};

/// The window's layout.
const LAYOUT: &str = "editor_part";

pub struct PartWindowPlugin;

impl Plugin for PartWindowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PartWindow>().add_systems(
            Update,
            (
                part_window.in_set(FrameSet::Input).after(shape_parts),
                publish_part_window,
            ),
        );
    }
}

/// Which part the window is for, which of its choices is dropped down, and why the last change
/// was refused.
#[derive(Resource, Default)]
pub struct PartWindow {
    part: Option<String>,
    open_field: Option<String>,
    problem: String,
}

/// `value` in `unit`, as the window shows it.
fn shown(value: f64, unit: Unit) -> String {
    match unit {
        Unit::Metres => format!("{value:.2} m"),
        Unit::Millimetres => format!("{value:.1} mm"),
        Unit::Percent => format!("{value:.0}%"),
        Unit::Degrees => format!("{value:.1}°"),
        Unit::G => format!("{value:.1} g"),
        Unit::Ratio => format!("{value:.2}"),
        Unit::Other(unit) => {
            let a = value.abs();
            if a >= 1e6 {
                format!("{:.2} M{unit}", value / 1e6)
            } else if a >= 1e3 {
                format!("{:.2} k{unit}", value / 1e3)
            } else {
                format!("{value:.2} {unit}")
            }
        }
    }
}

/// How far a step of a number goes: about a twentieth of it, or of its range at 0.
fn step_of(value: f64, least: f64, most: f64) -> f64 {
    let base = if value.abs() > 1e-9 {
        0.05 * value.abs()
    } else {
        0.01 * (most - least).abs()
    };
    nice_step(base)
}

#[allow(clippy::too_many_arguments)]
fn part_window(
    mut window: ResMut<PartWindow>,
    mut shaping: ResMut<Shaping>,
    mut editor: ResMut<Editor>,
    mut sim: ResMut<Sim>,
    mut ui: ResMut<LayoutVisibility>,
    content: Res<GameContent>,
    mut actions: MessageReader<UiAction>,
    mut stores: Local<Option<Arc<PartCatalog>>>,
) {
    if !editor.is_open() {
        if window.part.take().is_some() || ui.shown(LAYOUT) {
            ui.layouts.insert(LAYOUT.into(), false);
        }
        actions.clear();
        return;
    }
    if stores.is_none() || content.is_changed() {
        *stores = Some(Arc::new(catalog(&content.0.registry, &mut Vec::new())));
    }
    let Some(catalog) = stores.as_ref().map(Arc::clone) else {
        return;
    };
    // A part picked anew, or clicked again, opens the window beside the pointer.
    let picked = shaping.picked().map(str::to_owned);
    let clicked = shaping.take_click();
    if picked != window.part {
        window.open_field = None;
        window.problem.clear();
        window.part = picked.clone();
        ui.layouts.insert(LAYOUT.into(), picked.is_some());
    } else if clicked && picked.is_some() {
        ui.layouts.insert(LAYOUT.into(), true);
    }
    for action in actions.read() {
        let Some(arg) = action.arg() else {
            continue;
        };
        match action.name() {
            "editor.set" => {
                if let Some((field, text)) = arg.split_once(':') {
                    apply(
                        &mut window,
                        &mut editor,
                        &catalog,
                        &mut sim,
                        &mut ui,
                        field,
                        text,
                    );
                }
            }
            "editor.step" => {
                let Some((field, way)) = arg.split_once(':') else {
                    continue;
                };
                let part = (window.part.as_ref())
                    .and_then(|id| editor.builder()?.draft().parts.iter().find(|p| &p.id == id))
                    .cloned();
                let Some(part) = part else {
                    continue;
                };
                let found = properties::fields(&part, &catalog)
                    .into_iter()
                    .find(|f| f.id == field);
                if let Some(properties::Field {
                    value:
                        Value::Number {
                            value, least, most, ..
                        },
                    ..
                }) = found
                {
                    let up = way.trim_start_matches('+').parse::<f64>().unwrap_or(1.0);
                    let step = step_of(value, least, most);
                    // On a whole step, so that steps add up to round numbers.
                    let next = ((value / step).round() + up.signum()) * step;
                    let text = next.clamp(least, most).to_string();
                    apply(
                        &mut window,
                        &mut editor,
                        &catalog,
                        &mut sim,
                        &mut ui,
                        field,
                        &text,
                    );
                }
            }
            "editor.expand" => {
                window.open_field = if window.open_field.as_deref() == Some(arg) {
                    None
                } else {
                    Some(arg.to_owned())
                };
            }
            _ => {}
        }
    }
}

/// Set `field` of the window's part from `text` through the builder, and show the craft as it is
/// then; or say why not.
fn apply(
    window: &mut PartWindow,
    editor: &mut Editor,
    catalog: &PartCatalog,
    sim: &mut Sim,
    ui: &mut LayoutVisibility,
    field: &str,
    text: &str,
) {
    let Some(id) = window.part.clone() else {
        return;
    };
    let Some(builder) = editor.builder_mut() else {
        return;
    };
    match builder.set_field(catalog, &id, field, text) {
        Ok(change) => {
            // Whatever was dropped down closes once something is set.
            window.problem.clear();
            window.open_field = None;
            if !change.changed.is_empty() {
                let draft = builder.draft().clone();
                if let Err(e) = editor.redraw(&draft, catalog, sim, ui) {
                    error!("cannot show the craft: {e}");
                }
                editor.settle(sim);
            }
        }
        Err(refusal) => window.problem = format!("Not done: {refusal}"),
    }
}

/// The rows of the window for `part`, `open` the choice dropped down, with the sizes a handle in
/// hand has in place of the part's own.
fn rows(
    part: &CraftPartDef,
    catalog: &PartCatalog,
    open: Option<&str>,
    dragged: &[(&str, f64)],
) -> Vec<UiRecord> {
    let mut out = Vec::new();
    let mut section = "Part";
    let heading = |label: &str| record([("kind", "section".into()), ("label", label.into())]);
    for field in properties::fields(part, catalog) {
        if field.section != section {
            section = field.section;
            out.push(heading(section));
        }
        let id = field.id.as_str();
        let set = format!("editor.set:{id}:");
        let row = match field.value {
            Value::Text(text) => record([
                ("kind", "text".into()),
                ("label", field.label.into()),
                ("value", text.into()),
                ("set", set.into()),
            ]),
            Value::Number { value, unit, .. } => {
                let value = (dragged.iter())
                    .find(|(name, _)| *name == id)
                    .map_or(value, |(_, v)| *v);
                record([
                    ("kind", "number".into()),
                    ("label", field.label.into()),
                    ("value", shown(value, unit).into()),
                    ("set", set.into()),
                    ("down", format!("editor.step:{id}:-1").into()),
                    ("up", format!("editor.step:{id}:1").into()),
                ])
            }
            Value::Choice { value, options } => {
                let label = (options.iter())
                    .find(|(o, _)| *o == value)
                    .map_or(value.clone(), |(_, l)| l.clone());
                let choices: Vec<UiRecord> = (options.iter())
                    .map(|(option, shown)| {
                        record([
                            ("label", shown.clone().into()),
                            ("selected", (*option == value).into()),
                            ("action", format!("{set}{option}").into()),
                        ])
                    })
                    .collect();
                record([
                    ("kind", "choice".into()),
                    ("label", field.label.into()),
                    ("value", label.into()),
                    ("expand", format!("editor.expand:{id}").into()),
                    ("open", (open == Some(id)).into()),
                    ("options", UiValue::List(choices)),
                ])
            }
            Value::Toggle(on) => record([
                ("kind", "toggle".into()),
                ("label", field.label.into()),
                ("value", if on { "On" } else { "Off" }.into()),
                ("on", on.into()),
                ("toggle", format!("{set}{}", !on).into()),
            ]),
        };
        out.push(row);
    }
    let readouts = properties::readouts(part, catalog);
    if !readouts.is_empty() {
        out.push(heading("Comes to"));
        for (label, value) in readouts {
            out.push(record([
                ("kind", "readout".into()),
                ("label", label.into()),
                ("value", value.into()),
            ]));
        }
    }
    out
}

/// `editor.part.*`, see the module.
fn publish_part_window(
    window: Res<PartWindow>,
    shaping: Res<Shaping>,
    editor: Res<Editor>,
    content: Res<GameContent>,
    mut data: ResMut<UiData>,
    mut stores: Local<Option<PartCatalog>>,
    mut published: Local<
        Option<(
            Option<CraftPartDef>,
            Option<String>,
            String,
            Vec<(String, u64)>,
        )>,
    >,
) {
    if stores.is_none() || content.is_changed() {
        *stores = Some(catalog(&content.0.registry, &mut Vec::new()));
    }
    let Some(catalog) = stores.as_ref() else {
        return;
    };
    let part = (window.part.as_ref())
        .and_then(|id| editor.builder()?.draft().parts.iter().find(|p| &p.id == id))
        .cloned();
    let dragged = shaping.drag_values();
    let now = (
        part.clone(),
        window.open_field.clone(),
        window.problem.clone(),
        (dragged.iter())
            .map(|(name, value)| ((*name).to_owned(), value.to_bits()))
            .collect(),
    );
    if published.as_ref() == Some(&now) && !content.is_changed() {
        return;
    }
    *published = Some(now);
    data.set("editor.part.picked", part.is_some());
    data.set("editor.part.problem", window.problem.as_str());
    let Some(part) = part else {
        data.set("editor.part.name", "");
        data.set("editor.part.rows", Vec::<UiRecord>::new());
        return;
    };
    let name = part.name.clone().unwrap_or_else(|| part.id.clone());
    data.set("editor.part.name", name);
    data.set(
        "editor.part.rows",
        rows(&part, catalog, window.open_field.as_deref(), &dragged),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_in_their_units() {
        assert_eq!(shown(3.8, Unit::Metres), "3.80 m");
        assert_eq!(shown(88.0, Unit::Percent), "88%");
        assert_eq!(shown(1.0e6, Unit::Other("N")), "1.00 MN");
        assert_eq!(shown(4.0, Unit::Millimetres), "4.0 mm");
    }

    #[test]
    fn a_step_is_about_a_twentieth_of_the_number_and_round() {
        assert_eq!(step_of(3.8, 0.05, 200.0), 0.2);
        assert_eq!(step_of(1.9, 0.05, 200.0), 0.1);
        assert_eq!(step_of(0.0, -30.0, 30.0), 1.0);
    }
}
