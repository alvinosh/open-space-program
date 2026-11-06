//! The builder: operations on a draft craft that write plain parts. The one way the editor, the
//! craft generators and the tests change a craft (see `docs/build-editor-plan.md`).
//!
//! Each operation returns what changed, or refuses with the rule that stops it and leaves the
//! draft as it was. A pattern or a mirror writes plain parts and nothing remembers it afterwards
//! ([`Builder::pattern`], [`Builder::mirror`]). Every operation can be undone, back to the draft
//! it started from.
//!
//! The rules:
//!
//! - It must join: the craft has to build, with one root and every part held at a connector
//!   that exists.
//! - A part must not be held by a node much smaller than its own.
//! - What fills space must not overlap: tanks, solids and wings. A shell's walls fill nothing, so
//!   engines can stand in an interstage.
//! - What is held inside a container must fit inside it.
//! - What a fairing holds must fit in it.
//!
//! [`warnings`] says what would stop a craft working without making it wrong to build: engines
//! nothing feeds, and exhaust that hits something that stays on.

use std::collections::BTreeSet;

use glam::{DQuat, DVec3};
use serde::{Deserialize, Serialize};

use crate::{
    aero::{outer_triangles, voxel::Body},
    assembly::{Mount, Pose, Side},
    component::{BuiltPart, Component},
    craft::Craft,
    data::{CraftDef, CraftPartDef, PartCatalog, SurfaceAtDef},
    geometry::{
        Node,
        preset::{NodeDef, ShapeDef},
    },
    properties, shaping,
};

/// The biggest a part's node can be against the node that holds it.
const NODE_RATIO: f64 = 2.5;
/// Share of a part's surface that may be inside what fills another's space: meshes that only
/// meet, and a nozzle's lip sunk in a thrust structure, are not an overlap.
const OVERLAP: f64 = 0.05;
/// Share of a part's surface that may stick out of what holds it.
const STICKS_OUT: f64 = 0.01;
/// The surface is drawn in by this share towards the part's middle before it is compared, so that
/// parts that only touch do not count.
const SHRINK: f64 = 0.03;
/// How far off a connector a point is taken to tell whether a part held there is inside, m.
const OFF_CONNECTOR: f64 = 0.01;
/// The most surface points a part is compared by.
const SAMPLES: usize = 600;

/// Which rule stops an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// There is no such part, or it is the root and cannot be that.
    Part,
    /// The craft would not build: the connector is not there, say, or it would make a loop.
    Join,
    /// A part's node is too big for the one that holds it.
    Node,
    /// Two parts fill the same space.
    Overlap,
    /// It does not fit in what holds it, a container or a fairing.
    Fit,
}

/// Why an operation is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub rule: Rule,
    pub message: String,
}

impl Refusal {
    fn new(rule: Rule, message: impl Into<String>) -> Self {
        Self {
            rule,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// What an operation changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    /// Ids of the parts it put there.
    pub added: Vec<String>,
    /// Ids of the parts it took away.
    pub removed: Vec<String>,
    /// Ids of the parts it changed.
    pub changed: Vec<String>,
}

impl Change {
    fn between(before: &CraftDef, after: &CraftDef) -> Self {
        let find = |def: &CraftDef, id: &str| def.parts.iter().find(|p| p.id == id).cloned();
        let ids = |def: &CraftDef| def.parts.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
        Self {
            added: (ids(after).into_iter())
                .filter(|id| find(before, id).is_none())
                .collect(),
            removed: (ids(before).into_iter())
                .filter(|id| find(after, id).is_none())
                .collect(),
            changed: (ids(after).into_iter())
                .filter(|id| find(before, id).is_some_and(|p| Some(p) != find(after, id)))
                .collect(),
        }
    }

    /// Every part that moved or is new: the ones to check.
    fn touched(&self) -> impl Iterator<Item = &String> {
        self.added.iter().chain(&self.changed)
    }
}

/// Where a part is held: a connector of another part, and which of its own holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    /// `"<part id>.<connector>"`.
    pub on: String,
    /// Its own node or surface.
    pub by: String,
    /// Where on the surface, when one end of the joint is a surface.
    pub at: Option<SurfaceAtDef>,
    /// Degrees it is turned about the joint.
    pub roll: f64,
}

impl Place {
    /// Held by its `by` at a node of another part.
    pub fn node(on: impl Into<String>, by: impl Into<String>) -> Self {
        Self {
            on: on.into(),
            by: by.into(),
            at: None,
            roll: 0.0,
        }
    }

    /// Held by its `by` on a surface of another part, `at` a place on it.
    pub fn surface(on: impl Into<String>, by: impl Into<String>, at: SurfaceAtDef) -> Self {
        Self {
            at: Some(at),
            ..Self::node(on, by)
        }
    }

    fn write(&self, part: &mut CraftPartDef) {
        // A node it was slid or turned to by hand meant that place only.
        part.nodes
            .retain(|n| !n.name.starts_with(HELD) || n.name == self.by);
        part.on = Some(self.on.clone());
        part.by = self.by.clone();
        part.at = self.at;
        part.roll = self.roll;
    }
}

/// A draft craft and the operations on it. Each operation is checked against the parts in
/// `catalog`, which it is given each time, so a builder can be kept while the content changes.
#[derive(Debug, Clone)]
pub struct Builder {
    draft: CraftDef,
    undo: Vec<CraftDef>,
    redo: Vec<CraftDef>,
}

impl Builder {
    /// An empty draft: the first part placed is its root.
    pub fn new(id: impl Into<String>) -> Self {
        Self::of(CraftDef {
            id: id.into(),
            name: String::new(),
            parts: Vec::new(),
            sequences: None,
            triggers: Vec::new(),
            rcs_shut: Vec::new(),
            abort: Vec::new(),
        })
    }

