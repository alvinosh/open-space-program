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

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::{
    aero::{outer_triangles, voxel::Body},
    assembly::{Mount, Pose, Side},
    component::{BuiltPart, Component},
    craft::Craft,
    data::{CraftDef, CraftPartDef, PartCatalog, SurfaceAtDef},
    geometry::preset::{NodeDef, ShapeDef},
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
        part.on = Some(self.on.clone());
        part.by = self.by.clone();
        part.at = self.at;
        part.roll = self.roll;
    }
}

/// A draft craft and the operations on it.
#[derive(Debug, Clone)]
pub struct Builder<'a> {
    draft: CraftDef,
    catalog: &'a PartCatalog,
    undo: Vec<CraftDef>,
    redo: Vec<CraftDef>,
}

impl<'a> Builder<'a> {
    /// An empty draft: the first part placed is its root.
    pub fn new(id: impl Into<String>, catalog: &'a PartCatalog) -> Self {
        Self::of(
            CraftDef {
                id: id.into(),
                name: String::new(),
                parts: Vec::new(),
                sequences: None,
                triggers: Vec::new(),
                rcs_shut: Vec::new(),
                abort: Vec::new(),
            },
            catalog,
        )
    }

    /// A craft to go on with. It is taken as it is: only what is done to it is checked.
    pub fn of(draft: CraftDef, catalog: &'a PartCatalog) -> Self {
        Self {
            draft,
            catalog,
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

    /// Make the edit on a copy; if it is good, keep it and log the old one.
    fn edit(
        &mut self,
        edit: impl FnOnce(&mut CraftDef) -> Result<(), Refusal>,
    ) -> Result<Change, Refusal> {
        let mut next = self.draft.clone();
        edit(&mut next)?;
        let change = Change::between(&self.draft, &next);
        if !next.parts.is_empty() {
            let touched: Vec<&str> = change.touched().map(String::as_str).collect();
            check(&next, self.catalog, &touched)?;
        }
        self.undo.push(std::mem::replace(&mut self.draft, next));
        self.redo.clear();
        Ok(change)
    }

    /// Put `part` where `place` says, or as the root if there is none yet.
    pub fn place(
        &mut self,
        mut part: CraftPartDef,
        place: Option<Place>,
    ) -> Result<Change, Refusal> {
        self.edit(|draft| {
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
    pub fn remove(&mut self, id: &str) -> Result<Change, Refusal> {
        self.edit(|draft| {
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
    pub fn move_to(&mut self, id: &str, place: Place) -> Result<Change, Refusal> {
        self.edit(|draft| {
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
    pub fn rotate(&mut self, id: &str, degrees: f64) -> Result<Change, Refusal> {
        self.edit(|draft| {
            let part = part_mut(draft, id)?;
            part.roll = wrapped(part.roll + degrees);
            Ok(())
        })
    }

    /// Point `id` any way: `direction`, in its own axes, is what lies along the joint.
    pub fn orient(&mut self, id: &str, direction: DVec3) -> Result<Change, Refusal> {
        let catalog = self.catalog;
        self.edit(|draft| {
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
            let node = alone.assembly.parts()[0]
                .node(&name)
                .cloned()
                .ok_or_else(|| {
                    Refusal::new(
                        Rule::Join,
                        format!("`{id}` is not held by a node of its own"),
                    )
                })?;
            part.nodes.retain(|n| n.name != ORIENTED);
            part.nodes.push(NodeDef {
                name: ORIENTED.into(),
                position: node.position.to_array(),
                direction: direction.to_array(),
                size: node.size,
            });
            part.by = ORIENTED.into();
            Ok(())
        })
    }

    /// Make `id` the root: the joints back to the old root are written the other way round, so
    /// that every part is on one nearer it. Joints have no direction, so the parts sit as they did.
    pub fn reroot(&mut self, id: &str) -> Result<Change, Refusal> {
        self.edit(|draft| {
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
        part: CraftPartDef,
        place: Place,
        count: u32,
    ) -> Result<Change, Refusal> {
        let Some(at) = place.at else {
            return Err(Refusal::new(Rule::Join, "a pattern goes on a surface"));
        };
        self.edit(|draft| {
            for k in 0..count {
                let mut copy = part.clone();
                copy.id = format!("{}_{}", part.id, k + 1);
                if draft.parts.iter().any(|p| p.id == copy.id) {
                    return Err(Refusal::new(
                        Rule::Part,
                        format!("it already has a part `{}`", copy.id),
                    ));
                }
                let mut place = place.clone();
                place.at = Some(SurfaceAtDef {
                    u: (at.u + f64::from(k) / f64::from(count)).rem_euclid(1.0),
                    ..at
                });
                place.write(&mut copy);
                draft.parts.push(copy);
            }
            Ok(())
        })
    }

    /// Put a reflection of `id` and all it holds across the plane through the axis of what holds
    /// it, at turn `plane` (0 to 0.5) of that surface: named with `suffix` after each id. They are
    /// plain parts; nothing remembers the mirror.
    pub fn mirror(&mut self, id: &str, plane: f64, suffix: &str) -> Result<Change, Refusal> {
        self.edit(|draft| {
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

/// What a part oriented by hand is held by.
const ORIENTED: &str = "oriented";

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
        let holder = parents[index].map(|(parent, joint)| {
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
        while let Some((parent, joint)) = parents[at] {
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
