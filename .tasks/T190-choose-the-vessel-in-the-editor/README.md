# T190: Choose the vessel in the editor

The menu sends you to the editor; the vessel is chosen there. The menu's craft list goes.

## How it came out

An empty hangar stands a vessel with no parts (`Vessel::empty_display`), so the camera, lights and renderer need no empty case. The rail is 340 px, narrower than the menu's, to leave room for the hangar. The controls hint stayed on the right panel rather than moving to the help window, which the editor hides.

After the user tried it (2026-11-04): the list is out in an empty hangar, goes away when a vessel is chosen and comes back with the Vessels button beside Back (`editor.list`); the heading only shows until a vessel is chosen; each vessel is first seen from the same angle. The view is held inside the hall's dome (`Showing::reach`). It used to be limited by a circle on the floor, so zooming out could take it through the roof, where the screen went blank. `OSP_CAPTURE_EDITOR_VIEW=1` captures those views.

## Scope

- `game.new` and "Fly a new vessel" open the editor directly (`editor.open` with no id). The hangar opens empty: the camera, the hangar and the world's hiding work with nothing on show, the staging stack stays away, and Launch cannot be pressed until a vessel is chosen.
- `editor.show:<id>` swaps the vessel on show: remove it, add the new one, re-measure, resize the hangar. Staging changes go with the one left.
- Left rail, same frame as the menu: Back, heading ("New game" or "Fly a new vessel"), the list with the shown one selected and its part count on the right, grouped by mod. There are 43 craft with the realism mod, and no text input yet to search with.
- Right panel, once a vessel is chosen: name, height, mass, parts, Launch from as a `choice` with one line naming the site or orbit, Launch as `btn_primary`. With none chosen: "Choose a vessel" and Launch dimmed, doing nothing. No icons. The controls hint moves to the help window.
- The staging stack moves beside the rail while the editor is open: a dock in the editor's layout, and a small runtime override of a layout's dock. It would sit under the list otherwise.
- Back and Esc return to the screen it came from (Title or Pause), with the game untouched.

## Done when

- `PLAY_SCRIPT` goes New game, empty editor where `editor.launch` does nothing, choose, change staging, launch; then Fly a new vessel, Back, game unchanged; then choose another, launch to orbit.
- The palette in T017 and T019 has an obvious place: the same rail.
