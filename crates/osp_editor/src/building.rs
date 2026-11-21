//! Building a craft from parts in the editor, as in KSP with its procedural parts. `editor.new`
//! starts an empty craft, a rocket for the pad, or with `editor.new:plane` a plane, built lying as it
//! will stand on the runway. The parts list (`editor.parts` shows and hides it) is
//! [`crate::parts_list`]'s.
//!
//! A part is taken in hand by dragging its tile into the hangar, by clicking its tile
//! (`editor.add:<entry>`), or, for a part already on the craft, by dragging it off with the left
//! button, with everything held by it and the parts put on with it by symmetry. Dragging the root
//! carries the whole craft about the hangar instead: up off the floor to make room under it, say.
//! A drag that starts on nothing turns the view. The first part of an empty craft goes on as its
//! root, floating well over the floor. After that the craft's free nodes show as rings, and what is
//! in hand is shown where it would go, built on a thread of its own as the pointer moves:
//!
//! - On a ring that one of its own nodes comes near, as it is drawn at the pointer, facing the
//!   other way: by that node, not by where the pointer is (a new part as wide as that node, or on
//!   an engine's mount a tank sized to feed it, [`osp_sim::fitting`]); or, on a node that faces
//!   across the craft, by its side, upright beside what holds it, as a booster on a radial
//!   decoupler, when the pointer comes near the ring.
//! - On the side of any part, at the point under the pointer: fins, wings and RCS blocks by the
//!   node they stand on, sized for the hull there; a decoupler by its top, standing out of it, to
//!   hold a booster; anything else by its side, upright: a tank, an engine, a pod. A new tank,
//!   tube or decoupler there is half as wide as the hull, as a booster beside its core.
//!
//! Symmetry: X steps the radial count (1, 2, 3, 4, 6, 8; Shift+X back) and R switches to mirror,
//! across the craft's middle plane (its nose and belly), and back (`editor.symmetry`,
//! `editor.symmetry_mode`). Radial copies go round the part they are on, a mirror copy to the far
//! side of the plane. Whatever is put on a part that has copies by symmetry goes on each of them
//! as well, mirrored on mirror images: a nose cone on one of four boosters goes on all four. The
//! copies are plain parts; nothing remembers them (`osp_sim::builder::alike` finds them again).
//!
//! Q and E turn what is in hand about the joint, a turn step a press (15° unless C or `editor.snap`
//! changed it; finer with Shift); with nothing in hand
//! they turn the picked part, or the whole craft about its axis if that is the root. The builder's
//! rules are checked on the preview's thread: where it may not go it shows red and the hint says
//! why. Dragged, it goes where it is let go; taken by a click, where the next click is. The parts
//! list stays out for the next one; a click on the part picks it to shape it. Esc, a right click or
//! `editor.drop` puts it back. Delete or Backspace, or `editor.remove`, takes the picked part off
//! with everything held by it. A part dragged off the craft and let go where it cannot go on (out
//! in the hangar, or over the parts list) comes off, with its copies by symmetry and all they hold;
//! while it is there the craft is shown without them. All of it can be undone.
//!
//! Until it has somewhere to go, what is in hand is drawn at the pointer, in the plane of the craft
//! square to the view, whether it was taken from the list or dragged off the craft; over the hangar
//! that takes the place of the list's tile.
//!
//! Publishes `editor.holding`, `editor.floating` (what is in hand is drawn at the pointer),
//! `editor.symmetry` (the radial count as it reads: `1`, `x4`), `editor.mirror`, and `editor.hint`
//! while something is in hand.

use std::{collections::BTreeSet, sync::Arc, thread::JoinHandle};

use bevy::{
    math::{DQuat, DVec3},
    picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings},
    prelude::*,
};
use osp_sim::{
    assembly::{Mount, Side},
    builder::{self, Allowed, Loose, Place},
    component::ComponentDef,
    craft::Craft,
    data::{CraftDef, CraftPartDef, PartCatalog, SurfaceAtDef},
    fitting::{self, Against},
    geometry::preset::NodeDef,
    palette::{self, Entry},
    parts::{PartId, PartTree},
    properties,
    propulsion::Propulsion,
};
use osp_ui::{
    CLICK_SLOP, DragPayload, Formatters, LayoutVisibility, TextFocus, UiAction, UiData, ViewOverlay,
};

use osp_game::{
    FrameSet,
    camera::MainCamera,
    content::{GameContent, catalog},
    frame::{RenderFrame, to_render},
    orbit_lines::ViewPointer,
    part_highlight::PartMarks,
    part_info::ShowsPart,
    part_pictures::{LooseParts, PartInHand, ShowsLoose},
    sim::Sim,
};

use crate::{
    hangar::{Editor, editor_actions},
    moving::Moving,
    shaping::{Shaping, VesselFrame, shape_parts, tree_origin},
};

/// How far from the eye what is in hand is drawn while there is no craft yet, m.
const NO_CRAFT_DEPTH: f32 = 18.0;
/// Nearer the eye than this, m, what is in hand is not drawn at the pointer.
const MIN_DEPTH: f32 = 0.5;
/// How near a free node the pointer has to come for the part in hand to go there, px: what goes on
/// by its side, which has no node of its own to bring, and anything until it has been drawn.
const NODE_REACH_PX: f32 = 18.0;
/// How near a free node's ring one of the in-hand part's own nodes has to come, as it is drawn at
/// the pointer, for it to go on there, px; and how far it may then go before it comes off again, as
/// a share of that.
const NODE_SNAP_PX: f32 = 24.0;
const SNAP_HOLD: f32 = 1.5;
/// How squarely an own node has to face the other way from a free node to go on it (the cosine).
const FACING: f64 = 0.5;
/// How a free node's ring is drawn, px across, and the most drawn.
const RING_PX: f32 = 14.0;
const MOST_RINGS: usize = 64;
/// How far off a wall what the pointer touches may be and still be on it: the drawn mesh is a
/// little off the exact shape. m, and as a share of how far the wall is from the axis.
const ON_WALL: f64 = 0.15;
const ON_WALL_SHARE: f64 = 0.05;
/// The radial symmetries the X key steps through.
const SYMMETRIES: [u32; 6] = [1, 2, 3, 4, 6, 8];
/// A node whose way out is less than this far along the craft's axis (the cosine) faces across
/// it: a part with a side goes on there by its side, upright.
const ACROSS: f64 = 0.7;
/// How square to a part's axis its surface faces, at least, to be one of its ends: what would stand
/// beside the part goes on there along the axis instead (the cosine of 45 degrees).
const END_FACING: f64 = 0.707;
/// What a list row carries when dragged: `palette:<entry>`.
const PALETTE_DRAG: &str = "palette:";
/// The `id` of the parts list's rail in the layout: what is let go over it is thrown away.
const PARTS_RAIL: &str = "editor_parts_rail";
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
                    show_in_hand.after(place_parts).after(FrameSet::Input),
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
    /// A part of the craft, by its id, with everything held by it and the parts put on with it by
    /// symmetry.
    Moved(String),
    /// A group of parts taken off the craft and left about the hangar, by its index in the
    /// builder's ([`Loose`]).
    Loose(usize),
}

/// How what is in hand goes on a surface.
#[derive(Debug, Clone, PartialEq)]
enum OnSurface {
    /// By a node of its own, turned so far about the joint, degrees: a fin, a wing, an RCS block,
    /// or a decoupler by its top, standing out of the surface.
    By(String, f64),
    /// By its side, upright beside what it is on: a tank, an engine, a pod.
    Side,
    /// By a node of its own, turned to roll along the craft: a wheel.
    Rolling(String),
}

/// Where what is in hand would go. Parts are named by their ids.
#[derive(Debug, Clone, PartialEq)]
enum Spot {
    /// At a free node of `part`: by its own node `by`, a new part as wide as `width`; or by its
    /// side, upright, if `by` is none.
    Node {
        part: String,
        node: String,
        by: Option<String>,
        width: f64,
    },
    /// On a surface of `part` at `at`, a new part sized for a hull `width` across. On one of its
    /// ends (a tank's bottom, its thrust plate), `end` is the way out of that end along the part's
    /// axis, in the craft's frame.
    Surface {
        part: String,
        surface: String,
        at: SurfaceAtDef,
        width: f64,
        end: Option<DVec3>,
    },
}

