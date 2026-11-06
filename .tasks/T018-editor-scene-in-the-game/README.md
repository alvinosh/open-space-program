# T018: Editor scene in the game

`osp_editor::hangar`: the place where a craft is built, with camera, rendering, picking and placing by the mouse.

## Scope

- A 'Build' entry in the main menu and a toolbar button to come back to it.
- The hangar scene; the orbit camera and the orthographic front, side and top views.
- The draft drawn by the existing vessel renderer (build a `Vessel` from the draft after each edit).
- A ghost under the pointer; picking and selection with an outline (as `part_info.rs` picks parts); connectors lit when a part could go there.
- Snapping to nodes and surfaces, with (u, v) from where the ray hit.
- Handles to move, rotate and offset in 90, 15 and 5 degree steps; the symmetry preview; delete, duplicate, reroot.
- A greybox mode (parts shown plain).

## Done when

- A person places an engine under a tank, a pair of fins on its side, and a payload on top, with the mouse.

## Done so far

- `osp_editor::hangar`, view only for now: the hangar scene, the orbit camera (right or left drag, middle drag or WASD, Q and E, the wheel), and the craft drawn by the vessel renderer, standing up, or level if it rolls on wheels. It is entered through the Play page rather than a Build entry.

- 2026-11-06, picking and shaping (`osp_editor::shaping`): the part under the pointer is lit (a
  steady, faint blue), a click picks it (pulsing accent) and a click on nothing or Esc lets go. A
  picked part drawn as its shape shows round handles drawn over everything: length at the top,
  diameter at the side turned to face the view, the rims of an adapter, a fin's tip (span and
  sweep together), tip and root trailing edges. Dragging one changes the size in steps that suit
  the zoom (Shift a tenth), a width takes a joined part's width when near it ("as Instrument Unit"),
  the label says when it is as big or small as it goes, the pointer shows which way it moves, and
  handles too near each other on the screen leave the less important out. The craft is built
  again on a thread of its own while dragging (the Saturn V takes about 85 ms), so the handle keeps
  up with the pointer; let go, the builder checks it and keeps it or puts it back with the reason.
  Ctrl+Z / Ctrl+Shift+Z undo and redo (`editor.undo`, `editor.redo`, `editor.pick:<part>`). The
  edited craft is what launches. `OSP_CAPTURE_EDITOR_SHAPE=1` drives all of it with the pointer.

- 2026-11-06, building from nothing (`osp_editor::building`): New vessel starts an empty craft; a
  part taken from the Parts list goes on as the root if the craft is empty, else the craft's free
  nodes show as rings and the part is shown where it would go (built on its own thread, the
  builder's rules checked there too, red with the reason where it may not go): on a ring by its own
  node facing the other way, or on the side of a part for fins, wings and RCS blocks, with symmetry
  1 to 8 (X). A click puts it there and picks it; Esc or a right click puts it back; Delete or
  Backspace takes the picked part off. Undo covers all of it. The hall grows with the craft.
  `OSP_CAPTURE_EDITOR_BUILD=1` builds a capsule, decoupler, tank, engine and four fins with the
  pointer and flies it.

- 2026-11-06, drag and drop and turning: a row of the parts list dragged into the hangar goes
  where it is let go; a part of the craft dragged with the left button comes off with what it holds
  and goes where it is let go (a drag on nothing, or on the root, turns the view); Q and E turn what
  is in hand, or the picked part, about its joint (15°, 5° with Shift), and the part window has the
  same as a number.

## Still open

- New game and Fly a new vessel open the editor, and the vessel is chosen there: T190. There is no Build entry of its own unless the builder needs one.
- Radial attachment of anything but fins, wings and RCS (boosters on radial decouplers), tilting
  a part (only turning about its joint is there), mirror symmetry for wings, the orthographic views,
  the toolbar button, the greybox mode.
- Staging goes back to the default whenever a part is added or taken off.
- Shaping: a stack does not follow a widened part (the decoupler and interstage next to it keep
  their widths), surface-mounted parts keep their (u, v) rather than their distance from an end,
  paint layers stay where they were in metres, and a shape's other numbers (domes, skirts, a
  shell's profile) have no handle.
- Guesses to confirm: handles on the part rather than a panel of sliders; length grows at the top
  (the base stays on the floor); a handle is hidden when it would sit within 18 px of a more
  important one. The hangar is a plain hall with a floor and could look better.

## Where it comes from

- docs/build-editor-plan.md (E6)
