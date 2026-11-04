# T044: The lunar module from procedural parts, and the descent tool

A procedural cabin shell with crew, descent tanks, the LMDE and LMAE (records exist), RCS quads, legs, staging on the ground (the descent stage is the ascent stage's launch pad), replacing the drum in the Saturn V.

## Scope

- `mods/realism/craft/lunar_module.json5` and an `apollo_lm` assembly.
- A player tool for the descent: hold a descent rate, hold a hover height, pick a touchdown point; the smoke test flies with it.

## Done when

- The smoke test lands at under 3 m/s on a slope under 12 degrees, stays, lifts off from the descent stage, meets the CSM and docks.

## Notes

- 2026-11-04, from T041: the lander is on the Saturn V now, its parts copied from `lunar_module.json5` with an `lm_` prefix, the two files to be kept alike until an `apollo_lm` assembly replaces both. In the smoke mission it undocks in lunar orbit and is left there; this flies it down and back. It is rooted at its ascent stage in the stack too, held by a tiny ring (`lm_mount`) that goes with it.
- The scenario `apollo11_docking` starts the Saturn V with `sequences_done: 7` to leave the CSM alone, so changing the Saturn V's sequences has to keep that right.

## Done

- The lander flies inside the Saturn V's flight: two crew across, undock, a burn down to 15 km, down on the descent tool to 1 m/s, a stay on its batteries, up, a burn to meet the command module, its speed matched, the command module docks with it, the crew back.
- The descent tool is `Guidance` (`guidance.rs`): land by itself, hold a rate, or hover, in the Spacecraft window.

## Left out

Its parts are copied into `saturn_v.json5` rather than an assembly, and picking a touchdown point is not there. The Moon is still a smooth ball (T042), so no slope is tried. T194.

## Where it comes from

- docs/systems-plan.md (step 19, Landing legs and the lunar module)
- old issue list, issue 8 (docs/issues.md, removed; see git history)
