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

## Done (2026-11-13)

Asked again by the user on 2026-11-13: "they should only be fully deleted if you drag them back into
the parts folder, otherwise they should still be in the editor, so you can reattach them later".

- `builder::Loose` (a part and what it held, the first held by nothing, its middle in the craft's frame
  from the root) kept beside the draft in `Builder`; undo and redo carry both (`Snapshot`).
  `Builder::detach` (off the craft, the kept part left loose, its copies gone, one change),
  `put_loose` (back on, as `put_set`, no longer loose, one change), `move_loose`, `throw_loose`,
  `loose()`; `builder::fit_in` renames a group's ids the craft has taken since. The root cannot be
  left loose.
- Editor (`osp_editor::building`): a part dragged off and let go out in the hangar is left there,
  see-through, where it was let go; over the parts list (`editor_parts_rail`) it and its copies are
  thrown away; over the other panels it goes back. A loose group is taken by dragging it (or a
  click, then a click where it goes; one in front of the craft is taken before it), goes on by a ring
  or a surface as a part from the list does, with the symmetry set then, is left somewhere else in
  the hangar, or thrown away over the list. Esc or a right click puts it back where it was. The hint
  says which. `editor.loose` is how many groups are about.
- Drawn by the renderer (`osp_render::part_pictures`, `osp_game::part_pictures::LooseParts`,
  `ShowsLoose` for picking) as the hangar draws parts, each material a faded copy (35%), casting no
  shadow. Not built, launched, weighed or checked with the craft.
- Test: `builder::tests::parts_taken_off_stay_loose_until_put_back_or_thrown_away`. Capture
  `OSP_CAPTURE_EDITOR_LOOSE`: four fins, one dragged off into the hangar (two parts left, one loose
  group, see-through), dragged back on four round, undone, dragged onto the parts list and thrown
  away, undone and redone. `OSP_CAPTURE_EDITOR_BUILD`, `_CRAFT`, `_MOVE`, `_DRAG`, `_FIT` and
  `OSP_CAPTURE_CLUSTER` still pass.

## Guesses to confirm

- Loose parts are not saved with the craft (KSP keeps them); they last while the craft is in the
  editor.
- A fin set taken off leaves one loose fin; putting it back makes the copies again with the
  symmetry set then (as KSP does).
- Loose groups sit in the craft's frame, so they move and turn with the craft when it is carried or
  its root turned.