    /// A craft to go on with. It is taken as it is: only what is done to it is checked.
    pub fn of(draft: CraftDef) -> Self {
        Self {
            draft,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn draft(&self) -> &CraftDef {
        &self.draft
    }

    pub fn into_draft(self) -> CraftDef {
        self.draft
    }

    /// Whether there is an operation to go back over, or one to do again.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Make the edit on a copy; if it is good, keep it and log the old one.
    fn edit(
        &mut self,
        catalog: &PartCatalog,
        edit: impl FnOnce(&mut CraftDef) -> Result<(), Refusal>,
    ) -> Result<Change, Refusal> {
        self.edit_checking(catalog, &BTreeSet::new(), edit)
    }

    /// [`Self::edit`], checking the parts `also` as well as those it changed: what was carried
    /// along by a part that moved.
    fn edit_checking(
        &mut self,
        catalog: &PartCatalog,
        also: &BTreeSet<String>,
        edit: impl FnOnce(&mut CraftDef) -> Result<(), Refusal>,
    ) -> Result<Change, Refusal> {
        let mut next = self.draft.clone();
        edit(&mut next)?;
        let change = Change::between(&self.draft, &next);
        if !next.parts.is_empty() {
            let touched: BTreeSet<&str> =
                (change.touched().chain(also)).map(String::as_str).collect();
            check(&next, catalog, &Vec::from_iter(touched))?;
        }
        self.undo.push(std::mem::replace(&mut self.draft, next));
        self.redo.clear();
        Ok(change)
    }

    /// Put `part` where `place` says, or as the root if there is none yet.
    pub fn place(
        &mut self,
        catalog: &PartCatalog,
        mut part: CraftPartDef,
        place: Option<Place>,
    ) -> Result<Change, Refusal> {
        self.edit(catalog, |draft| {
            if draft.parts.iter().any(|p| p.id == part.id) {
                return Err(Refusal::new(
                    Rule::Part,
                    format!("it already has a part `{}`", part.id),
                ));
            }
            match place {
                Some(place) => place.write(&mut part),
                None if draft.parts.is_empty() => part.on = None,
                None => {
                    return Err(Refusal::new(
                        Rule::Join,
                        format!("`{}` has to be held by something", part.id),
                    ));
                }
            }
            draft.parts.push(part);
            Ok(())
        })
    }

    /// Take `id` away, with everything held by it, and out of what its sequences do.
    pub fn remove(&mut self, catalog: &PartCatalog, id: &str) -> Result<Change, Refusal> {
        self.edit(catalog, |draft| {
            let gone = held_by(draft, id)?;
            if gone.len() < draft.parts.len() && root(draft) == Some(id) {
                return Err(Refusal::new(
                    Rule::Part,
                    format!("`{id}` is the root: reroot before taking it away"),
                ));
            }
            draft.parts.retain(|p| !gone.contains(&p.id));
            forget(draft, &gone);
            Ok(())
        })
    }

    /// Hold `id` somewhere else, with everything it holds.
    pub fn move_to(
        &mut self,
        catalog: &PartCatalog,
        id: &str,
        place: Place,
    ) -> Result<Change, Refusal> {
        self.edit(catalog, |draft| {
            let holds = held_by(draft, id)?;
            let target = place
                .on
                .split_once('.')
                .map_or(place.on.as_str(), |(p, _)| p);
            if holds.iter().any(|p| p == target) {
                return Err(Refusal::new(
                    Rule::Join,
                    format!("`{id}` cannot be held by `{target}`, which it holds"),
                ));
            }
            if root(draft) == Some(id) {
                return Err(Refusal::new(
                    Rule::Part,
                    format!("`{id}` is the root: nothing holds it"),
                ));
            }
            place.write(part_mut(draft, id)?);
            Ok(())
        })
    }

    /// Turn `id` about the joint that holds it, degrees.
    pub fn rotate(
        &mut self,
        catalog: &PartCatalog,
        id: &str,
        degrees: f64,
    ) -> Result<Change, Refusal> {
        self.edit(catalog, |draft| {
            let part = part_mut(draft, id)?;
            part.roll = wrapped(part.roll + degrees);
            Ok(())
        })
    }

    /// Point `id` any way: `direction`, in its own axes, is what lies along the joint.
    pub fn orient(
        &mut self,
        catalog: &PartCatalog,
        id: &str,
        direction: DVec3,
    ) -> Result<Change, Refusal> {
        self.edit(catalog, |draft| {
            let Some(direction) = direction.try_normalize() else {
                return Err(Refusal::new(Rule::Join, "a direction has to have a length"));
            };
            let part = part_mut(draft, id)?;
            let alone = CraftDef {
                id: "alone".into(),
                name: String::new(),
                parts: vec![CraftPartDef {
                    on: None,
                    ..part.clone()
                }],
                sequences: None,
                triggers: Vec::new(),
                rcs_shut: Vec::new(),
                abort: Vec::new(),
            };
            let alone = Craft::build(&alone, catalog)
                .map_err(|e| Refusal::new(Rule::Join, e.to_string()))?;
            let name = part.by.clone();
            let held = format!("{HELD}{}", stands_for(&name));
            let node = alone.assembly.parts()[0]
                .node(&name)
                .cloned()
                .ok_or_else(|| {
                    Refusal::new(
                        Rule::Join,
                        format!("`{id}` is not held by a node of its own"),
                    )
                })?;
            part.nodes.retain(|n| n.name != held);
            part.nodes.push(NodeDef {
                name: held.clone(),
                position: node.position.to_array(),
                direction: direction.to_array(),
                size: node.size,
            });
            part.by = held;
            Ok(())
        })
    }

    /// Slide `id` `by` and turn it `turn` about the point it is held by, both in the craft's frame,
    /// with everything on it and the parts put on with it by symmetry: see [`shifted`]. What it
    /// carries is checked against the rules as well as what moved.
    pub fn shift(
        &mut self,
        catalog: &PartCatalog,
        id: &str,
        by: DVec3,
        turn: DQuat,
    ) -> Result<Change, Refusal> {
        let (next, moved) = shifted(&self.draft, catalog, id, by, turn)?;
        let mut carried = BTreeSet::new();
        for id in &moved {
            carried.extend(held_by(&next, id)?);
        }
        self.edit_checking(catalog, &carried, |draft| {
            *draft = next;
            Ok(())
        })
    }

    /// Make `id` the root: the joints back to the old root are written the other way round, so
    /// that every part is on one nearer it. Joints have no direction, so the parts sit as they did.
    pub fn reroot(&mut self, catalog: &PartCatalog, id: &str) -> Result<Change, Refusal> {
        self.edit(catalog, |draft| {
            let mut current = id.to_owned();
            let mut hold: (Option<String>, String, Option<SurfaceAtDef>, f64) = {
                let part = part_mut(draft, id)?;
                (None, part.by.clone(), None, 0.0)
            };
            loop {
                let part = part_mut(draft, &current)?;
                let was = (part.on.take(), part.by.clone(), part.at, part.roll);
                (part.on, part.by, part.at, part.roll) = hold;
                let Some(held) = was.0 else {
                    break;
                };
                let (parent, at_parent) = held.split_once('.').ok_or_else(|| {
                    Refusal::new(Rule::Join, format!("`{held}` names no connector"))
                })?;
                hold = (
                    Some(format!("{current}.{}", was.1)),
                    at_parent.to_string(),
                    was.2,
                    was.3,
                );
                current = parent.to_string();
            }
            Ok(())
        })
    }

    /// Put `count` copies of `part` round a surface: the one at `at` and the rest at equal turns
    /// from it, named `<id>_1` up. They are plain parts, as if placed one by one; nothing remembers
    /// the pattern.
    pub fn pattern(
        &mut self,
        catalog: &PartCatalog,
        part: CraftPartDef,
        place: Place,
        count: u32,
    ) -> Result<Change, Refusal> {
        let copies = patterned(&part, &place, count)?;
        self.edit(catalog, |draft| {
            for copy in copies {
                if draft.parts.iter().any(|p| p.id == copy.id) {
                    return Err(Refusal::new(
                        Rule::Part,
                        format!("it already has a part `{}`", copy.id),
                    ));
                }
                draft.parts.push(copy);
            }
            Ok(())
        })
    }

    /// Put a reflection of `id` and all it holds across the plane through the axis of what holds
    /// it, at turn `plane` (0 to 0.5) of that surface: named with `suffix` after each id. They are
    /// plain parts; nothing remembers the mirror.
    pub fn mirror(
        &mut self,
        catalog: &PartCatalog,
        id: &str,
        plane: f64,
        suffix: &str,
    ) -> Result<Change, Refusal> {
        self.edit(catalog, |draft| {
            let ids = held_by(draft, id)?;
            let renamed = |part: &str| format!("{part}{suffix}");
            let mut copies = Vec::new();
            for part in draft.parts.iter().filter(|p| ids.contains(&p.id)) {
                let mut copy = reflected(part, if part.id == id { plane } else { 0.0 });
                copy.id = renamed(&part.id);
                if part.id != id {
                    copy.on = (copy.on.as_ref()).map(|on| match on.split_once('.') {
                        Some((held, connector)) => format!("{}.{connector}", renamed(held)),
                        None => on.clone(),
                    });
                }
                copies.push(copy);
            }
            if let Some(clash) = copies
                .iter()
                .find(|c| draft.parts.iter().any(|p| p.id == c.id))
            {
                return Err(Refusal::new(
                    Rule::Part,
                    format!("it already has a part `{}`", clash.id),
                ));
            }
            draft.parts.extend(copies);
            Ok(())
        })
    }

    /// Change sizes of `id`'s shape (see [`shaping`]), each `(name, value)`, kept within what the
    /// shape allows, with what is mounted on it carried along: one operation, however many sizes. A
    /// part that already broke a rule before is only held to building: what is checked is what the
    /// change does.
    pub fn resize(
        &mut self,
        catalog: &PartCatalog,
        id: &str,
        sizes: &[(&str, f64)],
    ) -> Result<Change, Refusal> {
        let kept = check(&self.draft, catalog, &[id]).is_ok();
        let mut next = self.draft.clone();
        let part = part_mut(&mut next, id)?;
        for &(size, value) in sizes {
            shaping::resize(part, size, value).map_err(|e| Refusal::new(Rule::Part, e))?;
        }
        if kept {
            check(&next, catalog, &[id])?;
        } else {
            Craft::build(&next, catalog).map_err(|e| Refusal::new(Rule::Join, e.to_string()))?;
        }
        let change = Change::between(&self.draft, &next);
        if !change.changed.is_empty() {
            self.undo.push(std::mem::replace(&mut self.draft, next));
            self.redo.clear();
        }
        Ok(change)
    }

    /// Set field `field` of `id` (see [`properties`]) from `text`, as the part window does: a size,
    /// its shape, what its tank holds, its material, a sizer's requirement. Checked as
    /// [`Builder::resize`] is: only for what the change does.
    pub fn set_field(
        &mut self,
        catalog: &PartCatalog,
        id: &str,
        field: &str,
        text: &str,
    ) -> Result<Change, Refusal> {
        let kept = check(&self.draft, catalog, &[id]).is_ok();
        let mut next = self.draft.clone();
        let part = part_mut(&mut next, id)?;
        properties::set(part, field, text, catalog).map_err(|e| Refusal::new(Rule::Part, e))?;
        if kept {
            check(&next, catalog, &[id])?;
        } else {
            Craft::build(&next, catalog).map_err(|e| Refusal::new(Rule::Join, e.to_string()))?;
        }
        let change = Change::between(&self.draft, &next);
        if !change.changed.is_empty() {
            self.undo.push(std::mem::replace(&mut self.draft, next));
            self.redo.clear();
        }
        Ok(change)
    }

    /// Go back one operation; `false` if there is none to go back over.
    pub fn undo(&mut self) -> bool {
        let Some(before) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.draft, before));
        true
    }

