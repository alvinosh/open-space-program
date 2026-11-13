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

use std::collections::{BTreeMap, BTreeSet};

use glam::{DQuat, DVec3};
use serde::{Deserialize, Serialize};

use crate::{
    aero::{outer_triangles, voxel::Body},
    assembly::{Mount, Pose, Side},
    component::{BuiltPart, Component, ComponentDef, storage::StorageDef},
    craft::Craft,
    data::{CraftDef, CraftPartDef, PartCatalog, SurfaceAtDef, TankTypeDef},
    fitting,
    geometry::{
        Node,
        preset::{NodeDef, ShapeDef},
    },
    properties,
    shaping::{self, Limits},
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
    /// The game does not allow it yet: bigger than its tech tree allows, a tank type still locked
    /// ([`Allowed`]).
    Allowed,
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

/// What a game lets be built: in a career, procedural parts no bigger than its tech tree allows
/// so far and tanks of the types it has unlocked ([`crate::tech::ScienceTree::allowed`]); in a
/// sandbox, anything. A builder holds one ([`Builder::allow`]): it refuses to make a part bigger
/// than that, or of a locked tank type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Allowed {
    pub sizes: Limits,
    /// The tank types that may be built, the one to make a new tank of first; every one if none.
    pub tank_types: Option<Vec<String>>,
}

/// A length as a player reads it, m: `2`, `2.5`, `1.25`.
fn metres(value: f64) -> String {
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text} m")
}

fn storages(part: &CraftPartDef) -> impl Iterator<Item = &StorageDef> {
    part.components.iter().filter_map(|c| match c {
        ComponentDef::Storage(s) => Some(s),
        _ => None,
    })
}

impl Allowed {
    /// Anything: a sandbox.
    pub fn everything() -> Self {
        Self::default()
    }

    pub fn tank_type(&self, id: &str) -> bool {
        (self.tank_types.as_ref()).is_none_or(|types| types.iter().any(|t| t == id))
    }

    /// Why `part` cannot be built, if it cannot: a size past its limit, a tank type still locked.
    pub fn refuses(&self, part: &CraftPartDef) -> Option<String> {
        let mut why: Vec<String> = (self.sizes.over(part).into_iter())
            .map(|(size, limit)| {
                format!(
                    "{} {}, {} so far",
                    size.label.to_lowercase(),
                    metres(size.value),
                    metres(limit)
                )
            })
            .collect();
        for storage in storages(part).filter(|s| !self.tank_type(&s.tank_type)) {
            why.push(format!("{} tank", storage.tank_type));
        }
        (!why.is_empty()).then(|| why.join(", "))
    }

    /// What changing `before` into `after` does that is not allowed: a size made bigger past its
    /// limit, a tank type it did not have and is still locked. A part that was past a limit may
    /// stay so, or be made smaller.
    pub fn refuses_change(&self, before: &CraftPartDef, after: &CraftPartDef) -> Option<String> {
        let was: BTreeMap<&str, f64> = (shaping::part_sizes(before).into_iter())
            .map(|s| (s.name, s.value))
            .collect();
        for (size, limit) in self.sizes.over(after) {
            if was
                .get(size.name)
                .is_none_or(|v| size.value > v * (1.0 + 1e-9))
            {
                return Some(format!(
                    "the tech tree allows a {} of {} so far",
                    size.label.to_lowercase(),
                    metres(limit)
                ));
            }
        }
        let had: BTreeSet<&str> = storages(before).map(|s| s.tank_type.as_str()).collect();
        (storages(after))
            .find(|s| !self.tank_type(&s.tank_type) && !had.contains(s.tank_type.as_str()))
            .map(|s| format!("{} tanks are still locked in the tech tree", s.tank_type))
    }

    /// `part`, a new one, made so it can be built: sizes past their limits brought back to them,
    /// tanks of locked types made of allowed ones ([`Self::allowed_tank_types`]).
    pub fn bring_within(&self, part: &mut CraftPartDef, catalog: &PartCatalog) {
        self.sizes.shrink(part);
        self.allowed_tank_types(part, catalog);
    }

    /// `part` with each tank of a locked type made of the first allowed type that holds what it
    /// holds and, if it can, is as highly pressurised.
    pub fn allowed_tank_types(&self, part: &mut CraftPartDef, catalog: &PartCatalog) {
        let Some(types) = &self.tank_types else {
            return;
        };
        for component in &mut part.components {
            let ComponentDef::Storage(storage) = component else {
                continue;
            };
            if types.contains(&storage.tank_type) {
                continue;
            }
            let kind = |id: &str| catalog.tank_types.iter().find(|t| t.id == id);
            let holds = |t: &TankTypeDef| {
                storage.contents.keys().all(|name| {
                    (catalog.resources.iter().find(|r| &r.id == name))
                        .is_some_and(|r| crate::component::storage::cannot_hold(t, r).is_none())
                })
            };
            let high = |t: &TankTypeDef| t.pressure >= 1e6;
            let wanted = kind(&storage.tank_type);
            let usable: Vec<&TankTypeDef> = (types.iter())
                .filter_map(|id| kind(id))
                .filter(|t| holds(t) && wanted.is_none_or(|w| w.cast == t.cast))
                .collect();
            let alike = (usable.iter()).find(|t| wanted.is_none_or(|w| high(w) == high(t)));
            if let Some(choice) = alike.or(usable.first()) {
                storage.tank_type = choice.id.clone();
            }
        }
    }
}

/// Parts taken off the craft and left about the editor, not part of it, as KSP leaves detached
/// parts: a part and all it held, the first holding the rest as they were held on the craft, the
/// first held by nothing (its `by`, `at` and `roll` say how it was held). `at` is where their
/// middle is, in the craft's frame from its root, m. They are not built, launched or saved with
/// the craft; they can be put on again or thrown away.
#[derive(Debug, Clone, PartialEq)]
pub struct Loose {
    pub parts: Vec<CraftPartDef>,
    pub at: DVec3,
}

/// What an operation can be undone back to: the draft and what was loose about it.
#[derive(Debug, Clone)]
struct Snapshot {
    draft: CraftDef,
    loose: Vec<Loose>,
}

