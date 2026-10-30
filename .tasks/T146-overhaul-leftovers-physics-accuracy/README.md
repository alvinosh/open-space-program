# T146: Overhaul leftovers: physics accuracy

Small accuracy items the overhaul left open. Each is a line in the original notes.

## Scope

- Tank walls: reverse pressure on a common bulkhead, and compressive hoop stress at a shallow dome, are not checked.
- Joint loads are axial and per stage joint; joints inside a stage, shear and bending are not computed (see also the structure task).
- On the ground, the stages standing on it share its push by their mass.
- Vents narrower than a row of the aero cut are not seen at all; ports are drawn as gaps, and the air sees into an open interstage only as far as its exposed faces.
- The voxel shape is built on the sim thread when the parts change, not in the background (about 12 ms for the Saturn V).
- A control surface is part of its lifting surface and flies today's flap model (see the control surfaces task).

## Done when

- Each item is done or dropped with a reason.

## Where it comes from

- vessel overhaul plan, carry-over notes (docs/vessel-overhaul.md)
