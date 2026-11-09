//! Shaping parts in the editor. A click on a part picks it, and a click on nothing or Esc lets it
//! go; the part under the pointer is lit. A picked part drawn as its own shape shows round handles
//! where its sizes are taken hold of (`osp_sim::shaping`): the top of a tank for its length and its
//! side for its diameter, the rims of an adapter, a fin's tip and trailing edges. A handle dragged
//! changes its size as the pointer goes, in steps that suit how near the view is (Shift for steps a
//! tenth as big), and a width brought near the width of what the part is joined to takes it. What is
//! on show is built again as the size changes, so the height and mass beside it are the flight's; a
//! big craft takes a while to build, so that is done on a thread of its own, the newest size next, while
//! the handle and its label keep up with the pointer. Let go,
//! the change is checked by the builder's rules and kept, or undone with the reason under the launch
//! button; Esc while dragging puts it back. Ctrl+Z (Cmd+Z) undoes a change, Ctrl+Shift+Z or Ctrl+Y
//! does it again, and so do `editor.undo` and `editor.redo`. `editor.pick:<part id>` picks a part,
//! `editor.pick:` none.
//!
//! The handles are dots drawn over everything, with a label by the one in hand saying what it is and
//! how big. The pointer takes the shape of the way the handle under it goes.
//!
//! Publishes `editor.part.picked`, `editor.part.name`, `editor.part.sizes` (each `label` and
//! `value`, as text) and `editor.hint`, for the editor's panel.

use std::sync::Arc;

use bevy::{
    math::{DQuat, DVec2, DVec3},
    picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings},
    prelude::*,
    window::{CursorIcon, PrimaryWindow, SystemCursorIcon},
};
use osp_sim::{
    data::{CraftDef, PartCatalog},
    parts::{PartId, PartTree},
    propulsion::Propulsion,
    shaping::{self, Grip, Size},
    system::SolarSystem,
    time::SimTime,
    vessel::{Vessel, VesselId},
};
use osp_ui::{CLICK_SLOP, LayoutVisibility, UiAction, UiData, UiDocuments, ViewOverlay};

use osp_game::{
    FrameSet,
    camera::MainCamera,
    content::{GameContent, catalog},
    controls::InputLocked,
    frame::{RenderFrame, rotation_to_render, to_render},
    orbit_lines::ViewPointer,
    part_highlight::PartMarks,
    part_info::ShowsPart,
    sim::Sim,
};

use crate::{
    building::Placing,
    hangar::{Editor, editor_actions},
    moving::{Moving, Tool},
};

/// How wide a handle is drawn, and how near the pointer has to come to take it, px.
const GRIP_PX: f32 = 13.0;
const REACH_PX: f32 = 12.0;
/// Handles nearer each other than this on the screen are too close to tell apart: the less
/// important goes until the view comes nearer, px.
const APART_PX: f32 = 18.0;
/// A step of a size is about this far on the screen, px, wherever the view is.
const STEP_PX: f64 = 6.0;
/// A width this near another's on the screen takes it, px.
const MATCH_PX: f64 = 10.0;
/// Where the label goes from its handle, px.
const LABEL_OFFSET: Vec2 = Vec2::new(14.0, -11.0);
/// The HUD's colours: the accent, a handle's middle, and the label's ground and text.
const ACCENT: Color = Color::srgb(0.220, 0.878, 0.769);
const GRIP_FILL: Color = Color::srgb(0.055, 0.102, 0.141);
const LABEL_FILL: Color = Color::srgba(0.043, 0.086, 0.125, 0.94);
const LABEL_TEXT: Color = Color::srgb(0.902, 0.941, 0.961);
/// The most handles a part has.
const MOST_GRIPS: usize = 4;

pub struct ShapingPlugin;

impl Plugin for ShapingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Shaping>()
            .add_systems(Startup, spawn_grips)
            .add_systems(
                Update,
                (
                    shape_parts.in_set(FrameSet::Input).after(editor_actions),
                    // After the camera has moved this frame, so the handles stay on the part.
                    draw_grips.in_set(FrameSet::Render),
                    publish_shaping,
                ),
            );
    }
}

/// A handle of the picked part as it is now.
#[derive(Debug, Clone)]
struct Placed {
    /// Turned round the part's axis to where it is seen, in the part's frame.
    grip: Grip,
    /// Where it is in the vessel's frame: sim axes, from the base of its stack.
    at: DVec3,
    /// Where it was drawn last, px.
    screen: Option<Vec2>,
}

/// One size a handle in hand changes.
#[derive(Debug, Clone)]
struct Pulling {
    /// As it was when the drag began.
    size: Size,
    /// The way the handle goes as it grows, in the vessel's frame, and how fast it grows.
    way: DVec3,
    rate: f64,
    /// What it is on show as now.
    value: f64,
}

