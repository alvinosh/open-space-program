# T030: Actuated joints

The `Hinge` and `Slide` motions exist in `assembly.rs` and nothing drives them. Give them a state (stowed, moving, deployed, locked), a speed, limits, a power cost and an action to start them, and make the vessel follow the travel.

## Scope

- The aero shape follows (the voxels rebuild), and so do the centre of mass and the contacts.
- Used by legs, solar arrays, dishes, SLA panels, parachute doors, docking probes, grid fins, payload bay doors.

## Done when

- A leg or a panel deploys on an action, and the vessel's drag and centre of mass change with it.

## Notes

- Control surfaces move to hinges in their own task (o_control_surfaces).

## Where it comes from

- docs/systems-plan.md (step 5)
