//! Joining two vessels' parts in flight, and letting them go again as they were.
//!
//! A join takes the other vessel's parts, moved into this one's frame so that the two nodes meet
//! ([`crate::assembly::meet`]) and renumbered so their ids are their own, and hangs them from
//! this vessel's part by a [`crate::parts::JointWay::Join`]. Its tree is first re-rooted at its node. The tanks,
//! systems and everything else go with their parts; its sequences go after this vessel's. Mass,
//! inertia, plumbing and the power networks all follow from the parts, so they need no more than
//! working out again.
//!
//! Neither vessel was built to hang from the other: their sections keep the relations they were
//! built with ([`crate::parts::PartTree::holders`]), and neither gets in the way of the other's
//! exhaust. An engine of the vessel that joined goes out, unless it is a solid, which nothing
//! stops: it stays off until lit by hand, and is armed again when the vessel lets go. What the
//! burn executor and the readouts call the bottom of the stack is that of the vessel flown from
//! ([`Propulsion::flown`]).
//!
//! A [`Joined`] remembers what the joined vessel takes back. Letting go of the joint gives it
//! that, its own numbers, its old root and its own frame ([`Propulsion::go_home`]). A piece that
//! comes off anywhere else goes home to the frame it was built in as well, so that what comes off
//! a stack turned round is the right way up to itself.

use std::{collections::BTreeSet, ops::Range};

use glam::DVec3;

use super::{Propulsion, Released, Trigger, thrust_axis};
use crate::{
    assembly::{Pose, meet},
    parts::{PartId, PartTree, join::node_out},
    staging::Sequences,
    vessel::Vessel,
};

/// A node of a vessel, where it can be joined to another.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Port<'a> {
    pub part: PartId,
    pub node: &'a str,
}

/// A vessel that joined this one, and what it takes back when it lets go. Its `armed`,
/// `triggers` and `control_part` are in its own numbers; they are not in use until then.
#[derive(Debug, Clone)]
pub(crate) struct Joined {
    /// Its part at the join, which hangs from this vessel's.
    pub part: PartId,
    /// The part that was its root.
    root: PartId,
    /// The ids of its parts here, and what its own ids were raised by.
    parts: Range<u32>,
    part_shift: u32,
    /// The same of its sections.
    sections: Range<usize>,
    section_shift: usize,
    /// Which way it goes when it lets go, in this vessel's frame: straight out of the node it
    /// joined.
    pub away: DVec3,
    armed: BTreeSet<PartId>,
    triggers: Vec<Trigger>,
    control_part: Option<PartId>,
    needs_control: bool,
    throttle: f64,
    /// How many of its sequences had fired.
    fired: usize,
    /// The vessel it flew as, without its parts.
    pub vessel: Option<Box<Vessel>>,
}

impl Joined {
    /// Raise (or with negative amounts, lower) its numbers here.
    fn shift(&mut self, parts: i64, sections: i64) {
        let part = |id: u32| (i64::from(id) + parts) as u32;
        let section = |id: usize| (id as i64 + sections) as usize;
        self.part = PartId(part(self.part.0));
        self.root = PartId(part(self.root.0));
        self.parts = part(self.parts.start)..part(self.parts.end);
        self.part_shift = part(self.part_shift);
        self.sections = section(self.sections.start)..section(self.sections.end);
        self.section_shift = section(self.section_shift);
    }
}

impl Propulsion {
    /// No parts at all: what a vessel that joined another holds in place of its own.
    pub(crate) fn nothing() -> Self {
        Self::new(PartTree::default())
    }

    /// How `other`'s assembly frame would sit in this one's with `to` joined to `at`, the far
    /// node turned `roll` about the line they meet along. An error if either node cannot be joined.
    pub fn join_frame(
        &self,
        at: Port,
        other: &Propulsion,
        to: Port,
        roll: f64,
    ) -> Result<Pose, String> {
        let (near, mine, _) = self.parts.free_port(at)?;
        let (far, theirs, _) = other.parts.free_port(to)?;
        Ok(mine * meet(near, far, roll) * theirs.inverse())
    }

