//! Activation sequences: what each press of the stage key does.
//!
//! An activation is one [`Action`] on one part: ignite an engine, cut it off or limit its
//! thrust, release the joint a part hangs from (decouple), jettison a fairing panel, switch RCS
//! on, or anything a part's system offers ([`crate::systems::System::actions`]). The player can
//! do any of them by hand from the part's window
//! ([`crate::propulsion::Propulsion::part_actions`]). A sequence is only a list of them, done
//! in the order listed when the stage key is pressed; one done by hand leaves the list. The
//! player can move activations between sequences, reorder sequences and add empty ones; the
//! staging panel shows identical parts in a sequence as one icon with a count
//! ([`crate::propulsion::Propulsion::groups`]), and moves them together or one at a time.
//! [`crate::propulsion::Propulsion`] carries them out.

use crate::{parts::PartId, propulsion::Section};

/// Something a part can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Action {
    /// As written in craft files: `engines`, `decoupler`, or whatever a system calls it.
    pub kind: &'static str,
    /// What doing it is called on a button.
    pub verb: &'static str,
    /// What it is set to, for an action that has a setting: a thrust limit in percent.
    pub setting: Option<u16>,
}

impl Action {
    pub const fn new(kind: &'static str, verb: &'static str) -> Self {
        Self {
            kind,
            verb,
            setting: None,
        }
    }

    /// Light (arm) an engine.
    pub const IGNITE: Self = Self::new("engines", "Ignite");
    /// Shut an engine down; it can be lit again if it has ignitions left.
    pub const CUTOFF: Self = Self::new("cutoff", "Cut off");
    /// Release the joint a part hangs from: it and everything hanging from it fall away.
    pub const DECOUPLE: Self = Self::new("decoupler", "Decouple");
    /// Let go of the join a part hangs from: the vessel that joined leaves.
    pub const UNDOCK: Self = Self::new("undock", "Undock");
    /// Jettison a fairing panel.
    pub const JETTISON: Self = Self::new("fairing", "Jettison");
    /// Switch RCS on.
    pub const RCS_ON: Self = Self::new("rcs", "RCS on");

    /// Fly the vessel from this part: its axes are the nose, the pilot's right and feet.
    pub const CONTROL: Self = Self::new("control", "Control from here");

    /// Limit an engine's thrust to `percent` of full.
    pub const fn throttle(percent: u16) -> Self {
        Self {
            kind: "throttle",
            verb: "Limit thrust",
            setting: Some(percent),
        }
    }
}

/// Something a sequence activates: an action, by the part that does it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Activation {
    pub part: PartId,
    pub action: Action,
}

impl Activation {
    pub const fn new(part: PartId, action: Action) -> Self {
        Self { part, action }
    }

    pub const fn engine(part: PartId) -> Self {
        Self::new(part, Action::IGNITE)
    }

    pub const fn decoupler(part: PartId) -> Self {
        Self::new(part, Action::DECOUPLE)
    }

    pub const fn undock(part: PartId) -> Self {
        Self::new(part, Action::UNDOCK)
    }

    pub const fn fairing(part: PartId) -> Self {
        Self::new(part, Action::JETTISON)
    }

    pub const fn rcs(part: PartId) -> Self {
        Self::new(part, Action::RCS_ON)
    }

    pub fn part(self) -> PartId {
        self.part
    }

    /// As written in craft files.
    pub fn kind(self) -> &'static str {
        self.action.kind
    }

    /// What doing it is called on a button.
    pub fn verb(self) -> &'static str {
        self.action.verb
    }

    pub fn is_engine(self) -> bool {
        self.action == Action::IGNITE
    }

    pub fn is_decoupler(self) -> bool {
        self.action == Action::DECOUPLE
    }

    /// The activation of `part` that `kind` names (see [`Self::kind`]), with its `setting` if it
    /// takes one: one of the built-in actions, or one a kind of system declares.
    pub fn of_kind(kind: &str, part: PartId, setting: Option<u16>) -> Option<Self> {
        let action = match (kind, setting) {
            ("throttle", Some(percent)) => Action::throttle(percent),
            (_, Some(_)) | ("throttle", None) => return None,
            _ => [
                Action::IGNITE,
                Action::CUTOFF,
                Action::DECOUPLE,
                Action::UNDOCK,
                Action::JETTISON,
                Action::RCS_ON,
                Action::CONTROL,
            ]
            .into_iter()
            .chain(
                crate::systems::KINDS
                    .iter()
                    .flat_map(|k| k.actions.iter().copied()),
            )
            .find(|a| a.kind == kind)?,
        };
        Some(Self::new(part, action))
    }
}