    /// Do again what [`Builder::undo`] went back over.
    pub fn redo(&mut self) -> bool {
        let Some(after) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.draft, after));
        true
    }
}

/// `part` held where `place` says: what [`Builder::place`] puts in the craft.
pub fn held_at(mut part: CraftPartDef, place: &Place) -> CraftPartDef {
    place.write(&mut part);
    part
}

/// The parts [`Builder::pattern`] puts in the craft: `count` copies of `part` round the surface
/// `place` is on, from its place at equal turns, named `<id>_1` up.
pub fn patterned(
    part: &CraftPartDef,
    place: &Place,
    count: u32,
) -> Result<Vec<CraftPartDef>, Refusal> {
    let Some(at) = place.at else {
        return Err(Refusal::new(Rule::Join, "a pattern goes on a surface"));
    };
    Ok((0..count)
        .map(|k| {
            let mut copy = part.clone();
            copy.id = format!("{}_{}", part.id, k + 1);
            let mut place = place.clone();
            place.at = Some(SurfaceAtDef {
                u: (at.u + f64::from(k) / f64::from(count)).rem_euclid(1.0),
                ..at
            });
            place.write(&mut copy);
            copy
        })
        .collect())
}

/// What a part slid or turned by hand is held by: a node of its own where the node it was held by
/// went, named `held_` and that node's name. Nothing else in the craft needs to know.
const HELD: &str = "held_";

/// The node of its own that a part held by `by` stands for: `bottom` for `held_bottom`, and any
/// other the same.
pub fn stands_for(by: &str) -> &str {
    by.strip_prefix(HELD).unwrap_or(by)
}

/// How a part is held, in the craft's frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Holding {
    /// The connector of what holds it: +Z out of that part, and on a surface +X round it and +Y
    /// along it.
    pub frame: Pose,
    /// The point of the part it is held by, which it turns about: where its own node is, though
    /// it has been slid off the connector.
    pub pivot: DVec3,
}

/// How the part at `index` of `craft` is held; none for the root.
pub fn holding(craft: &Craft, index: usize) -> Option<Holding> {
    let (joint, parent) = (*craft.assembly.parents().get(index)?)?;
    let ends = &craft.assembly.joints()[joint].ends;
    let [a, b] = craft.assembly.frames(joint);
    let (near, far, own) = if ends[0].part == parent {
        (a, b, &ends[1].mount)
    } else {
        (b, a, &ends[0].mount)
    };
    let part = &craft.assembly.parts()[index];
    let at = match own {
        Mount::Node(name) => {
            (part.node(stands_for(name)).or(part.node(name))).map_or(far.position, |n| n.position)
        }
        Mount::Surface { .. } => far.position,
    };
    let poses = craft.assembly.poses();
    Some(Holding {
        frame: poses[parent] * near,
        pivot: poses[index].point(at),
    })
}

