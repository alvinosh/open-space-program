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

## Done so far

- 2026-11-06: the list is a grid of tiles, three across, each a picture of the part over its name
  (`osp_editor::parts_list`, which took the list out of `building.rs`). Where parts share a name
  (an engine's configs) the tile shows the config's id. A click on a tile takes the part in hand
  and a drag out of it carries `palette:<id>`, as rows did; the tile of the part in hand is lit.
- The pictures: the editor asks `osp_game::part_pictures::PartPictures` for the entries it shows
  and gets a cell of one image back for each. `osp_render::part_pictures` builds each part by
  itself, draws it as the hangar does (its model, or its geometry in its paint, an engine with no
  model in the engine metal), turned a little towards the viewer and scaled to fill a cell of a
  grid far below everything on a render layer of its own, lit by two lights and ambient light of
  its own, and one orthographic camera draws the grid. The camera runs only while a picture is on
  screen and one is unfinished; a cell is finished once drawn for 6 frames after its last change
  with no shader compiling, and the image keeps it. 120 cells, kept until another part needs one.
  Works in the dev version (blocky, matte) and the full one.
- Tabs are icons with tooltips, by what parts do: pods, tanks, engines, structure (tubes, nose
  cones, decouplers), aero (fins, wings), control (RCS, reaction wheels). `palette::Group` has these
  six in place of Procedural / Pods / Engines / Control.
- Pointing at a tile shows its name, its figures (thrust and Isp at sea level and in vacuum, what
  it burns, width and mass for an engine; crew, width and mass for a pod; size, volume and mass for
  a tank) and the start of its description under the grid (`editor.pointed*`).
- Capture: `OSP_CAPTURE=<dir> OSP_CAPTURE_EDITOR_PARTS=1 cargo run -p osp -- --dev` shows each tab,
  both filters, a pointed tile, a search and a tab's tooltip. `OSP_CAPTURE_EDITOR_BUILD` still
  builds its rocket from tiles, the decoupler dragged out of its tile.
- The menus' rule against pictures now lets the editor's layout have them (the test in
  `osp_ui::components`); the other rules hold. Text styles can say `text_align` and `break_words`,
  so a name sits centred under its picture and a long config id breaks instead of running out.
- Left: the list shares the rail with the part settings as before (T207 may want one panel). The
  realism models are not committed, so without them in `mods/realism/models` realism pods show
  nothing and their engines plain bells, as in the hangar; about 350 realism engines have no model
  at all and always show as bells. 60 tiles at most a tab, alphabetical; the rest by filter or
  search.