/// What a sequence does to a whole section, as craft files may write it: the
/// [`Activation`]s of its parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionActivation {
    /// Light all its engines.
    Engines(usize),
    /// Release the joint it hangs from.
    Decoupler(usize),
    /// Jettison its fairing's panels.
    Fairing(usize),
    /// Switch RCS on with its blocks.
    Rcs(usize),
    /// Cut off all its engines.
    Cutoff(usize),
    /// Limit all its engines' thrust to this percent of full.
    Throttle(usize, u16),
}

/// Where to put an activation that is moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Into this sequence.
    Into(usize),
    /// Into a new sequence inserted at this position.
    New(usize),
    /// Into a sequence, before the activation at this index of it.
    Before(usize, usize),
}

/// The sequences still to fire, the next first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sequences {
    list: Vec<Vec<Activation>>,
    /// How many have fired, so a sequence keeps its number as the ones before it fire.
    fired: usize,
}

impl Sequences {
    pub fn new(list: Vec<Vec<Activation>>) -> Self {
        Self { list, fired: 0 }
    }

    /// The usual order, like most rockets fly: light the first stage with the stages beside
    /// it, and release those once they have burnt out; then at each joint up the stack
    /// release the decoupler, light the stage above (and those beside it) and switch on its
    /// RCS together (so the engine lights while the propellant is still settled), and release
    /// what is beside it in turn; jettison a fairing once the stage below it is lit; a stage
    /// without engines (a payload) is released on its own. A stage that hangs on the axis from
    /// one that already has another under it goes as one beside it does. `parts` gives each
    /// section's part of it.
    pub fn default_for(
        stages: &[Section],
        parts: impl Fn(SectionActivation) -> Vec<Activation>,
    ) -> Self {
        use SectionActivation as S;
        // The stack, bottom first: from the root down through the stages hanging under.
        let mut stack = Vec::new();
        let mut at = stages.iter().rposition(|s| s.parent.is_none());
        while let Some(i) = at {
            stack.push(i);
            at = stages
                .iter()
                .position(|s| s.parent == Some(stages[i].id) && !s.beside);
        }
        stack.reverse();
        let in_stack: Vec<usize> = stack.iter().map(|&i| stages[i].id).collect();
        let beside = |stage: &Section| -> Vec<usize> {
            stages
                .iter()
                .filter(|s| s.parent == Some(stage.id) && !in_stack.contains(&s.id))
                .map(|s| s.id)
                .collect()
        };
        let mut list = Vec::new();
        let mut placed = Vec::new();
        let mut light = |list: &mut Vec<Vec<Activation>>, seq: &mut Vec<Activation>, k: usize| {
            let stage = &stages[stack[k]];
            let sides = beside(stage);
            let lit = std::iter::once(stage)
                .chain(stages.iter().filter(|s| sides.contains(&s.id)))
                .filter(|s| s.engines > 0);
            seq.extend(lit.flat_map(|s| parts(S::Engines(s.id))));
            if stage.rcs > 0 {
                seq.extend(parts(S::Rcs(stage.id)));
            }
            if !seq.is_empty() {
                list.push(std::mem::take(seq));
            }
            if !sides.is_empty() {
                list.push(
                    sides
                        .into_iter()
                        .flat_map(|s| parts(S::Decoupler(s)))
                        .collect(),
                );
            }
            if stage.engines > 0
                && let Some(above) = stack
                    .get(k + 1)
                    .map(|&i| &stages[i])
                    .filter(|a| a.fairing.is_some())
            {
                list.push(parts(S::Fairing(above.id)));
                placed.push(above.id);
            }
        };
        if !stack.is_empty() {
            light(&mut list, &mut Vec::new(), 0);
        }
        // Each joint up the stack: the stage under it and the one above (at `k + 1`).
        for k in 0..stack.len().saturating_sub(1) {
            let mut seq = parts(S::Decoupler(stages[stack[k]].id));
            light(&mut list, &mut seq, k + 1);
        }
        for stage in stages {
            if stage.fairing.is_some() && !placed.contains(&stage.id) {
                list.push(parts(S::Fairing(stage.id)));
            }
        }
        list.retain(|seq| !seq.is_empty());
        Self::new(list)
    }

