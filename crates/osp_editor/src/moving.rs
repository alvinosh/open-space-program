//! Moving and turning parts in the editor, as KSP's offset and rotate tools. The editor has three
//! tools, picked in its bar (`editor.tool:<shape|move|turn>`) or with the keys 1, 2 and 3, and
//! `editor.tool` says which is on: Shape, where a picked part shows the handles that size it
//! (`osp_editor::shaping`); Move, where it shows three arrows through its middle; and Turn, where it
//! shows three rings round the point it is held by.
//!
//! The arrows and rings lie along the connector that holds the part: on a surface, round it (red),
//! along it (green) and out of it (blue); on a node, across it (red, green) and along it (blue). An
//! arrow dragged slides the part and all it holds, in steps that suit how near the view is (Shift
//! for steps a tenth as big); a ring dragged turns it about that axis a turn step at a time: 15°,
//! or what `editor.snap` or C (Shift+C back) set, 1° to 90° (`editor.snap` reads it), finer with
//! Shift. The
//! parts put on with it by symmetry go with it, mirrored if they were (`osp_sim::builder::alike`).
//! What is on show is built again as the pointer goes, on a thread of its own; let go, the builder
//! checks the change and keeps it, or puts it back and says why under the launch button. Esc while
//! dragging puts it back. The root holds the rest, so it has no handles.
//!
//! The handles are drawn over everything, a size on the screen whatever the distance, and lit under
//! the pointer; a label by the pointer says how far it has gone.

use std::{sync::Arc, thread::JoinHandle};

use bevy::{
    math::{DQuat, DVec3},
    prelude::*,
    window::{CursorIcon, PrimaryWindow, SystemCursorIcon},
};
use osp_sim::{
    assembly::Pose,
    builder::{self, Holding},
    craft::Craft,
    data::{CraftDef, PartCatalog},
    propulsion::Propulsion,
    vessel::VesselId,
};
use osp_ui::{LayoutVisibility, TextFocus, UiAction, UiData, UiDocuments, ViewOverlay};

use osp_game::{
    FrameSet,
    camera::MainCamera,
    content::{GameContent, catalog},
    controls::InputLocked,
    frame::RenderFrame,
    orbit_lines::ViewPointer,
    sim::Sim,
};

use crate::{
    building::{Placing, place_parts},
    hangar::{Editor, editor_actions},
    shaping::{Shaping, VesselFrame, along_line, in_plane, nice_step, shape_parts},
};

/// Half an arrow's length and a ring's radius on the screen, px.
const ARROW_PX: f64 = 64.0;
const RING_PX: f64 = 52.0;
/// How near the pointer has to come to a handle to take it, px.
const REACH_PX: f32 = 9.0;
/// A step of a slide is about this far on the screen, px.
const STEP_PX: f64 = 6.0;
/// A step of a turn, degrees, and with Shift.
const TURN_STEP: f64 = 15.0;
const FINE_TURN_STEP: f64 = 5.0;
/// The steps a turn can go in, degrees: `editor.snap` and C step through them.
const ANGLE_STEPS: [f64; 6] = [1.0, 5.0, 15.0, 22.5, 45.0, 90.0];
/// A ring whose axis is less than this far out of the screen (the cosine of the angle between them)
/// turns as the pointer goes across it, not round it.
const EDGE_ON: f64 = 0.3;
/// How many straight pieces a ring is drawn and looked for with.
const RING_SIDES: usize = 48;
/// The axes' colours, X, Y and Z, and a handle under the pointer or in hand.
const AXES: [Color; 3] = [
    Color::srgb(0.937, 0.416, 0.380),
    Color::srgb(0.471, 0.851, 0.451),
    Color::srgb(0.361, 0.620, 0.980),
];
const LIT: Color = Color::srgb(1.0, 0.910, 0.451);
const LABEL_FILL: Color = Color::srgba(0.043, 0.086, 0.125, 0.94);
const LABEL_TEXT: Color = Color::srgb(0.902, 0.941, 0.961);
const LABEL_OFFSET: Vec2 = Vec2::new(16.0, -24.0);

pub struct MovingPlugin;

impl Plugin for MovingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Moving>()
            .init_gizmo_group::<HandleGizmos>()
            .add_systems(Startup, (configure_gizmos, spawn_label))
            .add_systems(
                Update,
                (
                    move_parts
                        .in_set(FrameSet::Input)
                        .after(editor_actions)
                        .before(place_parts)
                        .before(shape_parts),
                    // After the camera has moved this frame, so the handles stay on the part.
                    draw_handles.in_set(FrameSet::Render),
                    publish_tool,
                ),
            );
    }
}

