# T191: Boxes around vessels: click to target or switch

Vessels get the box the planets have. Click it to target the vessel or fly it.

## Which vessels

- Map view: every vessel that is not debris, at any distance. Debris keeps its dim dot, and a click on it opens the same menu.
- Flight view: vessels that are not debris within 100 km (`BOX_RANGE`), and the target at any distance.
- Never the vessel being flown. Not while the menu or editor is open. Not behind a body (`hidden`).

## The box

- `body_markers.rs` becomes one marker layer for bodies and vessels, with one crowding rule: the target first, then nearest.
- Box as for bodies: the object's size on screen plus padding, at least 18 px across. A vessel's size is its `propulsion.length()`. Name under it; in flight view with the distance, "Kestrel, 2.3 km".
- When a vessel is too big on screen for a box to help, only the name stays and stays clickable, so right-clicking its parts still works.
- Target's colour when targeted, white under the pointer.
- The other-vessel dot in `orbit_lines.rs` goes for those with a box; their orbits stay.

## The menu

- `marker_menu` is a popup in the `context_menu` classes: the name and distance, then for a vessel Set as target or Clear target, Switch to vessel, Centre view (map only); for a body Set as target or Clear target, Centre view.
- A vessel's box opens it on a left or right click. A planet's box opens it on a right click; a left click still centres the map on the planet, as it does today (the user's choice, 2026-11-04).
- Items fire the actions the Manifest already uses: `target.vessel`, `target.body`, `target.clear`, `vessel.fly`, `manifest.follow`, `manifest.follow_body`. No new targeting logic.
- Shift+click still targets directly.
- `game.fly` and `vessel.fly` become one action with one camera behaviour: frame the new vessel, stay in map view if there, clear the selected burn.

## Done when

- In `OSP_SCENARIO=rendezvous`, a scripted pointer clicks the other vessel's box in the flight view and in the map, sets it as target, clears it, and switches to it; checked by script and capture.
- A test for which vessels get a box: range, target, debris, the one flown.
- A map click on a box does not also drop a burn or select a node.
