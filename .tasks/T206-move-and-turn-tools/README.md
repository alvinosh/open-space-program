# T206: Move and turn tools: slide and turn a placed part

Asked for on 2026-11-06: "you can't move the parts up and down", and it is not clear "how to rotate
parts". A part could only be moved by dragging it off to another node or another place on a
surface, and turned with Q and E about its joint, which nothing on the screen told anyone.

As KSP's offset and rotate tools: tool buttons in the editor bar, Shape, Move and Turn (keys 1, 2
and 3), and with a part picked, handles for the tool on it.

## Scope

- Move: three arrows on the picked part, along the joint (up and down the part it is on) and
  across it. Dragging one slides the part, and everything on it, in steps that suit the view (Shift
  finer).
- Turn: three rings round the point it is held by. Dragging one turns the part about that axis, in
  15° steps (Shift 5°).
- Parts put on with symmetry move and turn with it, mirrored as they were placed, found by
  matching (symmetry is not stored).
- The builder: a moved or turned part is held by a node of its own where it was moved to, so the
  sim and craft files need nothing new. Undo, redo, rules checked on letting go.
- The bar says which tool is on and the hint says what the mouse does.

## Done when

- A fin set put on a tank is slid up and down it and round it and turned, all of them together, by
  dragging; a tank in a stack is turned about the stack; a slide that would leave a part hanging
  off its joint or inside another is refused with the reason; every change can be undone, and the
  craft launches as shown.

## Done so far

- 2026-11-06: `osp_editor::moving` (tools in the editor bar, keys 1 to 3, `editor.tool`), arrows and
  rings drawn over everything at a fixed size on the screen, lit under the pointer, a label with
  the distance or angle, live preview built on its own thread, Esc puts a drag back. Rings seen
  nearly edge on turn as the pointer goes across them. Sim: `builder::shifted`, `Builder::shift`,
  `builder::holding`, `builder::alike` (symmetric copies found by matching, mirrored ones too). A
  part on a surface slides over it (its `at` moves, stopping at the surface's ends); a part on a
  node, and any turn, is held by a node of its own `held_<node>` (`builder::stands_for`), and a
  node slide must still reach the joint (2 cm). Capture: `OSP_CAPTURE_EDITOR_MOVE=1`.
- Fixed on the way: `builder::check` and `lets_go_between` read `Assembly::parents()` as
  (parent, joint) when it is (joint, parent), so the node size and fits-inside rules were mostly
  skipped.

## Open

- The hangar stands the craft on its lowest point, so sliding a part below the rest lifts what
  is above it on the screen; the handles follow, but it can surprise.
- Turning the root (a whole craft lying down for a plane) is T210.