/// The handles are drawn over everything.
#[derive(Default, Reflect, GizmoConfigGroup)]
struct HandleGizmos;

fn configure_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<HandleGizmos>();
    config.depth_bias = -1.0;
    config.line.width = 3.0;
}

/// What the pointer does to a picked part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    /// Its sizes, by the handles of `osp_editor::shaping`.
    #[default]
    Shape,
    /// Slide it, by arrows.
    Move,
    /// Turn it, by rings.
    Turn,
}

impl Tool {
    fn id(self) -> &'static str {
        match self {
            Tool::Shape => "shape",
            Tool::Move => "move",
            Tool::Turn => "turn",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [Tool::Shape, Tool::Move, Tool::Turn]
            .into_iter()
            .find(|t| t.id() == text)
    }
}

/// An arrow or a ring on the picked part.
#[derive(Debug, Clone)]
struct Handle {
    /// Which axis of the connector it is on: 0 to 2, X to Z.
    axis_index: usize,
    /// The way it slides or the axis it turns about (unit), and where it is drawn from: an arrow's
    /// middle, a ring's centre. The vessel's frame.
    axis: DVec3,
    origin: DVec3,
    /// Two ways square to `axis` and to each other, which a ring lies along.
    across: [DVec3; 2],
    /// From the vessel's frame to the craft's, which the builder works in.
    to_craft: DQuat,
    /// Half the arrow's length or the ring's radius, m.
    size: f64,
    /// Where it was drawn last, px: its ends, or the points round it.
    screen: Vec<Vec2>,
}

/// A handle in hand.
#[derive(Debug, Clone)]
struct Drag {
    handle: usize,
    part: String,
    /// The craft as it was when it was taken hold of.
    start: CraftDef,
    /// Where it was taken hold of: m along the arrow or rad round the ring.
    grabbed: f64,
    /// The angle round the ring last seen, to count it past a whole turn.
    last: f64,
    /// How far it has slid (m) or turned (rad), in steps, how far what is on show has, and how far
    /// was last asked to be built.
    value: f64,
    shown: f64,
    asked: f64,
    /// Where the root was on show when it was taken hold of: the craft stands on its lowest point,
    /// so what is on show moves as a part below slides, and the root says how far.
    root: DVec3,
    /// The root itself in hand: where it was from the middle of the floor. Its arrows carry the
    /// whole craft about the hangar, and its rings turn the whole craft about it.
    anchor: Option<DVec3>,
    /// A step, m or rad.
    step: f64,
    /// A ring seen nearly edge on turns as the pointer goes across it instead: where the pointer
    /// was taken hold at, and the way across the screen that turns it forward.
    across: Option<(Vec2, Vec2)>,
    /// Esc: put it back as it was.
    cancel: bool,
}

/// The craft as a handle in hand has made it, being built to go on show.
struct Building {
    task: JoinHandle<Result<(CraftDef, Propulsion), String>>,
    value: f64,
}

/// The tool the pointer is, and its handles on the picked part.
#[derive(Resource)]
pub struct Moving {
    tool: Tool,
    /// How far a turn goes in a step, degrees.
    angle: f64,
    handles: Vec<Handle>,
    /// The handle under the pointer, by its place in `handles`.
    over: Option<usize>,
    drag: Option<Drag>,
    building: Option<Building>,
    /// How far what is on show has moved since the handle in hand was taken hold of.
    drift: DVec3,
    was_down: bool,
    /// How the picked part is held, found for that part on the vessel on show, and where the part
    /// is in the craft's frame, which the holding is in.
    held: Option<(String, VesselId, Option<(Holding, Pose)>)>,
}

impl Default for Moving {
    fn default() -> Self {
        Self {
            tool: Tool::default(),
            angle: TURN_STEP,
            handles: Vec::new(),
            over: None,
            drag: None,
            building: None,
            drift: DVec3::ZERO,
            was_down: false,
            held: None,
        }
    }
}

impl Moving {
    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// How far a turn goes in a step, degrees, and with Shift.
    pub fn angle_step(&self, fine: bool) -> f64 {
        if fine {
            fine_angle(self.angle)
        } else {
            self.angle
        }
    }

    /// Put back a handle in hand: whether there was one.
    pub fn let_go(&mut self) -> bool {
        match self.drag.as_mut() {
            Some(drag) => {
                drag.cancel = true;
                true
            }
            None => false,
        }
    }