/// The parts put on with `id` by symmetry, as far as can be told, each with whether it is its
/// mirror image: those on the same surface of the same part, as far along it and as far off it,
/// and the same in every other way, or the same as its reflection. Nothing remembers a pattern or
/// a mirror, so they are found by matching.
pub fn alike(draft: &CraftDef, id: &str) -> Vec<(String, bool)> {
    let Some(part) = draft.parts.iter().find(|p| p.id == id) else {
        return Vec::new();
    };
    if part.at.is_none() {
        return Vec::new();
    }
    let key = |p: &CraftPartDef| {
        let mut p = p.clone();
        p.id.clear();
        p.name = None;
        if let Some(at) = &mut p.at {
            at.u = 0.0;
        }
        p
    };
    let (same, mirror) = (key(part), key(&reflected(part, 0.0)));
    (draft.parts.iter())
        .filter(|p| p.id != id)
        .filter_map(|p| {
            let other = key(p);
            if other == same {
                Some((p.id.clone(), false))
            } else if other == mirror {
                Some((p.id.clone(), true))
            } else {
                None
            }
        })
        .collect()
}

/// How far a part held at a node may be slid off it: the point it is held at has to stay this near
/// the part, m. Further, it would hang in the air.
const REACH: f64 = 0.02;

/// `draft` with `id` slid `by` and turned `turn` about the point it is held by (both in the
/// craft's frame), and everything on it with it. The parts put on with it by symmetry ([`alike`])
/// go the same way, mirrored where they are mirror images. The rules are not checked: this is what
/// the editor shows while a handle is dragged. Also the ids of the parts it moved.
///
/// A part on a surface slides over it: the place it is held at goes with it, round and along the
/// surface and out from it, and stops at the surface's ends. A part on a node is held by a node of
/// its own where the one it was held by went, so the joint stays where it was; it may be slid only
/// as far as it still reaches that joint. A turn is always a node of its own, about the point the
/// part is held by.
pub fn shifted(
    draft: &CraftDef,
    catalog: &PartCatalog,
    id: &str,
    by: DVec3,
    turn: DQuat,
) -> Result<(CraftDef, Vec<String>), Refusal> {
    let on_surface = (draft.parts.iter())
        .find(|p| p.id == id)
        .is_some_and(|p| p.at.is_some());
    if on_surface && by != DVec3::ZERO {
        let (slid, moved) = slid_over_surface(draft, catalog, id, by)?;
        if turn == DQuat::IDENTITY {
            return Ok((slid, moved));
        }
        return held_anew(&slid, catalog, id, DVec3::ZERO, turn);
    }
    held_anew(draft, catalog, id, by, turn)
}

/// The craft, the part at `id` in it, and the joint that holds it towards the root: its index,
/// the part at its other end, and the frame of the connector there in that part's.
fn hold_of(
    draft: &CraftDef,
    catalog: &PartCatalog,
    id: &str,
) -> Result<(Craft, usize, usize, usize, Pose), Refusal> {
    let craft =
        Craft::build(draft, catalog).map_err(|e| Refusal::new(Rule::Join, e.to_string()))?;
    let index = (craft.find(id))
        .ok_or_else(|| Refusal::new(Rule::Part, format!("there is no part `{id}`")))?;
    let (j, parent) = craft.assembly.parents()[index].ok_or_else(|| {
        Refusal::new(
            Rule::Part,
            format!("`{id}` is the root: it holds the rest, so it stays where it is"),
        )
    })?;
    if craft.assembly.joints()[j].ends[0].part != parent {
        return Err(Refusal::new(
            Rule::Join,
            format!("`{id}` is held the wrong way round"),
        ));
    }
    let [near, _] = craft.assembly.frames(j);
    Ok((craft, index, j, parent, near))
}

/// [`shifted`] for a part on a surface, slid: the place it is held at goes `by` over the surface.
fn slid_over_surface(
    draft: &CraftDef,
    catalog: &PartCatalog,
    id: &str,
    by: DVec3,
) -> Result<(CraftDef, Vec<String>), Refusal> {
    let (craft, _, j, parent, _) = hold_of(draft, catalog, id)?;
    let Mount::Surface {
        surface,
        u,
        v,
        side,
        ..
    } = &craft.assembly.joints()[j].ends[0].mount
    else {
        return Err(Refusal::new(
            Rule::Join,
            format!("`{id}` is not on a surface"),
        ));
    };
    let geometry = &craft.assembly.parts()[parent].geometry;
    let off_it = || Refusal::new(Rule::Join, format!("`{id}` is off its surface"));
    let point = |u: f64, v: f64| geometry.surface_point(surface, u, v);
    let here = point(*u, *v).ok_or_else(off_it)?;
    // How the place moves with each coordinate, here: a step inside the surface's ends.
    let rate = |at: &dyn Fn(f64) -> Option<DVec3>, x: f64| -> Option<DVec3> {
        const STEP: f64 = 1e-4;
        let (a, b) = if x + STEP <= 1.0 {
            (x, x + STEP)
        } else {
            (x - STEP, x)
        };
        Some((at(b)? - at(a)?) / (b - a))
    };
    let round = rate(&|u| Some(point(u, *v)?.position), *u).ok_or_else(off_it)?;
    let along = rate(&|v| Some(point(*u, v)?.position), *v).ok_or_else(off_it)?;
    // In the craft's frame, with which way is out.
    let turned = craft.assembly.poses()[parent].rotation;
    let (round, along) = (turned * round, turned * along);
    let out = turned * here.normal * if *side == Side::Back { -1.0 } else { 1.0 };
    let (du, dv) = (
        by.dot(round) / round.length_squared(),
        by.dot(along) / along.length_squared(),
    );
    let off = by.dot(out);
    let mut next = draft.clone();
    let mut moved = vec![id.to_owned()];
    let slide = |part: &mut CraftPartDef, mirrored: bool| {
        if let Some(at) = &mut part.at {
            let du = if mirrored { -du } else { du };
            at.u = ((at.u + du).rem_euclid(1.0) * 1e9).round() / 1e9;
            at.v = (((at.v + dv).clamp(0.0, 1.0)) * 1e9).round() / 1e9;
            at.standoff = ((at.standoff + off).max(0.0) * 1e6).round() / 1e6;
        }
    };
    slide(part_mut(&mut next, id)?, false);
    for (other, mirrored) in alike(draft, id) {
        slide(part_mut(&mut next, &other)?, mirrored);
        moved.push(other);
    }
    Ok((next, moved))
}

