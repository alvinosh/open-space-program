//! Activation sequences, as in Kitten Space Agency: what each press of the stage key does.
//!
//! Stages are the vessel's sections and propellant pools ([`crate::propulsion::Stage`]);
//! sequences are separate. Each sequence is a list of activations (light a stage's engines,
//! release the decoupler above a stage, jettison a fairing, switch RCS on) fired together,
//! and the player can move activations between sequences, reorder sequences and add empty
//! ones. [`crate::propulsion::Propulsion`] carries them out.

use crate::propulsion::Stage;

/// Something a sequence activates. Stages are named by [`Stage::id`], their index in the
/// craft definition, which stays the same as stages fall away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Activation {
    /// Light (arm) the stage's engines.
    Engines(usize),
    /// Release the joint above the stage: it and everything below it fall away.
    Decoupler(usize),
    /// Jettison the stage's fairing.
    Fairing(usize),
    /// Switch RCS on.
    Rcs(usize),
}

impl Activation {
    pub fn stage(self) -> usize {
        match self {
            Self::Engines(s) | Self::Decoupler(s) | Self::Fairing(s) | Self::Rcs(s) => s,
        }
    }

    /// `engines`, `decoupler`, `fairing` or `rcs`, as written in craft files.
    pub fn kind(self) -> &'static str {
        match self {
            Self::Engines(_) => "engines",
            Self::Decoupler(_) => "decoupler",
            Self::Fairing(_) => "fairing",
            Self::Rcs(_) => "rcs",
        }
    }

    /// Order within a sequence: decouplers first, so the engines of the new bottom stage
    /// light, then the rest.
    fn rank(self) -> u8 {
        match self {
            Self::Decoupler(_) => 0,
            Self::Engines(_) => 1,
            Self::Fairing(_) => 2,
            Self::Rcs(_) => 3,
        }
    }

    /// Whether the stage has the part to activate.
    pub fn valid(self, stages: &[Stage]) -> bool {
        let Some(index) = stages.iter().position(|s| s.id == self.stage()) else {
            return false;
        };
        let stage = &stages[index];
        match self {
            Self::Engines(_) => !stage.engines.is_empty(),
            Self::Decoupler(_) => index + 1 < stages.len(),
            Self::Fairing(_) => stage.fairing.is_some(),
            Self::Rcs(_) => !stage.rcs.is_empty(),
        }
    }

    /// From `"<kind>:<stage name>"`.
    pub fn parse(text: &str, stages: &[Stage]) -> Result<Self, String> {
        let (kind, name) = text
            .split_once(':')
            .ok_or_else(|| format!("`{text}`: write it as `<kind>:<stage name>`"))?;
        let id = stages
            .iter()
            .find(|s| s.name == name.trim())
            .map(|s| s.id)
            .ok_or_else(|| format!("`{text}`: no stage named `{}`", name.trim()))?;
        let activation = match kind.trim() {
            "engines" => Self::Engines(id),
            "decoupler" => Self::Decoupler(id),
            "fairing" => Self::Fairing(id),
            "rcs" => Self::Rcs(id),
            other => {
                return Err(format!(
                    "`{text}`: unknown `{other}` (engines, decoupler, fairing or rcs)"
                ));
            }
        };
        if !activation.valid(stages) {
            return Err(format!(
                "`{text}`: that stage has nothing to activate there"
            ));
        }
        Ok(activation)
    }
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

    /// The usual order, like most rockets fly: light the first stage; then at each joint
    /// release the decoupler, light the stage above and switch on its RCS together (so
    /// the engine lights while the propellant is still settled); jettison a fairing once the
    /// stage below it is lit; a stage without engines (a payload) is released on its own.
    pub fn default_for(stages: &[Stage]) -> Self {
        let mut list = Vec::new();
        let mut placed = Vec::new();
        let mut light = |list: &mut Vec<Vec<Activation>>, seq: &mut Vec<Activation>, s: usize| {
            let stage = &stages[s];
            if !stage.engines.is_empty() {
                seq.push(Activation::Engines(stage.id));
            }
            if !stage.rcs.is_empty() {
                seq.push(Activation::Rcs(stage.id));
            }
            if !seq.is_empty() {
                list.push(std::mem::take(seq));
            }
            if !stage.engines.is_empty()
                && let Some(above) = stages.get(s + 1).filter(|a| a.fairing.is_some())
            {
                list.push(vec![Activation::Fairing(above.id)]);
                placed.push(above.id);
            }
        };
        if !stages.is_empty() {
            light(&mut list, &mut Vec::new(), 0);
        }
        // Each joint: the stage below it and the one above (at `k + 1`).
        for (k, joint) in stages.windows(2).enumerate() {
            let mut seq = vec![Activation::Decoupler(joint[0].id)];
            light(&mut list, &mut seq, k + 1);
        }
        for stage in stages {
            if stage.fairing.is_some() && !placed.contains(&stage.id) {
                list.push(vec![Activation::Fairing(stage.id)]);
            }
        }
        Self::new(list)
    }

    /// From a craft file's `sequences`.
    pub fn parse(def: &[Vec<String>], stages: &[Stage]) -> Result<Self, String> {
        let list = def
            .iter()
            .map(|seq| seq.iter().map(|a| Activation::parse(a, stages)).collect())
            .collect::<Result<_, _>>()?;
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

    /// Take the next sequence, in the order its activations should happen.
    pub fn pop_next(&mut self) -> Option<Vec<Activation>> {
        if self.list.is_empty() {
            return None;
        }
        let mut next = self.list.remove(0);
        self.fired += 1;
        next.sort_by_key(|a| a.rank());
        Some(next)
    }

    /// Move activation `index` of sequence `from` to `to`. A sequence it leaves empty stays,
    /// so it can take something else. Returns whether anything moved.
    pub fn move_activation(&mut self, from: usize, index: usize, to: Target) -> bool {
        if self.list.get(from).is_none_or(|s| index >= s.len()) {
            return false;
        }
        match to {
            Target::Into(to) if to == from || to >= self.list.len() => false,
            Target::Into(to) => {
                let a = self.list[from].remove(index);
                self.list[to].push(a);
                true
            }
            Target::New(at) => {
                let at = at.min(self.list.len());
                let a = self.list[from].remove(index);
                self.list.insert(at, vec![a]);
                true
            }
        }
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

    /// Drop the activations of stages that fell away, and the sequences that empties.
    pub fn forget(&mut self, gone: &[usize]) {
        self.list.retain_mut(|seq| {
            let had = !seq.is_empty();
            seq.retain(|a| !gone.contains(&a.stage()));
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

    #[test]
    fn moving_activations_and_sequences() {
        let mut s = seqs(vec![
            vec![Engines(0)],
            vec![Decoupler(0), Engines(1)],
            vec![Fairing(2)],
        ]);
        // Split ignition from separation into a new sequence after it.
        assert!(s.move_activation(1, 1, Target::New(2)));
        assert_eq!(
            s.list(),
            [
                vec![Engines(0)],
                vec![Decoupler(0)],
                vec![Engines(1)],
                vec![Fairing(2)]
            ]
        );
        // Fairing before the upper engine.
        assert!(s.move_sequence(3, 2));
        assert_eq!(s.list()[2], vec![Fairing(2)]);
        // Into an existing sequence; the one it leaves stays, empty, until removed.
        assert!(s.move_activation(2, 0, Target::Into(1)));
        assert_eq!(s.list()[1], vec![Decoupler(0), Fairing(2)]);
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
    fn firing_keeps_numbers_and_puts_decouplers_first() {
        let mut s = seqs(vec![
            vec![Engines(1), Rcs(1), Decoupler(0)],
            vec![Fairing(2)],
        ]);
        assert_eq!(s.number(1), 2);
        assert_eq!(s.pop_next().unwrap(), [Decoupler(0), Engines(1), Rcs(1)]);
        // The fairing's sequence is still number 2.
        assert_eq!(s.number(0), 2);
    }

    #[test]
    fn stages_that_fall_away_take_their_activations_with_them() {
        let mut s = seqs(vec![
            vec![Engines(0)],
            vec![],
            vec![Fairing(2)],
            vec![Decoupler(1)],
        ]);
        s.forget(&[0, 1]);
        // The emptied sequences go; the one that was empty already stays.
        assert_eq!(s.list(), [vec![], vec![Fairing(2)]]);
    }
}
