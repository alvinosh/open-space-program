//! Activation sequences: what each press of the stage key does.
//!
//! An activation is one action on one part: ignite an engine, release the joint a part
//! hangs from (decouple), jettison a fairing panel, switch RCS on. The player can do any of
//! them by hand from the part's window ([`crate::propulsion::Propulsion::part_actions`]).
//! A sequence is only a list of them, done in the order listed when the stage key is
//! pressed; one done by hand leaves the list. The player can move activations between
//! sequences, reorder sequences and add empty ones; the staging panel shows identical
//! parts in a sequence as one icon with a count
//! ([`crate::propulsion::Propulsion::groups`]), and moves them together or one at a time.
//! [`crate::propulsion::Propulsion`] carries them out.

use crate::{parts::PartId, propulsion::Section};

/// Something a sequence activates, by the part that does it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Activation {
    /// Light (arm) an engine.
    Engine(PartId),
    /// Release the joint a part hangs from: it and everything hanging from it fall away.
    Decoupler(PartId),
    /// Jettison a fairing panel.
    Fairing(PartId),
    /// Switch RCS on.
    Rcs(PartId),
}

impl Activation {
    pub fn part(self) -> PartId {
        match self {
            Self::Engine(p) | Self::Decoupler(p) | Self::Fairing(p) | Self::Rcs(p) => p,
        }
    }

    /// `engines`, `decoupler`, `fairing` or `rcs`, as written in craft files.
    pub fn kind(self) -> &'static str {
        match self {
            Self::Engine(_) => "engines",
            Self::Decoupler(_) => "decoupler",
            Self::Fairing(_) => "fairing",
            Self::Rcs(_) => "rcs",
        }
    }

    /// What doing it is called on a button: `Ignite`, `Decouple`, `Jettison` or `RCS on`.
    pub fn verb(self) -> &'static str {
        match self {
            Self::Engine(_) => "Ignite",
            Self::Decoupler(_) => "Decouple",
            Self::Fairing(_) => "Jettison",
            Self::Rcs(_) => "RCS on",
        }
    }

    /// The activation of `part` that `kind` names (see [`Self::kind`]).
    pub fn of_kind(kind: &str, part: PartId) -> Option<Self> {
        Some(match kind {
            "engines" => Self::Engine(part),
            "decoupler" => Self::Decoupler(part),
            "fairing" => Self::Fairing(part),
            "rcs" => Self::Rcs(part),
            _ => return None,
        })
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
}

/// Where to put an activation that is moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Into this sequence.
    Into(usize),
    /// Into a new sequence inserted at this position.
    New(usize),
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
    /// without engines (a payload) is released on its own. `parts` gives each section's part of
    /// it.
    pub fn default_for(
        stages: &[Section],
        parts: impl Fn(SectionActivation) -> Vec<Activation>,
    ) -> Self {
        use SectionActivation as S;
        let beside = |stage: &Section| -> Vec<usize> {
            stages
                .iter()
                .filter(|s| s.beside && s.parent == Some(stage.id))
                .map(|s| s.id)
                .collect()
        };
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

    /// From a craft file's `sequences`, each activation written `<kind>:<part id>` and
    /// turned into activations by `parse`.
    pub fn parse(
        def: &[Vec<String>],
        parse: impl Fn(&str, &str) -> Result<Vec<Activation>, String>,
    ) -> Result<Self, String> {
        let mut list = Vec::new();
        for seq in def {
            let mut out = Vec::new();
            for text in seq {
                let (kind, part) = text
                    .split_once(':')
                    .ok_or_else(|| format!("`{text}`: write it as `<kind>:<part id>`"))?;
                let found =
                    parse(kind.trim(), part.trim()).map_err(|e| format!("`{text}`: {e}"))?;
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
        let moved: Vec<Activation> = indices.iter().map(|&i| self.list[from][i]).collect();
        for &i in indices.iter().rev() {
            self.list[from].remove(i);
        }
        match to {
            Target::Into(to) => self.list[to].extend(moved),
            Target::New(at) => {
                let at = at.min(self.list.len());
                self.list.insert(at, moved);
            }
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

    /// Take `activation` out of whatever sequence holds it (it happened some other way).
    pub fn take(&mut self, activation: Activation) {
        for seq in &mut self.list {
            seq.retain(|a| *a != activation);
        }
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
    use super::{Activation::*, *};

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
        let parse = |kind: &str, part: &str| match (kind, part) {
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