/// [`shifted`] by a node of its own: `id` held where its own node would have to be for it to go
/// `by` and turn `turn` about the point it is held by.
fn held_anew(
    draft: &CraftDef,
    catalog: &PartCatalog,
    id: &str,
    by: DVec3,
    turn: DQuat,
) -> Result<(CraftDef, Vec<String>), Refusal> {
    let (craft, index, j, parent, near) = hold_of(draft, catalog, id)?;
    let Mount::Node(name) = &craft.assembly.joints()[j].ends[1].mount else {
        return Err(Refusal::new(
            Rule::Join,
            format!("`{id}` is held by a surface of its own, so it cannot be slid or turned"),
        ));
    };
    let node = (craft.assembly.parts()[index].node(name).cloned())
        .ok_or_else(|| Refusal::new(Rule::Join, format!("`{id}` has no node `{name}`")))?;
    let poses = craft.assembly.poses();
    let (pose, holder) = (poses[index], poses[parent]);
    // Where it is to be, in the frame of the connector that holds it.
    let pivot = holding(&craft, index).map_or(pose.point(node.position), |h| h.pivot);
    let motion = Pose::new(by + pivot, turn) * Pose::new(-pivot, DQuat::IDENTITY);
    let to_be = (holder * near).inverse() * motion * pose;
    // The node of its own that meets the connector there, facing it.
    let face = Pose::new(DVec3::ZERO, DQuat::from_rotation_x(std::f64::consts::PI));
    let meets = to_be.inverse() * face;
    let tidy = |v: DVec3, step: f64| (v / step).round() * step;
    let held = Node {
        name: format!("{HELD}{}", stands_for(name)),
        position: tidy(meets.position, 1e-6),
        direction: tidy(meets.rotation * DVec3::Z, 1e-9).normalize(),
        size: node.size,
    };
    // It still reaches the joint.
    let own = placed(&craft.parts[index].built, Pose::IDENTITY);
    let reach = DVec3::splat(REACH);
    let within =
        (own.lo - reach).cmple(held.position).all() && held.position.cmple(own.hi + reach).all();
    if !within {
        return Err(Refusal::new(
            Rule::Join,
            format!(
                "`{id}` would no longer reach `{}`, which holds it",
                craft.parts[parent].id
            ),
        ));
    }
    // The turn about the joint that is left, once the node faces the connector: about +Z only, but
    // for the node's place and way being rounded.
    let left = (to_be * crate::assembly::node_frame(&held)).rotation * face.rotation.inverse();
    if left.x.abs().max(left.y.abs()) > 1e-6 {
        return Err(Refusal::new(
            Rule::Join,
            format!("`{id}` cannot be held there"),
        ));
    }
    let roll = 2.0 * left.z.atan2(left.w);
    let roll = wrapped((roll.to_degrees() * 1e6).round() / 1e6);
    let mut next = draft.clone();
    let alike = alike(draft, id);
    let write = |part: &mut CraftPartDef, mirrored: bool| {
        let flip = |v: DVec3| {
            if mirrored {
                DVec3::new(-v.x, v.y, v.z)
            } else {
                v
            }
        };
        part.nodes.retain(|n| n.name != held.name);
        part.nodes.push(NodeDef {
            name: held.name.clone(),
            position: flip(held.position).to_array(),
            direction: flip(held.direction).to_array(),
            size: held.size,
        });
        part.by = held.name.clone();
        part.roll = if mirrored { wrapped(-roll) } else { roll };
    };
    write(part_mut(&mut next, id)?, false);
    let mut moved = vec![id.to_owned()];
    for (other, mirrored) in alike {
        write(part_mut(&mut next, &other)?, mirrored);
        moved.push(other);
    }
    Ok((next, moved))
}

/// `part` reflected across a plane through the axis of what holds it, at turn `plane`: its shape
/// mirrored (a catalogue record is its own mirror image) and held at the reflected place. Twice
/// is the part as it was.
pub fn reflected(part: &CraftPartDef, plane: f64) -> CraftPartDef {
    let mut copy = part.clone();
    copy.shape = copy.shape.map(|shape| match shape {
        ShapeDef::Mirrored(inner) => *inner,
        other => ShapeDef::Mirrored(Box::new(other)),
    });
    copy.at = copy.at.map(|at| SurfaceAtDef {
        u: (2.0 * plane - at.u).rem_euclid(1.0),
        ..at
    });
    copy.roll = wrapped(-copy.roll);
    // Its own nodes are in its frame, which is reflected with its shape.
    for node in &mut copy.nodes {
        node.position[0] = -node.position[0];
        node.direction[0] = -node.direction[0];
    }
    copy
}

/// Degrees in (-180, 180].
fn wrapped(degrees: f64) -> f64 {
    let turned = (degrees + 180.0).rem_euclid(360.0) - 180.0;
    if turned == -180.0 { 180.0 } else { turned }
}

fn part_mut<'d>(draft: &'d mut CraftDef, id: &str) -> Result<&'d mut CraftPartDef, Refusal> {
    (draft.parts.iter_mut().find(|p| p.id == id))
        .ok_or_else(|| Refusal::new(Rule::Part, format!("there is no part `{id}`")))
}

fn root(draft: &CraftDef) -> Option<&str> {
    draft
        .parts
        .iter()
        .find(|p| p.on.is_none())
        .map(|p| p.id.as_str())
}

/// `id` and every part held through it.
fn held_by(draft: &CraftDef, id: &str) -> Result<BTreeSet<String>, Refusal> {
    if !draft.parts.iter().any(|p| p.id == id) {
        return Err(Refusal::new(Rule::Part, format!("there is no part `{id}`")));
    }
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
            return Ok(ids);
        }
        ids.extend(more);
    }
}

/// Take the parts `gone` out of what the craft's sequences and triggers do.
fn forget(draft: &mut CraftDef, gone: &BTreeSet<String>) {
    let names = |activation: &str| {
        let mut fields = activation.split(':');
        fields.next();
        fields.next().is_some_and(|part| gone.contains(part))
    };
    if let Some(sequences) = &mut draft.sequences {
        for sequence in sequences.iter_mut() {
            sequence.retain(|a| !names(a));
        }
        sequences.retain(|s| !s.is_empty());
    }
    for trigger in &mut draft.triggers {
        trigger.fire.retain(|a| !names(a));
    }
    draft.triggers.retain(|t| !t.fire.is_empty());
}

/// A part's surface and what it fills, placed in the craft's frame.
struct Placed {
    pose: Pose,
    /// Points on its surface, in the craft's frame, drawn in towards its middle.
    samples: Vec<DVec3>,
    lo: DVec3,
    hi: DVec3,
}

fn placed(built: &BuiltPart, pose: Pose) -> Placed {
    let mut triangles = Vec::new();
    outer_triangles(built, |p| pose.point(p), &mut triangles);
    let stride = (triangles.len() / SAMPLES).max(1);
    let (mut lo, mut hi) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
    let mut points = Vec::new();
    for triangle in &triangles {
        for corner in triangle {
            lo = lo.min(*corner);
            hi = hi.max(*corner);
        }
    }
    for triangle in triangles.iter().step_by(stride) {
        points.push((triangle[0] + triangle[1] + triangle[2]) / 3.0);
        points.push(triangle[0]);
    }
    let middle = (lo + hi) / 2.0;
    let samples = (points.iter())
        .map(|p| middle + (*p - middle) * (1.0 - SHRINK))
        .collect();
    Placed {
        pose,
        samples,
        lo,
        hi,
    }
}