    /// From a craft file's `sequences`, each activation written `<kind>:<part id>`, or
    /// `<kind>:<part id>:<setting>` for one that takes a setting, and turned into activations
    /// by `parse`.
    pub fn parse(
        def: &[Vec<String>],
        parse: impl Fn(&str, &str, Option<u16>) -> Result<Vec<Activation>, String>,
    ) -> Result<Self, String> {
        let mut list = Vec::new();
        for seq in def {
            let mut out = Vec::new();
            for text in seq {
                let (kind, rest) = text
                    .split_once(':')
                    .ok_or_else(|| format!("`{text}`: write it as `<kind>:<part id>`"))?;
                let (part, setting) = match rest.split_once(':') {
                    Some((part, setting)) => {
                        let setting = setting
                            .trim()
                            .parse::<u16>()
                            .map_err(|_| format!("`{text}`: its setting is a whole number"))?;
                        (part, Some(setting))
                    }
                    None => (rest, None),
                };
                let found = parse(kind.trim(), part.trim(), setting)
                    .map_err(|e| format!("`{text}`: {e}"))?;
                for a in found {
                    if !out.contains(&a) {
                        out.push(a);
                    }
                }
            }
            list.push(out);
        }
        Ok(Self::new(list))
    }

    /// The sequences still to fire, the next first.
    pub fn list(&self) -> &[Vec<Activation>] {
        &self.list
    }