/// A handle in hand.
#[derive(Debug, Clone)]
struct Drag {
    /// The part, by its place in the craft, and its handle, by its place in its list.
    part: usize,
    grip: usize,
    /// Where the handle was taken hold of, in the vessel's frame.
    origin: DVec3,
    pulls: Vec<Pulling>,
    /// The craft as it was when the drag began.
    start: CraftDef,
    /// The widths a width can take, and whose they are.
    widths: Vec<(f64, String)>,
    /// Whose width it has taken.
    matched: Option<String>,
    /// The sizes what is on show was last built with.
    shown: Vec<f64>,
    /// Esc: put it back as it was.
    cancel: bool,
}

impl Drag {
    /// Where the handle is now, in the vessel's frame.
    fn at(&self) -> DVec3 {
        let moved = |p: &Pulling| p.way * (p.value - p.size.value) / p.rate;
        self.origin + self.pulls.iter().map(moved).sum::<DVec3>()
    }

    /// What the label says: each size and how big it is, whether it can go no further, and whose
    /// width it has taken.
    fn label(&self) -> String {
        let sizes: Vec<String> = (self.pulls.iter())
            .map(|p| {
                let size = format!("{} {}", p.size.label, metres(p.value));
                if p.value >= p.size.most {
                    format!("{size}, as big as it goes")
                } else if p.value <= p.size.least {
                    format!("{size}, as small as it goes")
                } else {
                    size
                }
            })
            .collect();
        match &self.matched {
            Some(whose) => format!("{} · as {whose}", sizes.join(" · ")),
            None => sizes.join(" · "),
        }
    }
}

/// A craft being built on a thread of its own, to go on show, with the sizes it has. Its own, as the
/// shared pools are kept busy by the ground and the trees.
struct Building {
    task: std::thread::JoinHandle<Result<Propulsion, String>>,
    craft: CraftDef,
    values: Vec<f64>,
}

/// The part picked in the editor and what is being done to it.
#[derive(Resource, Default)]
pub struct Shaping {
    /// The part picked, by its id, and the part under the pointer, by its place in the craft.
    picked: Option<String>,
    pointed: Option<usize>,
    /// The picked part's handles.
    grips: Vec<Placed>,
    /// The handle under the pointer, by its place in `grips`.
    over: Option<usize>,
    drag: Option<Drag>,
    /// What a handle in hand has made, being built to go on show.
    building: Option<Building>,
    /// Where the left button went down off the UI and off a handle, to tell a click from a drag of
    /// the view.
    press: Option<Vec2>,
    /// The left button was down last frame: a press is told by its change, so a script that presses
    /// it at any point in a frame is seen.
    was_down: bool,
    /// Where the pointer was when what is under it was last looked for, and on which vessel.
    looked: Option<(Vec2, VesselId)>,
    /// The craft last seen, to let go of the pick when another goes on show.
    craft: Option<String>,
    /// A move or turn handle (`osp_editor::moving`) is under the pointer, and one is in hand.
    handles: (bool, bool),
}

impl Shaping {
    /// A handle is in hand or under the pointer, so the left button is not for looking around.
    pub fn holds_pointer(&self) -> bool {
        self.drag.is_some() || self.over.is_some() || self.handles.0
    }

    /// Whether a move or turn handle is under the pointer, and whether one is in hand.
    pub(crate) fn set_handles(&mut self, over: bool, dragging: bool) {
        self.handles = (over, dragging);
    }

    /// Put a handle in hand back, or let go of the part picked: whether there was either.
    pub fn let_go(&mut self) -> bool {
        if let Some(drag) = self.drag.as_mut() {
            drag.cancel = true;
            return true;
        }
        self.picked.take().is_some()
    }

    /// The id of the part picked, if one is.
    pub fn picked(&self) -> Option<&str> {
        self.picked.as_deref()
    }

    /// The sizes a handle in hand has made, as they are on show.
    pub(crate) fn drag_values(&self) -> Vec<(&'static str, f64)> {
        (self.drag.iter())
            .flat_map(|d| &d.pulls)
            .map(|p| (p.size.name, p.value))
            .collect()
    }

    /// Pick `id`, or nothing.
    pub(crate) fn pick(&mut self, id: Option<String>) {
        self.drag = None;
        self.building = None;
        self.picked = id;
    }

    /// Where the handle for `size` of the picked part was drawn last, px: for scripted captures.
    pub fn grip_on_screen(&self, size: &str) -> Option<Vec2> {
        (self.grips.iter())
            .find(|g| g.grip.pulls.iter().any(|p| p.size == size))
            .and_then(|g| g.screen)
    }
}

/// `value` metres, as a size reads.
fn metres(value: f64) -> String {
    format!("{value:.2} m")
}

/// The smallest of 1, 2 and 5 times a power of ten that is at least `at_least`.
pub(crate) fn nice_step(at_least: f64) -> f64 {
    if at_least.is_nan() || at_least <= 0.0 || at_least.is_infinite() {
        return 0.01;
    }
    let ten = 10f64.powf(at_least.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|k| k * ten)
        .find(|step| *step >= at_least * (1.0 - 1e-9))
        .unwrap_or(10.0 * ten)
}

/// How far along the line through `origin` the way `way` (unit) the point nearest the ray from
/// `from` the way `look` (unit) is; none if they run the same way.
pub(crate) fn along_line(origin: DVec3, way: DVec3, from: DVec3, look: DVec3) -> Option<f64> {
    let across = origin - from;
    let b = way.dot(look);
    let denominator = 1.0 - b * b;
    if denominator < 1e-6 {
        return None;
    }
    Some((b * look.dot(across) - way.dot(across)) / denominator)
}