impl Placed {
    /// The share of its surface `holds` says is inside the part at `pose`.
    fn inside(&self, pose: Pose, holds: impl Fn(DVec3) -> bool) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let into = pose.inverse();
        let inside = (self.samples.iter())
            .filter(|p| holds(into.point(**p)))
            .count();
        inside as f64 / self.samples.len() as f64
    }

    fn meets(&self, other: &Placed) -> bool {
        self.lo.cmple(other.hi).all() && other.lo.cmple(self.hi).all()
    }
}

/// What fills space of a part, in its own frame: closed surfaces, so a point is in them if a ray
/// from it crosses them an odd number of times. A hole through a ring is not in it.
struct Solid {
    triangles: Vec<[DVec3; 3]>,
}

impl Solid {
    fn of(built: &BuiltPart) -> Option<Self> {
        let mut triangles = Vec::new();
        for mesh in built.geometry.solids() {
            let corner = |i: u32| DVec3::from(mesh.positions[i as usize].map(f64::from));
            triangles
                .extend((mesh.indices.chunks_exact(3)).map(|t| [0, 1, 2].map(|k| corner(t[k]))));
        }
        (!triangles.is_empty()).then_some(Self { triangles })
    }

    fn holds(&self, point: DVec3) -> bool {
        // Off every axis, so it does not run along the seams of a mesh.
        const RAY: DVec3 = DVec3::new(0.8123, 0.4711, 0.3431);
        let crossings = (self.triangles.iter())
            .filter(|[a, b, c]| {
                let (ab, ac) = (*b - *a, *c - *a);
                let p = RAY.cross(ac);
                let det = ab.dot(p);
                if det.abs() < 1e-12 {
                    return false;
                }
                let to = point - *a;
                let u = to.dot(p) / det;
                let q = to.cross(ab);
                let v = RAY.dot(q) / det;
                (0.0..=1.0).contains(&u) && v >= 0.0 && u + v <= 1.0 && ac.dot(q) / det > 0.0
            })
            .count();
        crossings % 2 == 1
    }
}

/// What it holds things in, in its own frame: all its outside surfaces enclose.
fn outer_body(built: &BuiltPart) -> Body {
    let mut triangles = Vec::new();
    outer_triangles(built, |p| p, &mut triangles);
    Body::new(&triangles)
}

/// Whether what is held at `mount` of `holder` is inside it.
fn held_inside(holder: &Craft, part: usize, mount: &Mount, body: &Body) -> bool {
    let assembly = &holder.assembly.parts()[part];
    if !(holder.parts[part].built.components.iter()).any(|c| matches!(c, Component::Container(_))) {
        return false;
    }
    if matches!(
        mount,
        Mount::Surface {
            side: Side::Back,
            ..
        }
    ) {
        return true;
    }
    let Some(frame) = assembly.frame(mount) else {
        return false;
    };
    body.encloses(frame.point(DVec3::Z * OFF_CONNECTOR))
}

/// Check `touched`, parts of `def`, against the rules.
pub fn check(def: &CraftDef, catalog: &PartCatalog, touched: &[&str]) -> Result<(), Refusal> {
    let craft = Craft::build(def, catalog).map_err(|e| Refusal::new(Rule::Join, e.to_string()))?;
    let poses = craft.assembly.poses();
    let parents = craft.assembly.parents();
    let places: Vec<Placed> = (craft.parts.iter().zip(&poses))
        .map(|(part, pose)| placed(&part.built, *pose))
        .collect();
    let solids: Vec<Option<Solid>> = (craft.parts.iter()).map(|p| Solid::of(&p.built)).collect();
    let mut fairing_clear = false;
    for id in touched {
        let Some(index) = craft.find(id) else {
            continue;
        };
        // Its holder: the joint that holds it towards the root.
        let holder = parents[index].map(|(joint, parent)| {
            let joint = &craft.assembly.joints()[joint];
            let mount = |part: usize| {
                (joint.ends.iter())
                    .find(|e| e.part == part)
                    .map(|e| e.mount.clone())
            };
            (parent, mount(parent), mount(index))
        });
        if let Some((parent, Some(near), Some(far))) = &holder {
            check_nodes(&craft, *parent, near, index, far)?;
        }
        // Inside what holds it, if that is where it is.
        let inside = holder.as_ref().and_then(|(parent, near, _)| {
            let near = near.as_ref()?;
            let body = outer_body(&craft.parts[*parent].built);
            held_inside(&craft, *parent, near, &body).then_some((*parent, body))
        });
        if let Some((parent, body)) = &inside {
            let out = 1.0 - places[index].inside(poses[*parent], |p| body.encloses(p));
            if out > STICKS_OUT {
                return Err(Refusal::new(
                    Rule::Fit,
                    format!(
                        "`{id}` does not fit inside `{}`: {:.0}% of it sticks out",
                        craft.parts[*parent].id,
                        100.0 * out
                    ),
                ));
            }
        }
        // Against everything else that fills space, both ways.
        for other in 0..craft.parts.len() {
            if other == index || !places[index].meets(&places[other]) {
                continue;
            }
            let share = |what: usize, of: usize| {
                solids[of].as_ref().map_or(0.0, |solid| {
                    places[what].inside(places[of].pose, |p| solid.holds(p))
                })
            };
            if share(index, other).max(share(other, index)) > OVERLAP {
                return Err(Refusal::new(
                    Rule::Overlap,
                    format!("`{id}` overlaps `{}`", craft.parts[other].id),
                ));
            }
        }
        fairing_clear = true;
    }
    if fairing_clear {
        let mut copy = def.clone();
        copy.fit_fairings(catalog)
            .map_err(|e| Refusal::new(Rule::Fit, e.to_string()))?;
    }
    Ok(())
}

/// A part's node must not be much bigger than the node that holds it. A surface takes any size,
/// and what has no size (a point, like the tip of a nose) holds and is held by anything.
fn check_nodes(
    craft: &Craft,
    parent: usize,
    near: &Mount,
    index: usize,
    far: &Mount,
) -> Result<(), Refusal> {
    let (Mount::Node(near), Mount::Node(far)) = (near, far) else {
        return Ok(());
    };
    let size = |part: usize, name: &str| craft.assembly.parts()[part].node(name).map(|n| n.size);
    let (Some(holds), Some(held)) = (size(parent, near), size(index, far)) else {
        return Ok(());
    };
    if holds > 0.0 && held > NODE_RATIO * holds {
        return Err(Refusal::new(
            Rule::Node,
            format!(
                "`{}`'s node `{far}` is {held:.2} m across, too big for `{}`'s `{near}` of {holds:.2} m",
                craft.parts[index].id, craft.parts[parent].id
            ),
        ));
    }
    Ok(())
}