    /// Where a point of the handle on `axis` (0 to 2) was drawn last, px: on an arrow `at` is from
    /// -1 at one tip to 1 at the other, on a ring the share of a turn round from where it starts.
    /// For scripted captures.
    pub fn handle_on_screen(&self, axis: usize, at: f32) -> Option<Vec2> {
        let handle = self.handles.iter().find(|h| h.axis_index == axis)?;
        match (self.tool, handle.screen.as_slice()) {
            (Tool::Move, [a, b]) => Some(a.lerp(*b, (at + 1.0) / 2.0)),
            (Tool::Turn, points) if points.len() == RING_SIDES => {
                let k = (at * RING_SIDES as f32).round() as isize;
                Some(points[k.rem_euclid(RING_SIDES as isize) as usize])
            }
            _ => None,
        }
    }
}

/// How far from `at` the nearest point of the line through `points` is, px.
fn distance_to(points: &[Vec2], closed: bool, at: Vec2) -> f32 {
    let count = points.len();
    let pieces = if closed {
        count
    } else {
        count.saturating_sub(1)
    };
    (0..pieces)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % count]);
            let ab = b - a;
            let t = ((at - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            at.distance(a + ab * t)
        })
        .fold(f32::INFINITY, f32::min)
}

/// Two unit ways square to `axis` (unit) and to each other, right-handed with it.
fn square_to(axis: DVec3) -> [DVec3; 2] {
    let a = axis.any_orthonormal_vector();
    [a, axis.cross(a)]
}

/// Degrees in (-180, 180] as radians, for an angle gone round past a whole turn.
fn wrapped(angle: f64) -> f64 {
    let turn = std::f64::consts::TAU;
    (angle + turn / 2.0).rem_euclid(turn) - turn / 2.0
}

