# T312: Space centre building models

Part of T310. Models for the buildings T311 places: vehicle assembly, plane hangar, research, mission
control, tracking station (dishes), and the pad's and runway's structures, each in a model per
facility level so an upgrade shows (T313). Made in Blender with the scripts in `tools/blender`,
headless (never resetting the user's open Blender); new `.glb` files need `git add -f`. Our own
designs, not KSP's or NASA's buildings copied.

## Done when

- Every building has a model per level, drawn on the space centre, and its picture is in the task.

## Built (2026-11-17)

- `tools/blender/space_centre.py` (headless: `blender -b --factory-startup --python
  tools/blender/space_centre.py`) makes `assets/core/models/space_centre/`: the Vehicle Assembly and
  the Hangar at each of the builder's four levels, Research (two wings and a domed drum), Mission
  Control (a glass front between fins, a tower with a mast) and the Tracking Station (a big dish on a
  pedestal, a small one, an operations block). Our own designs; 600 to 5,000 triangles each.
- A building's record has `facility` and `levels` (a model and the box round it at each level). The
  level that stands is the career's level of the facility, a new career's first, a sandbox's top one
  (`osp_game::space_centre::level_of`); it changes as soon as the facility is upgraded, and the
  click box follows it. The one pointed at glows teal over its own colours.
- `career_centre` upgrades the builder at the space centre and the Vehicle Assembly and Hangar grow.

Pictures: `career_level_2.png` (a career after its first builder upgrade), `sandbox_top_level.png`
(a sandbox, every building at its top level, Research pointed at).

## Left

- The pad's and the runway's structures per level: the pad is still drawn in code
  (`osp_render::launch_site`) the same at every level, and the runway has no levels yet (T313).
- The `.glb` files are gitignored: `git add -f assets/core/models/space_centre/*.glb` when committing.
