# T215: Parts list as a grid of pictures

Asked for on 2026-11-06, in one list of what is wrong with the editor: "the left hand side parts
menu is just full of text and not intuitive at all, there is overlap issues, also 5 buttons at the
top, all with text, not intuitive at all, also when you click on a part a menu comes up, we should
use the left hand side parts menu and display the information there [...] i want to be able to grab
a part and move it [...] we also need to add symetry modes, mirror modes, part clipping by
different offets, like 1degree 5degreee 20 degree 45 ect. And we should use icons where we can."
That list is T207 and T212 to T218.

Today the list is four text tabs (Procedural, Pods, Engines, Control) and rows of a name and a line
of description. It reads as a wall of text, and nothing shows what a part looks like.

## Scope

- Tabs as icons (T212) with tooltips.
- A grid of tiles, each a picture of the part: rendered once from its model or its procedural shape
  at the default size (a thumbnail, cached), with the name under it. The description and the
  figures that matter (thrust and Isp, crew, volume) in a tooltip or a detail line on hover.
- Dragging a tile or clicking it takes the part in hand, as rows do now; search stays.
- Shares the panel with the part settings (T207).
- Filters (T208) build on this.

## Done when

- Every part in the list shows as a picture, a capture shows each tab and a hover, and a rocket is
  built from tiles in `OSP_CAPTURE_EDITOR_BUILD`.
