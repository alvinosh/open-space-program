# T151: Trees glitch near the ground

Logged 2026-10-22: trees glitch near the ground. The camera-orbit load and unload part was fixed that day (hysteresis in `trees.rs`); the overlap and LOD-height suspects were left open. This comes from an earlier note, not the old issue list: check it still happens first.

## Done when

- Reproduce it, then fix it or drop the task.

## Checked

2026-11-01, `OSP_GRAPHICS=high`, captured with `OSP_CAPTURE_HIDE_UI=1`:

- On the pad, 60 m from the tower at a 3 degree pitch (`OSP_SCENARIO=pad OSP_CAMERA=60,3,60,5`).
- On the runway, 25 m from the glider at a 1 degree pitch, from four headings (`OSP_SCENARIO=runway OSP_CAMERA=<0|90|180|270>,1,25,1`).
- A launch from the pad with stills at 8, 14, 20 and 26 s (`OSP_CAPTURE_AT=8,14,20,26 OSP_CAMERA=200,6,150,30`).

Trees stand on the ground (no floating or sunken ones), cell borders show no overlap or gap, the detailed and cheap trees meet without a visible seam, and nothing popped between stills. Both suspects are also unlikely on reading `trees.rs`: cells are quadtree nodes that do not overlap, and the Cape is flat so the terrain mesh and `terrain.height` agree. Not tried: hilly ground (the only land with data is the Cape), moving the camera by hand, or the Low and Medium tree settings. Dropped for lack of a repro; if it shows again, note where and at what settings, and set it back to `todo`.

## Where it comes from

- `crates/osp_render/src/trees.rs`
