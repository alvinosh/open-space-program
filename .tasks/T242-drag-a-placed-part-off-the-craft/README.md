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
