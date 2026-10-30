# T018: Editor scene in the game

`osp_game::editor`: the place where a craft is built, with camera, rendering, picking and placing by the mouse.

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

## Where it comes from

- docs/build-editor-plan.md (E6)
