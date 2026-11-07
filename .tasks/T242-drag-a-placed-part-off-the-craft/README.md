# T242: Drag a placed part off the craft

Said on 2026-11-07: "i cannot drag parts away after i have placed them".

A drag on a placed part picks it up with what it holds (T216), but it can only go on somewhere
else on the craft: let go anywhere it cannot go on and it goes back where it was
(`building.rs`, "Let go where it cannot go: it goes back"). There is no way to drag a part off.

## Ask first

What "drag away" should do. Likely: let go over the parts list (as in KSP) takes it off the craft,
undoable; let go in empty space either does the same or goes back. Also check that a drag on a
placed part starts at all with a real mouse, not only the scripted pointer the captures use.

## Done when

- A placed part can be dragged off the craft and taken off it, and undo puts it back; a capture
  shows it.

## Done so far

- 2026-11-07, built without the user's answer to "Ask first" (they said "do both"): a part dragged
  from the craft and brought where it cannot go on (out in the hangar, or over the parts list,
  which gives no point in the view) shows the craft without it, its copies by symmetry and all
  they hold, and the hint says "Let go here to take the fin and its copies off". Letting go takes
  them off in one change (`Builder::remove_all`, `builder::removed`); undo puts them back. Over a
  ring or a side it still goes on there; Esc or a right click still puts it back.
- Tests: `builder` (four fins off together and back with one undo; the root takes everything).
  `OSP_CAPTURE_EDITOR_BUILD` drags a fin out into the hangar (`build_drag_off.png`), expects four
  parts left, undoes, and flies.
- Not checked: dragging with a real mouse (the captures use the scripted pointer); still to ask
  whether letting go in empty space should take it off or only over the list.

