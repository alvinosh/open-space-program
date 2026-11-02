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

## Done so far

- Engines at any pose (2026-11-01). The part's own thrust axis (`Engine::direction`) is the way it pushes; the gimbal swings across it. `attitude::actuate` and `torque_authority` use the lever arm across the thrust line, which for an engine along the stack is exactly the old height-above-the-centre-of-mass and distance-from-the-axis (a test writes the old formulas out and compares bit for bit, and the goldens and the Saturn V did not move). The game aims plumes from the same `Engine::thrust_direction`. All 11 engine cells of the placement matrix that were refused now pass all seven checks (112 red cells down to 101).
- RCS blocks at any pose (2026-11-01). A block has a 3D mount and a rotation; its four nozzles push along its own +Z, -Z, +Y and -Y, and force and torque come from the real lever to the centre of mass. The throttle pairing is the rim rule, written in the block's frame and reversed for a block whose centre-of-mass lever lies the other way along its X, so every existing block (which the stage converter turns to face outwards, a new test) gives the old figures. 6 more matrix cells are green (101 to 99 red).
- Tanks and pods at any pose (2026-11-02). A liquid settles against the vessel's aft side with its surface level, whichever way the tank points (`Mass::settled`, in `mass/settled.rs`: the cylinder cut by a plane, the volume in closed form, the plane found by bisection, the moments integrated over the chord angle). A tank standing up is the old column (the goldens did not move); upside down it is a column at the other end; on its side or tilted it is a pool. Burst pressure uses the depth of that pool along the acceleration (`Tank::pressure_leaning`), so a tank on its side has a shallow head. A tank that does not lie along the stage's axis has no buckling figure (it hangs off the stack). `Section.pod` became `pods`: a stage holds any number of pods, and a pod off the stage's axis is not its length. The storage row of the matrix is green in all twelve columns, the pod row in all but `inside` (T015) and the grey fairing cell (99 red cells down to 83).
- Tests: `mass::settled` (textbook half-cylinder, cube counting at five angles, closed-form volume against the integrals), `a_tank_on_its_side_pools_its_liquid_low`, `an_upside_down_tank_pools_its_liquid_at_the_end_that_is_down`, `a_tank_stood_on_end_either_way_pools_as_a_column`, `a_tank_on_its_side_has_a_shallower_head`, `a_pod_is_the_stages_nose_only_on_its_axis`.
- Tests: `a_block_turned_on_its_side_pushes_along_its_own_axes`, `a_block_below_the_centre_of_mass_turns_the_vessel_the_other_way`, `rim_blocks_face_outwards`.
- Tests for engines: `a_sideways_engine_pushes_and_turns_across_its_own_axis` (force, torque and authority worked out by hand), `an_engine_firing_along_the_axis_is_unchanged`, `thrust_points_along_the_engines_axis_tilted_by_its_gimbal`.

## Still open

- The RCS allocation (which nozzle serves which command) is the rim pairing: a block placed so that its nozzles serve other axes than pitch, yaw and roll as the rim rule has them gets less out of a command than it could. Force and torque are exact for any pose.
- The RCS blocks still have no surface for the air (their catalogue model has no proxy), so in every column outside a skin the matrix's aero check is red. Giving them one changes the drag of every craft with RCS blocks, so it comes with rerecorded goldens.

- A sphere's liquid is still a column in its bounding cylinder, upright in the vessel's frame (the same from every side, so a sphere's pose does not matter); a true spherical cap would change the goldens of craft with a liquid in a sphere. A tank's walls are still sized for the full column along its own axis whatever its pose (T032). `Section.tanks_top` is the top of the barrel along the stage's axis and means nothing for a tilted tank (only the stage converter's looks read it).
- The burn executor, ullage settling and the staging layout still treat an engine as pushing along the nose, so a canted engine flies but a burn planned with one steers the nose, not the thrust. A canted engine was not seen in the running game (plume aim is from the same function as the force, but no capture).
- The first order above changes: the battery and crew rows are about where a component's state lives, which is T028; they stay red until then. Next here: wings and gear, then containers, two components and the bottom decoupler.

## Where it comes from

- docs/build-editor-plan.md (E2)
- docs/systems-plan.md (The build core)
