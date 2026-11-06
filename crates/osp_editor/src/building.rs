//! Building a craft from parts in the editor, as in KSP with its procedural parts. `editor.new`
//! starts an empty craft. The parts list (`editor.parts` shows and hides it) has the palette's
//! entries (`osp_sim::palette`) a group at a time (`editor.group:<group>`), or those a search
//! matches in every group (`editor.search:<text>`).
//!
//! A part is taken in hand by dragging its row into the hangar, by clicking its row
//! (`editor.add:<entry>`), or, for a part already on the craft, by dragging it off with the left
//! button (with everything held by it; a drag that starts on nothing turns the view). The first part
//! of an empty craft goes on as its root. After that the craft's free nodes show as rings, and what
//! is in hand is shown where it would go, built on a thread of its own as the pointer moves: on a
//! ring it comes near, by its own node that faces the other way (a new part as wide as that node);
//! on the side of a part, for what goes on surfaces (fins, wings, RCS blocks), at the point under
//! the pointer, a new one sized for the hull there, with copies round it for symmetry (X steps
//! through 1, 2, 3, 4, 6 and 8, Shift+X back). Q and E turn it about the joint, 15° a press or 5°
//! with Shift; with nothing in hand they turn the picked part. The builder's rules are checked on
//! the same thread: where it may not go it shows red and the hint says why. Dragged, it goes where
//! it is let go; taken by a click, where the next click is. The parts list stays out for the next
//! one; a click on the part picks it to shape it. Esc, a right click or `editor.drop` puts it back. Delete or Backspace, or
//! `editor.remove`, takes the picked part off with everything held by it. All of it can be undone.
//!
//! Publishes `editor.palette_open`, `editor.group`, `editor.palette` (each `id`, `name`, `detail`,
//! `held`; at most [`MOST_ROWS`]), `editor.palette_more`, `editor.search_shown`, `editor.holding`
//! and `editor.hint` while something is in hand.

use std::{collections::BTreeSet, sync::Arc, thread::JoinHandle};

use bevy::{
    math::DVec3,
    picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings},
    prelude::*,
};
use osp_sim::{
    assembly::{Mount, Side},
    builder::{self, Place},
    craft::Craft,
    data::{CraftDef, CraftPartDef, PartCatalog, SurfaceAtDef},
    palette::{self, Entry, Group},
    parts::PartId,
    propulsion::Propulsion,
};
use osp_ui::{
    CLICK_SLOP, DragPayload, Formatters, LayoutVisibility, TextFocus, UiAction, UiData,
    ViewOverlay, record,
};

use osp_game::{
    FrameSet,
    camera::MainCamera,
    content::{GameContent, catalog},
    frame::RenderFrame,
    orbit_lines::ViewPointer,
    part_highlight::PartMarks,
    part_info::ShowsPart,
    sim::Sim,
};

use crate::{
    hangar::{Editor, editor_actions},
    shaping::{Shaping, VesselFrame, shape_parts},
};

/// How near a free node the pointer has to come for the part in hand to go there, px.
const NODE_REACH_PX: f32 = 18.0;
/// How a free node's ring is drawn, px across, and the most drawn.
const RING_PX: f32 = 14.0;
const MOST_RINGS: usize = 64;
/// The most rows the parts list shows at once.
pub const MOST_ROWS: usize = 60;
/// How far off a wall what the pointer touches may be and still be on it: the drawn mesh is a
/// little off the exact shape. m, and as a share of how far the wall is from the axis.
const ON_WALL: f64 = 0.15;
const ON_WALL_SHARE: f64 = 0.05;
/// The symmetries the X key steps through.
const SYMMETRIES: [u32; 6] = [1, 2, 3, 4, 6, 8];
/// How far Q and E turn a part, degrees, and with Shift.
const TURN: f64 = 15.0;
const FINE_TURN: f64 = 5.0;
/// What a list row carries when dragged: `palette:<entry>`.
const PALETTE_DRAG: &str = "palette:";
const ACCENT: Color = Color::srgb(0.220, 0.878, 0.769);

pub struct BuildingPlugin;

impl Plugin for BuildingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Placing>()
            .add_systems(Startup, spawn_rings)
            .add_systems(
                Update,
                (
                    place_parts
                        .in_set(FrameSet::Input)
                        .after(editor_actions)
                        .before(shape_parts),
                    draw_rings.in_set(FrameSet::Render),
                    publish_placing,
                ),
            );
    }
}

/// What is in hand.
#[derive(Debug, Clone, PartialEq)]
enum Held {
    /// A new part, from the palette entry with this id.
    New(String),
    /// A part of the craft, by its id, with everything held by it.
    Moved(String),
}

/// Where what is in hand would go. Parts are named by their ids.
#[derive(Debug, Clone, PartialEq)]
enum Spot {
    /// The first part, as the root.
    Root,
    /// At a free node of `part`, by its own node `by`, a new part as wide as `width`.
    Node {
        part: String,
        node: String,
        by: String,
        width: f64,
    },
    /// On a surface of `part`, by its own node `by` turned `roll` degrees, a new part sized for a
    /// hull `width` across.
    Surface {
        part: String,
        surface: String,
        at: SurfaceAtDef,
        by: String,
        roll: f64,
        width: f64,
    },
}

/// A node of the craft with nothing held there.
#[derive(Debug, Clone)]
struct FreeNode {
    part: String,
    name: String,
    /// Which way it faces in the craft's frame as built.
    way: DVec3,
    size: f64,
    /// Where it is in its part's frame, then in the vessel's frame on show, and where it was drawn
    /// last, px.
    local: DVec3,
    at: DVec3,
    screen: Option<Vec2>,
}