/// Where the ray from `from` the way `look` meets the plane through `origin` that `a` and `b`
/// (unit, square to each other) lie in, as distances along each; none if it runs along the plane.
pub(crate) fn in_plane(
    origin: DVec3,
    a: DVec3,
    b: DVec3,
    from: DVec3,
    look: DVec3,
) -> Option<(f64, f64)> {
    let normal = a.cross(b);
    let towards = normal.dot(look);
    if towards.abs() < 1e-6 {
        return None;
    }
    let hit = from + look * (normal.dot(origin - from) / towards);
    let off = hit - origin;
    Some((off.dot(a), off.dot(b)))
}

/// Where the parts of `tree` are measured from in the editor: its root, the origin of the craft's
/// frame. The tree's own frame stands on the base of its lowest stage, which moves as parts go on
/// and off.
pub(crate) fn tree_origin(tree: &PartTree) -> DVec3 {
    tree.root().map_or(DVec3::ZERO, |root| root.position)
}

/// Where the root of `vessel` is at `time`, from its parent body: the origin of the frame the
/// editor works in.
pub(crate) fn craft_origin(vessel: &Vessel, system: &SolarSystem, time: SimTime) -> DVec3 {
    let tree = vessel.propulsion.parts();
    let attitude = vessel.attitude_at(system, time);
    let from_base = tree_origin(tree) - DVec3::Z * tree.base();
    vessel.base_at(system, time) + attitude * from_base
}

/// How a vessel's frame (sim axes, from its root) sits in the view: as the renderer places it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct VesselFrame {
    translation: Vec3,
    rotation: DQuat,
}

impl VesselFrame {
    pub(crate) fn of(sim: &Sim, frame: &RenderFrame, vessel: VesselId) -> Option<Self> {
        let vessel = sim.universe.vessel(vessel)?;
        let system = &sim.universe.system;
        let attitude = vessel.attitude_at(system, frame.time);
        Some(Self {
            translation: frame.to_render(vessel.parent(), craft_origin(vessel, system, frame.time)),
            rotation: attitude,
        })
    }

    /// A way in the vessel's frame, in sim axes: the hangar's.
    pub(crate) fn sim_way(&self, way: DVec3) -> DVec3 {
        self.rotation * way
    }

    /// A way in sim axes, the hangar's, in the vessel's frame.
    pub(crate) fn vessel_way_from_sim(&self, way: DVec3) -> DVec3 {
        self.rotation.inverse() * way
    }

    /// The turn from sim axes, the hangar's, to the vessel's frame.
    pub(crate) fn turn_from_sim(&self) -> DQuat {
        self.rotation.inverse()
    }

    /// How the vessel's frame is turned in the view.
    pub(crate) fn view_rotation(&self) -> Quat {
        rotation_to_render(self.rotation)
    }

    /// A point of the vessel's frame in the view.
    pub(crate) fn view_point(&self, at: DVec3) -> Vec3 {
        self.translation + to_render(self.rotation * at)
    }

    /// A point of the view in the vessel's frame.
    pub(crate) fn vessel_point(&self, at: Vec3) -> DVec3 {
        let off = (at - self.translation).as_dvec3();
        self.rotation.inverse() * DVec3::new(off.x, -off.z, off.y)
    }

    /// A way in the view, in the vessel's frame.
    pub(crate) fn vessel_way(&self, way: Vec3) -> DVec3 {
        let way = way.as_dvec3();
        self.rotation.inverse() * DVec3::new(way.x, -way.z, way.y)
    }
}