/// What would stop `def` working, without being wrong to build: each said in a line.
pub fn warnings(def: &CraftDef, catalog: &PartCatalog) -> Vec<String> {
    let Ok(craft) = Craft::build(def, catalog) else {
        return vec!["it does not build".to_owned()];
    };
    let poses = craft.assembly.poses();
    let mut out = Vec::new();
    for (index, part) in craft.parts.iter().enumerate() {
        let Some(engine) = (part.built.components.iter()).find_map(|c| match c {
            Component::Engine(engine) => Some(engine),
            _ => None,
        }) else {
            continue;
        };
        if !fed(&craft, index) {
            out.push(format!("nothing feeds `{}`", part.id));
        }
        // Down its nozzle, far enough to clear what it is mounted on.
        let exit = engine.nozzle_length();
        let (axis, origin) = (poses[index].rotation * DVec3::NEG_Z, poses[index].position);
        for (other, thing) in craft.parts.iter().enumerate() {
            if other == index || lets_go_between(&craft, index, other) {
                continue;
            }
            let Some(solid) = Solid::of(&thing.built) else {
                continue;
            };
            let into = poses[other].inverse();
            let hit = (1..=8)
                .map(|k| origin + axis * (exit + f64::from(k) * 0.5))
                .any(|p| solid.holds(into.point(p)));
            if hit {
                out.push(format!(
                    "`{}` fires into `{}`, which stays on",
                    part.id, thing.id
                ));
            }
        }
    }
    out
}

/// Whether a tank or an engine's own reserve reaches the engine through joints that pass
/// propellant.
fn fed(craft: &Craft, engine: usize) -> bool {
    let holds = |part: usize| {
        (craft.parts[part].built.components.iter()).any(|c| matches!(c, Component::Storage(_)))
    };
    let joints = craft.assembly.joints();
    let mut seen = BTreeSet::from([engine]);
    let mut open = vec![engine];
    while let Some(at) = open.pop() {
        if holds(at) {
            return true;
        }
        for (j, joint) in joints.iter().enumerate() {
            let [a, b] = &joint.ends;
            let next = if a.part == at {
                b.part
            } else if b.part == at {
                a.part
            } else {
                continue;
            };
            if craft.assembly.carries(j).fluid && seen.insert(next) {
                open.push(next);
            }
        }
    }
    false
}