    /// Take `other`'s parts on, its node `to` joined to this vessel's `at`, turned `roll`. Returns
    /// what its part at the join is called here: the joint hanging from it lets go
    /// ([`crate::staging::Activation::decoupler`]). Nothing changes on an error.
    pub fn join(
        &mut self,
        mut other: Propulsion,
        at: Port,
        to: Port,
        roll: f64,
    ) -> Result<PartId, String> {
        let frame = self.join_frame(at, &other, to, roll)?;
        let (near, mine, near_connector) = self.parts.free_port(at)?;
        let (_, _, far_connector) = other.parts.free_port(to)?;
        let away = node_out(mine, near);
        let (part_shift, section_shift) = self.parts.next_ids();
        let up = |id: PartId| PartId(id.0 + part_shift);
        let root = other.parts.root().map_or(to.part, |p| p.id);
        let (count, sections) = other.parts.next_ids();

        let turned = other.parts.reroot(to.part);
        other.sequences.rename_joints(&turned);
        other.parts.move_parts(frame);
        other.parts.renumber(up, |s| s + section_shift);
        self.parts.graft(
            other.parts,
            at.part,
            near_connector,
            up(to.part),
            far_connector,
        );

        // Nothing stops a solid; the rest wait for it to let go.
        let (burning, armed): (BTreeSet<PartId>, BTreeSet<PartId>) = other
            .armed
            .iter()
            .copied()
            .partition(|&part| (other.engines.iter()).any(|e| e.part == part && e.is_solid()));
        self.armed.extend(burning.into_iter().map(up));

        for engine in &mut other.engines {
            engine.part = up(engine.part);
            engine.section += section_shift;
        }
        for block in &mut other.rcs {
            block.part = up(block.part);
            block.section += section_shift;
        }
        for tank in &mut other.tanks {
            tank.part = up(tank.part);
            tank.section += section_shift;
        }
        self.engines.append(&mut other.engines);
        self.rcs.append(&mut other.rcs);
        self.tanks.append(&mut other.tanks);
        self.jettisoned
            .extend(other.jettisoned.iter().map(|s| s + section_shift));
        self.systems.absorb(other.systems, up);
        let fired = other.sequences.fired();
        other.sequences.renumber(up);
        self.sequences.append(other.sequences);
        for mut nested in other.joined {
            nested.shift(i64::from(part_shift), section_shift as i64);
            nested.away = frame.rotation * nested.away;
            self.joined.push(nested);
        }
        self.joined.push(Joined {
            part: up(to.part),
            root: up(root),
            parts: part_shift..part_shift + count,
            part_shift,
            sections: section_shift..section_shift + sections,
            section_shift,
            away,
            armed,
            triggers: other.triggers,
            control_part: other.control_part,
            needs_control: other.needs_control,
            throttle: other.throttle,
            fired,
            vessel: None,
        });
        self.follow_parts();
        self.rebuild();
        Ok(up(to.part))
    }

    /// Remember `vessel` as what the vessel last joined flew as.
    pub(crate) fn keep_vessel(&mut self, vessel: Vessel) {
        if let Some(joined) = self.joined.last_mut() {
            joined.vessel = Some(Box::new(vessel));
        }
    }

    /// The parts held at the joins on this vessel: letting go of one lets that vessel go.
    pub fn joined_parts(&self) -> impl Iterator<Item = PartId> {
        self.joined.iter().map(|j| j.part)
    }

    /// Each engine and RCS block where its part is.
    fn follow_parts(&mut self) {
        for engine in &mut self.engines {
            if let Some(part) = self.parts.get(engine.part) {
                engine.mount = part.position;
                engine.direction = thrust_axis(part.rotation);
            }
        }
        for block in &mut self.rcs {
            if let Some(part) = self.parts.get(block.part) {
                block.mount = part.position;
                block.rotation = part.rotation;
            }
        }
    }

    /// The joined vessel section `id` came with, if any: the innermost, if one joined another.
    fn joined_with(&self, id: usize) -> Option<usize> {
        (0..self.joined.len())
            .filter(|&k| self.joined[k].sections.contains(&id))
            .min_by_key(|&k| self.joined[k].sections.len())
    }