impl Spot {
    /// Whether this is `was`, but for where the pointer met a surface moving less than
    /// [`SPOT_SLOP`]: rounding as the craft is drawn again, not a hand moving.
    fn barely_moved_from(&self, was: &Spot) -> bool {
        match (self, was) {
            (
                Spot::Surface {
                    part,
                    surface,
                    at,
                    width,
                    end,
                },
                Spot::Surface {
                    part: was_part,
                    surface: was_surface,
                    at: was_at,
                    width: was_width,
                    end: was_end,
                },
            ) => {
                let round = (at.u - was_at.u).abs();
                part == was_part
                    && surface == was_surface
                    && at.side == was_at.side
                    && end.is_some() == was_end.is_some()
                    && round.min(1.0 - round) < SPOT_SLOP
                    && (at.v - was_at.v).abs() < SPOT_SLOP
                    && (width - was_width).abs() < SPOT_SLOP * width.abs().max(1.0)
            }
            _ => false,
        }
    }
}

/// How far round or along a surface (a share of it) where the pointer meets it may move and still
/// be the same spot: a tenth of a millimetre on a 4 m tank.
const SPOT_SLOP: f64 = 2.5e-5;

/// What putting what is in hand somewhere does: the parts it puts on (the one in hand first, then
/// what it holds), each place they go (and whether as their mirror image), and the parts they
/// replace (what was picked up, and its copies).
#[derive(Debug, Clone, PartialEq)]
struct Plan {
    parts: Vec<CraftPartDef>,
    places: Vec<(Place, bool)>,
    replacing: BTreeSet<String>,
    /// The loose group the parts are, which is no longer loose once they are on.
    from_loose: Option<usize>,
}

impl Plan {
    /// Taking `replacing` off the craft, with all they hold: what a part dragged off the craft and
    /// let go where it cannot go on does.
    fn taking_off(replacing: BTreeSet<String>) -> Self {
        Self {
            parts: Vec::new(),
            places: Vec::new(),
            replacing,
            from_loose: None,
        }
    }

    fn takes_off(&self) -> bool {
        self.places.is_empty()
    }

    fn put(&self, draft: &CraftDef) -> Result<(CraftDef, Vec<String>), String> {
        if self.takes_off() {
            let craft = builder::removed(draft, &self.replacing).map_err(|r| r.to_string())?;
            return Ok((craft, Vec::new()));
        }
        let (craft, tops) = builder::put_set(draft, &self.parts, &self.places, &self.replacing)
            .map_err(|r| r.to_string())?;
        let mut ghost = Vec::new();
        for top in &tops {
            ghost.extend(held_through(&craft, top));
        }
        Ok((craft, ghost))
    }
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

/// One of the in-hand part's own free nodes, as the part is drawn at the pointer: its name, how far
/// from the pointer it is on the screen, px, and which way it faces in the craft's frame.
#[derive(Debug, Clone)]
struct OwnNode {
    name: String,
    offset: Vec2,
    way: DVec3,
}

/// What a preview is built into on its thread: the craft, its parts, the parts in hand, and why
/// the rules would not have it, if they would not.
type Built = Result<(CraftDef, Propulsion, Vec<String>, Option<String>), String>;

/// A preview being built.
struct Job {
    task: JoinHandle<Built>,
    wanted: Option<Plan>,
}

/// The whole craft being carried about the hangar by its root.
#[derive(Debug, Clone, Copy)]
struct Carry {
    /// Where the root was, from the middle of the floor.
    from: DVec3,
    /// The point taken hold of, from the middle of the floor then, and the way the view looked:
    /// it is carried in the plane square to that through the point.
    grabbed: DVec3,
    look: DVec3,
}

/// The parts list and what is in hand.
#[derive(Resource)]
pub struct Placing {
    open: bool,
    held: Option<Held>,
    /// It was dragged into the view or off the craft: it goes where it is let go.
    dropping: bool,
    /// How many go round, and whether one goes across the middle instead.
    symmetry: u32,
    mirror: bool,
    /// Degrees it is turned about the joint besides how it goes on.
    roll: f64,
    spot: Option<Spot>,
    plan: Option<Plan>,
    /// What the plan was worked out for: where, the symmetry, and the turn.
    asked: Option<(Spot, u32, bool, f64)>,
    nodes: Vec<FreeNode>,
    /// The craft without what is in hand, built to tell which of its nodes are free: the parts it
    /// was built from, what was in hand then, and the craft.
    built: Option<(Vec<CraftPartDef>, Option<Held>, Arc<Craft>)>,
    /// Parts built on their own, to tell how they go on: the last few.
    alone: Vec<(CraftPartDef, Arc<Craft>)>,
    /// What is on show: the draft as the plan would leave it, or the draft alone; and that craft.
    shown: Option<Plan>,
    shown_craft: Option<CraftDef>,
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
    /// The same on a loose group: its index and where.
    grabbed_loose: Option<(usize, Vec2)>,
    /// The pointer is over the parts list: what is let go there is thrown away.
    over_list: bool,
    /// What is in hand built by itself, as it is drawn: the parts, and the craft and tree.
    floating: Option<(Vec<CraftPartDef>, Arc<(Craft, PartTree)>)>,
    /// Its own free nodes on the screen, and the pair it went on by last: its node, and the part
    /// and node of the craft's.
    own: Vec<OwnNode>,
    snapped: Option<(String, String, String)>,
    /// Where the pointer brings the own node that goes on each free node onto its ring: the part
    /// and node, and the pointer, px. For scripted captures.
    brought: Vec<(String, String, Vec2)>,
    carry: Option<Carry>,
    /// Esc while carrying the craft: it goes back here.
    put_back: Option<DVec3>,
    /// A fuel line is on the craft that has yet to be told what it feeds.
    linking: bool,
    /// Esc while it is: the line comes off.
    cancel_link: bool,
}

impl Default for Placing {
    fn default() -> Self {
        Self {
            open: false,
            held: None,
            dropping: false,
            symmetry: 1,
            mirror: false,
            roll: 0.0,
            spot: None,
            plan: None,
            asked: None,
            nodes: Vec::new(),
            built: None,
            alone: Vec::new(),
            shown: None,
            shown_craft: None,
            job: None,
            ghost: Vec::new(),
            refused: None,
            restore: false,
            press: None,
            right_press: None,
            was_down: [false; 2],
            grabbed: None,
            grabbed_loose: None,
            over_list: false,
            floating: None,
            own: Vec::new(),
            snapped: None,
            brought: Vec::new(),
            carry: None,
            put_back: None,
            linking: false,
            cancel_link: false,
        }
    }
}

impl Placing {
    /// A preview of what is in hand is being built to go on show.
    pub fn is_building(&self) -> bool {
        self.job.is_some()
    }

    /// Whether the parts list is out (it shows unless a part is picked).
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Something is in hand, or the craft is being carried: the pointer is for that.
    pub fn is_holding(&self) -> bool {
        self.held.is_some() || self.carry.is_some() || self.linking
    }

    /// The palette entry a new part in hand is from, if one is.
    pub fn held_entry(&self) -> Option<&str> {
        match &self.held {
            Some(Held::New(entry)) => Some(entry),
            _ => None,
        }
    }

    /// Put back what is in hand, or the craft being carried, if anything is: whether it was.
    pub fn let_go(&mut self) -> bool {
        if let Some(carry) = self.carry.take() {
            self.put_back = Some(carry.from);
            return true;
        }
        let held = self.held.take().is_some();
        if held {
            self.spot = None;
            self.plan = None;
            self.asked = None;
            self.dropping = false;
            self.roll = 0.0;
            self.restore = true;
        }
        held
    }

    /// Where to put the pointer for what is in hand to go on node `name` of part `part`: so that
    /// its own node that faces most the other way is on the ring. The ring itself with nothing in
    /// hand, or none of its own facing the other way (it goes on by its side). For scripted
    /// captures.
    pub fn bring_to_node(&self, part: &str, name: &str) -> Option<Vec2> {
        let brought = (self.brought.iter()).find(|(p, n, _)| p == part && n == name);
        if let Some((.., pointer)) = brought {
            return Some(*pointer);
        }
        let free = (self.nodes.iter()).find(|n| n.part == part && n.name == name)?;
        free.screen
    }

    /// `parts` (what is in hand) built by themselves, as the renderer draws them.
    fn floating(
        &mut self,
        parts: &[CraftPartDef],
        catalog: &PartCatalog,
    ) -> Option<Arc<(Craft, PartTree)>> {
        if let Some((built, both)) = &self.floating
            && built == parts
        {
            return Some(both.clone());
        }
        let mut alone = parts.to_vec();
        let first = alone.first_mut()?;
        if first.id.is_empty() {
            first.id = "in_hand".into();
        }
        (first.on, first.at) = (None, None);
        let def = CraftDef {
            id: "in_hand".into(),
            name: String::new(),
            parts: alone,
            sequences: None,
            triggers: Vec::new(),
            rcs_shut: Vec::new(),
            abort: Vec::new(),
        };
        let craft = Craft::build(&def, catalog).ok()?;
        let tree = PartTree::from_craft(&craft, catalog).ok()?;
        let both = Arc::new((craft, tree));
        self.floating = Some((parts.to_vec(), both.clone()));
        Some(both)
    }

