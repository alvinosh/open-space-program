# T212: Editor icon set

Asked for on 2026-11-06, in one list of what is wrong with the editor: "the left hand side parts
menu is just full of text and not intuitive at all, there is overlap issues, also 5 buttons at the
top, all with text, not intuitive at all, also when you click on a part a menu comes up, we should
use the left hand side parts menu and display the information there [...] i want to be able to grab
a part and move it [...] we also need to add symetry modes, mirror modes, part clipping by
different offets, like 1degree 5degreee 20 degree 45 ect. And we should use icons where we can."
That list is T207 and T212 to T218.

One set of SVG icons drawn in the HUD's style (`assets/core/icons/`, stroke icons, the teal accent
for what is on), so the editor's bar, parts list and part settings can use pictures instead of words.

## Scope

- The bar: back, vessels, parts, shape, move, turn, undo, redo, symmetry (radial, with its count;
  mirror), snap, launch site.
- Part kinds for the list's tabs and rows: tank, structural, nose cone, decoupler, fin, wing, pod,
  engine, control (RCS, reaction wheel), plus what the existing ones (`engine.svg`,
  `decoupler.svg`, `structure.svg`, `rcs.svg`) already cover.
- Section icons for the part settings: shape, tank, placement, comes to.
- Every icon button has a tooltip with its name and key.

## Done when

- The icons are in `assets/core/icons/editor/`, each readable at 20 px, shown together in a capture
  next to their names.