#[allow(clippy::too_many_arguments)]
fn move_parts(
    mut moving: ResMut<Moving>,
    mut shaping: ResMut<Shaping>,
    mut editor: ResMut<Editor>,
    mut sim: ResMut<Sim>,
    mut ui: ResMut<LayoutVisibility>,
    content: Res<GameContent>,
    (buttons, keys, locked, typing): (
        Res<ButtonInput<MouseButton>>,
        Res<ButtonInput<KeyCode>>,
        Res<InputLocked>,
        Res<TextFocus>,
    ),
    pointer: ViewPointer,
    frame: Res<RenderFrame>,
    camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut actions: MessageReader<UiAction>,
    (mut parts_catalog, placing): (Local<Option<Arc<PartCatalog>>>, Res<Placing>),
) {
    let moving = &mut *moving;
    let down = buttons.pressed(MouseButton::Left);
    let went_down = down && !moving.was_down;
    moving.was_down = down;
    for action in actions.read() {
        if action.name() == "editor.tool"
            && let Some(tool) = action.arg().and_then(Tool::parse)
        {
            moving.tool = tool;
        }
        if action.name() == "editor.snap" {
            moving.angle = next_angle(moving.angle, true);
        }
    }
    if !locked.0 && !typing.typing() && keys.just_pressed(KeyCode::KeyC) {
        let back = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        moving.angle = next_angle(moving.angle, !back);
    }
    if !locked.0 && !typing.typing() && moving.drag.is_none() {
        let keyed = [
            (KeyCode::Digit1, Tool::Shape),
            (KeyCode::Digit2, Tool::Move),
            (KeyCode::Digit3, Tool::Turn),
        ];
        for (key, tool) in keyed {
            if keys.just_pressed(key) {
                moving.tool = tool;
            }
        }
    }
    let quiet = |moving: &mut Moving, shaping: &mut Shaping| {
        moving.handles.clear();
        moving.over = None;
        moving.drift = DVec3::ZERO;
        shaping.set_handles(false, false);
    };
    let (Some(vessel), Some(draft)) =
        (editor.vessel(), editor.builder().map(|b| b.draft().clone()))
    else {
        moving.drag = None;
        moving.building = None;
        moving.held = None;
        quiet(moving, &mut shaping);
        return;
    };
    if parts_catalog.is_none() || content.is_changed() {
        *parts_catalog = Some(Arc::new(catalog(&content.0.registry, &mut Vec::new())));
    }
    let Some(shared) = parts_catalog.clone() else {
        return;
    };
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let Some(vessel_frame) = VesselFrame::of(&sim, &frame, vessel) else {
        return;
    };
    let (camera, eye) = *camera;
    let cursor = pointer.position();
    let look = cursor
        .and_then(|at| camera.viewport_to_world(eye, at).ok())
        .map(|ray| {
            (
                vessel_frame.vessel_point(ray.origin),
                vessel_frame.vessel_way(*ray.direction).normalize(),
            )
        });

    // A handle in hand follows the pointer until it is let go.
    if moving.drag.is_some() {
        let finished = moving.drag.as_ref().is_some_and(|d| d.cancel) || !down;
        if finished {
            moving.building = None;
            if let Some(drag) = moving.drag.take() {
                let handle = moving.handles.get(drag.handle).cloned();
                finish(
                    drag,
                    handle,
                    moving.tool,
                    &mut editor,
                    &shared,
                    &mut sim,
                    &mut ui,
                );
            }
            moving.held = None;
            quiet(moving, &mut shaping);
            return;
        }
        let tool = moving.tool;
        let held = moving.drag.as_ref().map(|d| d.handle);
        let (Some(mut handle), Some(drag)) = (
            held.and_then(|i| moving.handles.get(i)).cloned(),
            moving.drag.as_mut(),
        ) else {
            return;
        };
        moving.drift = root_on_show(&sim, vessel, &draft).map_or(DVec3::ZERO, |at| at - drag.root);
        handle.origin += moving.drift;
        // The craft carried by its root moves under the pointer: the pointer is taken in the frame
        // it had when it was taken hold of.
        let carried = (drag.anchor.zip(editor.anchor())).map_or(DVec3::ZERO, |(from, now)| {
            vessel_frame.vessel_way_from_sim(now - from)
        });
        match (drag.across, cursor) {
            (Some((from, way)), Some(at)) => {
                let gone = f64::from((at - from).dot(way)) / RING_PX;
                drag.value = stepped(gone, tool, drag.step, shift);
            }
            (Some(_), None) => {}
            (None, _) => {
                if let Some((from, look)) = look {
                    follow(drag, &handle, tool, from + carried, look, shift);
                }
            }
        }
        if let (Some(from), Tool::Move) = (drag.anchor, tool) {
            let way = vessel_frame.sim_way(handle.axis * drag.value);
            editor.carry_to(from + way);
            shaping.set_handles(true, true);
            return;
        }
        // What has been built goes on show.
        let done = moving
            .building
            .as_ref()
            .is_some_and(|b| b.task.is_finished());
        if let Some(built) = moving.building.take_if(|_| done) {
            match built
                .task
                .join()
                .unwrap_or_else(|_| Err("building it failed".into()))
            {
                Ok((craft, propulsion)) => {
                    editor.redraw_built(&craft, propulsion, &mut sim, &mut ui);
                    editor.set_notice("");
                    drag.shown = built.value;
                }
                Err(e) => editor.set_notice(format!("Cannot: {e}")),
            }
        }
        // The newest place is built next, once nothing else is being built.
        if drag.value != drag.asked && moving.building.is_none() {
            drag.asked = drag.value;
            let (by, turn) = motion(tool, &handle, drag.value);
            let (start, id, catalog, value) = (
                drag.start.clone(),
                drag.part.clone(),
                shared.clone(),
                drag.value,
            );
            let root = drag.anchor.is_some();
            let task = std::thread::spawn(move || {
                let craft = if root {
                    builder::turned_root(&start, turn).map_err(|r| r.to_string())?
                } else {
                    builder::shifted(&start, &catalog, &id, by, turn)
                        .map_err(|r| r.to_string())?
                        .0
                };
                let propulsion =
                    Propulsion::from_craft(&craft, &catalog).map_err(|e| e.to_string())?;
                Ok((craft, propulsion))
            });
            moving.building = Some(Building { task, value });
        }
        shaping.set_handles(true, true);
        return;
    }

    // Handles go on the picked part, while the tool is not Shape and nothing is in hand.
    let picked = shaping.picked().map(str::to_owned);
    let index = picked
        .as_ref()
        .and_then(|id| draft.parts.iter().position(|p| &p.id == id));
    let (Some(id), Some(index), false, true) = (
        picked,
        index,
        placing.is_holding(),
        moving.tool != Tool::Shape,
    ) else {
        quiet(moving, &mut shaping);
        return;
    };
    let fresh = (moving.held.as_ref()).is_some_and(|(part, on, _)| *part == id && *on == vessel);
    if !fresh {
        let holding = Craft::build(&draft, &shared).ok().and_then(|craft| {
            let pose = *craft.assembly.poses().get(index)?;
            Some((builder::holding(&craft, index)?, pose))
        });
        moving.held = Some((id.clone(), vessel, holding));
    }
    let root = draft.parts[index].on.is_none();
    let holding = match moving.held.clone() {
        Some((_, _, Some(holding))) => Some(holding),
        _ if root => None,
        _ => {
            quiet(moving, &mut shaping);
            return;
        }
    };
    let Some(shown) = sim.universe.vessel(vessel) else {
        return;
    };
    let tree = shown.propulsion.parts();
    let base = crate::shaping::tree_origin(tree);
    let Some(on_show) = tree.parts().get(index) else {
        return;
    };
    let middle = on_show
        .built
        .as_ref()
        .map_or(DVec3::ZERO, |b| b.mass.center);
    let middle = on_show.position - base + on_show.rotation * middle;
    // How the handles lie, and the point it turns about.
    let (to_vessel, axes, pivot) = match holding {
        // The craft as built and the vessel on show need not share a frame: the part is the same
        // in both.
        Some((holding, in_craft)) => {
            let to_vessel =
                Pose::new(on_show.position - base, on_show.rotation) * in_craft.inverse();
            let axes = to_vessel.rotation * holding.frame.rotation;
            (to_vessel.rotation, axes, to_vessel.point(holding.pivot))
        }
        // The root: carried along the hangar's axes, and turned with everything about the craft's
        // own at its origin.
        None => {
            let axes = match moving.tool {
                Tool::Move => vessel_frame.turn_from_sim(),
                _ => DQuat::IDENTITY,
            };
            (DQuat::IDENTITY, axes, on_show.position - base)
        }
    };
    // Metres a pixel is worth where the handles are.
    let on_screen = |at: DVec3| {
        camera
            .world_to_viewport(eye, vessel_frame.view_point(at))
            .ok()
    };
    let right = vessel_frame.vessel_way(*eye.right()).normalize();
    let per_pixel = |at: DVec3| {
        let (Some(a), Some(b)) = (on_screen(at), on_screen(at + right)) else {
            return None;
        };
        Some(1.0 / f64::from(a.distance(b)).max(1e-3))
    };
    let (origin, px) = match moving.tool {
        Tool::Move => (middle, ARROW_PX),
        _ => (pivot, RING_PX),
    };
    let Some(per_pixel) = per_pixel(origin) else {
        quiet(moving, &mut shaping);
        return;
    };
    let old = std::mem::take(&mut moving.handles);
    moving.handles = (0..3)
        .map(|axis_index| {
            let axis = axes * [DVec3::X, DVec3::Y, DVec3::Z][axis_index];
            let screen = (old.iter())
                .find(|h| h.axis_index == axis_index)
                .map(|h| h.screen.clone())
                .unwrap_or_default();
            Handle {
                axis_index,
                axis,
                origin,
                across: square_to(axis),
                to_craft: to_vessel.inverse(),
                size: px * per_pixel,
                screen,
            }
        })
        .collect();
    let tool = moving.tool;
    moving.over = cursor.and_then(|at| {
        (moving.handles.iter().enumerate())
            .filter(|(_, h)| h.screen.len() > 1)
            .map(|(i, h)| (i, distance_to(&h.screen, tool == Tool::Turn, at)))
            .filter(|(_, d)| *d <= REACH_PX)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    });
    if went_down
        && let (Some(over), Some((from, look))) = (moving.over, look)
        && let Some(handle) = moving.handles.get(over)
        && let Some(grabbed) = taken_at(handle, tool, from, look)
    {
        let step = match tool {
            Tool::Move => nice_step(per_pixel * STEP_PX),
            _ => moving.angle.to_radians(),
        };
        // Seen edge on, the near side of the ring goes the way it turns.
        let across = (tool == Tool::Turn && handle.axis.dot(look).abs() < EDGE_ON)
            .then(|| {
                let forward = handle.axis.cross(-look).normalize_or_zero() * handle.size;
                let (a, b) = (
                    on_screen(handle.origin)?,
                    on_screen(handle.origin + forward)?,
                );
                Some((cursor?, (b - a).normalize_or_zero()))
            })
            .flatten();
        let root_picked = root;
        let root = root_on_show(&sim, vessel, &draft).unwrap_or_default();
        moving.drag = Some(Drag {
            handle: over,
            part: id,
            start: draft,
            grabbed,
            last: grabbed,
            value: 0.0,
            shown: 0.0,
            asked: 0.0,
            root,
            anchor: if root_picked { editor.anchor() } else { None },
            step,
            across,
            cancel: false,
        });
        moving.drift = DVec3::ZERO;
        editor.set_notice("");
    }
    let (over, dragging) = (moving.over.is_some(), moving.drag.is_some());
    shaping.set_handles(over || dragging, dragging);
}