/// A picked part's handles, turned to be seen from `eye` (the vessel's frame), and where each is.
fn place_grips(
    sim: &Sim,
    vessel: VesselId,
    part: usize,
    def: &CraftDef,
    eye: DVec3,
) -> Vec<Placed> {
    let Some(vessel) = sim.universe.vessel(vessel) else {
        return Vec::new();
    };
    let tree = vessel.propulsion.parts();
    let Some(on_show) = tree.parts().get(part) else {
        return Vec::new();
    };
    let base = tree_origin(tree);
    let (position, rotation) = (on_show.position - base, on_show.rotation);
    // Where the eye is from the part, in its own frame, to turn handles that can go round towards.
    let seen = rotation.inverse() * (eye - position);
    let round = DVec2::new(seen.x, seen.y);
    // Square to the way the eye looks at the axis, on its right as it looks: the look (away from
    // the eye) crossed with the axis.
    let side = if round.length() > 1e-6 {
        let toward = round.normalize();
        DVec2::new(-toward.y, toward.x)
    } else {
        DVec2::X
    };
    let turn = DQuat::from_rotation_z(side.y.atan2(side.x));
    shaping::part_grips(&def.parts[part])
        .into_iter()
        .map(|mut grip| {
            if grip.round {
                grip.at = turn * grip.at;
                for pull in &mut grip.pulls {
                    pull.along = turn * pull.along;
                }
            }
            Placed {
                at: position + rotation * grip.at,
                grip,
                screen: None,
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn shape_parts(
    mut shaping: ResMut<Shaping>,
    mut editor: ResMut<Editor>,
    mut sim: ResMut<Sim>,
    mut ui: ResMut<LayoutVisibility>,
    content: Res<GameContent>,
    (buttons, keys, locked): (
        Res<ButtonInput<MouseButton>>,
        Res<ButtonInput<KeyCode>>,
        Res<InputLocked>,
    ),
    pointer: ViewPointer,
    frame: Res<RenderFrame>,
    camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    (parts, parents): (Query<&ShowsPart>, Query<&ChildOf>),
    mut ray_cast: MeshRayCast,
    mut marks: ResMut<PartMarks>,
    mut actions: MessageReader<UiAction>,
    (mut parts_catalog, placing, moving): (
        Local<Option<Arc<PartCatalog>>>,
        Res<Placing>,
        Res<Moving>,
    ),
) {
    let shaping = &mut *shaping;
    let down = buttons.pressed(MouseButton::Left);
    let (went_down, went_up) = (down && !shaping.was_down, !down && shaping.was_down);
    shaping.was_down = down;
    // A part in hand has the pointer.
    if placing.is_holding() {
        shaping.pointed = None;
        shaping.over = None;
        shaping.press = None;
        shaping.grips.clear();
        marks.set_editor(None);
        return;
    }
    let (Some(vessel), Some(builder)) = (editor.vessel(), editor.builder()) else {
        shaping.picked = None;
        shaping.pointed = None;
        shaping.drag = None;
        shaping.grips.clear();
        shaping.craft = None;
        marks.set_editor(None);
        return;
    };
    let craft = builder.draft().id.clone();
    if shaping.craft.as_ref() != Some(&craft) {
        shaping.picked = None;
        shaping.drag = None;
        shaping.craft = Some(craft);
    }
    if parts_catalog.is_none() || content.is_changed() {
        *parts_catalog = Some(Arc::new(catalog(&content.0.registry, &mut Vec::new())));
    }
    let Some(shared) = parts_catalog.as_ref() else {
        return;
    };
    let catalog = &**shared;

    // Picking by name, undo and redo.
    let command = keys.any_pressed([
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
    ]);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let mut history = None;
    for action in actions.read() {
        match action.name() {
            "editor.pick" => {
                shaping.picked = action.arg().filter(|id| !id.is_empty()).map(str::to_owned);
            }
            "editor.undo" => history = Some(false),
            "editor.redo" => history = Some(true),
            _ => {}
        }
    }
    if !locked.0 && command && keys.just_pressed(KeyCode::KeyZ) {
        history = Some(shift);
    }
    if !locked.0 && command && keys.just_pressed(KeyCode::KeyY) {
        history = Some(true);
    }
    if let (Some(redo), None, false) = (history, &shaping.drag, shaping.handles.1) {
        let Some(builder) = editor.builder_mut() else {
            return;
        };
        let moved = if redo { builder.redo() } else { builder.undo() };
        if moved {
            let draft = builder.draft().clone();
            if let Err(e) = editor.redraw(&draft, catalog, &mut sim, &mut ui) {
                error!("cannot show the craft as it was: {e}");
            }
            editor.settle(&mut sim);
            editor.set_notice("");
        }
        return;
    }

    let Some(vessel_frame) = VesselFrame::of(&sim, &frame, vessel) else {
        return;
    };
    let (camera, eye) = *camera;
    let cursor = pointer.position();
    let ray = cursor.and_then(|at| camera.viewport_to_world(eye, at).ok());
    let look = ray.map(|ray| {
        (
            vessel_frame.vessel_point(ray.origin),
            vessel_frame.vessel_way(*ray.direction).normalize(),
        )
    });
    let Some(draft) = editor.builder().map(|b| b.draft().clone()) else {
        return;
    };

    // A handle in hand follows the pointer, until it is let go.
    if shaping.drag.is_some() {
        let finished = shaping.drag.as_ref().is_some_and(|d| d.cancel) || !down;
        if finished {
            // What is still being built is not wanted now.
            shaping.building = None;
            let Some(drag) = shaping.drag.take() else {
                return;
            };
            finish(drag, &mut editor, catalog, &mut sim, &mut ui);
        } else if let Some(drag) = shaping.drag.as_mut() {
            // Off the view, the handle stays where it was last.
            if let Some((from, look)) = look {
                follow(drag, from, look, shift, camera, eye, &vessel_frame);
            }
            let building = &mut shaping.building;
            // What has been built goes on show.
            let done = building.as_ref().is_some_and(|b| b.task.is_finished());
            if let Some(done) = building.take_if(|_| done) {
                let built = (done.task.join()).unwrap_or_else(|_| Err("building it failed".into()));
                match built {
                    Ok(propulsion) => {
                        editor.redraw_built(&done.craft, propulsion, &mut sim, &mut ui);
                        drag.shown = done.values;
                    }
                    Err(e) => warn!("cannot show it at that size: {e}"),
                }
            }
            // The newest sizes are built next, once nothing else is being built.
            let values: Vec<f64> = drag.pulls.iter().map(|p| p.value).collect();
            if values != drag.shown && building.is_none() {
                let mut craft = drag.start.clone();
                let part = &mut craft.parts[drag.part];
                let resized = (drag.pulls.iter())
                    .try_for_each(|p| shaping::resize(part, p.size.name, p.value));
                match resized {
                    Ok(()) => {
                        let (made, catalog) = (craft.clone(), shared.clone());
                        let task = std::thread::spawn(move || {
                            Propulsion::from_craft(&made, &catalog).map_err(|e| e.to_string())
                        });
                        *building = Some(Building {
                            task,
                            craft,
                            values,
                        });
                    }
                    Err(e) => warn!("cannot make it that size: {e}"),
                }
            }
        }
        if let Some(drag) = &shaping.drag
            && let Some(placed) = shaping.grips.get_mut(drag.grip)
        {
            placed.at = drag.at();
        }
        marks.set_editor(Some((
            vessel,
            None,
            Some(PartId(drag_part(shaping) as u32)),
        )));
        return;
    }

    let picked =
        (shaping.picked.as_ref()).and_then(|id| draft.parts.iter().position(|p| &p.id == id));
    if shaping.picked.is_some() && picked.is_none() {
        shaping.picked = None;
    }
    shaping.grips = match picked {
        Some(part) if moving.tool() == Tool::Shape => {
            let eye_at = vessel_frame.vessel_point(eye.translation());
            let mut placed = place_grips(&sim, vessel, part, &draft, eye_at);
            // Keep where they were drawn, for the pointer to find them.
            for (new, old) in placed.iter_mut().zip(&shaping.grips) {
                new.screen = old.screen;
            }
            placed
        }
        _ => Vec::new(),
    };
    shaping.over = cursor.and_then(|at| {
        (shaping.grips.iter().enumerate())
            .filter_map(|(i, g)| Some((i, g.screen?.distance(at))))
            .filter(|(_, d)| *d <= REACH_PX)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    });

    // Taking hold of a handle, or a click to pick a part.
    if went_down {
        shaping.press = None;
        if let (Some(over), Some(part)) = (shaping.over, picked) {
            shaping.drag = start_drag(shaping, over, part, &draft, catalog, &sim, vessel);
        } else if !shaping.handles.0 {
            shaping.press = cursor;
        }
    }
    let under = |ray_cast: &mut MeshRayCast| -> Option<usize> {
        let ray = ray?;
        let tagged = |e: Entity| {
            std::iter::once(e)
                .chain(parents.iter_ancestors(e))
                .find_map(|a| parts.get(a).ok().filter(|s| s.vessel == vessel))
        };
        let filter = |e: Entity| tagged(e).is_some();
        let settings = MeshRayCastSettings::default().with_filter(&filter);
        let (mesh, _) = ray_cast.cast_ray(ray, &settings).first()?;
        tagged(*mesh).map(|shows| shows.part.0 as usize)
    };
    if went_up
        && let (Some(from), Some(at)) = (shaping.press.take(), cursor)
        && from.distance(at) < CLICK_SLOP
    {
        shaping.picked = under(&mut ray_cast).map(|part| draft.parts[part].id.clone());
    }
    // What is under the pointer, looked for again when it moves.
    let looking = !down && !buttons.pressed(MouseButton::Right);
    match cursor {
        Some(at) if looking => {
            let moved = shaping
                .looked
                .is_none_or(|(was, on)| on != vessel || was.distance(at) > 0.5);
            if moved {
                shaping.pointed = under(&mut ray_cast);
                shaping.looked = Some((at, vessel));
            }
        }
        Some(_) => {}
        None => {
            shaping.pointed = None;
            shaping.looked = None;
        }
    }
    let pointed = (shaping.pointed).filter(|_| shaping.over.is_none() && !shaping.handles.0);
    let picked =
        (shaping.picked.as_ref()).and_then(|id| draft.parts.iter().position(|p| &p.id == id));
    marks.set_editor(Some((
        vessel,
        pointed.map(|p| PartId(p as u32)),
        picked.map(|p| PartId(p as u32)),
    )));
}

/// The part whose handle is in hand.
fn drag_part(shaping: &Shaping) -> usize {
    shaping.drag.as_ref().map_or(0, |d| d.part)
}

/// Take hold of handle `over` of `part`.
#[allow(clippy::too_many_arguments)]
fn start_drag(
    shaping: &Shaping,
    over: usize,
    part: usize,
    draft: &CraftDef,
    catalog: &PartCatalog,
    sim: &Sim,
    vessel: VesselId,
) -> Option<Drag> {
    let placed = shaping.grips.get(over)?;
    let sizes = shaping::part_sizes(&draft.parts[part]);
    let on_show = sim
        .universe
        .vessel(vessel)?
        .propulsion
        .parts()
        .parts()
        .get(part)?
        .rotation;
    let pulls = (placed.grip.pulls.iter())
        .filter_map(|pull| {
            let size = sizes.iter().find(|s| s.name == pull.size)?.clone();
            Some(Pulling {
                value: size.value,
                size,
                way: (on_show * pull.along).normalize(),
                rate: pull.rate,
            })
        })
        .collect::<Vec<_>>();
    if pulls.is_empty() {
        return None;
    }
    let widths = if pulls.iter().any(|p| p.size.width) {
        osp_sim::craft::Craft::build(draft, catalog)
            .map(|craft| shaping::neighbour_widths(&craft, part))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Some(Drag {
        part,
        grip: over,
        origin: placed.at,
        pulls,
        start: draft.clone(),
        widths,
        matched: None,
        shown: Vec::new(),
        cancel: false,
    })
}

/// Move the handle in hand to where the pointer is, a ray from `from` the way `look` in the vessel's
/// frame: its sizes in steps, a width matched to a neighbour's when it is near.
fn follow(
    drag: &mut Drag,
    from: DVec3,
    look: DVec3,
    fine: bool,
    camera: &Camera,
    eye: &GlobalTransform,
    frame: &VesselFrame,
) {
    let offsets = match drag.pulls.as_slice() {
        [one] => along_line(drag.origin, one.way, from, look).map(|t| vec![t]),
        [a, b] => in_plane(drag.origin, a.way, b.way, from, look).map(|(s, t)| vec![s, t]),
        _ => None,
    };
    let Some(offsets) = offsets else {
        return;
    };
    let on_screen = |at: DVec3| camera.world_to_viewport(eye, frame.view_point(at)).ok();
    drag.matched = None;
    for (pull, offset) in drag.pulls.iter_mut().zip(offsets) {
        // How much a pixel of the pointer's travel is worth, here and this way.
        let (Some(here), Some(on)) = (on_screen(drag.origin), on_screen(drag.origin + pull.way))
        else {
            continue;
        };
        let per_pixel = pull.rate / f64::from(here.distance(on)).max(1e-3);
        let step = nice_step(per_pixel * STEP_PX) / if fine { 10.0 } else { 1.0 };
        let raw = pull.size.value + offset * pull.rate;
        let mut value = (raw / step).round() * step;
        if pull.size.width {
            let near = (drag.widths.iter())
                .map(|(width, whose)| ((raw - width).abs() / per_pixel, width, whose))
                .filter(|(px, ..)| *px <= MATCH_PX)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, width, whose)) = near {
                value = *width;
                drag.matched = Some(whose.clone());
            }
        }
        pull.value = value.clamp(pull.size.least, pull.size.most);
    }
}

/// The handle is let go: keep the change if the builder's rules allow it, else put it back and
/// say why. What is on show is then the craft as it stands.
fn finish(
    drag: Drag,
    editor: &mut Editor,
    catalog: &PartCatalog,
    sim: &mut Sim,
    ui: &mut LayoutVisibility,
) {
    let values: Vec<f64> = drag.pulls.iter().map(|p| p.value).collect();
    let changed = drag.pulls.iter().any(|p| p.value != p.size.value);
    // What is on show is not yet what was let go, or is not what was there before.
    let behind = values != drag.shown && !(drag.shown.is_empty() && !changed);
    let Some(builder) = editor.builder_mut() else {
        return;
    };
    let id = drag.start.parts[drag.part].id.clone();
    let mut back = None;
    if drag.cancel {
        back = Some(String::new());
    } else if changed {
        let sizes: Vec<(&str, f64)> = drag.pulls.iter().map(|p| (p.size.name, p.value)).collect();
        match builder.resize(catalog, &id, &sizes) {
            Ok(_) => {
                editor.set_notice("");
                if behind {
                    back = Some(String::new());
                }
            }
            Err(refusal) => back = Some(format!("Cannot: {refusal}")),
        }
    } else if !drag.shown.is_empty() && drag.shown != values {
        // Dragged and brought back to where it was.
        back = Some(String::new());
    }
    if let Some(notice) = back {
        let draft = (editor.builder()).map_or(drag.start.clone(), |b| b.draft().clone());
        if let Err(e) = editor.redraw(&draft, catalog, sim, ui) {
            error!("cannot put `{id}` back: {e}");
        }
        editor.set_notice(notice);
    }
    editor.settle(sim);
}

/// The handles and their label.
#[derive(Component)]
struct GripDot(usize);
#[derive(Component)]
struct GripLabel;

fn spawn_grips(mut commands: Commands) {
    for index in 0..MOST_GRIPS {
        commands.spawn((
            GripDot(index),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(GRIP_PX),
                height: Val::Px(GRIP_PX),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BorderColor::all(ACCENT),
            BackgroundColor(GRIP_FILL),
            // The editor finds the handle under the pointer itself, so the view keeps the pointer.
            Pickable::IGNORE,
            ViewOverlay,
            GlobalZIndex(-5),
            Visibility::Hidden,
        ));
    }
    commands.spawn((
        GripLabel,
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

/// Put the handles where the picked part has them this frame, and the pointer in the shape of the
/// way the one under it goes.
#[allow(clippy::too_many_arguments)]
fn draw_grips(
    mut commands: Commands,
    mut shaping: ResMut<Shaping>,
    editor: Res<Editor>,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    docs: Res<UiDocuments>,
    camera: Single<(&Camera, &Transform), With<MainCamera>>,
    mut dots: Query<
        (&GripDot, &mut Node, &mut Visibility, &mut BackgroundColor),
        Without<GripLabel>,
    >,
    mut label: Single<
        (&mut Text, &mut TextFont, &mut Node, &mut Visibility),
        (With<GripLabel>, Without<GripDot>),
    >,
    window: Single<Entity, With<PrimaryWindow>>,
    (mut cursor, mut font_set): (Local<Option<SystemCursorIcon>>, Local<bool>),
) {
    let (camera, transform) = *camera;
    let eye = GlobalTransform::from(*transform);
    let vessel_frame = editor
        .vessel()
        .and_then(|v| VesselFrame::of(&sim, &frame, v));
    let shaping = &mut *shaping;
    let mut kept: Vec<Vec2> = Vec::new();
    for placed in &mut shaping.grips {
        placed.screen = vessel_frame
            .and_then(|f| camera.world_to_viewport(&eye, f.view_point(placed.at)).ok())
            // A handle too near one before it, which matters more, is left out.
            .filter(|at| kept.iter().all(|k| k.distance(*at) >= APART_PX));
        kept.extend(placed.screen);
    }
    let held = shaping.drag.as_ref().map(|d| d.grip);
    for (dot, mut node, mut visibility, mut fill) in &mut dots {
        // While one is in hand the others would be where the part was.
        let at = (shaping.grips.get(dot.0))
            .filter(|_| held.is_none_or(|h| h == dot.0))
            .and_then(|g| g.screen);
        *visibility = if at.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if let Some(at) = at {
            node.left = Val::Px(at.x - GRIP_PX / 2.0);
            node.top = Val::Px(at.y - GRIP_PX / 2.0);
        }
        let lit = held == Some(dot.0) || (held.is_none() && shaping.over == Some(dot.0));
        fill.0 = if lit { ACCENT } else { GRIP_FILL };
    }
    let (text, font, node, visibility) = &mut *label;
    let shown =
        (shaping.drag.as_ref()).and_then(|d| Some((d.label(), shaping.grips.get(d.grip)?.screen?)));
    let hovered = (shaping.over)
        .filter(|_| held.is_none())
        .and_then(|i| shaping.grips.get(i))
        .and_then(|g| {
            let draft = editor.builder()?.draft();
            let part = &draft.parts[shaping.picked_index(draft)?];
            let sizes = shaping::part_sizes(part);
            let names: Vec<String> = (g.grip.pulls.iter())
                .filter_map(|p| sizes.iter().find(|s| s.name == p.size))
                .map(|s| format!("{} {}", s.label, metres(s.value)))
                .collect();
            Some((names.join(" · "), g.screen?))
        });
    match shown.or(hovered) {
        Some((words, at)) => {
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
        None => **visibility = Visibility::Hidden,
    }

    // The pointer says which way the handle under it goes.
    let gripped = held.or(shaping.over).and_then(|i| shaping.grips.get(i));
    let icon = gripped.and_then(|g| {
        let frame = vessel_frame?;
        let here = camera
            .world_to_viewport(&eye, frame.view_point(g.at))
            .ok()?;
        if g.grip.pulls.len() > 1 {
            return Some(SystemCursorIcon::Move);
        }
        let on = g.at + frame_way(&sim, &editor, shaping, g)?;
        let there = camera.world_to_viewport(&eye, frame.view_point(on)).ok()?;
        let way = there - here;
        let angle = way.y.atan2(way.x).to_degrees().rem_euclid(180.0);
        Some(match angle {
            a if !(22.5..157.5).contains(&a) => SystemCursorIcon::EwResize,
            a if (67.5..112.5).contains(&a) => SystemCursorIcon::NsResize,
            a if a < 90.0 => SystemCursorIcon::NwseResize,
            _ => SystemCursorIcon::NeswResize,
        })
    });
    if *cursor != icon {
        *cursor = icon;
        let icon = icon.unwrap_or_default();
        commands.entity(*window).insert(CursorIcon::from(icon));
    }
}

/// The way handle `placed` goes, in the vessel's frame, a metre long.
fn frame_way(sim: &Sim, editor: &Editor, shaping: &Shaping, placed: &Placed) -> Option<DVec3> {
    let draft = editor.builder()?.draft();
    let part = shaping.picked_index(draft)?;
    let vessel = sim.universe.vessel(editor.vessel()?)?;
    let rotation = vessel.propulsion.parts().parts().get(part)?.rotation;
    Some(rotation * placed.grip.pulls.first()?.along)
}

impl Shaping {
    fn picked_index(&self, draft: &CraftDef) -> Option<usize> {
        let id = self.picked.as_ref()?;
        draft.parts.iter().position(|p| &p.id == id)
    }
}

/// `editor.part.*` and `editor.hint`, see the module.
fn publish_shaping(
    shaping: Res<Shaping>,
    editor: Res<Editor>,
    (placing, moving): (Res<Placing>, Res<Moving>),
    mut data: ResMut<UiData>,
    mut published: Local<Option<(Option<String>, bool, bool, bool, Tool)>>,
) {
    let draft = editor.builder().map(|b| b.draft());
    let part = draft.and_then(|d| Some(&d.parts[shaping.picked_index(d)?]));
    let empty = draft.is_some_and(|d| d.parts.is_empty());
    let now = (
        part.map(|p| p.id.clone()),
        draft.is_some(),
        empty,
        placing.is_holding(),
        moving.tool(),
    );
    if published.as_ref() == Some(&now) && !editor.is_changed() {
        return;
    }
    *published = Some(now);
    // With a part in hand, the hint is about where it goes (`osp_editor::building`).
    if placing.is_holding() {
        return;
    }
    let shaped = part.is_some_and(|p| !shaping::part_sizes(p).is_empty());
    let root = part.is_some_and(|p| p.on.is_none());
    let hint = match (moving.tool(), part) {
        (Tool::Move, Some(_)) if root => {
            "The first part holds the rest: its arrows carry the whole craft about the hangar."
        }
        (Tool::Turn, Some(_)) if root => {
            "The first part holds the rest: its rings turn the whole craft. Q and E roll it."
        }
        (Tool::Move, Some(_)) => {
            "Drag an arrow to slide it, with what it holds. Shift for small steps. 1 shapes, 3 turns."
        }
        (Tool::Turn, Some(_)) => {
            "Drag a ring to turn it, a step at a time (C sets the step, Shift finer). Q and E turn it \
             about the joint. 1 shapes."
        }
        (Tool::Move, None) if !empty => "Click a part to slide it. 1 shapes it, 3 turns it.",
        (Tool::Turn, None) if !empty => "Click a part to turn it. 1 shapes it, 2 slides it.",
        (_, part) => match part {
            Some(_) if shaped => {
                "Set it on the left, or drag a handle on it. 2 slides it, 3 turns it. Delete takes it off."
            }
            Some(_) => {
                "Set it on the left. 2 slides it, 3 turns it. Delete takes it off. Esc lets go."
            }
            None if empty => "Pick a part from the list to start the vessel with.",
            None if draft.is_some() => {
                "Click a part to change it, drag one to move it (the first carries the craft), or add \
                 one from Parts. Drag the background to look around."
            }
            None => "Drag to look around, middle drag or WASD, Q and E to move, wheel to zoom.",
        },
    };
    data.set("editor.hint", hint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_one_two_or_five_times_a_power_of_ten() {
        assert_eq!(nice_step(0.03), 0.05);
        assert_eq!(nice_step(0.05), 0.05);
        assert_eq!(nice_step(0.11), 0.2);
        assert_eq!(nice_step(0.7), 1.0);
        assert_eq!(nice_step(3.0), 5.0);
        assert_eq!(nice_step(0.0), 0.01);
        assert_eq!(nice_step(f64::NAN), 0.01);
    }

    #[test]
    fn a_handle_follows_the_point_of_its_line_nearest_the_pointer() {
        // Looking along +Y at a line up +Z through the origin, from 1 m up it and 10 m back.
        let t = along_line(DVec3::ZERO, DVec3::Z, DVec3::new(0.0, -10.0, 1.0), DVec3::Y);
        assert!((t.unwrap() - 1.0).abs() < 1e-12);
        // Seen from straight along it, there is nothing to follow.
        assert!(along_line(DVec3::ZERO, DVec3::Z, DVec3::Z * 10.0, DVec3::NEG_Z).is_none());
        let look = DVec3::new(0.0, 1.0, -1.0).normalize();
        let (s, t) = in_plane(
            DVec3::ZERO,
            DVec3::X,
            DVec3::Z,
            DVec3::new(2.0, -5.0, 8.0),
            look,
        )
        .unwrap();
        assert!((s - 2.0).abs() < 1e-12 && (t - 3.0).abs() < 1e-12);
    }

    fn pulling(name: &'static str, value: f64, way: DVec3, rate: f64) -> Pulling {
        Pulling {
            size: Size {
                name,
                label: "Length",
                value,
                least: 0.05,
                most: 200.0,
                width: false,
            },
            way,
            rate,
            value,
        }
    }

    #[test]
    fn a_handle_in_hand_is_where_its_sizes_put_it_and_says_them() {
        let mut drag = Drag {
            part: 0,
            grip: 0,
            origin: DVec3::new(1.0, 0.0, 10.0),
            pulls: vec![pulling("length", 10.0, DVec3::Z, 1.0)],
            start: serde_json::from_value(serde_json::json!({ "id": "c", "parts": [] })).unwrap(),
            widths: Vec::new(),
            matched: None,
            shown: Vec::new(),
            cancel: false,
        };
        drag.pulls[0].value = 12.5;
        assert_eq!(drag.at(), DVec3::new(1.0, 0.0, 12.5));
        assert_eq!(drag.label(), "Length 12.50 m");
        drag.matched = Some("S-II".into());
        assert_eq!(drag.label(), "Length 12.50 m · as S-II");
        drag.matched = None;
        drag.pulls[0].value = 200.0;
        assert_eq!(drag.label(), "Length 200.00 m, as big as it goes");
    }
}
