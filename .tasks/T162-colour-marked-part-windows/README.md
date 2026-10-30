# T162: Colour-marked part windows

Right-clicking a part (or its tile in the staging window) opens its window; the part should also be marked on the rocket, and with several windows open each gets a colour of its own, worn by the window, the part on the vessel and the staging tile, so it is clear which window belongs to which part.

## Scope

- Each open part window takes the first free colour of `MARK_COLORS` (six, none of them a stage colour) and gives it back when it closes.
- The part's meshes are tinted in that colour on the vessel (a slow pulse so a white part still stands out); it works from the part id, so models and gear are covered.
- The window's frame (`accent` on the window) and its tile in the staging window wear the colour.
- Right-click on a staging tile opens the window of the tile's first part and marks all parts of the tile (three identical boosters: all three).
- Hiding the flight UI group hides the marks.

## Done when

- Two pinned windows on two different parts show two colours on the frames, and the same two colours on the parts (captured screenshot).
- A right-click on the tile of three boosters tints all three and colours the tile.
- Closing a window removes its tint.

## Done

- `part_highlight.rs`: `PartMarks` (colours per window, tile parts), the tint (a child copy of each mesh of a marked part, unlit and blended, one shared material per colour).
- `osp_ui`: a node may have `context` (action on right-click) and `accent` (data key holding a colour for its border, `accent_width`).
- Staging tiles have `context: part.group:<vessel>:<part>,...` and `accent: item.mark`; `part_info` publishes `self.mark`.
- Tests: `windows_get_different_colours_and_give_them_back`, `a_window_from_a_tile_marks_all_its_parts`, `a_tile_names_its_vessel_and_parts`.

## Open

- Checked in a capture only (`OSP_CAPTURE_PARTS=1`, the scripted run in `smoke_test.rs`): a nine-engine tile and a fairing window got two colours on frames, tiles and parts, and closing one took its tint off. No z-fighting seen. Not tried with the pointer by hand, with more than two windows, or on engines with heat glow.
- Windows opened one after another at the same pointer spot overlap; they can be dragged apart.
- The tint has no outline; T157 may want a real outline pass for hover.