/// Where the root of `draft` is on show as `vessel`, in its frame.
fn root_on_show(sim: &Sim, vessel: VesselId, draft: &CraftDef) -> Option<DVec3> {
    let root = draft.parts.iter().position(|p| p.on.is_none())?;
    let tree = sim.universe.vessel(vessel)?.propulsion.parts();
    Some(tree.parts().get(root)?.position - crate::shaping::tree_origin(tree))
}

/// Where the pointer, a ray from `from` the way `look`, takes `handle`: m along an arrow, or the
/// angle round a ring (rad).
fn taken_at(handle: &Handle, tool: Tool, from: DVec3, look: DVec3) -> Option<f64> {
    match tool {
        Tool::Move => along_line(handle.origin, handle.axis, from, look),
        _ => {
            let [a, b] = handle.across;
            let (s, t) = in_plane(handle.origin, a, b, from, look)?;
            Some(t.atan2(s))
        }
    }
}

/// Follow the pointer with the handle in hand, in steps.
fn follow(drag: &mut Drag, handle: &Handle, tool: Tool, from: DVec3, look: DVec3, fine: bool) {
    let Some(now) = taken_at(handle, tool, from, look) else {
        return;
    };
    let gone = match tool {
        Tool::Move => now - drag.grabbed,
        _ => {
            // Round past a whole turn, the angle carries on.
            let turned = drag.last + wrapped(now - drag.last);
            drag.last = turned;
            turned - drag.grabbed
        }
    };
    drag.value = stepped(gone, tool, drag.step, fine);
}

