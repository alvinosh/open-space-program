# T025: Gate: the Saturn V built in the editor flies Apollo 11

The editor is accepted when `cargo smoke` builds the Saturn V in it and flies Apollo 11, and the GPU run does the same with the mouse.

## Done when

- Both runs pass. Nothing in the systems epics after the build core starts before this.

## Open question

This gate sits in front of the build core and every system (T028, T031, T033 wait for it). The edge is a plan rule, not a technical need. If the systems (power, comms, life support, heat) should come sooner, point T028, T031 and T033 at T020 (the editor launches a craft) and keep this gate for T052 only.

## Where it comes from

- docs/build-editor-plan.md (E13)