    /// Whether section `id` belongs to the vessel it is flown from: this one, unless it is
    /// controlled from a part of one that joined it.
    pub(super) fn flown(&self, id: usize) -> bool {
        if self.joined.is_empty() {
            return true;
        }
        let from = (self.control_part)
            .and_then(|part| self.parts.get(part))
            .and_then(|part| self.joined_with(part.stage));
        self.joined_with(id) == from
    }

    /// What the vessels joined here take with `parts`, just let go of: the record of the vessel
    /// that let go, if its join was the joint that opened, with its sequences; and those of the
    /// vessels joined to what came off.
    pub(super) fn released_joined(
        &mut self,
        parts: &PartTree,
    ) -> (Option<Joined>, Vec<Joined>, Sequences) {
        let (leaving, staying) = std::mem::take(&mut self.joined)
            .into_iter()
            .partition::<Vec<_>, _>(|j| parts.get(j.part).is_some());
        self.joined = staying;
        let root = parts.root().map(|p| p.id);
        let (undocked, nested): (Vec<_>, Vec<_>) =
            leaving.into_iter().partition(|j| Some(j.part) == root);
        let undocked = undocked.into_iter().next();
        let sequences = match &undocked {
            Some(joined) => (self.sequences).take_parts(|p| parts.get(p).is_some(), joined.fired),
            None => Sequences::default(),
        };
        (undocked, nested, sequences)
    }

    /// What came loose from a vessel, as one of its own: what it was if it had joined that
    /// vessel, in the frame it was built in.
    pub fn come_home(mut released: Released) -> Self {
        let undocked = released.undocked.take();
        let mut propulsion = Self::from_released(released);
        propulsion.go_home(undocked);
        propulsion
    }

    /// Back to the frame its root part was built in and, if it is the vessel `undocked`, to what
    /// that vessel was. Returns where that frame sat on the vessel it came off. On parts that
    /// never joined anything nothing changes.
    pub(crate) fn go_home(&mut self, undocked: Option<Joined>) -> Pose {
        let home = self.parts.root().map_or(Pose::IDENTITY, |p| p.frame);
        if home == Pose::IDENTITY && undocked.is_none() {
            return home;
        }
        if let Some(joined) = undocked {
            let Joined {
                root,
                parts,
                part_shift,
                sections,
                section_shift,
                armed,
                triggers,
                control_part,
                needs_control,
                throttle,
                ..
            } = joined;
            let own = |id: PartId| {
                PartId(if parts.contains(&id.0) {
                    id.0 - part_shift
                } else {
                    id.0
                })
            };
            let own_section = |id: usize| {
                if sections.contains(&id) {
                    id - section_shift
                } else {
                    id
                }
            };
            self.parts.renumber(own, own_section);
            for engine in &mut self.engines {
                engine.part = own(engine.part);
                engine.section = own_section(engine.section);
            }
            for block in &mut self.rcs {
                block.part = own(block.part);
                block.section = own_section(block.section);
            }
            for tank in &mut self.tanks {
                tank.part = own(tank.part);
                tank.section = own_section(tank.section);
            }
            self.armed = (self.armed.iter()).map(|&p| own(p)).chain(armed).collect();
            self.jettisoned = self.jettisoned.iter().map(|&s| own_section(s)).collect();
            self.sequences.renumber(own);
            self.systems.renumber(own);
            for nested in &mut self.joined {
                if parts.contains(&nested.part.0) {
                    nested.shift(-i64::from(part_shift), -(section_shift as i64));
                }
            }
            self.triggers = triggers;
            self.control_part = control_part;
            self.needs_control = needs_control;
            self.throttle = throttle;
            self.ignition_pending = false;
            let root = own(root);
            // Unless the part it was rooted at has come off since.
            if self.parts.get(root).is_some() {
                let turned = self.parts.reroot(root);
                self.sequences.rename_joints(&turned);
            }
        }
        self.parts.move_home(home);
        let back = home.rotation.inverse();
        for nested in &mut self.joined {
            nested.away = back * nested.away;
        }
        self.follow_parts();
        self.rebuild();
        home
    }
}
