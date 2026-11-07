# T228: Every edit reaches the symmetry copies

Moving or turning one of four fins moves all four (`builder::alike`). Changing its settings,
dragging its size handles or removing it changes that one only: `Builder::resize`,
`Builder::set_field` and `Builder::remove` touch the one id. The craft goes lopsided without a word
and flies crooked.

## Scope

- Resize, set a field and remove apply to the part and every copy `alike` finds, mirrored where
  the copy is, as one change that undoes in one step.
- A way to take one part out of its set on purpose ("Break symmetry"), for the rare case.
- The settings' title says how many it changes ("Fin x4").

## Done when

- Builder tests: a resize, a field and a removal on one of four radial fins and one of two
  mirrored wings reach every copy, and undo brings all of them back.

## Where it comes from

From the editor review of 2026-11-07: the user asked for everything wrong with the editor from a UX point of view, then for each finding to be filed as a task. The epic, T221, lists them all.