    fn take_in_hand(&mut self, held: Held, dropping: bool) {
        self.held = Some(held);
        self.dropping = dropping;
        self.spot = None;
        self.plan = None;
        self.asked = None;
        self.roll = 0.0;
        self.refused = None;
    }

    /// `part` built on its own, from the last few asked for.
    fn alone(&mut self, part: &CraftPartDef, catalog: &PartCatalog) -> Option<Arc<Craft>> {
        if let Some((_, craft)) = self.alone.iter().find(|(p, _)| p == part) {
            return Some(craft.clone());
        }
        let craft = Arc::new(builder::alone(part, catalog).ok()?);
        if self.alone.len() >= 8 {
            self.alone.remove(0);
        }
        self.alone.push((part.clone(), craft.clone()));
        Some(craft)
    }

    /// How the radial symmetry reads: `1`, `x4`.
    fn symmetry_label(&self) -> String {
        match self.symmetry {
            1 => "1".to_owned(),
            n => format!("x{n}"),
        }
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

/// The parts picked up with `id`: it, the parts put on with it by symmetry, and all they hold.
fn picked_up(draft: &CraftDef, id: &str) -> BTreeSet<String> {
    let mut ids = held_through(draft, id);
    for (other, _) in builder::alike(draft, id) {
        ids.extend(held_through(draft, &other));
    }
    ids
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
        // is taken too. A node of its own it is held by on its side is no place for anything else.
        let taken = |name: &str| {
            name == builder::SIDE
                || used
                    .iter()
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
fn by_node(alone: &Craft, used: &[String], way: DVec3) -> Option<String> {
    let own = &alone.assembly.parts()[0];
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

/// Everything about putting what is in hand at `spot` that does not depend on the pointer: the
/// craft as it stands (`draft`), without what is in hand (`built`), and how it is held.
struct Making<'a> {
    draft: &'a CraftDef,
    built: &'a Craft,
    catalog: &'a PartCatalog,
    entries: &'a [Entry],
    /// What the game may build: a new part is made no bigger, of no locked tank type.
    allowed: &'a Allowed,
    /// The groups of parts left about the hangar.
    loose: &'a [Loose],
    symmetry: u32,
    mirror: bool,
    roll: f64,
}

impl Making<'_> {
    /// The part in hand as it is to go on at `spot`, and how it goes on a surface.
    fn in_hand(
        &self,
        placing: &mut Placing,
        held: &Held,
    ) -> Result<(CraftPartDef, OnSurface), String> {
        match held {
            Held::New(id) => {
                let entry = (self.entries.iter())
                    .find(|e| &e.id == id)
                    .ok_or("no such part")?;
                let alone =
                    (placing.alone(&entry.part, self.catalog)).ok_or("it cannot be built")?;
                let how = match &entry.surface {
                    Some((by, _)) if builder::rolls(&alone) => OnSurface::Rolling(by.clone()),
                    Some((by, roll)) => OnSurface::By(by.clone(), *roll),
                    None if builder::lets_go(&alone) => OnSurface::By("top".into(), 0.0),
                    None => OnSurface::Side,
                };
                let mut part = entry.part.clone();
                part.id = new_id(self.draft, &entry.stem, 0);
                self.allowed.bring_within(&mut part, self.catalog);
                Ok((part, how))
            }
            Held::Moved(_) | Held::Loose(_) => {
                let part = match held {
                    Held::Moved(id) => (self.draft.parts.iter())
                        .find(|p| &p.id == id)
                        .ok_or("it is not on the craft")?
                        .clone(),
                    _ => (self.loose_group(held)?.into_iter().next()).ok_or("it is empty")?,
                };
                let alone = (placing.alone(&part, self.catalog)).ok_or("it cannot be built")?;
                let how = match () {
                    _ if builder::rolls(&alone) => OnSurface::Rolling(part.by.clone()),
                    _ if part.at.is_some() && part.by != builder::SIDE => {
                        OnSurface::By(part.by.clone(), part.roll)
                    }
                    _ if builder::lets_go(&alone) => OnSurface::By("top".into(), 0.0),
                    _ => OnSurface::Side,
                };
                Ok((part, how))
            }
        }
    }

    /// The loose group `held` is, named to go on the craft ([`builder::fit_in`]): the part first,
    /// then what it holds.
    fn loose_group(&self, held: &Held) -> Result<Vec<CraftPartDef>, String> {
        let Held::Loose(index) = held else {
            return Err("it is not loose".into());
        };
        let group = self.loose.get(*index).ok_or("it is not there any more")?;
        Ok(builder::fit_in(self.draft, &group.parts))
    }

    /// What putting `held` at `spot` does.
    fn plan(&self, placing: &mut Placing, held: &Held, spot: &Spot) -> Result<Plan, String> {
        let (mut part, how) = self.in_hand(placing, held)?;
        let new = matches!(held, Held::New(_));
        let holder_of = |id: &str| self.built.find(id).ok_or("it has nothing to go on");
        // Sized for where it goes, a new one: as wide as a node, or for the hull it stands on.
        // No wider than the game allows, though what it goes on is.
        // On an end of what it goes on, what would stand beside it goes on by its own node facing
        // that end, along the axis instead: engines under a tank, round it with symmetry.
        let end_by = match (spot, &how) {
            (Spot::Surface { end: Some(way), .. }, OnSurface::Side) => {
                let alone = placing
                    .alone(&part, self.catalog)
                    .ok_or("it cannot be built")?;
                by_node(&alone, &[], *way)
            }
            _ => None,
        };
        let limits = self.allowed.sizes;
        let side = match spot {
            Spot::Node {
                part: on,
                node,
                by,
                width,
            } => {
                if new {
                    // On an engine's mount, a tank sized to feed it. By its side on a node facing
                    // across (a booster on a radial decoupler), as wide as the node.
                    let holder = self.built.find(on).filter(|_| by.is_some());
                    let engine = holder.and_then(|h| fitting::feeding(self.built, h, node));
                    // Under an engine's nozzle: as wide as its stack, a decoupler an interstage.
                    let under = holder.and_then(|h| fitting::below_engine(self.built, h, node));
                    // Under the craft, a solid booster sized for what it lifts.
                    let lifts = holder.and_then(|h| {
                        fitting::lifted(&part, self.draft, self.built, h, node, self.catalog)
                    });
                    let against = Against {
                        engine,
                        under,
                        lifts,
                        limits,
                        ..Against::node(*width)
                    };
                    fitting::fit(&mut part, &against, self.catalog);
                    limits.shrink(&mut part);
                }
                by.is_none()
            }
            Spot::Surface { width, .. } => {
                if new {
                    let against = Against {
                        limits,
                        ..Against::surface(*width)
                    };
                    fitting::fit(&mut part, &against, self.catalog);
                    limits.shrink(&mut part);
                }
                how == OnSurface::Side && end_by.is_none()
            }
        };
        part.nodes.retain(|n| n.name != builder::SIDE);
        let side_node = if side {
            let alone = placing
                .alone(&part, self.catalog)
                .ok_or("it cannot be built")?;
            let node = builder::side_node(&alone);
            part.nodes.push(node.clone());
            Some(node)
        } else {
            None
        };
        let upright = |holder: usize, mount: &Mount| -> Result<f64, String> {
            let node = side_node.as_ref().ok_or("it has no side")?;
            builder::upright_roll(self.built, holder, mount, node, DVec3::Z)
                .ok_or_else(|| "it cannot stand upright there".to_owned())
        };
        // A wheel rolls along its own +Y: along the craft.
        let wheel_node = match &how {
            OnSurface::Rolling(by) => {
                let alone = placing
                    .alone(&part, self.catalog)
                    .ok_or("it cannot be built")?;
                let node = (alone.assembly.parts()[0].node(by))
                    .ok_or_else(|| format!("it has no node `{by}`"))?;
                Some(NodeDef {
                    name: node.name.clone(),
                    position: node.position.to_array(),
                    direction: node.direction.to_array(),
                    size: node.size,
                })
            }
            _ => None,
        };
        let rolling = |holder: usize, mount: &Mount| -> Result<f64, String> {
            let node = wheel_node.as_ref().ok_or("it has no wheel")?;
            builder::upright_roll(self.built, holder, mount, node, DVec3::Y)
                .ok_or_else(|| "it cannot roll that way there".to_owned())
        };
        let (place, holder) = match spot {
            Spot::Node {
                part: on, node, by, ..
            } => {
                let holder = holder_of(on)?;
                let (by, roll) = match by {
                    Some(by) => (by.clone(), 0.0),
                    None => (
                        builder::SIDE.to_owned(),
                        upright(holder, &Mount::Node(node.clone()))?,
                    ),
                };
                let mut place = Place::node(format!("{on}.{node}"), by);
                place.roll = roll;
                (place, holder)
            }
            Spot::Surface {
                part: on,
                surface,
                at,
                ..
            } => {
                let holder = holder_of(on)?;
                let mut at = *at;
                at.axial = end_by.is_some();
                let mount = Mount::Surface {
                    surface: surface.clone(),
                    side: at.side,
                    u: at.u,
                    v: at.v,
                    standoff: at.standoff,
                    axial: at.axial,
                };
                let (by, roll) = match (&end_by, &how) {
                    (Some(by), _) => (by.clone(), 0.0),
                    (None, OnSurface::By(by, roll)) if !side => (by.clone(), *roll),
                    (None, OnSurface::Rolling(by)) => (by.clone(), rolling(holder, &mount)?),
                    _ => (builder::SIDE.to_owned(), upright(holder, &mount)?),
                };
                let mut place = Place::surface(format!("{on}.{surface}"), by, at);
                place.roll = roll;
                (place, holder)
            }
        };
        let mut place = place;
        place.roll = wrapped(place.roll + self.roll);
        let places = self.symmetric(&part, place, holder);
        let (parts, replacing) = match held {
            Held::New(_) => (vec![part], BTreeSet::new()),
            Held::Loose(_) => {
                let mut group = self.loose_group(held)?;
                group[0] = part;
                (group, BTreeSet::new())
            }
            Held::Moved(id) => {
                let below = held_through(self.draft, id);
                let mut parts = vec![part];
                parts.extend(
                    (self.draft.parts.iter())
                        .filter(|p| &p.id != id && below.contains(&p.id))
                        .cloned(),
                );
                let mut replacing = BTreeSet::from([id.clone()]);
                replacing.extend(builder::alike(self.draft, id).into_iter().map(|(c, _)| c));
                (parts, replacing)
            }
        };
        Ok(Plan {
            parts,
            places,
            replacing,
            from_loose: match held {
                Held::Loose(index) => Some(*index),
                _ => None,
            },
        })
    }

    /// `part` put at `place` on the part at `holder` in the built craft, and the places of its
    /// copies by symmetry, each with whether it is a mirror image.
    fn symmetric(&self, part: &CraftPartDef, place: Place, holder: usize) -> Vec<(Place, bool)> {
        let Some((holder_id, connector)) = place.on.split_once('.') else {
            return vec![(place, false)];
        };
        // How the mirror image of `part` put at `place` is turned about its joint.
        let image_roll = |place: &Place| {
            let mut placed = part.clone();
            (placed.on, placed.by) = (Some(place.on.clone()), place.by.clone());
            (placed.at, placed.roll) = (place.at, place.roll);
            let on = place.on.split_once('.').map(|(on, _)| on);
            let holder = on.and_then(|on| self.draft.parts.iter().find(|p| p.id == on));
            builder::mirrored_roll(&placed, holder)
        };
        // On the part it goes on, and on each put on with that by symmetry.
        let mut holders = vec![(holder_id.to_owned(), false)];
        holders.extend(builder::alike(self.draft, holder_id));
        let mut places = Vec::new();
        for (on, mirrored) in holders {
            let mut here = place.clone();
            here.on = format!("{on}.{connector}");
            if mirrored {
                here.roll = image_roll(&here);
                if let Some(at) = &mut here.at {
                    at.u = (-at.u).rem_euclid(1.0);
                }
            }
            let Some(at) = here.at else {
                places.push((here, mirrored));
                continue;
            };
            let index = if on == holder_id {
                Some(holder)
            } else {
                self.built.find(&on)
            };
            let surface = connector;
            places.push((here.clone(), mirrored));
            if self.mirror {
                let across = index.and_then(|i| builder::mirrored_u(self.built, i, surface, at));
                if let Some(u) = across {
                    let mut image = here.clone();
                    image.roll = image_roll(&here);
                    image.at = Some(SurfaceAtDef { u, ..at });
                    places.push((image, !mirrored));
                }
            } else {
                for k in 1..self.symmetry {
                    let mut copy = here.clone();
                    let turn = f64::from(k) / f64::from(self.symmetry);
                    copy.at = Some(SurfaceAtDef {
                        u: ((at.u + turn).rem_euclid(1.0) * 1e9).round() / 1e9,
                        ..at
                    });
                    places.push((copy, mirrored));
                }
            }
        }
        places
    }
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
    (parts, parents, payloads, loose_drawn, rails): (
        Query<&ShowsPart>,
        Query<&ChildOf>,
        Query<&DragPayload, With<GlobalZIndex>>,
        Query<&ShowsLoose>,
        Query<(
            &Name,
            &ComputedNode,
            &UiGlobalTransform,
            &InheritedVisibility,
        )>,
    ),
    mut ray_cast: MeshRayCast,
    mut marks: ResMut<PartMarks>,
    mut actions: MessageReader<UiAction>,
    mut stores: Local<Option<(Arc<PartCatalog>, Arc<Vec<Entry>>)>>,
    (moving, mut commands, hand): (Res<Moving>, Commands, Res<PartInHand>),
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
            symmetry: placing.symmetry,
            mirror: placing.mirror,
            ..default()
        };
        marks.set_ghost(None);
        return;
    }

    for action in actions.read() {
        match action.name() {
            // A craft with changes is not thrown away without asking (hangar).
            "editor.new" if editor.asks_first(action, &mut commands) => {}
            "editor.new" => {
                editor.start_new(action.arg() == Some("plane"), &mut sim, &mut ui);
                shaping.pick(None);
                placing.held = None;
                placing.carry = None;
                placing.open = true;
                // A plane's wings and gear go on in mirror pairs.
                placing.mirror = action.arg() == Some("plane");
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
                placing.cancel_link = placing.linking;
            }
            "editor.symmetry" => {
                if placing.mirror {
                    placing.mirror = false;
                } else {
                    placing.symmetry = next_symmetry(placing.symmetry, true);
                }
            }
            "editor.symmetry_mode" => placing.mirror = !placing.mirror,
            "editor.remove" => remove_picked(&mut shaping, &mut editor, catalog, &mut sim, &mut ui),
            _ => {}
        }
    }
    let Some(draft) = editor.builder().map(|b| b.draft().clone()) else {
        placing.held = None;
        placing.carry = None;
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
        let command = keys.any_pressed([
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
        ]);
        if keys.just_pressed(KeyCode::KeyX) && !command {
            placing.mirror = false;
            placing.symmetry = next_symmetry(placing.symmetry, !shift);
        }
        if keys.just_pressed(KeyCode::KeyR) && !command {
            placing.mirror = !placing.mirror;
        }
        if !command && keys.any_just_pressed([KeyCode::Delete, KeyCode::Backspace]) {
            remove_picked(&mut shaping, &mut editor, catalog, &mut sim, &mut ui);
        }
        // Q and E turn what is in hand, or the part picked, a turn step at a time.
        let turn = moving.angle_step(shift);
        let turned = f64::from(keys.just_pressed(KeyCode::KeyE))
            - f64::from(keys.just_pressed(KeyCode::KeyQ));
        if turned != 0.0 && !command {
            if placing.held.is_some() {
                placing.roll = wrapped(placing.roll + turned * turn);
            } else if placing.carry.is_none() {
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

    // Over the parts list what is in hand is not over the view: let go there, it is thrown away.
    placing.over_list = pointer.anywhere().is_some_and(|at| {
        rails.iter().any(|(name, node, place, visible)| {
            let scale = node.inverse_scale_factor();
            let half = node.size() * scale / 2.0;
            name.as_str() == PARTS_RAIL
                && visible.get()
                && (at - place.translation * scale).abs().cmple(half).all()
        })
    });
    let cursor = pointer.position().filter(|_| !placing.over_list);
    let (camera, eye) = *camera;
    let vessel = editor.vessel();
    let loose: Vec<Loose> = (editor.builder()).map_or_else(Vec::new, |b| b.loose().to_vec());
    // The loose group a press lands on, and how far along the pointer's ray: one in front of the
    // craft is taken before it.
    let loose_hit = (went_down[0] && placing.held.is_none() && !loose.is_empty())
        .then(|| {
            let ray = camera.viewport_to_world(eye, cursor?).ok()?;
            let tagged = |e: Entity| {
                std::iter::once(e)
                    .chain(parents.iter_ancestors(e))
                    .find_map(|a| loose_drawn.get(a).ok())
                    .copied()
            };
            let filter = |e: Entity| tagged(e).is_some();
            let settings = MeshRayCastSettings::default().with_filter(&filter);
            let (mesh, hit) = ray_cast.cast_ray(ray, &settings).first()?;
            Some((tagged(*mesh)?.0, hit.distance))
        })
        .flatten();
    let vessel_frame = vessel.and_then(|v| VesselFrame::of(&sim, &frame, v));
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
    // The pointer's ray from the middle of the floor, sim axes, with the root at `anchor`.
    let ray_on_floor = |anchor: DVec3| -> Option<(DVec3, DVec3)> {
        let frame = vessel_frame?;
        let ray = camera.viewport_to_world(eye, cursor?).ok()?;
        let from = frame.sim_way(frame.vessel_point(ray.origin)) + anchor;
        let way = frame.sim_way(frame.vessel_way(*ray.direction)).normalize();
        Some((from, way))
    };

    // The whole craft carried by its root follows the pointer until it is let go, or goes back.
    if let Some(at) = placing.put_back.take() {
        editor.carry_to(at);
    }
    if let Some(carry) = placing.carry {
        if !down[0] {
            placing.carry = None;
        } else if let Some((from, way)) = editor.anchor().and_then(ray_on_floor) {
            let across = way.dot(carry.look);
            if across.abs() > 1e-6 {
                let hit = from + way * (carry.grabbed - from).dot(carry.look) / across;
                editor.carry_to(carry.from + hit - carry.grabbed);
            }
        }
        editor.stop_view_drag();
        marks.set_ghost(None);
        return;
    }

    // A fuel line just put on is told what it feeds by a click on that part; a right click or Esc
    // takes it off.
    let unlinked = unlinked_fuel_lines(&draft);
    placing.linking = placing.held.is_none() && !unlinked.is_empty();
    if placing.linking {
        if went_down[0] {
            placing.press = cursor;
        }
        if went_down[1] {
            placing.right_press = cursor;
        }
        let near = |from: Option<Vec2>| {
            from.zip(cursor)
                .is_some_and(|(from, at)| from.distance(at) < CLICK_SLOP)
        };
        let clicked = went_up[0] && near(placing.press.take());
        let right_clicked = went_up[1] && near(placing.right_press.take());
        if std::mem::take(&mut placing.cancel_link) || right_clicked {
            let lines: BTreeSet<String> = unlinked.iter().cloned().collect();
            if let Some(builder) = editor.builder_mut() {
                let _ = builder.remove_all(catalog, &lines);
            }
            redraw_draft(&mut editor, catalog, &mut sim, &mut ui);
            editor.settle(&mut sim);
            placing.linking = false;
        } else if clicked
            && let Some((target, _)) = under(&|id| unlinked.iter().any(|line| line == id))
        {
            let tree = vessel
                .and_then(|v| sim.universe.vessel(v))
                .map(|v| v.propulsion.parts());
            let position = |id: &str| Some(tree?.parts().get(draft_index(id)?)?.position);
            let links = fuel_line_links(&draft, &unlinked, &target, position);
            let linked = (editor.builder_mut())
                .ok_or_else(|| "there is no craft to build".to_owned())
                .and_then(|b| {
                    b.link_fuel_lines(catalog, &links)
                        .map_err(|r| r.to_string())
                });
            match linked {
                Ok(_) => {
                    redraw_draft(&mut editor, catalog, &mut sim, &mut ui);
                    editor.settle(&mut sim);
                    editor.set_notice("");
                    placing.linking = false;
                }
                Err(e) => editor.set_notice(format!("Cannot feed that: {e}")),
            }
        }
        marks.set_ghost(None);
        return;
    }
    placing.cancel_link = false;

    // A drag that starts on a part of the craft picks it up with what it holds, or, on the root,
    // carries the whole craft; one that starts on nothing turns the view.
    if placing.held.is_none() && !draft.parts.is_empty() {
        if went_down[0] && !shaping.holds_pointer() {
            let part = under(&|_| false);
            let eye_at = eye.translation();
            let nearer_loose = loose_hit.filter(|(_, along)| {
                part.as_ref()
                    .is_none_or(|(_, point)| *along < point.distance(eye_at))
            });
            match nearer_loose {
                Some((index, _)) => {
                    placing.grabbed = None;
                    placing.grabbed_loose = cursor.map(|at| (index, at));
                }
                None => {
                    placing.grabbed = part.map(|(id, _)| id).zip(cursor);
                    placing.grabbed_loose = None;
                }
            }
        }
        // A loose group pressed on: dragged, it goes where it is let go; clicked, where the next
        // click is.
        if let (Some((index, from)), Some(at)) = (placing.grabbed_loose, cursor) {
            let dragged = from.distance(at) >= CLICK_SLOP;
            if dragged || went_up[0] {
                placing.grabbed_loose = None;
                placing.take_in_hand(Held::Loose(index), dragged);
                shaping.pick(None);
                editor.stop_view_drag();
                editor.set_notice("");
                return;
            }
        }
        if !down[0] {
            placing.grabbed = None;
            placing.grabbed_loose = None;
        }
        if let (Some((id, from)), Some(at)) = (placing.grabbed.clone(), cursor)
            && from.distance(at) >= CLICK_SLOP
        {
            placing.grabbed = None;
            let root = draft.parts.iter().any(|p| p.id == id && p.on.is_none());
            if root {
                // Taken hold of where the press was, in the plane square to the view.
                let anchor = editor.anchor();
                let start = (anchor.and_then(ray_on_floor), anchor);
                if let (Some((origin, way)), Some(anchor)) = start {
                    let middle = vessel.and_then(|_| {
                        let frame = vessel_frame?;
                        let on_show = sim.universe.vessel(vessel?)?;
                        let tree = on_show.propulsion.parts();
                        let part = tree.parts().get(draft_index(&id)?)?;
                        let at = part.position - tree_origin(tree);
                        Some(frame.sim_way(at) + anchor)
                    });
                    let depth = middle.map_or(10.0, |m| (m - origin).dot(way));
                    placing.carry = Some(Carry {
                        from: anchor,
                        grabbed: origin + way * depth,
                        look: way,
                    });
                }
            } else {
                // Its copies by symmetry come with it: the symmetry is theirs again while it is
                // in hand.
                let copies = builder::alike(&draft, &id);
                let same_holder = |other: &str| {
                    let holder = |p: &str| {
                        (draft.parts.iter())
                            .find(|q| q.id == p)
                            .and_then(|q| q.on.clone())
                    };
                    holder(other) == holder(&id)
                };
                let beside: Vec<&(String, bool)> =
                    copies.iter().filter(|(c, _)| same_holder(c)).collect();
                if beside.iter().any(|(_, mirrored)| *mirrored) {
                    placing.mirror = true;
                } else if !beside.is_empty() {
                    placing.mirror = false;
                    placing.symmetry = beside.len() as u32 + 1;
                } else {
                    placing.mirror = false;
                    placing.symmetry = 1;
                }
                placing.take_in_hand(Held::Moved(id), true);
                shaping.pick(None);
            }
            editor.stop_view_drag();
            editor.set_notice("");
        }
    }

    // The first part of an empty craft goes on as its root: straight away from a click on its
    // row, or where it is let go in the view.
    if let Some(held @ Held::New(_)) = placing.held.clone()
        && draft.parts.is_empty()
    {
        let now = !placing.dropping || (went_up[0] && cursor.is_some());
        if now {
            match put_root(&mut editor, catalog, &held, entries) {
                Ok(()) => {
                    redraw_draft(&mut editor, catalog, &mut sim, &mut ui);
                    editor.frame_first_part(&mut sim);
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
        Some(Held::Moved(id)) => picked_up(&draft, id),
        _ => BTreeSet::new(),
    };
    let stale = (placing.built.as_ref())
        .is_none_or(|(parts, held, _)| *parts != draft.parts || *held != placing.held);
    if stale {
        let mut rest = draft.clone();
        rest.parts.retain(|p| !moved.contains(&p.id));
        placing.built = Craft::build(&rest, catalog)
            .ok()
            .map(|craft| (draft.parts.clone(), placing.held.clone(), Arc::new(craft)));
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
    let Some(vessel_frame) = vessel_frame else {
        return;
    };
    // Where the free nodes are on show.
    if let Some(on_show) = sim.universe.vessel(vessel) {
        let tree = on_show.propulsion.parts();
        let base = tree_origin(tree);
        for node in &mut placing.nodes {
            let shown = draft_index(&node.part).and_then(|i| tree.parts().get(i));
            if let Some(part) = shown {
                node.at = part.position - base + part.rotation * node.local;
            }
        }
    }

    // How what is in hand goes on: by its side at a node facing across the craft, if it goes on
    // surfaces that way.
    let Some(built) = placing.built.as_ref().map(|(_, _, craft)| craft.clone()) else {
        return;
    };
    let allowed = (editor.builder()).map_or_else(Allowed::everything, |b| b.allowed().clone());
    let making = Making {
        draft: &draft,
        built: &built,
        catalog,
        entries,
        allowed: &allowed,
        loose: &loose,
        symmetry: placing.symmetry,
        mirror: placing.mirror,
        roll: placing.roll,
    };
    let (in_hand, how) = match making.in_hand(placing, &held) {
        Ok(found) => found,
        Err(e) => {
            editor.set_notice(format!("Cannot hold it: {e}"));
            placing.held = None;
            return;
        }
    };
    // Its own nodes that hold something already cannot hold it on.
    let group = match &held {
        Held::Loose(_) => making.loose_group(&held).unwrap_or_default(),
        _ => Vec::new(),
    };
    let used: Vec<String> = (draft.parts.iter().chain(&group))
        .filter_map(|p| p.on.as_deref()?.split_once('.'))
        .filter(|(holder, _)| *holder == in_hand.id)
        .map(|(_, node)| node.to_owned())
        .collect();

    // Where its own free nodes are on the screen, as it is drawn at the pointer; or as it would be
    // in the middle of the view, while the pointer is off it.
    let drawn =
        (editor.builder()).map_or_else(Vec::new, |b| held_parts(&held, b, entries, catalog));
    let reference = cursor.or_else(|| camera.logical_viewport_size().map(|size| size / 2.0));
    let no_craft = draft.parts.is_empty();
    placing.own.clear();
    placing.brought.clear();
    if let (Some(at), Some(middle)) = (reference, hand.middle())
        && let Some(both) = placing.floating(&drawn, catalog)
    {
        let own_at = |pointer: Vec2| -> Vec<OwnNode> {
            at_pointer(pointer, camera, eye, vessel_frame, no_craft)
                .map_or_else(Vec::new, |place| {
                    own_nodes(&both, &used, middle, place, pointer, (camera, eye))
                })
        };
        placing.own = own_at(at);
        // For scripted captures, where the pointer brings the own node that goes on each free node
        // onto its ring: moved until it does, as how far a node is from the pointer on the screen
        // changes as the pointer moves.
        let mut brought = Vec::new();
        for free in &placing.nodes {
            let way = free.way.normalize_or_zero();
            let own = (placing.own.iter())
                .filter(|o| o.way.dot(way) < -FACING)
                .min_by(|a, b| a.way.dot(way).total_cmp(&b.way.dot(way)));
            let (Some(ring), Some(own)) = (free.screen, own) else {
                continue;
            };
            let mut pointer = ring - own.offset;
            for _ in 0..4 {
                let Some(now) = own_at(pointer).into_iter().find(|o| o.name == own.name) else {
                    break;
                };
                pointer += ring - (pointer + now.offset);
            }
            brought.push((free.part.clone(), free.name.clone(), pointer));
        }
        placing.brought = brought;
    }

    // Where it would go. On a free node one of its own nodes comes near, facing the other way, as
    // it is drawn at the pointer; it stays there a little further than it came on, so that it does
    // not flicker at the edge.
    let snap = cursor.and_then(|at| {
        let mut best: Option<(&OwnNode, &FreeNode, f32)> = None;
        for own in &placing.own {
            for free in &placing.nodes {
                let Some(ring) = free.screen else {
                    continue;
                };
                if own.way.dot(free.way.normalize_or_zero()) >= -FACING {
                    continue;
                }
                let held_on = (placing.snapped.as_ref())
                    .is_some_and(|(o, p, n)| *o == own.name && *p == free.part && *n == free.name);
                let reach = if held_on {
                    NODE_SNAP_PX * SNAP_HOLD
                } else {
                    NODE_SNAP_PX
                };
                let off = ring.distance(at + own.offset);
                if off <= reach && best.is_none_or(|(.., b)| off < b) {
                    best = Some((own, free, off));
                }
            }
        }
        best.map(|(own, free, _)| (own.name.clone(), free.clone()))
    });
    placing.snapped =
        (snap.as_ref()).map(|(own, free)| (own.clone(), free.part.clone(), free.name.clone()));
    // By the pointer on the ring: what goes on by its side (it has no node of its own to bring),
    // and anything not drawn yet.
    let near = cursor.filter(|_| snap.is_none()).and_then(|at| {
        (placing.nodes.iter())
            .filter(|n| {
                let across = n.way.normalize_or_zero().z.abs() < ACROSS;
                placing.own.is_empty() || (across && how == OnSurface::Side)
            })
            .filter_map(|n| Some((n, n.screen?.distance(at))))
            .filter(|(_, d)| *d <= NODE_REACH_PX)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(n, _)| n.clone())
    });
    let spot = if let Some((own, node)) = snap {
        Some(Spot::Node {
            part: node.part.clone(),
            node: node.name.clone(),
            by: Some(own),
            width: node.size,
        })
    } else if let Some(node) = near {
        let across = node.way.normalize_or_zero().z.abs() < ACROSS;
        if across && how == OnSurface::Side {
            Some(Spot::Node {
                part: node.part.clone(),
                node: node.name.clone(),
                by: None,
                width: node.size,
            })
        } else {
            let alone = placing.alone(&in_hand, catalog);
            alone
                .and_then(|alone| by_node(&alone, &used, node.way))
                .map(|by| Spot::Node {
                    part: node.part.clone(),
                    node: node.name.clone(),
                    by: Some(by),
                    width: node.size,
                })
        }
    } else {
        under(&|id| moved.contains(id)).and_then(|(part_id, point)| {
            let on_show = sim.universe.vessel(vessel)?;
            let tree = on_show.propulsion.parts();
            let shown = tree.parts().get(draft_index(&part_id)?)?;
            let base = tree_origin(tree);
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
            // Its outside facing along its axis: one of its ends.
            let end = (side == Side::Front && point.normal.z.abs() >= END_FACING)
                .then(|| shown.rotation * DVec3::Z * point.normal.z.signum());
            Some(Spot::Surface {
                part: part_id,
                surface,
                at: SurfaceAtDef {
                    u,
                    v,
                    side,
                    standoff: 0.0,
                    axial: false,
                },
                // Across the hull there, from its shape: not from where the pointer met its facets.
                width: 2.0 * point.position.truncate().length(),
                end,
            })
        })
    };
    // Worked out again only when where it would go, or how, changes. A spot on a surface the
    // pointer has not moved off is the one already asked about: the craft redrawn with what is in
    // hand on it puts the surface a rounding error away from where it was, and each new answer
    // would be drawn and asked about again, for ever.
    let spot = match (spot, &placing.asked) {
        (Some(now), Some((was, ..))) if now.barely_moved_from(was) => Some(was.clone()),
        (spot, _) => spot,
    };
    let asked = spot
        .clone()
        .map(|s| (s, placing.symmetry, placing.mirror, placing.roll));
    if asked != placing.asked || stale {
        placing.asked = asked;
        placing.plan = None;
        match &spot {
            Some(spot) => match making.plan(placing, &held, spot) {
                Ok(plan) => placing.plan = Some(plan),
                Err(e) => placing.refused = Some(e),
            },
            None => {
                placing.refused = None;
                // Nowhere on the craft, over the parts list or out in the hangar: a part taken
                // from the craft comes off it, with its copies by symmetry and all they hold.
                if let Held::Moved(id) = &held {
                    let mut replacing = BTreeSet::from([id.clone()]);
                    replacing.extend(builder::alike(&draft, id).into_iter().map(|(c, _)| c));
                    placing.plan = Some(Plan::taking_off(replacing));
                }
            }
        }
    }
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
    // Let go off the craft, a part from it or a loose one: over the parts list it is thrown
    // away, out in the hangar it is left there, loose; over the other panels it goes back.
    if put_now && placing.spot.is_none() && matches!(held, Held::Moved(_) | Held::Loose(_)) {
        let at = cursor.and_then(|cursor| {
            let (point, _) = at_pointer(cursor, camera, eye, vessel_frame, false)?;
            Some(vessel_frame.vessel_point(point))
        });
        let replacing =
            (placing.plan.as_ref()).map_or_else(BTreeSet::new, |plan| plan.replacing.clone());
        let Some(builder) = editor.builder_mut() else {
            return;
        };
        let done = match (&held, placing.over_list, at) {
            (Held::Moved(_), true, _) => builder.remove_all(catalog, &replacing).map(|_| ()),
            (Held::Moved(id), false, Some(at)) => {
                builder.detach(catalog, &replacing, id, at).map(|_| ())
            }
            (Held::Loose(index), true, _) => builder.throw_loose(*index),
            (Held::Loose(index), false, Some(at)) => builder.move_loose(*index, at),
            _ => {
                placing.let_go();
                return;
            }
        };
        match done {
            Ok(()) => {
                if matches!(held, Held::Moved(_)) {
                    let matched = placing.shown.as_ref() == placing.plan.as_ref();
                    if !matched {
                        redraw_draft(&mut editor, catalog, &mut sim, &mut ui);
                    }
                    editor.settle(&mut sim);
                }
                editor.set_notice("");
                *placing = Placing {
                    open: placing.open,
                    symmetry: placing.symmetry,
                    mirror: placing.mirror,
                    was_down: placing.was_down,
                    alone: std::mem::take(&mut placing.alone),
                    ..default()
                };
                marks.set_ghost(None);
            }
            Err(e) => {
                editor.set_notice(format!("Cannot do that: {e}"));
                placing.let_go();
            }
        }
        return;
    }
    if put_now {
        let Some(plan) = placing.plan.clone() else {
            // Let go where it cannot go: it goes back.
            if placing.dropping {
                placing.let_go();
            }
            return;
        };
        let put = editor
            .builder_mut()
            .ok_or_else(|| "there is no craft to build".to_owned())
            .and_then(|builder| {
                if plan.takes_off() {
                    builder
                        .remove_all(catalog, &plan.replacing)
                        .map(|change| (change, Vec::new()))
                } else if let Some(index) = plan.from_loose {
                    builder.put_loose(catalog, index, &plan.parts, &plan.places)
                } else {
                    builder.put_set(catalog, &plan.parts, &plan.places, &plan.replacing)
                }
                .map_err(|r| r.to_string())
            });
        match put {
            Ok(_) => {
                // What is on show is already this, if the preview was of it and the builder did
                // nothing more (a tank made bigger for an engine put under it is not previewed).
                let same = |a: &CraftDef, b: &CraftDef| {
                    serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
                };
                let draft = editor.builder().map(|b| b.draft());
                let matched = placing.shown.as_ref() == Some(&plan)
                    && placing.refused.is_none()
                    && (placing.shown_craft.as_ref().zip(draft)).is_some_and(|(a, b)| same(a, b));
                if !matched {
                    redraw_draft(&mut editor, catalog, &mut sim, &mut ui);
                }
                editor.settle(&mut sim);
                editor.set_notice("");
                *placing = Placing {
                    open: placing.open,
                    symmetry: placing.symmetry,
                    mirror: placing.mirror,
                    was_down: placing.was_down,
                    alone: std::mem::take(&mut placing.alone),
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
    let wanted = placing.plan.clone();
    preview(
        placing,
        &mut editor,
        &draft,
        shared,
        wanted,
        &mut sim,
        &mut ui,
    );
    let ghost = (placing.shown_craft.as_ref())
        .filter(|_| placing.shown.is_some())
        .map(|craft| {
            let shown = editor
                .vessel()
                .and_then(|v| sim.universe.vessel(v))
                .map_or(0, |v| v.propulsion.parts().parts().len());
            (placing.ghost.iter())
                .filter_map(|id| craft.parts.iter().position(|p| &p.id == id))
                .filter(|&i| i < shown)
                .map(|i| PartId(i as u32))
                .collect::<Vec<_>>()
        })
        .filter(|ghost| !ghost.is_empty())
        .map(|ghost| {
            (
                editor.vessel().unwrap_or(vessel),
                ghost,
                placing.refused.is_some(),
            )
        });
    marks.set_ghost(ghost);
}

/// The fuel lines on `draft` not yet told what they feed.
fn unlinked_fuel_lines(draft: &CraftDef) -> Vec<String> {
    (draft.parts.iter())
        .filter(|p| {
            (p.components.iter())
                .any(|c| matches!(c, ComponentDef::FuelLine(line) if line.to.is_empty()))
        })
        .map(|p| p.id.clone())
        .collect()
}

/// What each of `lines` feeds when the player clicked `clicked`: copies by symmetry each feed the
/// copy of that part nearest them, so pairs of boosters can feed the next pair in.
fn fuel_line_links(
    draft: &CraftDef,
    lines: &[String],
    clicked: &str,
    position: impl Fn(&str) -> Option<DVec3>,
) -> Vec<(String, String)> {
    let mut choices = vec![clicked.to_owned()];
    choices.extend(builder::alike(draft, clicked).into_iter().map(|(id, _)| id));
    (lines.iter())
        .map(|line| {
            let away = |choice: &String| {
                let (a, b) = (position(line), position(choice));
                a.zip(b).map_or(f64::INFINITY, |(a, b)| a.distance(b))
            };
            let nearest = if lines.len() == 1 {
                None
            } else {
                choices.iter().min_by(|a, b| away(a).total_cmp(&away(b)))
            };
            (
                line.clone(),
                nearest.map_or(clicked, |c| c.as_str()).to_owned(),
            )
        })
        .collect()
}

/// What is in hand as it is drawn at the pointer: the part first, then what it holds. A new part as
/// it will go on, no bigger than the game allows.
fn held_parts(
    held: &Held,
    builder: &builder::Builder,
    entries: &[Entry],
    catalog: &PartCatalog,
) -> Vec<CraftPartDef> {
    match held {
        Held::New(id) => (entries.iter())
            .find(|e| &e.id == id)
            .map(|e| {
                let mut part = e.part.clone();
                builder.allowed().bring_within(&mut part, catalog);
                vec![part]
            })
            .unwrap_or_default(),
        Held::Moved(id) => {
            let draft = builder.draft();
            let held = held_through(draft, id);
            let mut parts: Vec<_> = (draft.parts.iter())
                .filter(|p| held.contains(&p.id))
                .cloned()
                .collect();
            parts.sort_by_key(|p| p.id != *id);
            parts
        }
        Held::Loose(index) => {
            (builder.loose().get(*index)).map_or_else(Vec::new, |group| group.parts.clone())
        }
    }
}

/// The free nodes of the part in hand (`floating`, built by itself: its craft and its tree), those
/// not `used` nor stood in for, on the screen as it is drawn with its `middle` (the renderer's) at
/// `place`, the pointer at `at`.
fn own_nodes(
    floating: &(Craft, PartTree),
    used: &[String],
    middle: Vec3,
    (point, rotation): (Vec3, Quat),
    at: Vec2,
    (camera, eye): (&Camera, &GlobalTransform),
) -> Vec<OwnNode> {
    let (craft, tree) = floating;
    let (Some(top), Some(own)) = (tree.parts().first(), craft.assembly.parts().first()) else {
        return Vec::new();
    };
    // As the renderer draws it: the tree from its base, its middle at the point.
    let base = DVec3::Z * tree.base();
    let root = point - rotation * middle;
    own.nodes()
        .filter(|n| !used.contains(&n.name) && builder::stands_for(&n.name) == n.name)
        .filter_map(|n| {
            let local = to_render(top.position - base + top.rotation * n.position);
            let screen = camera
                .world_to_viewport(eye, root + rotation * local)
                .ok()?;
            Some(OwnNode {
                name: n.name.clone(),
                offset: screen - at,
                way: top.rotation * n.direction,
            })
        })
        .collect()
}

/// Where what is in hand is drawn in the view while it has nowhere to go: under the pointer, as far
/// from the eye as the craft, in the plane square to the view through its root; with no craft yet,
/// near enough to see well.
fn at_pointer(
    pointer: Vec2,
    camera: &Camera,
    eye: &GlobalTransform,
    frame: VesselFrame,
    no_craft: bool,
) -> Option<(Vec3, Quat)> {
    let ray = camera.viewport_to_world(eye, pointer).ok()?;
    let forward = *eye.forward();
    let depth = if no_craft {
        NO_CRAFT_DEPTH
    } else {
        (frame.view_point(DVec3::ZERO) - ray.origin).dot(forward)
    };
    let along = ray.direction.dot(forward);
    (depth > MIN_DEPTH && along > 1e-3).then(|| {
        (
            ray.origin + *ray.direction * (depth / along),
            frame.view_rotation(),
        )
    })
}

/// Draw what is in hand at the pointer while it is not shown on the craft, and put away the list's
/// tile while the pointer is over the hangar, where the part takes its place.
#[allow(clippy::too_many_arguments)]
fn show_in_hand(
    placing: Res<Placing>,
    editor: Res<Editor>,
    content: Res<GameContent>,
    (sim, frame): (Res<Sim>, Res<RenderFrame>),
    pointer: ViewPointer,
    camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    (mut hand, mut loose): (ResMut<PartInHand>, ResMut<LooseParts>),
    mut data: ResMut<UiData>,
    mut tiles: Query<(Entity, &mut Visibility, Has<GlobalZIndex>), With<DragPayload>>,
    mut hidden: Local<Option<Entity>>,
    mut entries: Local<Option<(Vec<Entry>, PartCatalog)>>,
) {
    if entries.is_none() || content.is_changed() {
        let parts = catalog(&content.0.registry, &mut Vec::new());
        *entries = Some((palette::entries(&parts), parts));
    }
    let draft = editor.builder().map(|b| b.draft());
    let parts = match (&placing.held, editor.builder(), entries.as_ref()) {
        (Some(held), Some(builder), Some((entries, catalog))) => {
            held_parts(held, builder, entries, catalog)
        }
        _ => Vec::new(),
    };
    // What is left about the hangar, see-through where it was left: not what is in hand.
    let frame_now = (editor.vessel()).and_then(|v| VesselFrame::of(&sim, &frame, v));
    let groups = (editor.builder().filter(|_| editor.is_open()))
        .map_or(&[][..], |b| b.loose())
        .iter()
        .enumerate()
        .map(|(index, group)| {
            let in_hand = placing.held == Some(Held::Loose(index));
            let place = (frame_now.filter(|_| !in_hand))
                .map(|f| (f.view_point(group.at), f.view_rotation()));
            (group.parts.clone(), place)
        })
        .collect();
    loose.set(groups);
    let on_craft = placing.spot.is_some() && placing.shown.is_some() && !placing.ghost.is_empty();
    let over_hangar = pointer.position().filter(|_| !parts.is_empty());
    let (camera, eye) = *camera;
    let place = over_hangar.filter(|_| !on_craft).and_then(|at| {
        let vessel = VesselFrame::of(&sim, &frame, editor.vessel()?)?;
        at_pointer(
            at,
            camera,
            eye,
            vessel,
            draft.is_none_or(|d| d.parts.is_empty()),
        )
    });
    data.set("editor.floating", place.is_some());
    hand.set(parts, place);

    let from_list = placing.dropping && matches!(placing.held, Some(Held::New(_)));
    for (tile, mut visibility, dragged) in &mut tiles {
        if dragged && from_list && over_hangar.is_some() {
            visibility.set_if_neq(Visibility::Hidden);
            *hidden = Some(tile);
        } else if *hidden == Some(tile) {
            visibility.set_if_neq(Visibility::Inherited);
            *hidden = None;
        }
    }
}

/// Degrees in (-180, 180].
fn wrapped(degrees: f64) -> f64 {
    let turned = (degrees + 180.0).rem_euclid(360.0) - 180.0;
    if turned == -180.0 { 180.0 } else { turned }
}

/// Put the first part of an empty craft on as its root.
fn put_root(
    editor: &mut Editor,
    catalog: &PartCatalog,
    held: &Held,
    entries: &[Entry],
) -> Result<(), String> {
    let Some(builder) = editor.builder_mut() else {
        return Err("there is no craft to build".into());
    };
    let Held::New(entry) = held else {
        return Err("it is on the craft already".into());
    };
    let entry = (entries.iter())
        .find(|e| &e.id == entry)
        .ok_or("no such part")?;
    let mut part = entry.part.clone();
    part.id = new_id(builder.draft(), &entry.stem, 0);
    builder.allowed().bring_within(&mut part, catalog);
    builder
        .place(catalog, part, None)
        .map(|_| ())
        .map_err(|r| r.to_string())
}

/// Turn the picked part `degrees` about the joint that holds it; the root turns the whole craft
/// about its axis.
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
    let Some(part) = builder.draft().parts.iter().find(|p| p.id == id) else {
        return;
    };
    let turned = if part.on.is_some() {
        builder.rotate(catalog, &id, degrees)
    } else {
        let turn = DQuat::from_rotation_z(degrees.to_radians());
        builder.turn_root(catalog, turn)
    };
    match turned {
        Ok(_) => {
            redraw_draft(editor, catalog, sim, ui);
            editor.settle(sim);
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
    draft: &CraftDef,
    catalog: &Arc<PartCatalog>,
    wanted: Option<Plan>,
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
            Ok((craft, propulsion, ghost, refused)) => {
                editor.redraw_built(&craft, propulsion, sim, ui);
                placing.ghost = ghost;
                placing.refused = refused;
                placing.shown_craft = Some(craft);
            }
            Err(e) => {
                warn!("cannot show it there: {e}");
                placing.ghost.clear();
                placing.refused = Some(e);
                placing.shown_craft = None;
            }
        }
        placing.shown = job.wanted;
    }
    if wanted == placing.shown || placing.job.is_some() {
        return;
    }
    let Some(plan) = wanted.clone() else {
        // Nowhere to go: the draft as it stands, with nothing in hand on it.
        if placing.shown.is_some() {
            redraw_draft(editor, catalog, sim, ui);
            placing.shown = None;
            placing.shown_craft = None;
            placing.ghost.clear();
        }
        return;
    };
    let (draft, catalog) = (draft.clone(), catalog.clone());
    let task = std::thread::spawn(move || {
        let (mut craft, ghost) = plan.put(&draft)?;
        // As the builder will have it: new tanks take what their engines burn. A tank made bigger
        // for an engine put under it is not shown until it is let go: grown here, it would move the
        // ring the engine is held to.
        properties::fill_from_engines(&mut craft, &catalog, &ghost);
        let propulsion = Propulsion::from_craft(&craft, &catalog).map_err(|e| e.to_string())?;
        let touched: Vec<&str> = ghost.iter().map(String::as_str).collect();
        let refused = (!touched.is_empty())
            .then(|| builder::check(&craft, &catalog, &touched).err())
            .flatten()
            .map(|r| r.to_string());
        Ok((craft, propulsion, ghost, refused))
    });
    placing.job = Some(Job { task, wanted });
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
    placing.plan = None;
    placing.asked = None;
    placing.job = None;
    if placing.restore || placing.shown.is_some() {
        if placing.shown.is_some() && !draft.parts.is_empty() {
            redraw_draft(editor, catalog, sim, ui);
        }
        placing.shown = None;
        placing.shown_craft = None;
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
    let holding = placing.held.is_some();
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

/// `editor.holding` and, with something in hand, `editor.hint`; see the module.
#[allow(clippy::type_complexity)]
fn publish_placing(
    placing: Res<Placing>,
    editor: Res<Editor>,
    content: Res<GameContent>,
    mut data: ResMut<UiData>,
    mut entries: Local<Option<Vec<Entry>>>,
    mut published: Local<
        Option<(
            Option<Held>,
            u32,
            bool,
            Option<String>,
            bool,
            i64,
            bool,
            bool,
            usize,
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
        placing.held.clone(),
        placing.symmetry,
        placing.mirror,
        placing.refused.clone(),
        placing.spot.is_some(),
        placing.roll.round() as i64,
        placing.linking,
        placing.over_list,
        editor.builder().map_or(0, |b| b.loose().len()),
    );
    if published.as_ref() == Some(&now) && !content.is_changed() {
        return;
    }
    *published = Some(now);
    data.set("editor.linking", placing.linking);
    if placing.linking {
        data.set(
            "editor.hint",
            "Click the tank or engine this fuel line feeds. A right click or Esc takes it off.",
        );
    }
    data.set("editor.holding", placing.held.is_some());
    data.set(
        "editor.loose",
        (editor.builder()).map_or(0, |b| b.loose().len()) as f64,
    );
    data.set("editor.symmetry", placing.symmetry_label());
    data.set("editor.mirror", placing.mirror);
    let Some(held) = &placing.held else {
        return;
    };
    let name = match held {
        Held::New(id) => (entries.iter())
            .find(|e| &e.id == id)
            .map_or("part".to_owned(), |e| e.name.to_lowercase()),
        Held::Moved(id) => {
            let draft = editor.builder().map(|b| b.draft());
            let part = draft.and_then(|d| d.parts.iter().find(|p| &p.id == id));
            part.and_then(|p| p.name.clone())
                .unwrap_or_else(|| id.clone())
                .to_lowercase()
        }
        Held::Loose(index) => (editor.builder())
            .and_then(|b| b.loose().get(*index))
            .and_then(|g| g.parts.first())
            .map_or("part".to_owned(), |p| {
                p.name
                    .clone()
                    .unwrap_or_else(|| p.id.clone())
                    .to_lowercase()
            }),
    };
    let how = if placing.dropping {
        "Let go with one of its nodes on a ring"
    } else {
        "Click with one of its nodes on a ring"
    };
    let symmetry = if placing.mirror {
        "mirrored".to_owned()
    } else if placing.symmetry > 1 {
        format!("{} round", placing.symmetry)
    } else {
        "one".to_owned()
    };
    let off = matches!(held, Held::Moved(_) | Held::Loose(_)) && placing.spot.is_none();
    let copies = matches!(held, Held::Moved(_)) && (placing.mirror || placing.symmetry > 1);
    let mut hint = if off && placing.over_list {
        // Over the parts list: letting go throws it away.
        let copies = if copies { " and its copies" } else { "" };
        format!("Let go here to throw the {name}{copies} away, with what it holds.")
    } else if off {
        // Out in the hangar: letting go leaves it there, off the craft.
        let copies = if copies { " (its copies go)" } else { "" };
        format!(
            "Let go here to leave the {name} off the craft{copies}, to put on later; over the \
             parts list to throw it away."
        )
    } else {
        format!(
            "{how}, or on the side of a part, to put the {name} there ({symmetry}: X and R \
             change it). Q and E turn it."
        )
    };
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