/// What a preview shows: where, how many round the axis, and turned how far besides.
type Wanted = Option<(Spot, u32, f64)>;

/// What a preview is built into on its thread: the craft, its parts, and why the rules would not
/// have it, if they would not.
type Built = Result<(CraftDef, Propulsion, Option<String>), String>;

/// A preview being built.
struct Job {
    task: JoinHandle<Built>,
    wanted: Wanted,
    ghost: Vec<String>,
}

/// The parts list and what is in hand.
#[derive(Resource)]
pub struct Placing {
    open: bool,
    group: Group,
    search: String,
    held: Option<Held>,
    /// It was dragged into the view or off the craft: it goes where it is let go.
    dropping: bool,
    symmetry: u32,
    /// Degrees it is turned about the joint besides how it goes on.
    roll: f64,
    spot: Option<Spot>,
    nodes: Vec<FreeNode>,
    /// The craft without what is in hand, built to tell which of its nodes are free: the parts it
    /// was built from, what was in hand then, and the craft.
    built: Option<(Vec<CraftPartDef>, Option<Held>, Craft)>,
    /// What is on show: the draft with what is in hand where `shown` says, or the draft alone.
    shown: Wanted,
    job: Option<Job>,
    /// The parts a preview shows in hand, and whether the rules would have them there, or why not.
    ghost: Vec<String>,
    refused: Option<String>,
    /// Put back: what is on show goes back to the draft.
    restore: bool,
    press: Option<Vec2>,
    right_press: Option<Vec2>,
    was_down: [bool; 2],
    /// A left press on a part of the craft, not yet a click or a drag: the part and where.
    grabbed: Option<(String, Vec2)>,
}

impl Default for Placing {
    fn default() -> Self {
        Self {
            open: false,
            group: Group::Procedural,
            search: String::new(),
            held: None,
            dropping: false,
            symmetry: 1,
            roll: 0.0,
            spot: None,
            nodes: Vec::new(),
            built: None,
            shown: None,
            job: None,
            ghost: Vec::new(),
            refused: None,
            restore: false,
            press: None,
            right_press: None,
            was_down: [false; 2],
            grabbed: None,
        }
    }
}

impl Placing {
    /// Whether the parts list is out (it shows unless a part is picked).
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Something is in hand: the pointer is for putting it somewhere.
    pub fn is_holding(&self) -> bool {
        self.held.is_some()
    }

    /// Put back what is in hand, if anything is: whether it was.
    pub fn let_go(&mut self) -> bool {
        let held = self.held.take().is_some();
        if held {
            self.spot = None;
            self.dropping = false;
            self.roll = 0.0;
            self.restore = true;
        }
        held
    }

    /// Where the ring on node `name` of the part with id `part` was drawn last, px: for scripted
    /// captures.
    pub fn node_on_screen(&self, part: &str, name: &str) -> Option<Vec2> {
        (self.nodes.iter())
            .find(|n| n.part == part && n.name == name)
            .and_then(|n| n.screen)
    }

    fn take_in_hand(&mut self, held: Held, dropping: bool) {
        self.held = Some(held);
        self.dropping = dropping;
        self.spot = None;
        self.roll = 0.0;
        self.refused = None;
    }
}

/// A new id for a part from `stem`, not yet in `draft`, nor any of the `copies` a pattern makes of
/// it (`<id>_1` up).
fn new_id(draft: &CraftDef, stem: &str, copies: u32) -> String {
    let taken = |id: &str| draft.parts.iter().any(|p| p.id == id);
    let free = |id: &str| !taken(id) && (1..=copies).all(|k| !taken(&format!("{id}_{k}")));
    (1..)
        .map(|n| {
            if n == 1 {
                stem.to_owned()
            } else {
                format!("{stem}_{n}")
            }
        })
        .find(|id| free(id))
        .unwrap_or_else(|| stem.to_owned())
}

/// `id` and every part held through it, by id.
fn held_through(draft: &CraftDef, id: &str) -> BTreeSet<String> {
    let mut ids = BTreeSet::from([id.to_owned()]);
    loop {
        let more: Vec<String> = (draft.parts.iter())
            .filter(|p| !ids.contains(&p.id))
            .filter(|p| {
                let holder = p.on.as_deref().and_then(|on| on.split_once('.'));
                holder.is_some_and(|(part, _)| ids.contains(part))
            })
            .map(|p| p.id.clone())
            .collect();
        if more.is_empty() {
            return ids;
        }
        ids.extend(more);
    }
}

/// Where a spot puts a part, turned `roll` degrees besides.
fn place_at(spot: &Spot, roll: f64) -> Option<Place> {
    match spot {
        Spot::Root => None,
        Spot::Node { part, node, by, .. } => {
            let mut place = Place::node(format!("{part}.{node}"), by.clone());
            place.roll = roll;
            Some(place)
        }
        Spot::Surface {
            part,
            surface,
            at,
            by,
            roll: own,
            ..
        } => {
            let mut place = Place::surface(format!("{part}.{surface}"), by.clone(), *at);
            place.roll = own + roll;
            Some(place)
        }
    }
}

fn width_of(spot: &Spot) -> Option<f64> {
    match spot {
        Spot::Root => None,
        Spot::Node { width, .. } | Spot::Surface { width, .. } => Some(*width),
    }
}

/// The new part to put in from `entry`, sized for `spot`.
fn new_part(entry: &Entry, spot: &Spot, draft: &CraftDef, symmetry: u32) -> CraftPartDef {
    let mut part = entry.part.clone();
    let copies = match spot {
        Spot::Surface { .. } if symmetry > 1 => symmetry,
        _ => 0,
    };
    part.id = new_id(draft, &entry.stem, copies);
    if let Some(width) = width_of(spot) {
        palette::fit(&mut part, width);
    }
    part
}

