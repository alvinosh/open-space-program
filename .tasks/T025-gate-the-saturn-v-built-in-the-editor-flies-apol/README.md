# T025: Gate: the Saturn V built in the editor flies Apollo 11

The editor is accepted when `cargo smoke` builds the Saturn V in it and flies Apollo 11, and the GPU run does the same with the mouse.

## Done when

- Both runs pass. The Saturn V rebuilt from procedural parts only (T052) does not start before this.

## Decided

The gate used to sit in front of the build core and every system. On 2026-11-01 the systems were put first: T028, T031, T033 (and T042, T047, T051) now depend on T014, so ship parts and systems come before the editor. This gate now closes the editor chain (T015 to T024) and still stands in front of T052.

## Where it comes from

- docs/build-editor-plan.md (E13)