    /// Number of the sequence at `index` in [`Self::list`], counting from 1 for the first
    /// that ever fired.
    pub fn number(&self, index: usize) -> usize {
        self.fired + index + 1
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Whether `other` holds the same activations at the same point in the flight, however they
    /// are arranged: nothing has fired or fallen away between the two.
    pub fn same_activations(&self, other: &Self) -> bool {
        let all = |s: &Self| s.list.iter().flatten().copied().collect::<Vec<_>>();
        let (mine, theirs) = (all(self), all(other));
        self.fired == other.fired
            && mine.len() == theirs.len()
            && mine.iter().all(|a| theirs.contains(a))
    }

    /// Take the next sequence, its activations in the order they were listed.
    pub fn pop_next(&mut self) -> Option<Vec<Activation>> {
        if self.list.is_empty() {
            return None;
        }
        self.fired += 1;
        Some(self.list.remove(0))
    }

    /// Move activation `index` of sequence `from` to `to`. A sequence it leaves empty stays,
    /// so it can take something else. Returns whether anything moved.
    pub fn move_activation(&mut self, from: usize, index: usize, to: Target) -> bool {
        self.move_activations(from, &[index], to)
    }

    /// Move activations `indices` of sequence `from` to `to` together, in their order.
    pub fn move_activations(&mut self, from: usize, indices: &[usize], to: Target) -> bool {
        let Some(seq) = self.list.get(from) else {
            return false;
        };
        if indices.is_empty() || indices.iter().any(|&i| i >= seq.len()) {
            return false;
        }
        if matches!(to, Target::Into(to) if to == from || to >= self.list.len()) {
            return false;
        }
        let mut indices = indices.to_vec();
        indices.sort_unstable();
        indices.dedup();
        // What it goes before, which is not one of those moved: their leaving may shift it.
        let before = match to {
            Target::Before(to, at) => {
                let Some(&anchor) = self.list.get(to).and_then(|seq| seq.get(at)) else {
                    return false;
                };
                if to == from && indices.contains(&at) {
                    return false;
                }
                Some((to, anchor))
            }
            _ => None,
        };
        let moved: Vec<Activation> = indices.iter().map(|&i| self.list[from][i]).collect();
        for &i in indices.iter().rev() {
            self.list[from].remove(i);
        }
        match (to, before) {
            (Target::Into(to), _) => self.list[to].extend(moved),
            (Target::New(at), _) => {
                let at = at.min(self.list.len());
                self.list.insert(at, moved);
            }
            (_, Some((to, anchor))) => {
                let at = self.list[to].iter().position(|a| *a == anchor).unwrap_or(0);
                self.list[to].splice(at..at, moved);
            }
            (Target::Before(..), None) => unreachable!("worked out above"),
        }
        true
    }

    /// Move sequence `from` to position `to` (the others shift).
    pub fn move_sequence(&mut self, from: usize, to: usize) -> bool {
        if from >= self.list.len() || to >= self.list.len() || from == to {
            return false;
        }
        let seq = self.list.remove(from);
        self.list.insert(to, seq);
        true
    }

    /// Insert an empty sequence at `at` (clamped to the end).
    pub fn insert_empty(&mut self, at: usize) {
        let at = at.min(self.list.len());
        self.list.insert(at, Vec::new());
    }

    /// Remove sequence `at` if it is empty.
    pub fn remove_empty(&mut self, at: usize) -> bool {
        if self.list.get(at).is_some_and(Vec::is_empty) {
            self.list.remove(at);
            return true;
        }
        false
    }

    /// Put sequence `from` into the one before it, its activations after theirs.
    pub fn merge_down(&mut self, from: usize) -> bool {
        if from == 0 || from >= self.list.len() {
            return false;
        }
        let moved = self.list.remove(from);
        self.list[from - 1].extend(moved);
        true
    }

    /// Give `part` a sequence of its own, just before the one that holds it: it fires first.
    /// Returns false if it is in none, or has that one to itself already.
    pub fn split_off(&mut self, part: PartId) -> bool {
        let Some(from) = self
            .list
            .iter()
            .position(|seq| seq.iter().any(|a| a.part() == part))
        else {
            return false;
        };
        if self.list[from].iter().all(|a| a.part() == part) {
            return false;
        }
        let indices: Vec<usize> = (self.list[from].iter().enumerate())
            .filter(|(_, a)| a.part() == part)
            .map(|(i, _)| i)
            .collect();
        self.move_activations(from, &indices, Target::New(from))
    }

    /// Take `activation` out of whatever sequence holds it (it happened some other way).
    pub fn take(&mut self, activation: Activation) {
        for seq in &mut self.list {
            seq.retain(|a| *a != activation);
        }
    }

    /// Put `other`'s sequences after these, as a join does.
    pub(crate) fn append(&mut self, other: Self) {
        self.list.extend(other.list);
    }

    /// How many have fired.
    pub(crate) fn fired(&self) -> usize {
        self.fired
    }

    /// Call every part what `id` says.
    pub(crate) fn renumber(&mut self, id: impl Fn(PartId) -> PartId) {
        for activation in self.list.iter_mut().flatten() {
            activation.part = id(activation.part);
        }
    }

    /// A joint that lets go is named by the part hanging from it, so when re-rooting turns joints
    /// round (`renamed`: the part that named one, and the part that names it now) what releases
    /// them follows.
    pub(crate) fn rename_joints(&mut self, renamed: &[(PartId, PartId)]) {
        for activation in self.list.iter_mut().flatten() {
            if matches!(activation.kind(), "decoupler" | "undock" | "fairing")
                && let Some(&(_, new)) = renamed.iter().find(|(old, _)| *old == activation.part)
            {
                activation.part = new;
            }
        }
    }

    /// Take the activations of parts `gone` says are leaving, as sequences of their own that
    /// `fired` have gone before. A sequence they leave empty goes; one that was empty stays here.
    pub(crate) fn take_parts(&mut self, gone: impl Fn(PartId) -> bool, fired: usize) -> Self {
        let mut taken = Vec::new();
        self.list.retain_mut(|seq| {
            let had = !seq.is_empty();
            let (leaving, staying) = std::mem::take(seq)
                .into_iter()
                .partition::<Vec<_>, _>(|a| gone(a.part()));
            *seq = staying;
            if !leaving.is_empty() {
                taken.push(leaving);
            }
            !(had && seq.is_empty())
        });
        Self { list: taken, fired }
    }

    /// Drop the activations of parts that fell away, and the sequences that empties.
    pub fn forget(&mut self, gone: impl Fn(PartId) -> bool) {
        self.list.retain_mut(|seq| {
            let had = !seq.is_empty();
            seq.retain(|a| !gone(a.part()));
            !(had && seq.is_empty())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(non_snake_case)]
    fn Engine(part: PartId) -> Activation {
        Activation::engine(part)
    }

    #[test]
    fn an_activation_can_go_before_another_in_its_own_sequence_or_another() {
        let (a, b, c, d) = (PartId(1), PartId(2), PartId(3), PartId(4));
        let mut sequences =
            Sequences::new(vec![vec![Engine(a), Engine(b), Engine(c)], vec![Engine(d)]]);
        // Within a row: the last one first.
        assert!(sequences.move_activations(0, &[2], Target::Before(0, 0)));
        assert_eq!(sequences.list()[0], [Engine(c), Engine(a), Engine(b)]);
        // Its leaving shifts what it goes before.
        assert!(sequences.move_activations(0, &[0], Target::Before(0, 2)));
        assert_eq!(sequences.list()[0], [Engine(a), Engine(c), Engine(b)]);
        // Into another row, in front.
        assert!(sequences.move_activations(1, &[0], Target::Before(0, 1)));
        assert_eq!(
            sequences.list()[0],
            [Engine(a), Engine(d), Engine(c), Engine(b)]
        );
        // Not before itself, nor anything that is not there.
        assert!(!sequences.move_activations(0, &[1], Target::Before(0, 1)));
        assert!(!sequences.move_activations(0, &[0], Target::Before(0, 9)));
    }

    #[test]
    fn a_part_split_off_fires_before_the_rest_of_its_sequence() {
        let (a, b, c) = (PartId(1), PartId(2), PartId(3));
        let mut sequences = Sequences::new(vec![vec![Engine(a), Engine(b)], vec![Engine(c)]]);
        assert!(sequences.split_off(b));
        assert_eq!(
            sequences.list(),
            [vec![Engine(b)], vec![Engine(a)], vec![Engine(c)]]
        );
        // On its own already, or not in any.
        assert!(!sequences.split_off(b));
        assert!(!sequences.split_off(PartId(9)));
    }

    #[test]
    fn a_sequence_merges_into_the_one_before_it() {
        let (a, b, c) = (PartId(1), PartId(2), PartId(3));
        let mut sequences = Sequences::new(vec![vec![Engine(a)], vec![Engine(b)], vec![Engine(c)]]);
        assert!(!sequences.merge_down(0));
        assert!(!sequences.merge_down(3));
        assert!(sequences.merge_down(2));
        assert_eq!(
            sequences.list(),
            [vec![Engine(a)], vec![Engine(b), Engine(c)]]
        );
    }

    #[test]
    fn rearranging_keeps_the_same_activations_and_firing_does_not() {
        let (a, b) = (PartId(1), PartId(2));
        let before = Sequences::new(vec![vec![Engine(a)], vec![Engine(b)]]);
        let mut moved = before.clone();
        assert!(moved.move_sequence(0, 1));
        assert!(moved.same_activations(&before));
        moved.pop_next();
        assert!(!moved.same_activations(&before));
        let mut lost = before.clone();
        lost.forget(|p| p == a);
        assert!(!lost.same_activations(&before));
    }

    #[allow(non_snake_case)]
    fn Decoupler(part: PartId) -> Activation {
        Activation::decoupler(part)
    }

    #[allow(non_snake_case)]
    fn Fairing(part: PartId) -> Activation {
        Activation::fairing(part)
    }

    #[allow(non_snake_case)]
    fn Rcs(part: PartId) -> Activation {
        Activation::rcs(part)
    }

    fn seqs(list: Vec<Vec<Activation>>) -> Sequences {
        Sequences::new(list)
    }

    const P: [PartId; 6] = [
        PartId(0),
        PartId(1),
        PartId(2),
        PartId(3),
        PartId(4),
        PartId(5),
    ];

    #[test]
    fn moving_activations_and_sequences() {
        let mut s = seqs(vec![
            vec![Engine(P[0])],
            vec![Decoupler(P[1]), Engine(P[2])],
            vec![Fairing(P[3])],
        ]);
        // Split ignition from separation into a new sequence after it.
        assert!(s.move_activation(1, 1, Target::New(2)));
        assert_eq!(
            s.list(),
            [
                vec![Engine(P[0])],
                vec![Decoupler(P[1])],
                vec![Engine(P[2])],
                vec![Fairing(P[3])]
            ]
        );
        // Fairing before the upper engine.
        assert!(s.move_sequence(3, 2));
        assert_eq!(s.list()[2], vec![Fairing(P[3])]);
        // Into an existing sequence; the one it leaves stays, empty, until removed.
        assert!(s.move_activation(2, 0, Target::Into(1)));
        assert_eq!(s.list()[1], vec![Decoupler(P[1]), Fairing(P[3])]);
        assert!(s.list()[2].is_empty());
        assert!(s.remove_empty(2));
        assert!(!s.remove_empty(0));
        assert_eq!(s.list().len(), 3);
        // Nonsense moves do nothing.
        assert!(!s.move_activation(0, 5, Target::Into(1)));
        assert!(!s.move_activation(0, 0, Target::Into(0)));
        assert!(!s.move_sequence(0, 9));
    }

    #[test]
    fn several_parts_move_together() {
        let mut s = seqs(vec![
            vec![Engine(P[0]), Engine(P[1]), Engine(P[2]), Rcs(P[3])],
            vec![Decoupler(P[4])],
        ]);
        // Two of a cluster of three into the next sequence, the third left to light first.
        assert!(s.move_activations(0, &[2, 1], Target::Into(1)));
        assert_eq!(s.list()[0], [Engine(P[0]), Rcs(P[3])]);
        assert_eq!(s.list()[1], [Decoupler(P[4]), Engine(P[1]), Engine(P[2])]);
        assert!(!s.move_activations(0, &[0, 7], Target::New(0)));
        assert!(!s.move_activations(0, &[], Target::New(0)));
    }

    #[test]
    fn firing_keeps_numbers_and_the_listed_order() {
        let mut s = seqs(vec![
            vec![Engine(P[1]), Rcs(P[2]), Decoupler(P[0])],
            vec![Fairing(P[3])],
        ]);
        assert_eq!(s.number(1), 2);
        assert_eq!(
            s.pop_next().unwrap(),
            [Engine(P[1]), Rcs(P[2]), Decoupler(P[0])]
        );
        // The fairing's sequence is still number 2.
        assert_eq!(s.number(0), 2);
    }

    #[test]
    fn parts_that_fall_away_take_their_activations_with_them() {
        let mut s = seqs(vec![
            vec![Engine(P[0])],
            vec![],
            vec![Fairing(P[2])],
            vec![Decoupler(P[1])],
        ]);
        s.forget(|p| p == P[0] || p == P[1]);
        // The emptied sequences go; the one that was empty already stays.
        assert_eq!(s.list(), [vec![], vec![Fairing(P[2])]]);
    }

    #[test]
    fn a_craft_names_parts_or_their_section() {
        // `engines:core` lights both of the core's engines; naming one engine lights it
        // alone, once however often it is named.
        let parse = |kind: &str, part: &str, _: Option<u16>| match (kind, part) {
            ("engines", "core") => Ok(vec![Engine(P[0]), Engine(P[1])]),
            ("engines", "core_engine_2") => Ok(vec![Engine(P[1])]),
            ("decoupler", "booster") => Ok(vec![Decoupler(P[2])]),
            _ => Err("no".to_string()),
        };
        let def = |list: &[&[&str]]| -> Vec<Vec<String>> {
            (list.iter())
                .map(|seq| seq.iter().map(|a| a.to_string()).collect())
                .collect()
        };
        let s = Sequences::parse(
            &def(&[
                &["engines:core_engine_2", "engines:core"],
                &["decoupler: booster"],
            ]),
            parse,
        )
        .unwrap();
        assert_eq!(
            s.list(),
            [vec![Engine(P[1]), Engine(P[0])], vec![Decoupler(P[2])]]
        );
        let bad = Sequences::parse(&def(&[&["engines"]]), parse);
        assert!(bad.unwrap_err().contains("<kind>:<part id>"));
        assert!(Sequences::parse(&def(&[&["wings:core"]]), parse).is_err());
    }
}