/// Whether a joint that lets go stands between two parts of the craft.
fn lets_go_between(craft: &Craft, a: usize, b: usize) -> bool {
    let parents = craft.assembly.parents();
    let path = |mut at: usize| {
        let mut path = Vec::new();
        while let Some((joint, parent)) = parents[at] {
            path.push(joint);
            at = parent;
        }
        path
    };
    let (up, down) = (path(a), path(b));
    (up.iter().filter(|j| !down.contains(j)))
        .chain(down.iter().filter(|j| !up.contains(j)))
        .any(|&j| craft.assembly.releasable(j))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette;

    fn catalog() -> PartCatalog {
        crate::vessel::tests::catalog()
    }

    fn entry(id: &str, catalog: &PartCatalog) -> CraftPartDef {
        let all = palette::entries(catalog);
        let mut part = all.iter().find(|e| e.id == id).unwrap().part.clone();
        palette::fit(&mut part, 2.0);
        part
    }

    /// A 2 m tank, another under it, and four fins round the first's barrel, half way up.
    fn stack(catalog: &PartCatalog) -> Builder {
        let mut builder = Builder::new("test");
        let mut top = entry("tank", catalog);
        top.id = "top".into();
        builder.place(catalog, top, None).unwrap();
        let mut lower = entry("tank", catalog);
        lower.id = "lower".into();
        let under = Place::node("top.bottom", "top");
        builder.place(catalog, lower, Some(under)).unwrap();
        let all = palette::entries(catalog);
        let (by, roll) = all
            .iter()
            .find(|e| e.id == "fin")
            .unwrap()
            .surface
            .clone()
            .unwrap();
        let mut fin = entry("fin", catalog);
        fin.id = "fin".into();
        let mut place = Place::surface(
            "top.barrel",
            by,
            SurfaceAtDef {
                u: 0.0,
                v: 0.5,
                side: Side::Front,
                standoff: 0.0,
            },
        );
        place.roll = roll;
        builder.pattern(catalog, fin, place, 4).unwrap();
        builder
    }

    fn poses(builder: &Builder, catalog: &PartCatalog) -> Vec<(String, Pose)> {
        let craft = Craft::build(builder.draft(), catalog).unwrap();
        let poses = craft.assembly.poses();
        (craft.parts.iter().zip(poses))
            .map(|(p, pose)| (p.id.clone(), pose))
            .collect()
    }

    fn pose(all: &[(String, Pose)], id: &str) -> Pose {
        all.iter().find(|(p, _)| p == id).unwrap().1
    }

    fn near(a: DVec3, b: DVec3) -> bool {
        (a - b).length() < 1e-6
    }

    #[test]
    fn a_stacked_part_turns_about_its_joint_and_is_put_back_by_undo() {
        let catalog = catalog();
        let mut builder = stack(&catalog);
        let before = poses(&builder, &catalog);
        let quarter = DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2);
        let change = builder
            .shift(&catalog, "lower", DVec3::ZERO, quarter)
            .unwrap();
        assert_eq!(change.changed, ["lower"]);
        let after = poses(&builder, &catalog);
        let (was, now) = (pose(&before, "lower"), pose(&after, "lower"));
        assert!(near(now.position, was.position));
        assert!((now.rotation * was.rotation.inverse()).angle_between(quarter) < 1e-6);
        let lower = builder
            .draft()
            .parts
            .iter()
            .find(|p| p.id == "lower")
            .unwrap();
        assert_eq!(lower.by, "held_top");
        assert_eq!(stands_for(&lower.by), "top");
        // Turned twice, it is held by the one node of its own.
        builder
            .shift(&catalog, "lower", DVec3::ZERO, quarter)
            .unwrap();
        let lower = builder
            .draft()
            .parts
            .iter()
            .find(|p| p.id == "lower")
            .unwrap();
        assert_eq!(
            lower
                .nodes
                .iter()
                .filter(|n| n.name.starts_with(HELD))
                .count(),
            1
        );
        assert!(builder.undo() && builder.undo());
        assert_eq!(poses(&builder, &catalog), before);
    }

    #[test]
    fn a_stacked_part_slides_only_while_it_reaches_its_joint_and_hits_nothing() {
        let catalog = catalog();
        let mut builder = stack(&catalog);
        let parts = builder.draft().parts.clone();
        let off = builder.shift(&catalog, "lower", DVec3::Z * -0.5, DQuat::IDENTITY);
        assert_eq!(off.map_err(|r| r.rule), Err(Rule::Join));
        let into = builder.shift(&catalog, "lower", DVec3::Z * 2.0, DQuat::IDENTITY);
        assert_eq!(into.map_err(|r| r.rule), Err(Rule::Overlap));
        let root = builder.shift(&catalog, "top", DVec3::Z, DQuat::IDENTITY);
        assert_eq!(root.map_err(|r| r.rule), Err(Rule::Part));
        assert!(builder.draft().parts == parts);
        // A hair's gap is still held.
        let before = poses(&builder, &catalog);
        let down = DVec3::Z * -0.015;
        builder
            .shift(&catalog, "lower", down, DQuat::IDENTITY)
            .unwrap();
        let moved =
            pose(&poses(&builder, &catalog), "lower").position - pose(&before, "lower").position;
        assert!(near(moved, down), "{moved:?}");
    }

    #[test]
    fn fins_put_on_together_slide_and_turn_together() {
        let catalog = catalog();
        let mut builder = stack(&catalog);
        let fins = ["fin_1", "fin_2", "fin_3", "fin_4"];
        let others = alike(builder.draft(), "fin_1");
        assert_eq!(others.len(), 3);
        assert!(
            others
                .iter()
                .all(|(id, mirrored)| fins.contains(&id.as_str()) && !mirrored)
        );
        let before = poses(&builder, &catalog);
        let up = DVec3::Z * 0.4;
        let change = builder
            .shift(&catalog, "fin_1", up, DQuat::IDENTITY)
            .unwrap();
        assert_eq!(change.changed.len(), 4);
        let after = poses(&builder, &catalog);
        for fin in fins {
            let moved = pose(&after, fin).position - pose(&before, fin).position;
            assert!(near(moved, up), "{fin} {moved:?}");
        }
        // Slid along the barrel, the place it is held at goes with it: no node of its own.
        let fin = |b: &Builder, id: &str| {
            (b.draft().parts.iter().find(|p| p.id == id))
                .unwrap()
                .clone()
        };
        assert_eq!(fin(&builder, "fin_1").by, "root");
        // Slid round, it stays on the barrel, as far out, and the others keep their spacing.
        let craft = Craft::build(builder.draft(), &catalog).unwrap();
        let held = holding(&craft, craft.find("fin_1").unwrap()).unwrap();
        let round = held.frame.rotation * DVec3::X;
        builder
            .shift(&catalog, "fin_1", round * 0.3, DQuat::IDENTITY)
            .unwrap();
        let spun = poses(&builder, &catalog);
        let radius = |p: DVec3| p.truncate().length();
        for fin in fins {
            let (a, b) = (pose(&after, fin).position, pose(&spun, fin).position);
            assert!((radius(a) - radius(b)).abs() < 1e-6, "{fin}");
            assert!((a.z - b.z).abs() < 1e-6 && a.distance(b) > 0.25, "{fin}");
        }
        assert!(builder.undo());
        // As far up as it goes, it stops at the barrel's end.
        let _ = builder.shift(&catalog, "fin_1", DVec3::Z * 50.0, DQuat::IDENTITY);
        assert!(fin(&builder, "fin_1").at.unwrap().v <= 1.0);
        let _ = builder.undo();
        // Turned about the line out of the barrel: each is still the first turned round the axis.
        let craft = Craft::build(builder.draft(), &catalog).unwrap();
        let held = holding(&craft, craft.find("fin_1").unwrap()).unwrap();
        let out = held.frame.rotation * DVec3::Z;
        let turn = DQuat::from_axis_angle(out, 30f64.to_radians());
        builder.shift(&catalog, "fin_1", DVec3::ZERO, turn).unwrap();
        let turned = poses(&builder, &catalog);
        let first = pose(&turned, "fin_1");
        let spun = first.rotation * (pose(&after, "fin_1").rotation.inverse());
        assert!(spun.angle_between(turn) < 1e-6);
        // Still at its own node: turned about it.
        let pivot = holding(&Craft::build(builder.draft(), &catalog).unwrap(), 2).unwrap();
        assert!(near(pivot.pivot, held.pivot));
        for (k, fin) in fins.iter().enumerate() {
            let round = DQuat::from_rotation_z(k as f64 * std::f64::consts::FRAC_PI_2);
            let expected = Pose::new(DVec3::ZERO, round) * first;
            let pose = pose(&turned, fin);
            assert!(near(pose.position, expected.position), "{fin}");
            assert!(
                pose.rotation.angle_between(expected.rotation) < 1e-6,
                "{fin}"
            );
        }
    }

    #[test]
    fn a_mirror_image_slides_and_turns_as_the_mirror_of_the_original() {
        let catalog = catalog();
        let mut builder = stack(&catalog);
        for fin in ["fin_2", "fin_3", "fin_4"] {
            builder.remove(&catalog, fin).unwrap();
        }
        // The first fin a little round from +X, and its mirror across the XZ plane.
        let mut draft = builder.draft().clone();
        part_mut(&mut draft, "fin_1")
            .unwrap()
            .at
            .as_mut()
            .unwrap()
            .u = 0.1;
        let mut builder = Builder::of(draft);
        builder.mirror(&catalog, "fin_1", 0.0, "_m").unwrap();
        assert_eq!(
            alike(builder.draft(), "fin_1"),
            [("fin_1_m".to_owned(), true)]
        );
        assert_eq!(
            alike(builder.draft(), "fin_1_m"),
            [("fin_1".to_owned(), true)]
        );
        let pivots = |b: &Builder| {
            let craft = Craft::build(b.draft(), &catalog).unwrap();
            ["fin_1", "fin_1_m"].map(|id| holding(&craft, craft.find(id).unwrap()).unwrap().pivot)
        };
        let reflect = |v: DVec3| DVec3::new(v.x, -v.y, v.z);
        let [a, b] = pivots(&builder);
        assert!(near(b, reflect(a)));
        let by = DVec3::new(0.05, 0.1, 0.3);
        let turn = DQuat::from_axis_angle(DVec3::new(0.3, 0.5, 0.8).normalize(), 0.4);
        let (next, moved) = shifted(builder.draft(), &catalog, "fin_1", by, turn).unwrap();
        assert_eq!(moved, ["fin_1", "fin_1_m"]);
        let craft = Craft::build(&next, &catalog).unwrap();
        let poses = craft.assembly.poses();
        let [one, other] = ["fin_1", "fin_1_m"].map(|id| poses[craft.find(id).unwrap()]);
        // Every point of the one, reflected, is the same point of the other, itself reflected.
        for p in [DVec3::ZERO, DVec3::X, DVec3::new(0.2, -0.4, 1.0)] {
            let mirrored = DVec3::new(-p.x, p.y, p.z);
            assert!(near(reflect(one.point(p)), other.point(mirrored)), "{p:?}");
        }
        let [a2, b2] = pivots(&Builder::of(next));
        assert!(near(b2, reflect(a2)));
        assert!((a2.z - a.z - by.z).abs() < 1e-6 && (a2 - a).length() > 0.3);
    }
}
