# T157: Highlight the parts of a stage on the rocket

Engines and stages are not marked on the rocket. Hovering an icon in the staging window should show which parts it is, and the next stage to fire should be marked.

## Scope

- Hovering a staging row or icon outlines or tints its parts on the vessel (engines, decouplers, tanks of that stage).
- The engines about to light are outlined while the sequence is next; a running engine glows with its throttle (T120 is the heat glow, this is the selection).
- In the editor (T019) the same highlight is used for a selected part.
- It works from the part id, so it also works for a part inside a cluster or a symmetry group.

## Done when

- Hover on an icon of three identical boosters outlines all three; hover on one of a split count outlines that one (captured screenshot).

## Done

- `osp_ui`: a node may have `hover` (an action template, like `context`); `UiHover` holds the filled-in text of the node under the pointer. The staging tiles and a row's number say `part.group:<vessel>:<parts>`; rows have a `window` key (every part of the row) beside the tiles'.
- `part_highlight.rs` marks three things with the tint T162 made, one per part, in this order: the parts under the pointer (strong blue; white vanished on white engines), a part window's colour, and the engines of the next sequence while the staging window shows (steady teal, the HUD accent). Hover goes with the flight UI group.
- It works from part ids, so a tile of three identical boosters marks all three, and a tile split off a count marks just that one.
- `part.hover:<vessel>:<parts>` (empty to clear) stands in for the pointer in capture scripts. `OSP_CAPTURE_HOVER=1 OSP_CAMERA=200,22,13,-14 OSP_SCENARIO=pad` captured the nine-engine tile blue, then one engine split off into a sequence of its own, blue alone among the dark ones (teal before any hover).
- Tests: `the_node_under_the_pointer_says_its_hover_text`, `the_parts_of_a_tile_under_the_pointer_are_hovered`, `hover_is_gone_with_the_flight_ui_and_a_script_can_set_it`, `the_pointer_beats_a_window_which_beats_the_next_engines`, `the_next_engines_are_those_of_the_first_sequence_only`, `a_tile_and_a_row_name_the_parts_to_mark`.

## Open

- Not tried with the real pointer by hand; the capture used the scripted hover. The path from the pointer to `UiHover` is only tested with a node set to hovered.
- A tint, not an outline, so a part the colour of its tint fades (a blue part under blue). A real outline pass would need a shader.
- Hovering a row marks its activations' parts (engines, decouplers, fairings, RCS), not the tanks of the stage.
- The editor (T018, T019) has no selection to mark yet; it can set `PartMarks`' hover for its selected part (needs a public setter then).
- A running engine's glow is T120.

