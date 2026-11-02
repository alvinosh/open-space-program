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

## Done

- `docking_port` component (node, kind, impulse, crossfeed) on any part with that node, and a procedural `docking_port` ring sized from its diameter and load. Probe on the CM's nose and drogue on the rim of Eagle's tunnel, where their meshes really meet.
- `docking.rs`: two free ports that fit catch when touching, within 40 cm of the axis and 15 degrees of facing, closing under 0.5 m/s, sliding under 0.3 m/s and turning under 3 degrees/s. Soft capture then draws them together with a spring and damper, equal and opposite, and the latches close within 8 s (`Universe::join`, at the roll they came in at). The vessel being flown stays the vessel.
- Undock in the part window, from either side, with the port's spring.
- `docking_info` opens by itself within 500 m and reads gap, closing speed, off axis and sliding along the pilot's axes, and what would stop a capture. In the world, a ring on each port and rings down a line along the target port's axis, green when it will catch. C looks out of the port through a crosshair. SAS `target_port` faces the port, with its navball marker. Control from here works on a port and looks out of it, translation included.
- Only the vessel flown from fires its RCS and has delta-v in the readout, carrying the other as a load.
- Mission conditions `Docked` and `Undocked`; `others` can start with `sequences_done` and `holds_attitude`; a scenario can start with `rcs`; a row in the build matrix.
- Scenario `apollo11_docking`, and the smoke mission `docking` that flies it with on/off keys changed four times a second on the window's numbers, its first try deliberately too fast. `OSP_CAPTURE_DOCKING=1` captures it in the game.

## What the first hand tests found, 2026-11-02

This was marked done on a scripted pilot alone. By hand it was first "the parts just go right through each other", then "the colliders are awful, and docking is almost impossible". What was wrong:

- Vessels had no contact, then only a round tube from the aero outline. Colliders are now the drawn triangles (T092).
- Eagle's port was 85 cm above its tunnel, at the top of the model's bounds, which is an aerial 2.3 m off to one side. The CM's was 2 cm off its nose, and both were the wrong size. `tests/collision.rs` now holds ports to their hardware, and checks that coming straight in the rims are what meet.
- Above 4x warp nothing could catch. Warp now stops 200 m short of a vessel being closed on.
- The CSM's RCS was 8.7 times too strong (T181).
- A hit left Eagle tumbling for good. It now holds its attitude.
- The readout was closed by default and nothing in the world showed where to fly.

## Still open

- A script with on/off keys is not a person: this needs a hand test before it is done.
- The target port is the nearest one that fits: a station with several gives no way to choose yet.
- A picture-in-picture port camera waits for the viewports (T123); C takes over the main view for now. The crew tunnel is T049.
- The target is forgotten when two dock, so it has to be chosen again after undocking.
- `apollo11_docking` says `sequences_done: 8` of the Saturn V's nine: T044 changes those sequences and has to keep this.
- The Saturn V and the LM are not yet flying together: T041, T044.

## Where it comes from

- docs/systems-plan.md (step 15, Docking)
- old issue list, issue 8 (docs/issues.md, removed; see git history)
