# T014: Physics on any part at any pose

Replace the sorting of parts into eleven kinds in `parts/gather.rs` with a direct reading of the assembly: every part at its own pose with its own geometry and any number of components. Today a tilted engine, an RCS block that is not upright, or a part with two components is refused, and unknown components (batteries) are silently ignored.

## Scope

- `PartTree::from_craft` stops being the gate; `Hull` and the stage layout go.
- Release groups stay derived, as `Section`s already are.
- Mass, centre of mass, inertia, aero and joint loads come from each part's own pose and geometry.

## Done when

- The golden figures do not move (`tests/golden/craft.json`).
- 'until the physics runs on parts' is gone from the code.
- The placement matrix cells for tilted and sideways engines, RCS blocks, pods and tanks, and for parts with two components, are green.

## Notes

- Also covers: porting procedural tanks, fins, engines, RCS, wheels and pods fully to parts, so craft files are part trees.

## First look (2026-11-01)

`cargo smoke build` is 144 cells, 112 red, 111 of them this task's. Why they fail, by row:

- Battery and crew (24 cells): the part loads, but `gather.rs` drops the component, so its mass and place are lost (`kept` fails). They are inert as far as the physics goes; keeping them is the small first step, and the open store for their state is T028.
- Tank, engine, RCS, pod (about 45 cells): each is refused unless it stands upright on its stage's axis ("an engine stands upright, firing down its stage", "a pod sits upright on the axis of its stage"). Tilted, sideways, radial and ring placements all fall here.
- Wing and gear (23 cells): refused unless straight out from the stage's side, the same rule.
- Container, a part with two components, a decoupler on a bottom node (25 cells): "the stage with `x` needs one body" and "a stage has no part like it": the stage layout (`Hull`, `PartKind`, 133 uses in `propulsion`, `parts`, `staged`, `vessel` and the game's `vessel_render` and `part_info`) is what has to go.
- A suggested order, each step keeping the golden figures: keep every component with its mass (battery, crew); lift the pose rule for tanks, engines, RCS and pods; then wings and gear; then containers, two components and the bottom decoupler, which is the real removal of `PartKind` and `Hull`.

## Where it comes from

- docs/build-editor-plan.md (E2)
- docs/systems-plan.md (The build core)