/// A draft craft and the operations on it. Each operation is checked against the parts in
/// `catalog`, which it is given each time, so a builder can be kept while the content changes.
/// Parts taken off and left about ([`Loose`]) go back and forth with undo and redo as the draft
/// does.
#[derive(Debug, Clone)]
pub struct Builder {
    draft: CraftDef,
    loose: Vec<Loose>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    allowed: Allowed,
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
            loose: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            allowed: Allowed::default(),
        }
    }

    /// Hold its changes to what `allowed` lets be built from now on.
    pub fn allow(&mut self, allowed: Allowed) {
        self.allowed = allowed;
    }

    pub fn allowed(&self) -> &Allowed {
        &self.allowed
    }

    /// Whether changing `id` from as it is in the draft to as it is in `next` is allowed.
    fn allows(&self, next: &CraftDef, id: &str) -> Result<(), Refusal> {
        let find = |draft: &CraftDef| draft.parts.iter().find(|p| p.id == id).cloned();
        if let (Some(before), Some(after)) = (find(&self.draft), find(next))
            && let Some(why) = self.allowed.refuses_change(&before, &after)
        {
            return Err(Refusal::new(Rule::Allowed, why));
        }
        Ok(())
    }

    pub fn draft(&self) -> &CraftDef {
        &self.draft
    }

    /// The parts taken off the craft and left about the editor, each group in the order it came
    /// off.
    pub fn loose(&self) -> &[Loose] {
        &self.loose
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
        self.edit_both(catalog, also, |draft, _| edit(draft))
    }

    /// [`Self::edit_checking`], with what is loose about the craft to change as well.
    fn edit_both(
        &mut self,
        catalog: &PartCatalog,
        also: &BTreeSet<String>,
        edit: impl FnOnce(&mut CraftDef, &mut Vec<Loose>) -> Result<(), Refusal>,
    ) -> Result<Change, Refusal> {
        let mut next = self.draft.clone();
        let mut loose = self.loose.clone();
        edit(&mut next, &mut loose)?;
        let change = Change::between(&self.draft, &next);
        if !next.parts.is_empty() {
            let touched: BTreeSet<&str> =
                (change.touched().chain(also)).map(String::as_str).collect();
            check(&next, catalog, &Vec::from_iter(touched))?;
        }
        let before = Snapshot {
            draft: std::mem::replace(&mut self.draft, next),
            loose: std::mem::replace(&mut self.loose, loose),
        };
        self.undo.push(before);
        self.redo.clear();
        Ok(change)
    }

    /// Keep `next` as the draft, the one it replaces to go back to.
    fn log(&mut self, next: CraftDef) {
        let before = Snapshot {
            draft: std::mem::replace(&mut self.draft, next),
            loose: self.loose.clone(),
        };
        self.undo.push(before);
        self.redo.clear();
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

    /// Take every part of `ids` away, each with everything held by it, as one change: a part
    /// dragged off the craft with its copies by symmetry. See [`removed`].
    pub fn remove_all(
        &mut self,
        catalog: &PartCatalog,
        ids: &BTreeSet<String>,
    ) -> Result<Change, Refusal> {
        let next = removed(&self.draft, ids)?;
        self.edit(catalog, |draft| {
            *draft = next;
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

    /// Turn the whole craft `turn` about its root, in the craft's frame: the root's `points` and
    /// `roll` change, and everything held by it turns with it.
    pub fn turn_root(&mut self, catalog: &PartCatalog, turn: DQuat) -> Result<Change, Refusal> {
        let next = turned_root(&self.draft, turn)?;
        let all: BTreeSet<String> = next.parts.iter().map(|p| p.id.clone()).collect();
        self.edit_checking(catalog, &all, |draft| {
            *draft = next;
            Ok(())
        })
    }

    /// Put `parts` (a part first, then what it holds, held by each other) at each of `places`,
    /// taking `replacing` and what they hold off first: what the editor does with a part in hand
    /// and its symmetry. See [`put_set`]. Tanks still as the parts list gave them that the parts
    /// put on join take what their engines burn ([`properties::fill_from_engines`]). Also the ids
    /// of the parts at each place, top first.
    pub fn put_set(
        &mut self,
        catalog: &PartCatalog,
        parts: &[CraftPartDef],
        places: &[(Place, bool)],
        replacing: &BTreeSet<String>,
    ) -> Result<(Change, Vec<String>), Refusal> {
        let (mut next, tops) = put_set(&self.draft, parts, places, replacing)?;
        let mut carried = BTreeSet::new();
        for top in &tops {
            carried.extend(held_by(&next, top)?);
        }
        let put: Vec<String> = carried.iter().cloned().collect();
        carried.extend(properties::fill_from_engines(&mut next, catalog, &put));
        let change = self.edit_checking(catalog, &carried, |draft| {
            *draft = next;
            Ok(())
        })?;
        Ok((change, tops))
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
        self.allows(&next, id)?;
        if kept {
            check(&next, catalog, &[id])?;
        } else {
            Craft::build(&next, catalog).map_err(|e| Refusal::new(Rule::Join, e.to_string()))?;
        }
        let change = Change::between(&self.draft, &next);
        if !change.changed.is_empty() {
            self.log(next);
        }
        Ok(change)
    }

    /// Set field `field` of `id` (see [`properties`]) from `text`, as the part window does: a size,
    /// its shape, what its tank holds (what the engines it feeds burn), its material, a sizer's
    /// requirement. Checked as [`Builder::resize`] is: only for what the change does.
    pub fn set_field(
        &mut self,
        catalog: &PartCatalog,
        id: &str,
        field: &str,
        text: &str,
    ) -> Result<Change, Refusal> {
        let kept = check(&self.draft, catalog, &[id]).is_ok();
        let mixes = properties::mixes(&self.draft, catalog, id);
        let mut next = self.draft.clone();
        let part = part_mut(&mut next, id)?;
        let had = storages(part)
            .map(|s| s.tank_type.clone())
            .collect::<Vec<_>>();
        properties::set(part, field, text, &mixes, catalog)
            .map_err(|e| Refusal::new(Rule::Part, e))?;
        // A tank given something else to hold may have been given a type to hold it in: one
        // allowed, if the old one cannot.
        if field != "tank_type" && storages(part).any(|s| !had.contains(&s.tank_type)) {
            self.allowed.allowed_tank_types(part, catalog);
        }
        // Another configuration of an engine may burn something else: tanks as the parts list gave
        // them follow it, as they follow an engine put on.
        if field == "configuration" {
            properties::fill_from_engines(&mut next, catalog, &[id.to_owned()]);
        }
        self.allows(&next, id)?;
        if kept {
            check(&next, catalog, &[id])?;
        } else {
            Craft::build(&next, catalog).map_err(|e| Refusal::new(Rule::Join, e.to_string()))?;
        }
        let change = Change::between(&self.draft, &next);
        if !change.changed.is_empty() {
            self.log(next);
        }
        Ok(change)
    }

    /// Say what each of these fuel lines feeds, as one operation: `(line, part it feeds)`. A line
    /// feeds a tank or an engine other than what it is on.
    pub fn link_fuel_lines(
        &mut self,
        catalog: &PartCatalog,
        links: &[(String, String)],
    ) -> Result<Change, Refusal> {
        let mut next = self.draft.clone();
        for (line, to) in links {
            let target = (next.parts.iter()).find(|p| &p.id == to);
            let takes =
                |c: &ComponentDef| matches!(c, ComponentDef::Storage(_) | ComponentDef::Engine(_));
            let is_engine = |p: &CraftPartDef| {
                (p.template.as_ref()).is_some_and(|t| catalog.engines.iter().any(|e| &e.id == t))
            };
            let takes_propellant =
                target.is_some_and(|p| p.components.iter().any(takes) || is_engine(p));
            if !takes_propellant {
                return Err(Refusal::new(
                    Rule::Part,
                    "a fuel line feeds a tank or an engine",
                ));
            }
            let part = part_mut(&mut next, line)?;
            if part
                .on
                .as_deref()
                .and_then(|on| on.split_once('.'))
                .map(|(h, _)| h)
                == Some(to.as_str())
            {
                return Err(Refusal::new(
                    Rule::Part,
                    "a fuel line feeds another part than the one it is on",
                ));
            }
            let found = part.components.iter_mut().find_map(|c| match c {
                ComponentDef::FuelLine(def) => Some(def),
                _ => None,
            });
            let Some(def) = found else {
                return Err(Refusal::new(
                    Rule::Part,
                    format!("`{line}` is not a fuel line"),
                ));
            };
            def.to = to.clone();
        }
        Craft::build(&next, catalog).map_err(|e| Refusal::new(Rule::Join, e.to_string()))?;
        let change = Change::between(&self.draft, &next);
        if !change.changed.is_empty() {
            self.log(next);
        }
        Ok(change)
    }

    /// Take `ids` off the craft, each with everything held by it, as [`Builder::remove_all`]
    /// does, but leave `kept` (one of them) and what it holds about the editor at `at`, loose
    /// ([`Loose`]): a part dragged off the craft, whose copies by symmetry go. One change.
    pub fn detach(
        &mut self,
        catalog: &PartCatalog,
        ids: &BTreeSet<String>,
        kept: &str,
        at: DVec3,
    ) -> Result<Change, Refusal> {
        if root(&self.draft) == Some(kept) {
            return Err(Refusal::new(
                Rule::Part,
                format!("`{kept}` is the root: it holds the craft"),
            ));
        }
        let group = held_by(&self.draft, kept)?;
        let mut parts: Vec<CraftPartDef> = (self.draft.parts.iter())
            .filter(|p| group.contains(&p.id))
            .cloned()
            .collect();
        // The part first: it holds the rest.
        parts.sort_by_key(|p| p.id != kept);
        parts[0].on = None;
        let mut ids = ids.clone();
        ids.insert(kept.to_owned());
        let next = removed(&self.draft, &ids)?;
        self.edit_both(catalog, &BTreeSet::new(), |draft, loose| {
            *draft = next;
            loose.push(Loose { parts, at });
            Ok(())
        })
    }

    /// Put the loose group `index` on the craft: `parts` (the group as [`fit_in`] names it) at
    /// each of `places`, as [`Builder::put_set`] does, the group no longer loose. One change.
    /// Also the ids of the parts at each place, top first.
    pub fn put_loose(
        &mut self,
        catalog: &PartCatalog,
        index: usize,
        parts: &[CraftPartDef],
        places: &[(Place, bool)],
    ) -> Result<(Change, Vec<String>), Refusal> {
        if index >= self.loose.len() {
            return Err(Refusal::new(Rule::Part, "there is no such loose part"));
        }
        let (mut next, tops) = put_set(&self.draft, parts, places, &BTreeSet::new())?;
        let mut carried = BTreeSet::new();
        for top in &tops {
            carried.extend(held_by(&next, top)?);
        }
        let put: Vec<String> = carried.iter().cloned().collect();
        carried.extend(properties::fill_from_engines(&mut next, catalog, &put));
        let change = self.edit_both(catalog, &carried, |draft, loose| {
            *draft = next;
            loose.remove(index);
            Ok(())
        })?;
        Ok((change, tops))
    }

    /// Leave the loose group `index` somewhere else about the editor, its middle at `at`.
    pub fn move_loose(&mut self, index: usize, at: DVec3) -> Result<(), Refusal> {
        self.change_loose(|loose| loose.get_mut(index).map(|group| group.at = at).is_some())
    }

    /// Throw the loose group `index` away.
    pub fn throw_loose(&mut self, index: usize) -> Result<(), Refusal> {
        self.change_loose(|loose| (index < loose.len()).then(|| loose.remove(index)).is_some())
    }

    /// Change what is loose about the craft, the craft as it is: one change, if `change` finds
    /// what it changes.
    fn change_loose(
        &mut self,
        change: impl FnOnce(&mut Vec<Loose>) -> bool,
    ) -> Result<(), Refusal> {
        let mut loose = self.loose.clone();
        if !change(&mut loose) {
            return Err(Refusal::new(Rule::Part, "there is no such loose part"));
        }
        let before = Snapshot {
            draft: self.draft.clone(),
            loose: std::mem::replace(&mut self.loose, loose),
        };
        self.undo.push(before);
        self.redo.clear();
        Ok(())
    }

    /// Go back one operation; `false` if there is none to go back over.
    pub fn undo(&mut self) -> bool {
        let Some(before) = self.undo.pop() else {
            return false;
        };
        let now = self.restore(before);
        self.redo.push(now);
        true
    }

    /// Do again what [`Builder::undo`] went back over.
    pub fn redo(&mut self) -> bool {
        let Some(after) = self.redo.pop() else {
            return false;
        };
        let now = self.restore(after);
        self.undo.push(now);
        true
    }

    /// Go to `snapshot`, giving back what there was.
    fn restore(&mut self, snapshot: Snapshot) -> Snapshot {
        Snapshot {
            draft: std::mem::replace(&mut self.draft, snapshot.draft),
            loose: std::mem::replace(&mut self.loose, snapshot.loose),
        }
    }
}

/// `part` held where `place` says: what [`Builder::place`] puts in the craft.
pub fn held_at(mut part: CraftPartDef, place: &Place) -> CraftPartDef {
    place.write(&mut part);
    part
}

/// `draft` with the whole craft turned `turn` about its root, in the craft's frame: what
/// [`Builder::turn_root`] makes, without the rules.
pub fn turned_root(draft: &CraftDef, turn: DQuat) -> Result<CraftDef, Refusal> {
    let mut next = draft.clone();
    let root = (next.parts.iter_mut())
        .find(|p| p.on.is_none())
        .ok_or_else(|| Refusal::new(Rule::Part, "there is no root to turn"))?;
    let now = crate::craft::root_rotation(root).ok_or_else(|| {
        Refusal::new(Rule::Join, format!("the root `{}` points nowhere", root.id))
    })?;
    (root.points, root.roll) = crate::craft::root_turn((turn * now).normalize());
    Ok(next)
}

/// `draft` with `parts` (a part first, then what it holds, each held by one before it) put at each
/// of `places`, without the rules: first `replacing` and everything they hold are taken off. At
/// the first place the parts keep their ids; at the others they are copies, each id made new from
/// its own (`<id>_2` up). A place marked mirrored has the mirror image of them all: each shape
/// reflected, its own nodes too, and what is held on a surface at the reflected place, as
/// [`Builder::mirror`] makes them; the place itself is as given, for the reflected part. Also the
/// ids of the parts at each place, top first.
pub fn put_set(
    draft: &CraftDef,
    parts: &[CraftPartDef],
    places: &[(Place, bool)],
    replacing: &BTreeSet<String>,
) -> Result<(CraftDef, Vec<String>), Refusal> {
    let Some(top) = parts.first() else {
        return Err(Refusal::new(Rule::Part, "there is nothing to put on"));
    };
    let mut next = draft.clone();
    let mut gone = BTreeSet::new();
    for id in replacing {
        if next.parts.iter().any(|p| &p.id == id) {
            gone.extend(held_by(&next, id)?);
        }
    }
    if gone.len() == next.parts.len() && !gone.is_empty() && !places.is_empty() {
        return Err(Refusal::new(
            Rule::Part,
            format!("`{}` holds the rest: it cannot be put on anything", top.id),
        ));
    }
    next.parts.retain(|p| !gone.contains(&p.id));
    let kept: BTreeSet<&str> = parts.iter().map(|p| p.id.as_str()).collect();
    let forgotten: BTreeSet<String> = (gone.iter())
        .filter(|id| !kept.contains(id.as_str()))
        .cloned()
        .collect();
    forget(&mut next, &forgotten);
    let mut tops = Vec::new();
    for (k, (place, mirrored)) in places.iter().enumerate() {
        let mut names = std::collections::BTreeMap::new();
        for part in parts {
            let id = if k == 0 {
                part.id.clone()
            } else {
                fresh_id(&next, &part.id)
            };
            if next.parts.iter().any(|p| p.id == id) {
                return Err(Refusal::new(
                    Rule::Part,
                    format!("it already has a part `{id}`"),
                ));
            }
            names.insert(part.id.clone(), id.clone());
            let mut copy = if *mirrored {
                reflected(part, 0.0)
            } else {
                part.clone()
            };
            copy.id = id;
            if part.id == top.id {
                // A part of its own, not the root any more: it points as its joint says.
                copy.points = None;
                place.write(&mut copy);
                tops.push(copy.id.clone());
            } else {
                copy.on = (copy.on.as_ref()).map(|on| match on.split_once('.') {
                    Some((held, connector)) => match names.get(held) {
                        Some(renamed) => format!("{renamed}.{connector}"),
                        None => on.clone(),
                    },
                    None => on.clone(),
                });
            }
            next.parts.push(copy);
        }
    }
    Ok((next, tops))
}

/// `parts`, a loose group (a part first, then what it holds), ready to go on `draft` again: each
/// part whose id `draft` now has takes a new one, and what it holds follows it.
pub fn fit_in(draft: &CraftDef, parts: &[CraftPartDef]) -> Vec<CraftPartDef> {
    let mut taken: CraftDef = draft.clone();
    let mut names = std::collections::BTreeMap::new();
    for part in parts {
        if taken.parts.iter().any(|p| p.id == part.id) {
            let id = fresh_id(&taken, &part.id);
            names.insert(part.id.clone(), id);
        }
        let mut stand_in = part.clone();
        stand_in.id = names
            .get(&part.id)
            .cloned()
            .unwrap_or_else(|| part.id.clone());
        taken.parts.push(stand_in);
    }
    (parts.iter())
        .map(|part| {
            let mut part = part.clone();
            if let Some(id) = names.get(&part.id) {
                part.id = id.clone();
            }
            part.on = (part.on.as_ref()).map(|on| match on.split_once('.') {
                Some((held, connector)) => match names.get(held) {
                    Some(renamed) => format!("{renamed}.{connector}"),
                    None => on.clone(),
                },
                None => on.clone(),
            });
            part
        })
        .collect()
}

/// `draft` without the parts `ids`, each with everything held by it, and out of what its
/// sequences do. The root holds everything, so it goes with all of it.
pub fn removed(draft: &CraftDef, ids: &BTreeSet<String>) -> Result<CraftDef, Refusal> {
    let mut next = draft.clone();
    let mut gone = BTreeSet::new();
    for id in ids {
        if next.parts.iter().any(|p| &p.id == id) {
            gone.extend(held_by(&next, id)?);
        }
    }
    next.parts.retain(|p| !gone.contains(&p.id));
    forget(&mut next, &gone);
    Ok(next)
}

/// An id for a copy of the part `id`, not yet in `draft`: `<id>_2` up, after any number it
/// already ends with.
fn fresh_id(draft: &CraftDef, id: &str) -> String {
    let stem = match id.rsplit_once('_') {
        Some((stem, n)) if !stem.is_empty() && n.parse::<u32>().is_ok() => stem,
        _ => id,
    };
    (2..)
        .map(|n| format!("{stem}_{n}"))
        .find(|candidate| !draft.parts.iter().any(|p| &p.id == candidate))
        .unwrap_or_else(|| format!("{stem}_copy"))
}

/// The name of the node of its own a part is held by on its side ([`by_side`]).
pub const SIDE: &str = "held_side";

/// `part` as it would be alone, the root of a craft of its own.
pub fn alone(part: &CraftPartDef, catalog: &PartCatalog) -> Result<Craft, Refusal> {
    let mut part = part.clone();
    part.id = "alone".into();
    (part.on, part.at, part.points, part.roll) = (None, None, None, 0.0);
    part.nodes.retain(|n| !n.name.starts_with(HELD));
    let alone = CraftDef {
        id: "alone".into(),
        name: String::new(),
        parts: vec![part],
        sequences: None,
        triggers: Vec::new(),
        rcs_shut: Vec::new(),
        abort: Vec::new(),
    };
    Craft::build(&alone, catalog).map_err(|e| Refusal::new(Rule::Join, e.to_string()))
}

/// Whether `part` lets go of what is on its nodes: a decoupler, which goes on a surface by its
/// top, standing out of it, to hold a booster there.
pub fn lets_go(alone: &Craft) -> bool {
    (alone.parts[0].built.components.iter()).any(|c| matches!(c, Component::Decoupler(_)))
}

/// The node of its own a part goes on by its side ([`SIDE`]): on its outside at its -X, half way
/// up, facing out, a point that takes any size. `alone` is the part built on its own.
pub fn side_node(alone: &Craft) -> NodeDef {
    let built = &alone.parts[0].built;
    let mut triangles = Vec::new();
    outer_triangles(built, |p| p, &mut triangles);
    let (mut lo, mut hi) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
    for corner in triangles.iter().flatten() {
        lo = lo.min(*corner);
        hi = hi.max(*corner);
    }
    if !lo.is_finite() {
        (lo, hi) = (DVec3::splat(-0.5), DVec3::splat(0.5));
    }
    let middle = (lo + hi) / 2.0;
    let geometry = &alone.assembly.parts()[0].geometry;
    // Where its outside is nearest a point well off its -X side, if that faces that way.
    let probe = DVec3::new(lo.x - (hi - lo).max_element() - 1.0, middle.y, middle.z);
    let on_wall = (geometry.surface_near(probe))
        .and_then(|(surface, u, v, _)| geometry.surface_point(&surface, u, v))
        .filter(|p| p.normal.x < -0.7);
    let (position, direction) = match on_wall {
        Some(p) => (p.position, DVec3::NEG_X),
        None => (DVec3::new(lo.x, middle.y, middle.z), DVec3::NEG_X),
    };
    let tidy = |v: DVec3| v.to_array().map(|x| (x * 1e9).round() / 1e9 + 0.0);
    NodeDef {
        name: SIDE.into(),
        position: tidy(position),
        direction: tidy(direction),
        size: 0.0,
    }
}

/// Whether the part, built on its own, is a landing gear leg: it goes on rolling along the craft.
pub fn rolls(alone: &Craft) -> bool {
    (alone.parts[0].built.components.iter()).any(|c| matches!(c, Component::Gear(_)))
}

/// How far a part held at `node` (its own, in its frame) on connector `near` of the part at
/// `holder` in `craft` must be turned about the joint to stand upright: its own `axis` (+Z for a
/// tank, +Y for a wheel, which rolls along it) along the craft's +Z, as near as the connector lets
/// it. Degrees.
pub fn upright_roll(
    craft: &Craft,
    holder: usize,
    near: &Mount,
    node: &NodeDef,
    axis: DVec3,
) -> Option<f64> {
    let frame = craft.assembly.parts()[holder].frame(near)?;
    let held = Node {
        name: node.name.clone(),
        position: DVec3::from(node.position),
        direction: DVec3::from(node.direction),
        size: node.size,
    };
    let far = crate::assembly::node_frame(&held);
    let connector = craft.assembly.poses()[holder] * frame;
    // The part's +Z, turned no further, in the craft's frame, and the way it should go: both
    // square to the line out of the connector, which the turn is about.
    let out = connector.rotation * DVec3::Z;
    let unturned = (connector * crate::assembly::meet(Pose::IDENTITY, far, 0.0)).rotation * axis;
    let up = DVec3::Z.reject_from(out);
    let up = if up.length() > 1e-6 {
        up.normalize()
    } else {
        // Standing on an end of the craft: along the connector's own +Y.
        connector.rotation * DVec3::Y
    };
    let unturned = unturned.reject_from(out).try_normalize()?;
    let angle = out.dot(unturned.cross(up)).atan2(unturned.dot(up));
    Some(wrapped((angle.to_degrees() * 1e6).round() / 1e6))
}

/// Where on surface `surface` of the part at `holder` in `craft` the mirror image of what is held
/// at `at` goes, across the craft's middle plane (+Y and +Z, which a plane's nose and belly lie
/// in): its `u` there, if the part is its own mirror image across that plane, and the image is
/// somewhere else than the original.
pub fn mirrored_u(craft: &Craft, holder: usize, surface: &str, at: SurfaceAtDef) -> Option<f64> {
    let part = &craft.assembly.parts()[holder];
    let pose = craft.assembly.poses()[holder];
    let point = part.geometry.surface_point(surface, at.u, at.v)?.position;
    let here = pose.point(point);
    let there = DVec3::new(-here.x, here.y, here.z);
    if here.distance(there) < 0.01 {
        return None;
    }
    let local = pose.inverse().point(there);
    let (found, u, v, off) = part.geometry.surface_near(local)?;
    let again = part.geometry.surface_point(&found, u, v)?.position;
    (found == surface
        && off < 0.01
        && (v - at.v).abs() < 1e-3
        && pose.point(again).distance(there) < 0.01)
        .then(|| (u * 1e9).round() / 1e9)
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
/// and the same in every other way, or the same as its reflection; and those held the same way by
/// the parts put on with its holder by symmetry (a nose cone on each of four boosters). Nothing
/// remembers a pattern or a mirror, so they are found by matching.
pub fn alike(draft: &CraftDef, id: &str) -> Vec<(String, bool)> {
    counterparts(draft, id, 0)
}

/// [`alike`], `depth` holders down.
fn counterparts(draft: &CraftDef, id: &str, depth: usize) -> Vec<(String, bool)> {
    let Some(part) = draft.parts.iter().find(|p| p.id == id) else {
        return Vec::new();
    };
    let Some((holder, connector)) = part.on.as_deref().and_then(|on| on.split_once('.')) else {
        return Vec::new();
    };
    // Its holder and the holder's own, each with whether it is the holder's mirror image.
    let mut holders = vec![(holder.to_owned(), false)];
    if depth < 32 {
        holders.extend(counterparts(draft, holder, depth + 1));
    }
    let key = |p: &CraftPartDef| {
        let mut p = p.clone();
        p.id.clear();
        p.name = None;
        p.on =
            p.on.and_then(|on| on.split_once('.').map(|(_, c)| c.to_owned()));
        // Copies of a fuel line feed copies of what it feeds.
        for component in &mut p.components {
            if let ComponentDef::FuelLine(line) = component {
                line.to.clear();
            }
        }
        let tidy = |x: &mut f64, per: f64| *x = (*x * per).round() / per + 0.0;
        if let Some(at) = &mut p.at {
            at.u = 0.0;
            tidy(&mut at.v, 1e6);
            tidy(&mut at.standoff, 1e6);
        }
        tidy(&mut p.roll, 1e4);
        for node in &mut p.nodes {
            for x in node.position.iter_mut().chain(node.direction.iter_mut()) {
                tidy(x, 1e6);
            }
        }
        p
    };
    let (same, mirror) = (key(part), key(&reflected(part, 0.0)));
    (draft.parts.iter())
        .filter(|p| p.id != id)
        .filter_map(|p| {
            let (on, at) = p.on.as_deref()?.split_once('.')?;
            if at != connector || !holders.iter().any(|(h, _)| h == on) {
                return None;
            }
            // Two parts cannot share a node: on its own holder, only a surface has others.
            if on == holder && part.at.is_none() {
                return None;
            }
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
/// and what has no size (a point, like the tip of a nose) holds and is held by anything. So does an
/// engine's front, a thrust structure: a tank on an engine is as good as the engine under the tank,
/// whichever of the two holds the other.
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
    if holds > 0.0 && held > NODE_RATIO * holds && !fitting::engine_front(craft, parent, near) {
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
        crate::fitting::fit(&mut part, &crate::fitting::Against::node(2.0), catalog);
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
                axial: false,
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

    /// A career's builder makes nothing bigger than its tree allows, nor a tank of a locked type,
    /// and a new part is brought within both.
    #[test]
    fn a_career_holds_sizes_and_tank_types_to_what_it_has_unlocked() {
        let catalog = catalog();
        let mut builder = stack(&catalog);
        let types = |ids: &[&str]| Some(ids.iter().map(|t| t.to_string()).collect());
        builder.allow(Allowed {
            sizes: Limits {
                diameter: 2.5,
                length: f64::INFINITY,
            },
            tank_types: types(&["Stiffened Al 2219", "Balloon Stainless", "COPV"]),
        });
        builder
            .resize(&catalog, "top", &[("diameter", 2.5)])
            .unwrap();
        let wider = builder.resize(&catalog, "top", &[("diameter", 3.0)]);
        assert_eq!(wider.as_ref().map_err(|r| r.rule), Err(Rule::Allowed));
        assert_eq!(
            wider.unwrap_err().message,
            "the tech tree allows a diameter of 2.5 m so far"
        );
        // Past the limit already, it may stay so or shrink, not grow.
        builder.allow(Allowed {
            sizes: Limits {
                diameter: 2.0,
                length: f64::INFINITY,
            },
            ..builder.allowed().clone()
        });
        builder.resize(&catalog, "top", &[("length", 5.0)]).unwrap();
        builder
            .resize(&catalog, "top", &[("diameter", 2.2)])
            .unwrap();
        let wider = builder.resize(&catalog, "top", &[("diameter", 2.3)]);
        assert_eq!(wider.map_err(|r| r.rule), Err(Rule::Allowed));
        // A locked tank type is refused, an unlocked one taken.
        let locked = builder.set_field(&catalog, "top", "tank_type", "Isogrid Al-Li");
        assert_eq!(locked.map_err(|r| r.rule), Err(Rule::Allowed));
        builder
            .set_field(&catalog, "top", "tank_type", "Balloon Stainless")
            .unwrap();
        // A new tank of a locked type is made of the first allowed one as lightly pressurised.
        let mut new = entry("tank", &catalog);
        resize_part(&mut new, "diameter", 4.0);
        builder.allow(Allowed {
            tank_types: types(&["COPV", "Balloon Stainless"]),
            ..builder.allowed().clone()
        });
        builder.allowed().bring_within(&mut new, &catalog);
        assert!(builder.allowed().refuses(&new).is_none());
        let kind = (new.components.iter()).find_map(|c| match c {
            ComponentDef::Storage(s) => Some(s.tank_type.as_str()),
            _ => None,
        });
        assert_eq!(kind, Some("Balloon Stainless"));
    }

    fn resize_part(part: &mut CraftPartDef, size: &str, value: f64) {
        shaping::resize(part, size, value).unwrap();
    }

    #[test]
    fn a_tank_is_filled_with_what_the_engine_on_the_craft_burns() {
        let catalog = catalog();
        let mut builder = stack(&catalog);
        let mut engine = entry("engine:K-0", &catalog);
        engine.id = "engine".into();
        let under = Place::node("lower.bottom", "top");
        builder.place(&catalog, engine, Some(under)).unwrap();
        // Nothing on it burns hydrogen.
        let hydrolox = builder.set_field(&catalog, "top", "contents", "hydrolox");
        assert_eq!(hydrolox.map_err(|r| r.rule), Err(Rule::Part));
        // Kerolox is the K-0's, 2.4 parts oxygen to one of kerosene.
        builder
            .set_field(&catalog, "top", "contents", "kerolox")
            .unwrap();
        let top = (builder.draft().parts.iter())
            .find(|p| p.id == "top")
            .unwrap();
        let held = top.components.iter().find_map(|c| match c {
            crate::component::ComponentDef::Storage(s) => Some(&s.contents),
            _ => None,
        });
        let held = held.unwrap();
        assert!(
            (held["LqdOxygen"] / held["Kerosene"] - 2.4).abs() < 1e-9,
            "{held:?}"
        );
    }

    #[test]
    fn an_engine_on_a_tanks_side_burns_what_the_tank_holds() {
        let catalog = catalog();
        let mut builder = stack(&catalog);
        let craft = Craft::build(builder.draft(), &catalog).unwrap();
        let lower = craft.find("lower").unwrap();
        let mut engine = entry("engine:K-0", &catalog);
        engine.id = "side_engine".into();
        let node = side_node(&alone(&engine, &catalog).unwrap());
        engine.nodes.push(node.clone());
        let at = SurfaceAtDef {
            u: 0.3,
            v: 0.5,
            side: Side::Front,
            standoff: 0.0,
            axial: false,
        };
        let mount = Mount::Surface {
            surface: "barrel".into(),
            side: Side::Front,
            u: at.u,
            v: at.v,
            standoff: 0.0,
            axial: false,
        };
        let mut place = Place::surface("lower.barrel", SIDE, at);
        place.roll = upright_roll(&craft, lower, &mount, &node, DVec3::Z).unwrap();
        builder
            .put_set(&catalog, &[engine], &[(place, false)], &BTreeSet::new())
            .unwrap();
        // The tank it is on, and the one over that, are filled for it and offer what it burns.
        let draft = builder.draft();
        let held = |id: &str| {
            let part = draft.parts.iter().find(|p| p.id == id).unwrap();
            (part.components.iter())
                .find_map(|c| match c {
                    crate::component::ComponentDef::Storage(s) => Some(s.contents.clone()),
                    _ => None,
                })
                .unwrap()
        };
        for tank in ["lower", "top"] {
            let contents = held(tank);
            let ratio = contents["LqdOxygen"] / contents["Kerosene"];
            assert!((ratio - 2.4).abs() < 1e-9, "{tank}: {ratio}");
            let offered = properties::mixes(draft, &catalog, tank);
            assert_eq!(offered.len(), 1, "{tank}: {offered:?}");
        }
        // In flight the tanks feed it.
        let propulsion = crate::propulsion::Propulsion::from_craft(draft, &catalog).unwrap();
        let index = draft
            .parts
            .iter()
            .position(|p| p.id == "side_engine")
            .unwrap();
        let engine = crate::parts::PartId(index as u32);
        assert!(propulsion.reachable(engine, "Kerosene") > 0.0);
    }

    /// A fin set dragged off is left about as one loose fin, its copies gone; it goes back on
    /// four times round, is moved and thrown away, and undo and redo walk back through each step.
    #[test]
    fn parts_taken_off_stay_loose_until_put_back_or_thrown_away() {
        let catalog = catalog();
        let mut builder = stack(&catalog);
        let full = builder.draft().clone();
        let fins: BTreeSet<String> = (full.parts.iter())
            .filter(|p| p.id.starts_with("fin"))
            .map(|p| p.id.clone())
            .collect();
        assert_eq!(fins.len(), 4);
        let kept = fins.iter().next().unwrap().clone();
        let at = DVec3::new(3.0, 0.0, 1.0);
        builder.detach(&catalog, &fins, &kept, at).unwrap();
        assert_eq!(builder.draft().parts.len(), full.parts.len() - 4);
        let loose = builder.loose().to_vec();
        assert_eq!(loose.len(), 1);
        assert_eq!(loose[0].at, at);
        assert_eq!(loose[0].parts.len(), 1);
        assert_eq!(loose[0].parts[0].id, kept);
        assert!(loose[0].parts[0].on.is_none());
        // The root holds the craft: it cannot be left about.
        let root = BTreeSet::from(["top".to_owned()]);
        assert!(builder.detach(&catalog, &root, "top", at).is_err());

        assert!(builder.undo());
        assert_eq!(builder.draft().parts, full.parts);
        assert!(builder.loose().is_empty());
        assert!(builder.redo());
        assert_eq!(builder.loose(), loose.as_slice());

        // A new part has taken the loose fin's id since: it goes back on under a new one.
        let mut other = entry("fin", &catalog);
        other.id = kept.clone();
        let (by, roll) = palette::entries(&catalog)
            .into_iter()
            .find(|e| e.id == "fin")
            .unwrap()
            .surface
            .unwrap();
        let place_at = |u: f64| {
            let mut place = Place::surface(
                "lower.barrel",
                by.clone(),
                SurfaceAtDef {
                    u,
                    v: 0.5,
                    side: Side::Front,
                    standoff: 0.0,
                    axial: false,
                },
            );
            place.roll = roll;
            place
        };
        builder
            .place(&catalog, other, Some(place_at(0.125)))
            .unwrap();
        let group = fit_in(builder.draft(), &builder.loose()[0].parts);
        assert_ne!(group[0].id, kept);
        let places: Vec<(Place, bool)> = (0..4)
            .map(|k| (place_at(0.25 * f64::from(k)), false))
            .collect();
        let before = builder.draft().parts.len();
        builder.put_loose(&catalog, 0, &group, &places).unwrap();
        assert_eq!(builder.draft().parts.len(), before + 4);
        assert!(builder.loose().is_empty());
        assert!(builder.undo());
        assert_eq!(builder.loose(), loose.as_slice());

        builder.move_loose(0, DVec3::new(-2.0, 0.0, 0.0)).unwrap();
        assert_eq!(builder.loose()[0].at, DVec3::new(-2.0, 0.0, 0.0));
        builder.throw_loose(0).unwrap();
        assert!(builder.loose().is_empty());
        assert!(builder.undo());
        assert!(builder.undo());
        assert_eq!(builder.loose(), loose.as_slice());
        assert!(builder.throw_loose(3).is_err());
    }

    /// Built from the engine up, a tank much wider than the engine goes on its front, as the engine
    /// goes under the tank built the other way; a part much wider than a probe core's node still
    /// may not hang from it.
    #[test]
    fn an_engine_s_front_holds_a_tank_of_any_width() {
        let catalog = catalog();
        let mut builder = Builder::new("test");
        let mut engine = entry("engine:K-0", &catalog);
        engine.id = "engine".into();
        builder.place(&catalog, engine, None).unwrap();
        let mut tank = entry("tank", &catalog);
        tank.id = "tank".into();
        let on = Place::node("engine.top", "bottom");
        builder.place(&catalog, tank, Some(on)).unwrap();

        let mut builder = Builder::new("test");
        let mut core = entry("probe_core", &catalog);
        core.id = "core".into();
        builder.place(&catalog, core, None).unwrap();
        let mut tank = entry("tank", &catalog);
        tank.id = "tank".into();
        let under = Place::node("core.bottom", "top");
        let hung = builder.place(&catalog, tank, Some(under));
        assert_eq!(hung.map_err(|r| r.rule), Err(Rule::Node));
    }

    /// Four engines put under a tank's domed end in one go, along its axis round it: they stand
    /// upright side by side at one height, the tank feeds them, and one undo takes all four off.
    #[test]
    fn four_engines_go_under_a_tanks_end_along_its_axis() {
        let catalog = catalog();
        let mut builder = stack(&catalog);
        let parts = builder.draft().parts.len();
        let mut engine = entry("engine:K-0", &catalog);
        engine.id = "cluster".into();
        let places: Vec<(Place, bool)> = (0..4)
            .map(|k| {
                let at = SurfaceAtDef {
                    u: f64::from(k) * 0.25,
                    v: 0.4,
                    side: Side::Front,
                    standoff: 0.0,
                    axial: true,
                };
                (Place::surface("lower.aft_dome", "top", at), false)
            })
            .collect();
        let (_, tops) = builder
            .put_set(&catalog, &[engine], &places, &BTreeSet::new())
            .unwrap();
        assert_eq!(tops.len(), 4);
        let draft = builder.draft().clone();
        assert_eq!(draft.parts.len(), parts + 4);
        let craft = Craft::build(&draft, &catalog).unwrap();
        let poses = craft.assembly.poses();
        let mut heights = Vec::new();
        let mut spots = Vec::new();
        for id in &tops {
            let pose = poses[craft.find(id).unwrap()];
            // Upright: its nozzle down the craft's axis.
            let up = pose.rotation * DVec3::Z;
            assert!(up.dot(DVec3::Z) > 0.999, "{id}: {up}");
            heights.push(pose.position.z);
            spots.push(pose.position.truncate());
        }
        assert!(
            heights.iter().all(|h| (h - heights[0]).abs() < 1e-6),
            "{heights:?}"
        );
        // Apart, round the axis.
        let r = spots[0].length();
        assert!(r > 0.1 && spots.iter().all(|s| (s.length() - r).abs() < 1e-6));
        assert!(spots[0].distance(spots[1]) > 0.25, "{spots:?}");
        let propulsion = crate::propulsion::Propulsion::from_craft(&draft, &catalog).unwrap();
        for id in &tops {
            let index = draft.parts.iter().position(|p| &p.id == id).unwrap();
            let engine = crate::parts::PartId(index as u32);
            assert!(propulsion.reachable(engine, "Kerosene") > 0.0, "{id}");
        }
        assert!(builder.undo());
        assert_eq!(builder.draft().parts.len(), parts);
    }

    #[test]
    fn a_part_and_its_copies_come_off_together_and_go_back_with_one_undo() {
        let catalog = catalog();
        let mut builder = stack(&catalog);
        let before = builder.draft().parts.clone();
        let first = (builder.draft().parts.iter())
            .find(|p| p.id.starts_with("fin"))
            .map(|p| p.id.clone())
            .unwrap();
        let mut fins = BTreeSet::from([first.clone()]);
        fins.extend(alike(builder.draft(), &first).into_iter().map(|(c, _)| c));
        assert_eq!(fins.len(), 4);
        let change = builder.remove_all(&catalog, &fins).unwrap();
        assert_eq!(change.removed.len(), 4);
        let left: Vec<&str> = (builder.draft().parts.iter())
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(left, ["top", "lower"]);
        assert!(builder.undo());
        assert_eq!(builder.draft().parts, before);
        // The lower tank goes alone; the root holds everything and takes it all.
        let lower = BTreeSet::from(["lower".to_owned()]);
        assert_eq!(removed(builder.draft(), &lower).unwrap().parts.len(), 5);
        let root = BTreeSet::from(["top".to_owned()]);
        assert!(removed(builder.draft(), &root).is_ok_and(|d| d.parts.is_empty()));
    }

    #[test]
    fn an_engine_put_under_new_tanks_fills_them_and_one_undo_takes_both_back() {
        let catalog = catalog();
        let mut builder = stack(&catalog);
        let before = builder.draft().parts.clone();
        let mut engine = entry("engine:K-0", &catalog);
        engine.id = "engine".into();
        let under = Place::node("lower.bottom", "top");
        let (change, _) = builder
            .put_set(&catalog, &[engine], &[(under, false)], &BTreeSet::new())
            .unwrap();
        // Both tanks feed it, through each other: both hold its 2.4 to one.
        let ratio = |id: &str| {
            let part = (builder.draft().parts.iter()).find(|p| p.id == id).unwrap();
            let held = (part.components.iter())
                .find_map(|c| match c {
                    crate::component::ComponentDef::Storage(s) => Some(s.contents.clone()),
                    _ => None,
                })
                .unwrap();
            held["LqdOxygen"] / held["Kerosene"]
        };
        assert!((ratio("top") - 2.4).abs() < 1e-9, "{}", ratio("top"));
        assert!((ratio("lower") - 2.4).abs() < 1e-9, "{}", ratio("lower"));
        assert!(change.changed.contains(&"top".to_owned()), "{change:?}");
        assert!(builder.undo());
        assert_eq!(builder.draft().parts, before);
    }

    #[test]
    fn the_root_turns_and_everything_turns_with_it() {
        let catalog = catalog();
        let mut builder = stack(&catalog);
        let before = poses(&builder, &catalog);
        // Laid down, nose along +X, then rolled: everything goes round the root with it.
        let turn =
            DQuat::from_rotation_y(std::f64::consts::FRAC_PI_2) * DQuat::from_rotation_z(0.3);
        builder.turn_root(&catalog, turn).unwrap();
        let root = &builder.draft().parts[0];
        assert!(root.points.is_some());
        let after = poses(&builder, &catalog);
        for (id, was) in &before {
            let now = pose(&after, id);
            assert!(near(now.position, turn * was.position), "{id}");
            assert!(
                now.rotation.angle_between(turn * was.rotation) < 1e-6,
                "{id}"
            );
        }
        // Written and read back, a turn is the same turn.
        let (points, roll) = crate::craft::root_turn(turn);
        let again = CraftPartDef {
            points,
            roll,
            ..root.clone()
        };
        let read = crate::craft::root_rotation(&again).unwrap();
        // Its roll is kept to a millionth of a degree.
        assert!(read.angle_between(turn) < 1e-7);
        // Turned back, it is as it was, and nothing is written for no turn.
        builder.turn_root(&catalog, turn.inverse()).unwrap();
        assert_eq!(builder.draft().parts[0].points, None);
        assert_eq!(builder.draft().parts[0].roll, 0.0);
        assert!(builder.undo() && builder.undo());
        assert_eq!(poses(&builder, &catalog), before);
    }

    #[test]
    fn a_part_and_its_copies_go_on_as_one_change_and_are_found_again() {
        let catalog = catalog();
        let mut builder = stack(&catalog);
        // A tank on each side of the lower tank, by a node of its own on its side, put on as a
        // set of two, the second the first's mirror image.
        let mut side = entry("tank", &catalog);
        side.id = "booster".into();
        side.nodes.push(NodeDef {
            name: "held_side".into(),
            position: [-1.0, 0.0, 2.0],
            direction: [-1.0, 0.0, 0.0],
            size: 0.0,
        });
        let at = |u: f64| SurfaceAtDef {
            u,
            v: 0.5,
            side: Side::Front,
            standoff: 0.0,
            axial: false,
        };
        let mut first = Place::surface("lower.barrel", "held_side", at(0.0));
        first.roll = 90.0;
        let mut second = Place::surface("lower.barrel", "held_side", at(0.5));
        second.roll = -90.0;
        let (change, tops) = builder
            .put_set(
                &catalog,
                std::slice::from_ref(&side),
                &[(first, false), (second, true)],
                &BTreeSet::new(),
            )
            .unwrap();
        assert_eq!(tops, ["booster", "booster_2"]);
        assert_eq!(change.added.len(), 2);
        assert_eq!(
            alike(builder.draft(), "booster"),
            [("booster_2".to_owned(), true)]
        );
        // A nose cone on one goes on as a set with one on the other, found by matching.
        let mut nose = entry("nose_cone", &catalog);
        nose.id = "nose".into();
        let places = [
            (Place::node("booster.top", "bottom"), false),
            (Place::node("booster_2.top", "bottom"), true),
        ];
        builder
            .put_set(&catalog, &[nose], &places, &BTreeSet::new())
            .unwrap();
        assert_eq!(
            alike(builder.draft(), "nose"),
            [("nose_2".to_owned(), true)]
        );
        // The cones stand on opposite sides of the axis, at the same height.
        let all = poses(&builder, &catalog);
        let (a, b) = (pose(&all, "nose"), pose(&all, "nose_2"));
        assert!((a.position.z - b.position.z).abs() < 1e-6);
        assert!(near(
            a.position.truncate().extend(0.0),
            -b.position.truncate().extend(0.0)
        ));
        // Put back elsewhere as one, the copies taken off with what they hold.
        let replacing = BTreeSet::from(["booster".to_owned(), "booster_2".to_owned()]);
        let subtree: Vec<CraftPartDef> = (builder.draft().parts.iter())
            .filter(|p| p.id == "booster" || p.id == "nose")
            .cloned()
            .collect();
        let mut one = Place::surface("lower.barrel", "held_side", at(0.25));
        one.roll = 90.0;
        builder
            .put_set(&catalog, &subtree, &[(one, false)], &replacing)
            .unwrap();
        let ids: Vec<&str> = (builder.draft().parts.iter())
            .map(|p| p.id.as_str())
            .collect();
        assert!(ids.contains(&"booster") && ids.contains(&"nose"));
        assert!(!ids.contains(&"booster_2") && !ids.contains(&"nose_2"));
        assert!(builder.undo());
        assert!(alike(builder.draft(), "nose").len() == 1);
    }

    #[test]
    fn a_tank_on_the_side_of_another_stands_upright_against_it_and_has_a_mirror_place() {
        let catalog = catalog();
        let builder = stack(&catalog);
        let craft = Craft::build(builder.draft(), &catalog).unwrap();
        let lower = craft.find("lower").unwrap();
        let mut booster = entry("tank", &catalog);
        booster.id = "booster".into();
        let node = side_node(&alone(&booster, &catalog).unwrap());
        assert_eq!(node.name, SIDE);
        assert!(node.position[0] < -0.9 && node.direction == [-1.0, 0.0, 0.0]);
        booster.nodes.push(node.clone());
        let at = SurfaceAtDef {
            u: 0.1,
            v: 0.5,
            side: Side::Front,
            standoff: 0.0,
            axial: false,
        };
        let mount = Mount::Surface {
            surface: "barrel".into(),
            side: Side::Front,
            u: at.u,
            v: at.v,
            standoff: 0.0,
            axial: false,
        };
        let roll = upright_roll(&craft, lower, &mount, &node, DVec3::Z).unwrap();
        let mut place = Place::surface("lower.barrel", SIDE, at);
        place.roll = roll;
        let mut placed = builder.clone();
        let (_, tops) = placed
            .put_set(
                &catalog,
                std::slice::from_ref(&booster),
                &[(place, false)],
                &BTreeSet::new(),
            )
            .unwrap();
        let all = poses(&placed, &catalog);
        let (core, side) = (pose(&all, "lower"), pose(&all, &tops[0]));
        // Upright, beside the core, its axis two radii off the core's.
        assert!((side.rotation * DVec3::Z).angle_between(DVec3::Z) < 1e-6);
        let apart = (side.position - core.position).truncate().length();
        assert!((apart - 2.0).abs() < 0.05, "{apart}");
        // Its mirror image across the craft's middle plane is on the same barrel.
        let u = mirrored_u(&craft, lower, "barrel", at).unwrap();
        assert!((u - 0.4).abs() < 1e-6, "{u}");
        // Half way round from +X, on the plane itself, there is none.
        let on_plane = SurfaceAtDef { u: 0.25, ..at };
        assert_eq!(mirrored_u(&craft, lower, "barrel", on_plane), None);
    }
}