/// `gone` in whole steps: `step`, or with Shift a tenth of it for a slide and a finer one for a
/// turn ([`fine_angle`]).
fn stepped(gone: f64, tool: Tool, step: f64, fine: bool) -> f64 {
    let step = match tool {
        Tool::Move if fine => step / 10.0,
        Tool::Move => step,
        _ if fine => fine_angle(step.to_degrees()).to_radians(),
        _ => step,
    };
    (gone / step).round() * step
}

/// The finer step Shift turns in, degrees, for steps of `angle`: 5°, or 1° for steps of 5° or less.
fn fine_angle(angle: f64) -> f64 {
    if angle > FINE_TURN_STEP + 1e-9 {
        FINE_TURN_STEP
    } else {
        1.0
    }
}

/// The turn step after `now`, up or down the list, round from the end.
fn next_angle(now: f64, up: bool) -> f64 {
    let at = (ANGLE_STEPS.iter())
        .position(|&a| (a - now).abs() < 1e-9)
        .unwrap_or(2);
    let n = ANGLE_STEPS.len();
    ANGLE_STEPS[if up { (at + 1) % n } else { (at + n - 1) % n }]
}

/// How far `handle` moved `value` slides and turns the part, in the craft's frame.
fn motion(tool: Tool, handle: &Handle, value: f64) -> (DVec3, DQuat) {
    let axis = handle.to_craft * handle.axis;
    match tool {
        Tool::Move => (axis * value, DQuat::IDENTITY),
        _ => (DVec3::ZERO, DQuat::from_axis_angle(axis, value)),
    }
}

/// The handle is let go: keep the change if the builder's rules allow it, else put it back and say
/// why. What is on show is then the craft as it stands.
fn finish(
    drag: Drag,
    handle: Option<Handle>,
    tool: Tool,
    editor: &mut Editor,
    catalog: &PartCatalog,
    sim: &mut Sim,
    ui: &mut LayoutVisibility,
) {
    // The craft carried by its root is where it was let go, or back where it was.
    if let (Some(from), Tool::Move) = (drag.anchor, tool) {
        if drag.cancel {
            editor.carry_to(from);
        }
        editor.set_notice("");
        return;
    }
    let mut redraw = drag.shown != 0.0;
    let mut notice = String::new();
    if !drag.cancel
        && drag.value != 0.0
        && let (Some(handle), Some(builder)) = (handle, editor.builder_mut())
    {
        let (by, turn) = motion(tool, &handle, drag.value);
        let changed = if drag.anchor.is_some() {
            builder.turn_root(catalog, turn)
        } else {
            builder.shift(catalog, &drag.part, by, turn)
        };
        match changed {
            // What is on show is the change, unless the pointer moved on while it was built.
            Ok(_) => redraw = drag.shown != drag.value,
            Err(refusal) => {
                notice = format!("Cannot: {refusal}");
                redraw = true;
            }
        }
    }
    if redraw {
        let draft = (editor.builder()).map_or(drag.start.clone(), |b| b.draft().clone());
        if let Err(e) = editor.redraw(&draft, catalog, sim, ui) {
            error!("cannot show the craft as it is: {e}");
        }
    }
    editor.set_notice(notice);
    editor.settle(sim);
}

/// The label by a handle under the pointer or in hand.
#[derive(Component)]
struct HandleLabel;

