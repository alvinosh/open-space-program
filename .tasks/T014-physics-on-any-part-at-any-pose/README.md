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

## Where it comes from

- docs/build-editor-plan.md (E2)
- docs/systems-plan.md (The build core)
