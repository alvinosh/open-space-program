# T310: Space centre: a view of the Cape, with buildings

Said by the user on 2026-11-13: "the tech tree and upgrades should not be in the editor, it should be
similar to ksp where you first have a view of the cape canaveral, and you can go into different
buildings for the missions, tech tree, building a rocket, or building a plane".

Today a new game or a loaded one opens the editor's empty hangar. The editor's bar has the tech tree
(`tree.open`) and the contracts (`ui.toggle:contracts`), and its right panel shows the career's money,
science and each facility with its upgrade (T294). New rocket and New plane are rows in its left rail.

## What it becomes

A space centre screen between the title screen and everything else: a view of Cape Canaveral, the
buildings on it picked to go in. Each building is one thing:

- Rockets (as KSP's VAB): the editor, new rockets and the craft list for the pad.
- Planes (as KSP's SPH): the editor for planes, for the runway.
- Research (R&D): the tech tree.
- Mission control: contracts and the scenario's goal.
- The launch pad and runway: their levels and upgrades, and launching what was last built.
- Tracking (maybe): vessels in flight, to go to one.

Facility upgrades move out of the editor to the building they upgrade. The editor keeps only building
the craft and launching it. The money and science stay in sight on the space centre.

## To ask before building

1. The view: the game's own 3D Cape (its terrain, the pad and the landing strip as they are, the
   buildings as models placed on it, the camera slowly circling) or a drawn map of the Cape with the
   buildings on it. Recommended: the 3D Cape, as KSP does, since the site and its terrain are there
   already; buildings would need models (Blender, `tools/blender`).
2. Which buildings, and whether Tracking is one.
3. A sandbox game: the same space centre without research, contracts and upgrades, or straight into
   the editor as now.
4. Whether a building's upgrade shows on the view (a bigger pad, a taller VAB) or only as its level.

## Answers (2026-11-13)

1. 3D: the game's own Cape, with building models on it.
2. A tracking station is one of the buildings.
3. Not answered yet: a sandbox game. Guess until then: the same space centre, with Research, Mission
   control and the upgrades closed (they belong to a career).
4. Upgrades show in the view: a bigger pad, a taller building.

## Split into

- T311 The space centre screen (the view, the camera, going into a building), placeholder blocks.
- T312 Building models, one per level.
- T313 Upgrades on the buildings, shown in the view; out of the editor.
- T314 Research and Mission control buildings: the tech tree and contracts, out of the editor.
- T315 The tracking station.

## Done when

- T311 to T315 are done, and a new career starts on the space centre and goes from building to
  building to launch.