fn spawn_label(mut commands: Commands) {
    commands.spawn((
        HandleLabel,
        Text::new(""),
        TextColor(LABEL_TEXT),
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::axes(Val::Px(7.0), Val::Px(3.0)),
            border_radius: BorderRadius::all(Val::Px(3.0)),
            ..default()
        },
        BackgroundColor(LABEL_FILL),
        Pickable::IGNORE,
        ViewOverlay,
        GlobalZIndex(-4),
        Visibility::Hidden,
    ));
}

/// What the handle on `axis` is called, for a part held on a surface or on a node.
fn handle_name(tool: Tool, axis: usize, on_surface: bool) -> &'static str {
    match (tool, axis, on_surface) {
        (Tool::Move, 0, true) => "Slide round",
        (Tool::Move, 1, true) => "Slide along",
        (Tool::Move, 2, true) => "Slide out",
        (Tool::Move, 2, false) => "Slide along",
        (Tool::Move, _, false) => "Slide across",
        (_, 2, true) => "Turn about the line out",
        (_, 2, false) => "Turn about the joint",
        _ => "Tilt",
    }
}

/// Draw the handles where the picked part has them this frame, the label, and the pointer's shape.
#[allow(clippy::too_many_arguments)]
fn draw_handles(
    mut commands: Commands,
    mut moving: ResMut<Moving>,
    editor: Res<Editor>,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    docs: Res<UiDocuments>,
    camera: Single<(&Camera, &Transform), With<MainCamera>>,
    mut gizmos: Gizmos<HandleGizmos>,
    mut label: Single<(&mut Text, &mut TextFont, &mut Node, &mut Visibility), With<HandleLabel>>,
    window: Single<Entity, With<PrimaryWindow>>,
    pointer: ViewPointer,
    (mut cursor, mut font_set): (Local<Option<SystemCursorIcon>>, Local<bool>),
) {
    let (camera, transform) = *camera;
    let eye = GlobalTransform::from(*transform);
    let vessel_frame = editor
        .vessel()
        .and_then(|v| VesselFrame::of(&sim, &frame, v));
    let moving = &mut *moving;
    let tool = moving.tool;
    let held = moving.drag.as_ref().map(|d| d.handle);
    let lit = held.or(moving.over);
    // While one is in hand, it alone is drawn, where the part has gone; the root carried takes its
    // frame along, so its arrow stays on it as it is.
    let gone = (moving.drag.as_ref())
        .filter(|d| d.anchor.is_none())
        .map_or(0.0, |d| d.value);
    let drift = moving.drift;
    for (i, handle) in moving.handles.iter_mut().enumerate() {
        let Some(frame) = vessel_frame.filter(|_| held.is_none_or(|h| h == i)) else {
            handle.screen.clear();
            continue;
        };
        let color = if lit == Some(i) {
            LIT
        } else {
            AXES[handle.axis_index]
        };
        let origin = match tool {
            Tool::Move if held == Some(i) => handle.origin + handle.axis * gone,
            _ => handle.origin,
        } + drift;
        let view = |at: DVec3| frame.view_point(at);
        let to_screen = |at: DVec3| camera.world_to_viewport(&eye, view(at)).ok();
        match tool {
            Tool::Move => {
                let ends = [
                    origin - handle.axis * handle.size,
                    origin + handle.axis * handle.size,
                ];
                gizmos
                    .arrow(view(ends[0]), view(ends[1]), color)
                    .with_double_end()
                    .with_tip_length((handle.size * 0.28) as f32);
                handle.screen = ends.iter().filter_map(|e| to_screen(*e)).collect();
            }
            _ => {
                let [a, b] = handle.across;
                let points: Vec<DVec3> = (0..RING_SIDES)
                    .map(|k| {
                        let angle = std::f64::consts::TAU * k as f64 / RING_SIDES as f64;
                        origin + (a * angle.cos() + b * angle.sin()) * handle.size
                    })
                    .collect();
                let mut drawn: Vec<Vec3> = points.iter().map(|p| view(*p)).collect();
                drawn.push(drawn[0]);
                gizmos.linestrip(drawn, color);
                // The way it has turned, from where it was taken.
                if held == Some(i)
                    && let Some(drag) = &moving.drag
                {
                    let start = a * drag.grabbed.cos() + b * drag.grabbed.sin();
                    let end = DQuat::from_axis_angle(handle.axis, drag.value) * start;
                    gizmos.line(view(origin), view(origin + start * handle.size), color);
                    gizmos.line(view(origin), view(origin + end * handle.size), LIT);
                }
                handle.screen = points.iter().filter_map(|p| to_screen(*p)).collect();
                if handle.screen.len() != points.len() {
                    handle.screen.clear();
                }
            }
        }
    }

    // The label: what the handle does, and how far it has gone.
    let (text, font, node, visibility) = &mut *label;
    let on_surface = (moving.held.as_ref())
        .and_then(|(id, ..)| {
            let draft = editor.builder()?.draft();
            draft.parts.iter().find(|p| &p.id == id)
        })
        .is_some_and(|p| p.at.is_some());
    let words = lit.and_then(|i| moving.handles.get(i)).map(|handle| {
        let name = handle_name(tool, handle.axis_index, on_surface);
        match (&moving.drag, tool) {
            // Plus nought, so that no -0 shows.
            (Some(drag), Tool::Move) => format!("{name} {:+.2} m", drag.value + 0.0),
            (Some(drag), _) => format!("{name} {:+.0}°", drag.value.to_degrees().round() + 0.0),
            (None, _) => name.to_owned(),
        }
    });
    match (words, pointer.position()) {
        (Some(words), Some(at)) => {
            if text.0 != words {
                text.0 = words;
            }
            if docs.is_changed() || !*font_set {
                **font = docs.text_font(13.0);
                *font_set = true;
            }
            node.left = Val::Px(at.x + LABEL_OFFSET.x);
            node.top = Val::Px(at.y + LABEL_OFFSET.y);
            **visibility = Visibility::Inherited;
        }
        _ => **visibility = Visibility::Hidden,
    }

    // The pointer says a handle can be taken, or is.
    let icon = match (held, moving.over) {
        (Some(_), _) => Some(SystemCursorIcon::Grabbing),
        (None, Some(_)) => Some(SystemCursorIcon::Grab),
        _ => None,
    };
    if *cursor != icon {
        // Another's shape is left alone when this has none to give.
        if let Some(icon) = icon.or(cursor.is_some().then_some(SystemCursorIcon::Default)) {
            commands.entity(*window).insert(CursorIcon::from(icon));
        }
        *cursor = icon;
    }
}