/// `draft` with what is in hand at `spot`, without the rules: what a preview shows. And the ids
/// of the parts in hand there.
fn with_held(
    draft: &CraftDef,
    held: &Held,
    entries: &[Entry],
    spot: &Spot,
    symmetry: u32,
    roll: f64,
) -> Result<(CraftDef, Vec<String>), String> {
    let place = place_at(spot, roll);
    let mut craft = draft.clone();
    match held {
        Held::New(entry) => {
            let entry = entries
                .iter()
                .find(|e| &e.id == entry)
                .ok_or("no such part")?;
            let part = new_part(entry, spot, draft, symmetry);
            let added = match (&place, spot) {
                (Some(place), Spot::Surface { .. }) if symmetry > 1 => {
                    builder::patterned(&part, place, symmetry).map_err(|r| r.to_string())?
                }
                (Some(place), _) => vec![builder::held_at(part, place)],
                (None, _) => vec![part],
            };
            let ids = added.iter().map(|p| p.id.clone()).collect();
            craft.parts.extend(added);
            Ok((craft, ids))
        }
        Held::Moved(id) => {
            let place = place.ok_or("it has to be held by something")?;
            let index = (craft.parts.iter())
                .position(|p| &p.id == id)
                .ok_or("it is not on the craft")?;
            let moved = builder::held_at(craft.parts[index].clone(), &place);
            craft.parts[index] = moved;
            Ok((craft, held_through(draft, id).into_iter().collect()))
        }
    }
}

/// Put what is in hand at `spot` through the builder, with its rules: what a click or a drop does.
/// The ids of the parts it put there.
fn put(
    editor: &mut Editor,
    catalog: &PartCatalog,
    held: &Held,
    entries: &[Entry],
    spot: &Spot,
    symmetry: u32,
    roll: f64,
) -> Result<Vec<String>, String> {
    let Some(builder) = editor.builder_mut() else {
        return Err("there is no craft to build".into());
    };
    let place = place_at(spot, roll);
    let change = match held {
        Held::New(entry) => {
            let entry = entries
                .iter()
                .find(|e| &e.id == entry)
                .ok_or("no such part")?;
            let part = new_part(entry, spot, builder.draft(), symmetry);
            match place {
                Some(place) if symmetry > 1 && matches!(spot, Spot::Surface { .. }) => {
                    builder.pattern(catalog, part, place, symmetry)
                }
                place => builder.place(catalog, part, place),
            }
        }
        Held::Moved(id) => {
            let place = place.ok_or("it has to be held by something")?;
            builder.move_to(catalog, id, place).map(|mut c| {
                c.added = vec![id.clone()];
                c
            })
        }
    };
    change.map(|c| c.added).map_err(|r| r.to_string())
}

/// The free nodes of `craft`, by part id.
fn free_nodes(craft: &Craft) -> Vec<FreeNode> {
    let poses = craft.assembly.poses();
    let mut out = Vec::new();
    for (index, part) in craft.assembly.parts().iter().enumerate() {
        let used: Vec<&str> = (craft.assembly.joints().iter())
            .flat_map(|joint| joint.ends.iter())
            .filter(|end| end.part == index)
            .filter_map(|end| match &end.mount {
                Mount::Node(name) => Some(name.as_str()),
                Mount::Surface { .. } => None,
            })
            .collect();
        // A part slid or turned is held by a node of its own that stands for one it has: that one
        // is taken too.
        let taken = |name: &str| {
            used.iter()
                .any(|u| *u == name || builder::stands_for(u) == name)
        };
        for node in part.nodes().filter(|n| !taken(&n.name)) {
            out.push(FreeNode {
                part: part.id.clone(),
                name: node.name.clone(),
                way: poses[index].rotation * node.direction,
                size: node.size,
                local: node.position,
                at: node.position,
                screen: None,
            });
        }
    }
    out
}

/// Which of `part`'s own nodes goes on a node facing `way` (the craft's frame): the one that faces
/// most the other way as the part stands, of those nothing is held at (`used`). None if there is
/// none.
fn by_node(
    part: &CraftPartDef,
    used: &[String],
    catalog: &PartCatalog,
    way: DVec3,
) -> Option<String> {
    let mut part = part.clone();
    part.id = "held".into();
    (part.on, part.at) = (None, None);
    let alone = CraftDef {
        id: "held".into(),
        name: String::new(),
        parts: vec![part],
        sequences: None,
        triggers: Vec::new(),
        rcs_shut: Vec::new(),
        abort: Vec::new(),
    };
    let craft = Craft::build(&alone, catalog).ok()?;
    let own = &craft.assembly.parts()[0];
    own.nodes()
        // Not one it was slid or turned to by hand, which meant that place only.
        .filter(|n| !used.contains(&n.name) && builder::stands_for(&n.name) == n.name)
        .max_by(|a, b| (-a.direction.dot(way)).total_cmp(&-b.direction.dot(way)))
        .map(|n| n.name.clone())
}

