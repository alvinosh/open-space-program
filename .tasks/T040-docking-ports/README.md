# T040: Docking ports

A procedural docking port (ring diameter, type: probe and drogue, androgynous, berthing; mass from diameter and load; passes fluid, power, data, crew) and the last metres of a docking.

## Scope

- Contact between port rings, on the colliders of the colliders task.
- Capture sequence: contact, soft capture on a spring, retract, hard latches.
- Alignment readout, closing speed, the target port, a camera per port (the secondary viewports task), SAS target port; undock with a spring impulse.
- Placed with the builder like any part.

## Done when

- The smoke test docks the CSM with something and pulls away.

## Notes

- Also wanted: latching by contact and kinetic energy, off-centre ports.

## Notes

- 2026-11-02: moved up for the Moon mission. It no longer waits for colliders (T092), the contact solver decision (T074), actuated joints (T030) or secondary viewports (T123): the first version captures when two ports are close, aligned and closing slowly. Real ring contact and the port camera come with those tasks.

- 2026-11-02: T039 gives this `Universe::join(a, at, b, to, roll)`, `Propulsion::join_frame` (where the other would sit), `PartTree::free_nodes`, and undocking by `Activation::decoupler` on `Propulsion::joined_parts()`. A decoupler component's `impulse` on the node is the spring, and its `crossfeed` says whether propellant passes (a plain node passes it). The capture picks `roll` from the two attitudes.

## Where it comes from

- docs/systems-plan.md (step 15, Docking)
- old issue list, issue 8 (docs/issues.md, removed; see git history)
