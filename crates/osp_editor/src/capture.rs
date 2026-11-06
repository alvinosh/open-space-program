//! The steps of the game's scripted captures (`osp_game::smoke_test`) that look inside the
//! editor: turning its view, putting the scripted pointer on a handle, a ring or a part, and
//! checking what the craft being built is. The script sends them as [`EditorStep`]s; each is taken
//! the frame it is sent, and one that fails ends the capture with an error.

use bevy::{
    math::{DVec2, DVec3},
    prelude::*,
};
use osp_game::{
    camera::MainCamera,
    content::{GameContent, catalog},
    frame::RenderFrame,
    orbit_lines::ScriptedPointer,
    sim::Sim,
    smoke_test::{Action, CaptureSet, EditorStep},
};
use osp_sim::{
    data::CraftPartDef,
    properties::{self, Value},
};

use crate::{
    building::Placing,
    hangar::Editor,
    moving::Moving,
    shaping::{Shaping, VesselFrame},
};

pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            take_steps
                .in_set(CaptureSet::Editor)
                .run_if(on_message::<EditorStep>),
        );
    }
}

/// What a step looks at.
struct Looking<'a> {
    editor: &'a Editor,
    shaping: &'a Shaping,
    placing: &'a Placing,
    moving: &'a Moving,
    sim: &'a Sim,
    frame: &'a RenderFrame,
    camera: (&'a Camera, &'a Transform),
    content: &'a GameContent,
}

#[allow(clippy::too_many_arguments)]
fn take_steps(
    mut steps: MessageReader<EditorStep>,
    mut editor: ResMut<Editor>,
    (shaping, placing, moving): (Res<Shaping>, Res<Placing>, Res<Moving>),
    (sim, frame, content): (Res<Sim>, Res<RenderFrame>, Res<GameContent>),
    camera: Single<(&Camera, &Transform), With<MainCamera>>,
    mut pointer: ResMut<ScriptedPointer>,
    mut exit: MessageWriter<AppExit>,
) {
    for &EditorStep(action) in steps.read() {
        // The view turns before anything is looked at.
        if let Action::EditorLook(x, y, zoom) = action {
            editor.look(DVec2::new(x as f64, y as f64), zoom as f64);
            continue;
        }
        let looking = Looking {
            editor: &editor,
            shaping: &shaping,
            placing: &placing,
            moving: &moving,
            sim: &sim,
            frame: &frame,
            camera: *camera,
            content: &content,
        };
        match take(action, &looking) {
            Ok(Some(at)) => pointer.0 = Some(at),
            Ok(None) => {}
            Err(e) => {
                error!("smoke test: {e}");
                exit.write(AppExit::error());
            }
        }
    }
}

