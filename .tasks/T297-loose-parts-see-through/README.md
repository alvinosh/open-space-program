# T297: Parts taken off the craft stay in the hangar, see-through

Said by the user on 2026-11-12: "on the builder, if you detatch a part or a set of parts it should just
become transparent, like in ksp".

Today (T242) a part dragged off the craft and let go where it cannot go on is taken off and gone, with
what it holds and its copies; only undo brings it back. In KSP it stays where it was let go, drawn
see-through, not part of the craft, and can be picked up again and put on.

## Scope

- Let go in the hangar, off the craft: the part and what it holds stay there as a loose group, drawn
  see-through, not counted in the craft's mass, checks or delta-v, and not launched.
- A loose group can be grabbed and put on the craft again, by any of its free nodes, as a part from
  the list is.
- Let go over the parts list: thrown away, as now. Undo and redo for every step.
- Symmetry copies: taking one of four fins off takes all four off (as now); they stay as four loose
  fins or one loose fin? Guess: one, the copies are made again when it goes back on with symmetry.
- Kept with the draft while editing; to decide whether a saved craft keeps them (KSP does).

## Done when

- In a capture, a fin set is dragged off into the hangar and shows see-through, the craft's mass
  drops, the fin is dragged back on, and undo walks back through each step.
