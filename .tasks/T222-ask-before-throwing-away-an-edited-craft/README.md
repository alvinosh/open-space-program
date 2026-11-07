# T222: Ask before throwing away an edited craft

Choosing another vessel from the list, New rocket or New plane, Back, or Esc once too often
throws away the craft on show at once, with every change made to it. Nothing asks, and there is no
craft save yet (T020).

## Scope

- Each of those asks first, with the game's one question dialog, when the craft has changes since
  it was put on show (the builder can undo). Discard goes on; Cancel keeps the craft as it is.
- A craft built from nothing and a changed stock craft are asked about in their own words.
- Staging changes are not counted: they are not the builder's.

## Done when

- Building or changing a craft and then doing any of the four asks first; Cancel keeps it, Discard
  goes on. A craft with no changes asks nothing.
- The editor's capture scripts answer the question where they change one craft and go to another.

## Later

- A Save answer, once T020 saves craft.

## Where it comes from

From the editor review of 2026-11-07: the user asked for everything wrong with the editor from a UX point of view, then for each finding to be filed as a task. The epic, T221, lists them all.

## Done so far

- 2026-11-07: built (uncommitted). `Editor::asks_first` (hangar.rs) stops `editor.show` (another
  craft), `editor.new`, Back and Esc while the builder can undo, and sends
  `editor.ask_discard:<action>`; the question opens the frame after, because the dialog closes on
  the same Esc or Back that asked. Discard sends `editor.discard:<action>`, which lets that action
  through once. A craft built from nothing is asked about as "the vessel you are building", a
  shipped one as "your changes to <name>", saying the shipped craft stays as it was.
- `OSP_CAPTURE_EDITOR_CRAFT` checks Esc (Cancel keeps the rocket), another craft from the list
  (Cancel) and a new plane (Discard); `OSP_CAPTURE_EDITOR_SHAPE` checks that a craft whose every
  change was undone asks nothing. Play, leaving and revert captures run as before.
- Not counted as changes: staging, and a craft brought back by revert to hangar (its history
  starts empty there).