/// Take `action`: where the pointer goes, if it moves, or why it failed.
fn take(action: Action, looking: &Looking) -> Result<Option<Vec2>, String> {
    let Looking {
        editor,
        shaping,
        placing,
        moving,
        ..
    } = looking;
    let part_def = |part: &str| {
        let draft = editor.builder().map(|b| b.draft());
        draft.and_then(|d| d.parts.iter().find(|p| p.id == part))
    };
    match action {
        Action::ExpectList(wanted) => {
            if editor.list_open() != wanted {
                return Err(format!("the list of vessels should be open: {wanted}"));
            }
        }
        Action::ExpectEditor(wanted) => {
            let found = editor.craft();
            if found != wanted {
                return Err(format!("the editor has {found:?}, wanted {wanted:?}"));
            }
        }
        Action::EditorPointAtGrip(size, right, down) => {
            let at = shaping.grip_on_screen(size);
            let at = at.ok_or_else(|| format!("no handle for `{size}` on screen"))?;
            return Ok(Some(at + Vec2::new(right, down)));
        }
        Action::EditorPointAtHandle(axis, at) => {
            let at = moving.handle_on_screen(axis, at);
            return Ok(Some(
                at.ok_or_else(|| format!("no handle on axis {axis} on screen"))?,
            ));
        }
        Action::ExpectAlong(part, low, high) => {
            let found = part_def(part).and_then(|p| p.at).map(|at| at.v);
            if !found.is_some_and(|v| (low..=high).contains(&v)) {
                return Err(format!(
                    "`{part}` is {found:?} along, wanted {low} to {high}"
                ));
            }
        }
        Action::ExpectHeldBy(part, wanted) => {
            let found = part_def(part).map(|p| p.by.clone());
            if found.as_deref() != Some(wanted) {
                return Err(format!("`{part}` is held by {found:?}, wanted `{wanted}`"));
            }
        }
        Action::EditorPointAtPart(part) => {
            let at = on_screen(looking, part, |built| Some(built.mass.center));
            return Ok(Some(
                at.ok_or_else(|| format!("`{part}` is not on screen"))?,
            ));
        }
        Action::ExpectField(part, field, wanted) => {
            let found = part_def(part).and_then(|p| field_value(p, field, looking.content));
            let matches = match &found {
                Some(Value::Number { value, .. }) => {
                    (wanted.parse::<f64>()).is_ok_and(|w| (w - value).abs() < 1e-6)
                }
                Some(Value::Choice { value, .. }) => value == wanted,
                Some(Value::Text(text)) => text == wanted,
                Some(Value::Toggle(on)) => on.to_string() == wanted,
                None => false,
            };
            if !matches {
                return Err(format!("`{part}` {field} is {found:?}, wanted {wanted}"));
            }
        }
        Action::EditorPointAtNode(part, node) => {
            let at = placing.node_on_screen(part, node);
            return Ok(Some(
                at.ok_or_else(|| format!("no ring on `{part}.{node}`"))?,
            ));
        }
        Action::EditorPointAtSurface(part, surface, u, v) => {
            let at = on_screen(looking, part, |built| {
                Some(built.geometry.surface_point(surface, u, v)?.position)
            });
            return Ok(Some(at.ok_or_else(|| {
                format!("`{part}` has no `{surface}` on screen")
            })?));
        }
        Action::ExpectPartCount(count) => {
            let found = editor.builder().map_or(0, |b| b.draft().parts.len());
            if found != count {
                return Err(format!("the craft has {found} parts, wanted {count}"));
            }
        }
        Action::ExpectPicked(wanted) => {
            let found = shaping.picked().unwrap_or("");
            if found != wanted {
                return Err(format!("`{found}` is picked, wanted `{wanted}`"));
            }
        }
        Action::ExpectSize(part, size, low, high) => {
            let found = part_def(part)
                .and_then(|p| {
                    (osp_sim::shaping::part_sizes(p).into_iter()).find(|s| s.name == size)
                })
                .map(|s| s.value);
            if !found.is_some_and(|v| (low..=high).contains(&v)) {
                return Err(format!(
                    "`{part}` {size} is {found:?}, wanted {low} to {high}"
                ));
            }
        }
        _ => return Err("not a step for the editor".into()),
    }
    Ok(None)
}

/// The value of `field` (`osp_sim::properties`) of `part`.
fn field_value(part: &CraftPartDef, field: &str, content: &GameContent) -> Option<Value> {
    let catalog = catalog(&content.0.registry, &mut Vec::new());
    (properties::fields(part, &catalog).into_iter())
        .find(|f| f.id == field)
        .map(|f| f.value)
}

/// Where on the screen `point` of `part` of the craft on show is: a point in the part's own frame,
/// from what it was built as.
fn on_screen(
    looking: &Looking,
    part: &str,
    point: impl FnOnce(&osp_sim::component::BuiltPart) -> Option<DVec3>,
) -> Option<Vec2> {
    let editor = looking.editor;
    let index = (editor.builder()?.draft().parts.iter()).position(|p| p.id == part)?;
    let vessel = editor.vessel()?;
    let shown = looking.sim.universe.vessel(vessel)?;
    let tree = shown.propulsion.parts();
    let on_show = tree.parts().get(index)?;
    let at = point(on_show.built.as_ref()?)?;
    let base = DVec3::Z * tree.base();
    let local = on_show.position - base + on_show.rotation * at;
    let view = VesselFrame::of(looking.sim, looking.frame, vessel)?.view_point(local);
    let (camera, transform) = looking.camera;
    (camera.world_to_viewport(&GlobalTransform::from(*transform), view)).ok()
}