/// `editor.tool`.
fn publish_tool(
    moving: Res<Moving>,
    mut data: ResMut<UiData>,
    mut published: Local<Option<(Tool, u64)>>,
) {
    let now = (moving.tool, moving.angle.to_bits());
    if *published == Some(now) {
        return;
    }
    *published = Some(now);
    data.set("editor.tool", moving.tool.id());
    let angle = format!("{}°", moving.angle);
    data.set("editor.snap", angle);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_point_of_a_line_on_the_screen_is_found() {
        let line = [Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)];
        assert_eq!(distance_to(&line, false, Vec2::new(5.0, 3.0)), 3.0);
        assert_eq!(distance_to(&line, false, Vec2::new(14.0, 3.0)), 5.0);
        let square = [
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        // Closed, the last side counts too.
        assert_eq!(distance_to(&square, true, Vec2::new(-2.0, 5.0)), 2.0);
        assert_eq!(
            distance_to(&square, false, Vec2::new(-2.0, 5.0)),
            5.0f32.hypot(2.0)
        );
    }

    #[test]
    fn a_ring_counts_its_turn_past_half_way_round() {
        let handle = Handle {
            axis_index: 2,
            axis: DVec3::Z,
            origin: DVec3::ZERO,
            across: [DVec3::X, DVec3::Y],
            to_craft: DQuat::IDENTITY,
            size: 1.0,
            screen: Vec::new(),
        };
        let mut drag = Drag {
            handle: 0,
            part: "p".into(),
            start: serde_json::from_value(serde_json::json!({ "id": "c", "parts": [] })).unwrap(),
            grabbed: 0.0,
            last: 0.0,
            value: 0.0,
            shown: 0.0,
            asked: 0.0,
            root: DVec3::ZERO,
            anchor: None,
            step: TURN_STEP.to_radians(),
            across: None,
            cancel: false,
        };
        // Looking down at the ring, the pointer going round it a little past half a turn.
        for degrees in [40.0f64, 100.0, 170.0, 200.0] {
            let at = DVec3::new(degrees.to_radians().cos(), degrees.to_radians().sin(), 0.0);
            follow(
                &mut drag,
                &handle,
                Tool::Turn,
                at + DVec3::Z * 10.0,
                DVec3::NEG_Z,
                false,
            );
        }
        assert!(
            (drag.value.to_degrees() - 195.0).abs() < 1e-9,
            "{}",
            drag.value.to_degrees()
        );
        // Fine steps of 5°.
        let at = DVec3::new(203f64.to_radians().cos(), 203f64.to_radians().sin(), 0.0);
        follow(
            &mut drag,
            &handle,
            Tool::Turn,
            at + DVec3::Z * 10.0,
            DVec3::NEG_Z,
            true,
        );
        assert!((drag.value.to_degrees() - 205.0).abs() < 1e-9);
    }
}
