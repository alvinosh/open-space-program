# T207: Part settings in the left panel, not a popup

First asked for on 2026-11-06 as "the popup window can actually be on the right hand side,
replacing the parts window". Later the same day the open question (which panel) was answered: "when
you click on a part a menu comes up, we should use the left hand side parts menu and display the
information there, and not in a popup menu, no reason to have it like that".

Asked for on 2026-11-06, in one list of what is wrong with the editor: "the left hand side parts
menu is just full of text and not intuitive at all, there is overlap issues, also 5 buttons at the
top, all with text, not intuitive at all, also when you click on a part a menu comes up, we should
use the left hand side parts menu and display the information there [...] i want to be able to grab
a part and move it [...] we also need to add symetry modes, mirror modes, part clipping by
different offets, like 1degree 5degreee 20 degree 45 ect. And we should use icons where we can."
That list is T207 and T212 to T218.

Today the part window (`editor_part` layout) opens beside the pointer, or top left in captures,
where it covers the left panel and half of the Shape / Move / Turn buttons (see T213).

## Scope

- No popup. The left panel has two states: the parts list, and the picked part's settings (every
  field the part window has now: shape, sizes, contents, tank type, utilization, material, roll,
  what it comes to, Remove).
- Picking a part shows its settings there; letting go of it (click on nothing, Esc) brings back the
  parts list. Taking a part from the list in hand shows the list until it is put on, then its
  settings.
- A way back to the list while a part is picked, and the part's name and icon at the top.
- Section headers and buttons with icons where one says it (T212).
- The panel scrolls when the settings are longer than the screen.

## Done when

- Picking a part shows its settings in the left panel, nothing covers the craft or the editor bar,
  and letting go brings the parts list back; `OSP_CAPTURE_EDITOR_BUILD` shows both states, at the
  default size and at 1280x720.

## Done so far

- 2026-11-06: the popup (`editor_part` layout) is gone. The rail on the left shows one screen at a
  time, named by `editor.rail` (`vessels`, `part`, `parts`): a picked part's settings replace the
  parts list, with "‹ Parts" over the part's name, and letting go (click on nothing, Esc, ‹ Parts or
  the bar's Parts) brings the list back. `osp_editor::part_window` became `part_settings`.
- Built from the menu's pieces, as the editor layout has to be (`osp_ui` rule test: menu controls,
  the 12/15/20/28 type scale, no images). The part icon in the header waits for T212, which has to
  let icons into the editor's rules.
- Changed on the way: putting a part on no longer picks it, so the parts list stays out while
  building and a click on a part shows its settings. Before, every part put on opened its window;
  in the rail that would have swapped the list away after each part.
- `OSP_CAPTURE_EDITOR_BUILD` checks `editor.rail` through both states and passes at 1600x900 and
  1280x720 (the settings scroll there). `_MOVE` and `_SHAPE` pass.