/// What a palette row being dragged carries, if one is: its entry's id.
fn dragged_entry(
    payloads: &Query<&DragPayload, With<GlobalZIndex>>,
    data: &UiData,
    formatters: &Formatters,
) -> Option<String> {
    payloads.iter().find_map(|payload| {
        let carried = payload.0.render(data, formatters);
        carried.strip_prefix(PALETTE_DRAG).map(str::to_owned)
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn place_parts(
    mut placing: ResMut<Placing>,
    mut shaping: ResMut<Shaping>,
    mut editor: ResMut<Editor>,
    mut sim: ResMut<Sim>,
    mut ui: ResMut<LayoutVisibility>,
    content: Res<GameContent>,
    (buttons, keys, typing): (
        Res<ButtonInput<MouseButton>>,
        Res<ButtonInput<KeyCode>>,
        Res<TextFocus>,
    ),
    pointer: ViewPointer,
    (frame, data, formatters): (Res<RenderFrame>, Res<UiData>, Res<Formatters>),
    camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    (parts, parents, payloads): (
        Query<&ShowsPart>,
        Query<&ChildOf>,
        Query<&DragPayload, With<GlobalZIndex>>,
    ),
    mut ray_cast: MeshRayCast,
    mut marks: ResMut<PartMarks>,
    mut actions: MessageReader<UiAction>,
    mut stores: Local<Option<(Arc<PartCatalog>, Arc<Vec<Entry>>)>>,
) {
    let placing = &mut *placing;
    let down = [
        buttons.pressed(MouseButton::Left),
        buttons.pressed(MouseButton::Right),
    ];
    let went_down = [0, 1].map(|b| down[b] && !placing.was_down[b]);
    let went_up = [0, 1].map(|b| !down[b] && placing.was_down[b]);
    placing.was_down = down;
    if stores.is_none() || content.is_changed() {
        let parts = catalog(&content.0.registry, &mut Vec::new());
        let entries = palette::entries(&parts);
        *stores = Some((Arc::new(parts), Arc::new(entries)));
    }
    let Some((shared, entries)) = stores.as_ref() else {
        return;
    };
    let catalog = &**shared;
    if !editor.is_open() {
        *placing = Placing {
            group: placing.group,
            search: std::mem::take(&mut placing.search),
            ..default()
        };
        marks.set_ghost(None);
        return;
    }

    for action in actions.read() {
        match action.name() {
            "editor.new" => {
                editor.start_new(&mut sim, &mut ui);
                shaping.pick(None);
                placing.held = None;
                placing.open = true;
            }
            // With a part's settings in the rail, the list comes back in their place.
            "editor.parts" => {
                placing.open = !placing.open || shaping.picked().is_some();
                shaping.pick(None);
                if placing.open {
                    editor.set_list_open(false);
                }
            }
            "editor.list" => placing.open = false,
            "editor.group" => {
                if let Some(group) = action.arg().and_then(Group::parse) {
                    placing.group = group;
                    placing.search.clear();
                }
            }
            "editor.search" => placing.search = action.arg().unwrap_or("").trim().to_owned(),
            "editor.add" => {
                let entry = action
                    .arg()
                    .and_then(|id| entries.iter().find(|e| e.id == id));
                if let Some(entry) = entry {
                    placing.take_in_hand(Held::New(entry.id.clone()), false);
                    shaping.pick(None);
                    editor.set_notice("");
                }
            }
            "editor.drop" => {
                placing.let_go();
            }
            "editor.symmetry" => placing.symmetry = next_symmetry(placing.symmetry, true),
            "editor.remove" => remove_picked(&mut shaping, &mut editor, catalog, &mut sim, &mut ui),
            _ => {}
        }
    }
    let Some(draft) = editor.builder().map(|b| b.draft().clone()) else {
        placing.held = None;
        marks.set_ghost(None);
        return;
    };
    // A row of the list dragged out takes its part in hand, to go where it is let go.
    if let Some(entry) = dragged_entry(&payloads, &data, &formatters)
        && placing.held != Some(Held::New(entry.clone()))
        && entries.iter().any(|e| e.id == entry)
    {
        placing.take_in_hand(Held::New(entry), true);
        shaping.pick(None);
        editor.set_notice("");
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if !typing.typing() {
        if keys.just_pressed(KeyCode::KeyX) {
            placing.symmetry = next_symmetry(placing.symmetry, !shift);
        }
        let command = keys.any_pressed([
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
        ]);
        if !command && keys.any_just_pressed([KeyCode::Delete, KeyCode::Backspace]) {
            remove_picked(&mut shaping, &mut editor, catalog, &mut sim, &mut ui);
        }
        // Q and E turn what is in hand, or the part picked.
        let turn = if shift { FINE_TURN } else { TURN };
        let turned = f64::from(keys.just_pressed(KeyCode::KeyE))
            - f64::from(keys.just_pressed(KeyCode::KeyQ));
        if turned != 0.0 && !command {
            if placing.is_holding() {
                placing.roll = wrapped(placing.roll + turned * turn);
            } else {
                turn_picked(
                    &shaping,
                    &mut editor,
                    catalog,
                    &mut sim,
                    &mut ui,
                    turned * turn,
                );
            }
        }
    }

    let cursor = pointer.position();
    let (camera, eye) = *camera;
    let vessel = editor.vessel();
    let draft_index = |id: &str| draft.parts.iter().position(|p| p.id == id);
    // The part of the draft under the pointer, of those `skip` does not leave out, and where the
    // pointer meets it in the view.
    let mut under = |skip: &dyn Fn(&str) -> bool| -> Option<(String, Vec3)> {
        let vessel = vessel?;
        let ray = camera.viewport_to_world(eye, cursor?).ok()?;
        let tagged = |e: Entity| {
            std::iter::once(e)
                .chain(parents.iter_ancestors(e))
                .find_map(|a| parts.get(a).ok())
                .filter(|s| s.vessel == vessel)
                .and_then(|s| draft.parts.get(s.part.0 as usize))
                .filter(|p| !skip(&p.id))
        };
        let filter = |e: Entity| tagged(e).is_some();
        let settings = MeshRayCastSettings::default().with_filter(&filter);
        let (mesh, hit) = ray_cast.cast_ray(ray, &settings).first()?;
        Some((tagged(*mesh)?.id.clone(), hit.point))
    };

    // A drag that starts on a part of the craft picks it up with what it holds; one that starts
    // on nothing turns the view.
    if !placing.is_holding() && !draft.parts.is_empty() {
        if went_down[0] && !shaping.holds_pointer() {
            placing.grabbed = under(&|_| false).map(|(id, _)| id).zip(cursor);
        }
        if !down[0] {
            placing.grabbed = None;
        }
        if let (Some((id, from)), Some(at)) = (placing.grabbed.clone(), cursor)
            && from.distance(at) >= CLICK_SLOP
        {
            placing.grabbed = None;
            // The root holds everything: dragging it turns the view instead.
            if draft.parts.iter().any(|p| p.id == id && p.on.is_some()) {
                placing.take_in_hand(Held::Moved(id), true);
                shaping.pick(None);
                editor.stop_view_drag();
                editor.set_notice("");
            }
        }
    }

    // The first part of an empty craft goes on as its root: straight away from a click on its
    // row, or where it is let go in the view.
    if let Some(held @ Held::New(_)) = placing.held.clone()
        && draft.parts.is_empty()
    {
        let now = !placing.dropping || (went_up[0] && cursor.is_some());
        if now {
            match put(&mut editor, catalog, &held, entries, &Spot::Root, 1, 0.0) {
                Ok(_) => {
                    redraw_draft(&mut editor, catalog, &mut sim, &mut ui);
                    editor.frame_view(&mut sim);
                }
                Err(e) => editor.set_notice(format!("Cannot start with it: {e}")),
            }
            placing.held = None;
            placing.shown = None;
        } else if went_up[0] {
            placing.let_go();
        }
        return;
    }

    // Which nodes are free, on the draft without what is in hand.
    let moved = match &placing.held {
        Some(Held::Moved(id)) => held_through(&draft, id),
        _ => BTreeSet::new(),
    };
    let stale = (placing.built.as_ref())
        .is_none_or(|(parts, held, _)| *parts != draft.parts || *held != placing.held);
    if stale {
        let mut rest = draft.clone();
        rest.parts.retain(|p| !moved.contains(&p.id));
        placing.built = Craft::build(&rest, catalog)
            .ok()
            .map(|craft| (draft.parts.clone(), placing.held.clone(), craft));
        placing.nodes = (placing.built.as_ref()).map_or(Vec::new(), |(_, _, c)| free_nodes(c));
    }

    let (Some(vessel), Some(held)) = (vessel, placing.held.clone()) else {
        finish_preview(
            placing,
            &mut editor,
            catalog,
            &mut sim,
            &mut ui,
            &mut marks,
            &draft,
        );
        return;
    };
    let Some(vessel_frame) = VesselFrame::of(&sim, &frame, vessel) else {
        return;
    };
    // Where the free nodes are on show.
    if let Some(on_show) = sim.universe.vessel(vessel) {
        let tree = on_show.propulsion.parts();
        let base = DVec3::Z * tree.base();
        for node in &mut placing.nodes {
            let shown = draft_index(&node.part).and_then(|i| tree.parts().get(i));
            if let Some(part) = shown {
                node.at = part.position - base + part.rotation * node.local;
            }
        }
    }

    // What is in hand, and how it can go on.
    let (in_hand, surface) = match &held {
        Held::New(id) => {
            let Some(entry) = entries.iter().find(|e| &e.id == id) else {
                placing.held = None;
                return;
            };
            (entry.part.clone(), entry.surface.clone())
        }
        Held::Moved(id) => {
            let Some(part) = draft.parts.iter().find(|p| &p.id == id) else {
                placing.held = None;
                return;
            };
            // On a surface again only if it was on one.
            let surface = part.at.is_some().then(|| (part.by.clone(), part.roll));
            (part.clone(), surface)
        }
    };
    // Its own nodes that hold something already cannot hold it on.
    let used: Vec<String> = (draft.parts.iter())
        .filter_map(|p| p.on.as_deref()?.split_once('.'))
        .filter(|(holder, _)| *holder == in_hand.id)
        .map(|(_, node)| node.to_owned())
        .collect();

    // Where it would go.
    let near = cursor.and_then(|at| {
        (placing.nodes.iter())
            .filter_map(|n| Some((n, n.screen?.distance(at))))
            .filter(|(_, d)| *d <= NODE_REACH_PX)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(n, _)| n.clone())
    });
    let spot = if let Some(node) = near {
        by_node(&in_hand, &used, catalog, node.way).map(|by| Spot::Node {
            part: node.part.clone(),
            node: node.name.clone(),
            by,
            width: node.size,
        })
    } else if let Some((by, roll)) = surface {
        under(&|id| moved.contains(id)).and_then(|(part_id, point)| {
            let on_show = sim.universe.vessel(vessel)?;
            let tree = on_show.propulsion.parts();
            let shown = tree.parts().get(draft_index(&part_id)?)?;
            let base = DVec3::Z * tree.base();
            let local = shown.rotation.inverse()
                * (vessel_frame.vessel_point(point) - (shown.position - base));
            let geometry = &shown.built.as_ref()?.geometry;
            let (surface, u, v, off) = geometry.surface_near(local)?;
            let out = local.truncate().length();
            if off > ON_WALL + ON_WALL_SHARE * out {
                return None;
            }
            // The side facing the eye: inside only a container, which holds things there.
            let point = geometry.surface_point(&surface, u, v)?;
            let eye_at = vessel_frame.vessel_point(eye.translation());
            let toward = eye_at - (shown.position - base + shown.rotation * point.position);
            let container = (shown.built.as_ref()?.components.iter())
                .any(|c| matches!(c, osp_sim::component::Component::Container(_)));
            let side = if container && (shown.rotation * point.normal).dot(toward) < 0.0 {
                Side::Back
            } else {
                Side::Front
            };
            Some(Spot::Surface {
                part: part_id,
                surface,
                at: SurfaceAtDef {
                    u,
                    v,
                    side,
                    standoff: 0.0,
                },
                by,
                roll,
                width: 2.0 * out,
            })
        })
    } else {
        None
    };
    placing.spot = spot;

    // A right click puts it back. A click puts it where it is shown, or, dragged, letting go does.
    if went_down[1] {
        placing.right_press = cursor;
    }
    if went_up[1]
        && let (Some(from), Some(at)) = (placing.right_press.take(), cursor)
        && from.distance(at) < CLICK_SLOP
    {
        placing.let_go();
        return;
    }
    if went_down[0] {
        placing.press = cursor;
    }
    let clicked = went_up[0]
        && (placing.press.take().zip(cursor))
            .is_some_and(|(from, at)| from.distance(at) < CLICK_SLOP);
    let put_now = if placing.dropping {
        went_up[0]
    } else {
        clicked
    };
    if put_now {
        let Some(spot) = placing.spot.clone() else {
            // Let go where it cannot go: it goes back.
            if placing.dropping {
                placing.let_go();
            }
            return;
        };
        let (symmetry, roll) = (placing.symmetry, placing.roll);
        let symmetry = if matches!(held, Held::New(_)) {
            symmetry
        } else {
            1
        };
        match put(&mut editor, catalog, &held, entries, &spot, symmetry, roll) {
            Ok(_) => {
                // What is on show is already this, if the preview was of it.
                let matched =
                    placing.shown == Some((spot, symmetry, roll)) && placing.refused.is_none();
                if !matched {
                    redraw_draft(&mut editor, catalog, &mut sim, &mut ui);
                }
                editor.settle(&mut sim);
                editor.set_notice("");
                *placing = Placing {
                    open: placing.open,
                    group: placing.group,
                    search: std::mem::take(&mut placing.search),
                    symmetry: placing.symmetry,
                    was_down: placing.was_down,
                    ..default()
                };
                // Not picked: the parts list stays for the next one, and a click on the part shows
                // its settings in its place.
                marks.set_ghost(None);
                return;
            }
            Err(e) => {
                editor.set_notice(format!("Cannot put it there: {e}"));
                if placing.dropping {
                    placing.let_go();
                }
            }
        }
    }

    // Show it where it would go.
    let symmetry = if matches!(held, Held::New(_)) {
        placing.symmetry
    } else {
        1
    };
    let wanted = placing.spot.clone().map(|s| (s, symmetry, placing.roll));
    preview(
        placing,
        &mut editor,
        &held,
        entries,
        &draft,
        shared,
        wanted,
        &mut sim,
        &mut ui,
    );
    let ghost: Vec<PartId> = (placing.ghost.iter())
        .filter_map(|id| {
            let shown = sim.universe.vessel(editor.vessel()?)?;
            let craft_index = match &held {
                Held::New(_) => draft.parts.len() + placing.ghost.iter().position(|g| g == id)?,
                Held::Moved(_) => draft_index(id)?,
            };
            (craft_index < shown.propulsion.parts().parts().len())
                .then_some(PartId(craft_index as u32))
        })
        .collect();
    let ghost = (!ghost.is_empty() && placing.shown.is_some()).then(|| {
        (
            editor.vessel().unwrap_or(vessel),
            ghost,
            placing.refused.is_some(),
        )
    });
    marks.set_ghost(ghost);
}

/// Degrees in (-180, 180].
fn wrapped(degrees: f64) -> f64 {
    let turned = (degrees + 180.0).rem_euclid(360.0) - 180.0;
    if turned == -180.0 { 180.0 } else { turned }
}

/// Turn the picked part `degrees` about the joint that holds it.
fn turn_picked(
    shaping: &Shaping,
    editor: &mut Editor,
    catalog: &PartCatalog,
    sim: &mut Sim,
    ui: &mut LayoutVisibility,
    degrees: f64,
) {
    let Some(id) = shaping.picked().map(str::to_owned) else {
        return;
    };
    let Some(builder) = editor.builder_mut() else {
        return;
    };
    let held = builder
        .draft()
        .parts
        .iter()
        .any(|p| p.id == id && p.on.is_some());
    if !held {
        return;
    }
    match builder.rotate(catalog, &id, degrees) {
        Ok(_) => {
            redraw_draft(editor, catalog, sim, ui);
            editor.set_notice("");
        }
        Err(e) => editor.set_notice(format!("Cannot turn it: {e}")),
    }
}

/// Bring what is on show in step with `wanted`: put on show what has been built, and build what is
/// wanted next once nothing else is being built.
#[allow(clippy::too_many_arguments)]
fn preview(
    placing: &mut Placing,
    editor: &mut Editor,
    held: &Held,
    entries: &[Entry],
    draft: &CraftDef,
    catalog: &Arc<PartCatalog>,
    wanted: Wanted,
    sim: &mut Sim,
    ui: &mut LayoutVisibility,
) {
    let done = placing
        .job
        .as_ref()
        .is_some_and(|job| job.task.is_finished());
    if let Some(job) = placing.job.take_if(|_| done) {
        match job
            .task
            .join()
            .unwrap_or_else(|_| Err("building it failed".into()))
        {
            Ok((craft, propulsion, refused)) => {
                editor.redraw_built(&craft, propulsion, sim, ui);
                placing.ghost = job.ghost;
                placing.refused = refused;
            }
            Err(e) => {
                warn!("cannot show it there: {e}");
                placing.ghost.clear();
                placing.refused = Some(e);
            }
        }
        placing.shown = job.wanted;
    }
    if wanted == placing.shown || placing.job.is_some() {
        return;
    }
    let made = match &wanted {
        Some((spot, symmetry, roll)) => with_held(draft, held, entries, spot, *symmetry, *roll),
        None => Ok((draft.clone(), Vec::new())),
    };
    let (craft, ghost) = match made {
        Ok(made) => made,
        Err(e) => {
            placing.refused = Some(e);
            placing.shown = wanted;
            return;
        }
    };
    let (catalog, touched) = (catalog.clone(), ghost.clone());
    let task = std::thread::spawn(move || {
        let propulsion = Propulsion::from_craft(&craft, &catalog).map_err(|e| e.to_string())?;
        let touched: Vec<&str> = touched.iter().map(String::as_str).collect();
        let refused = (!touched.is_empty())
            .then(|| builder::check(&craft, &catalog, &touched).err())
            .flatten()
            .map(|r| r.to_string());
        Ok((craft, propulsion, refused))
    });
    placing.job = Some(Job {
        task,
        wanted,
        ghost,
    });
}

/// Nothing is in hand: if a preview is on show, the draft goes back on show.
fn finish_preview(
    placing: &mut Placing,
    editor: &mut Editor,
    catalog: &PartCatalog,
    sim: &mut Sim,
    ui: &mut LayoutVisibility,
    marks: &mut PartMarks,
    draft: &CraftDef,
) {
    placing.spot = None;
    placing.job = None;
    if placing.restore || placing.shown.is_some() {
        if placing.shown.is_some() && !draft.parts.is_empty() {
            redraw_draft(editor, catalog, sim, ui);
        }
        placing.shown = None;
        placing.restore = false;
        placing.ghost.clear();
        placing.refused = None;
    }
    marks.set_ghost(None);
}

/// Put the draft on show as it stands.
fn redraw_draft(
    editor: &mut Editor,
    catalog: &PartCatalog,
    sim: &mut Sim,
    ui: &mut LayoutVisibility,
) {
    let Some(draft) = editor.builder().map(|b| b.draft().clone()) else {
        return;
    };
    if let Err(e) = editor.redraw(&draft, catalog, sim, ui) {
        error!("cannot show the craft: {e}");
    }
}

/// Take the picked part off, with everything held by it.
fn remove_picked(
    shaping: &mut Shaping,
    editor: &mut Editor,
    catalog: &PartCatalog,
    sim: &mut Sim,
    ui: &mut LayoutVisibility,
) {
    let Some(id) = shaping.picked().map(str::to_owned) else {
        return;
    };
    let Some(builder) = editor.builder_mut() else {
        return;
    };
    // The root of a craft with more on it is refused: everything is held by it.
    match builder.remove(catalog, &id) {
        Ok(_) => {
            shaping.pick(None);
            redraw_draft(editor, catalog, sim, ui);
            editor.settle(sim);
            editor.set_notice("");
        }
        Err(e) => editor.set_notice(format!("Cannot take it off: {e}")),
    }
}

/// The symmetry after `now`, going up or down the list, round from the end.
fn next_symmetry(now: u32, up: bool) -> u32 {
    let at = SYMMETRIES.iter().position(|&s| s == now).unwrap_or(0);
    let n = SYMMETRIES.len();
    SYMMETRIES[if up { (at + 1) % n } else { (at + n - 1) % n }]
}

/// The rings on free nodes.
#[derive(Component)]
struct Ring(usize);

fn spawn_rings(mut commands: Commands) {
    for index in 0..MOST_RINGS {
        commands.spawn((
            Ring(index),
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(RING_PX),
                height: Val::Px(RING_PX),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BorderColor::all(ACCENT),
            BackgroundColor(Color::NONE),
            Pickable::IGNORE,
            ViewOverlay,
            GlobalZIndex(-5),
            Visibility::Hidden,
        ));
    }
}

/// Put a ring on each free node while something is in hand, filled on the one it would go on.
fn draw_rings(
    mut placing: ResMut<Placing>,
    editor: Res<Editor>,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    camera: Single<(&Camera, &Transform), With<MainCamera>>,
    mut rings: Query<(&Ring, &mut Node, &mut Visibility, &mut BackgroundColor)>,
) {
    let (camera, transform) = *camera;
    let eye = GlobalTransform::from(*transform);
    let holding = placing.is_holding();
    let vessel_frame = editor
        .vessel()
        .and_then(|v| VesselFrame::of(&sim, &frame, v));
    let target = match &placing.spot {
        Some(Spot::Node { part, node, .. }) => Some((part.clone(), node.clone())),
        _ => None,
    };
    for node in &mut placing.nodes {
        node.screen = (vessel_frame.filter(|_| holding))
            .and_then(|f| camera.world_to_viewport(&eye, f.view_point(node.at)).ok());
    }
    for (ring, mut style, mut visibility, mut fill) in &mut rings {
        let node = placing.nodes.get(ring.0);
        let at = node.and_then(|n| n.screen);
        *visibility = if at.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if let (Some(at), Some(node)) = (at, node) {
            style.left = Val::Px(at.x - RING_PX / 2.0);
            style.top = Val::Px(at.y - RING_PX / 2.0);
            let on = (target.as_ref()).is_some_and(|(p, n)| *p == node.part && *n == node.name);
            fill.0 = if on { ACCENT } else { Color::NONE };
        }
    }
}

/// `editor.palette*`, `editor.holding` and, with something in hand, `editor.hint`; see the module.
#[allow(clippy::type_complexity)]
fn publish_placing(
    placing: Res<Placing>,
    editor: Res<Editor>,
    content: Res<GameContent>,
    mut data: ResMut<UiData>,
    mut entries: Local<Option<Vec<Entry>>>,
    mut published: Local<
        Option<(
            bool,
            Group,
            String,
            Option<Held>,
            u32,
            Option<String>,
            bool,
            i64,
        )>,
    >,
) {
    if entries.is_none() || content.is_changed() {
        let parts = catalog(&content.0.registry, &mut Vec::new());
        *entries = Some(palette::entries(&parts));
    }
    let Some(entries) = entries.as_ref() else {
        return;
    };
    let now = (
        placing.open && editor.is_open(),
        placing.group,
        placing.search.clone(),
        placing.held.clone(),
        placing.symmetry,
        placing.refused.clone(),
        placing.spot.is_some(),
        placing.roll.round() as i64,
    );
    if published.as_ref() == Some(&now) && !content.is_changed() {
        return;
    }
    *published = Some(now);
    data.set("editor.palette_open", placing.open && editor.is_open());
    data.set("editor.group", placing.group.id());
    let shown: Vec<&Entry> = if placing.search.is_empty() {
        entries
            .iter()
            .filter(|e| e.group == placing.group)
            .collect()
    } else {
        entries
            .iter()
            .filter(|e| e.matches(&placing.search))
            .collect()
    };
    let held_entry = match &placing.held {
        Some(Held::New(id)) => Some(id.as_str()),
        _ => None,
    };
    let rows: Vec<_> = (shown.iter().take(MOST_ROWS))
        .map(|e| {
            record([
                ("id", e.id.clone().into()),
                ("name", e.name.clone().into()),
                ("detail", e.detail.clone().into()),
                ("held", (held_entry == Some(e.id.as_str())).into()),
            ])
        })
        .collect();
    data.set("editor.palette", rows);
    let more = shown.len().saturating_sub(MOST_ROWS);
    data.set(
        "editor.palette_more",
        if more > 0 {
            format!("{more} more: search to find them")
        } else if shown.is_empty() {
            "Nothing found".to_owned()
        } else {
            String::new()
        },
    );
    data.set(
        "editor.search_shown",
        if placing.search.is_empty() {
            "Search parts"
        } else {
            placing.search.as_str()
        },
    );
    data.set("editor.holding", placing.held.is_some());
    let Some(held) = &placing.held else {
        return;
    };
    let (name, surface) = match held {
        Held::New(id) => {
            let entry = entries.iter().find(|e| &e.id == id);
            (
                entry.map_or("part".to_owned(), |e| e.name.to_lowercase()),
                entry.is_some_and(|e| e.surface.is_some()),
            )
        }
        Held::Moved(id) => {
            // Back on a surface if it was on one.
            let draft = editor.builder().map(|b| b.draft());
            let part = draft.and_then(|d| d.parts.iter().find(|p| &p.id == id));
            let name = part
                .and_then(|p| p.name.clone())
                .unwrap_or_else(|| id.clone());
            (name.to_lowercase(), part.is_some_and(|p| p.at.is_some()))
        }
    };
    let how = if placing.dropping {
        "Let go on a ring"
    } else {
        "Click a ring"
    };
    let mut hint = if surface && matches!(held, Held::New(_)) {
        format!(
            "{how} or the side of a part to put the {name} there. X: symmetry, now {}.",
            placing.symmetry
        )
    } else if surface {
        format!("{how} or the side of a part to put the {name} there.")
    } else {
        format!("{how} to put the {name} there.")
    };
    hint.push_str(" Q and E turn it.");
    if let Some(why) = &placing.refused {
        hint = format!("It cannot go there: {why}. {hint}");
    }
    hint.push_str(" Esc or a right click puts it back.");
    data.set("editor.hint", hint);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn craft(parts: serde_json::Value) -> CraftDef {
        serde_json::from_value(serde_json::json!({ "id": "c", "parts": parts })).unwrap()
    }

    fn ids(names: &[&str]) -> CraftDef {
        let parts: Vec<serde_json::Value> = (names.iter())
            .map(|id| serde_json::json!({ "id": id, "template": "x" }))
            .collect();
        craft(serde_json::Value::Array(parts))
    }

    #[test]
    fn a_new_part_gets_an_id_no_part_and_none_of_its_copies_has() {
        assert_eq!(new_id(&ids(&[]), "tank", 0), "tank");
        assert_eq!(new_id(&ids(&["tank"]), "tank", 0), "tank_2");
        assert_eq!(new_id(&ids(&["tank", "tank_2"]), "tank", 0), "tank_3");
        // Four fins would be fin_1 to fin_4: one of those is taken.
        assert_eq!(new_id(&ids(&["fin_3"]), "fin", 4), "fin_2");
    }

    #[test]
    fn symmetry_steps_up_and_back_round_its_list() {
        assert_eq!(next_symmetry(1, true), 2);
        assert_eq!(next_symmetry(8, true), 1);
        assert_eq!(next_symmetry(1, false), 8);
        assert_eq!(next_symmetry(5, true), 2);
    }

    #[test]
    fn a_part_picked_up_takes_what_it_holds_and_nothing_else() {
        let draft = craft(serde_json::json!([
            { "id": "pod", "template": "x" },
            { "id": "tank", "on": "pod.bottom", "template": "x" },
            { "id": "engine", "on": "tank.bottom", "template": "x" },
            { "id": "fin", "on": "tank.barrel", "template": "x" },
            { "id": "rcs", "on": "pod.side", "template": "x" },
        ]));
        let taken = held_through(&draft, "tank");
        assert_eq!(
            taken.into_iter().collect::<Vec<_>>(),
            ["engine", "fin", "tank"]
        );
        assert_eq!(held_through(&draft, "rcs").len(), 1);
    }

    #[test]
    fn turns_go_round_and_stay_within_a_half_turn_each_way() {
        assert_eq!(wrapped(190.0), -170.0);
        assert_eq!(wrapped(-180.0), 180.0);
        assert_eq!(wrapped(45.0), 45.0);
    }
}
