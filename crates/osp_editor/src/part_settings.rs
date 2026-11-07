//! The picked part's settings, in the editor's rail on the left in place of the parts list, as
//! KSP's part menu with its procedural parts: everything that can be set on that part
//! (`osp_sim::properties`): its shape and every size, what its tank holds, its tank type and
//! utilization, its material, a sizer's requirement. Numbers are typed (`editor.set:<field>:
//! <text>`) or stepped (`editor.step:<field>:<-1|1>`), choices drop down (`editor.expand:<field>`)
//! and are picked with `editor.set`. What the part comes to (volume, propellant, dry and full mass)
//! is under them. Each change goes through the builder, so it is checked and can be undone; a
//! refusal says why under them. The handles on the part stay, for dragging a size by eye.
//!
//! They show while a part is picked: a click on a part, or a part just put on. Letting go of it (a
//! click on nothing, Esc, the close button or Parts) brings the parts list back.
//!
//! Publishes `editor.rail`, which of the rail's screens shows (`vessels`, `parts`, `part`, or
//! nothing), and `editor.part.picked`, `editor.part.name`, `editor.part.rows` and
//! `editor.part.problem` (see the `editor` layout).

use std::sync::Arc;

use bevy::prelude::*;
use osp_sim::{
    data::{CraftPartDef, PartCatalog},
    properties::{self, Mix, Unit, Value},
};
use osp_ui::{LayoutVisibility, UiAction, UiData, UiRecord, UiValue, record};

use osp_game::{
    FrameSet,
    content::{GameContent, catalog},
    sim::Sim,
};

use crate::{
    building::Placing,
    hangar::Editor,
    shaping::{Shaping, nice_step, shape_parts},
};

pub struct PartSettingsPlugin;

impl Plugin for PartSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PartSettings>().add_systems(
            Update,
            (
                part_settings.in_set(FrameSet::Input).after(shape_parts),
                publish_part_settings,
                publish_rail,
            ),
        );
    }
}

/// Which part the settings are for, which of its choices is dropped down, and why the last change
/// was refused.
#[derive(Resource, Default)]
pub struct PartSettings {
    part: Option<String>,
    open_field: Option<String>,
    problem: String,
}

/// `value` in `unit`, as the settings show it.
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
fn part_settings(
    mut window: ResMut<PartSettings>,
    shaping: Res<Shaping>,
    mut editor: ResMut<Editor>,
    mut sim: ResMut<Sim>,
    mut ui: ResMut<LayoutVisibility>,
    content: Res<GameContent>,
    mut actions: MessageReader<UiAction>,
    mut stores: Local<Option<Arc<PartCatalog>>>,
) {
    if !editor.is_open() {
        window.part = None;
        actions.clear();
        return;
    }
    if stores.is_none() || content.is_changed() {
        *stores = Some(Arc::new(catalog(&content.0.registry, &mut Vec::new())));
    }
    let Some(catalog) = stores.as_ref().map(Arc::clone) else {
        return;
    };
    // A part picked anew starts with nothing dropped down and nothing refused.
    let picked = shaping.picked().map(str::to_owned);
    if picked != window.part {
        window.open_field = None;
        window.problem.clear();
        window.part = picked;
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
                let Some(draft) = editor.builder().map(|b| b.draft()) else {
                    continue;
                };
                let part =
                    (window.part.as_ref()).and_then(|id| draft.parts.iter().find(|p| &p.id == id));
                let Some(part) = part else {
                    continue;
                };
                let mixes = properties::mixes(draft, &catalog, &part.id);
                let found = properties::fields(part, &mixes, &catalog)
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

/// Set `field` of the picked part from `text` through the builder, and show the craft as it is
/// then; or say why not.
fn apply(
    window: &mut PartSettings,
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

/// The rows of the settings for `part`, `open` the choice dropped down, with the sizes a handle in
/// hand has in place of the part's own. Its tank can hold `mixes`, what the engines it feeds burn.
fn rows(
    part: &CraftPartDef,
    mixes: &[Mix],
    catalog: &PartCatalog,
    open: Option<&str>,
    dragged: &[(&str, f64)],
) -> Vec<UiRecord> {
    let mut out = Vec::new();
    let mut section = "Part";
    let heading = |label: &str| record([("kind", "section".into()), ("label", label.into())]);
    for field in properties::fields(part, mixes, catalog) {
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
        // A tank that feeds no engine has nothing to be filled with until one goes on under it.
        if id == "contents" && mixes.is_empty() {
            out.push(record([
                ("kind", "readout".into()),
                ("label", "Feeds".into()),
                ("value", "No engine yet".into()),
            ]));
        }
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
fn publish_part_settings(
    window: Res<PartSettings>,
    shaping: Res<Shaping>,
    editor: Res<Editor>,
    content: Res<GameContent>,
    mut data: ResMut<UiData>,
    mut stores: Local<Option<PartCatalog>>,
    mut fed: Local<Option<(Vec<CraftPartDef>, String, Vec<Mix>)>>,
    mut published: Local<
        Option<(
            Option<CraftPartDef>,
            Vec<Mix>,
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
    let draft = editor.builder().map(|b| b.draft());
    let part = (window.part.as_ref())
        .and_then(|id| draft?.parts.iter().find(|p| &p.id == id))
        .cloned();
    // What the engines it feeds burn, which its tank can hold: an engine put on or taken off
    // changes them while the part stays as it was. Working them out builds the craft, so only
    // when the craft or the part changes.
    if content.is_changed() {
        *fed = None;
    }
    let mixes = match (&part, draft) {
        (Some(part), Some(draft)) => {
            let fresh = (fed.as_ref())
                .is_some_and(|(parts, id, _)| *id == part.id && *parts == draft.parts);
            if !fresh {
                let mixes = properties::mixes(draft, catalog, &part.id);
                *fed = Some((draft.parts.clone(), part.id.clone(), mixes));
            }
            fed.as_ref()
                .map(|(.., mixes)| mixes.clone())
                .unwrap_or_default()
        }
        _ => Vec::new(),
    };
    let dragged = shaping.drag_values();
    let now = (
        part.clone(),
        mixes.clone(),
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
        rows(
            &part,
            &mixes,
            catalog,
            window.open_field.as_deref(),
            &dragged,
        ),
    );
}

/// Which of the rail's screens shows: the list of vessels while it is out, the picked part's
/// settings, the parts list while it is out, or nothing.
fn rail(list_open: bool, picked: bool, parts_open: bool) -> &'static str {
    if list_open {
        "vessels"
    } else if picked {
        "part"
    } else if parts_open {
        "parts"
    } else {
        ""
    }
}

/// `editor.rail`, see the module.
fn publish_rail(
    editor: Res<Editor>,
    shaping: Res<Shaping>,
    placing: Res<Placing>,
    mut data: ResMut<UiData>,
    mut published: Local<&'static str>,
) {
    let now = rail(
        editor.list_open(),
        shaping.picked().is_some(),
        placing.is_open(),
    );
    if *published != now || data.get("editor.rail").is_none() {
        *published = now;
        data.set("editor.rail", now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rail_shows_the_list_then_a_picked_part_then_the_parts() {
        assert_eq!(rail(true, true, true), "vessels");
        assert_eq!(rail(false, true, true), "part");
        assert_eq!(rail(false, true, false), "part");
        assert_eq!(rail(false, false, true), "parts");
        assert_eq!(rail(false, false, false), "");
    }

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
