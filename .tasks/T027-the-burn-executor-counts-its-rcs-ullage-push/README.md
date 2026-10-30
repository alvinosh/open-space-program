# T027: The burn executor counts its RCS ullage push

The executor pushes the stack forward on RCS for 3 s before an engine that needs ullage lights, and only counts delta-v from ignition, so every burn overshoots by about 0.9 m/s (measured: planned 22.53 and 1.68 m/s, changed by 23.39 and 2.52 m/s). It also made the return miss the entry corridor.

## Scope

- Count the push in `autopilot::BurnExecutor::account` (or plan for it).
- Allow RCS-only burns.
- Remove the smoke test's RCS trim helper (`planning::rcs_trim`) once burns are accurate.

## Done when

- A planned 22.5 m/s delivers 22.5 within 0.1 m/s, and the mission smoke test's 'executor does not deliver the planned delta-v' finding flips to a pass.

## Where it comes from

- docs/systems-plan.md (step 2)
- old issue list, issue 10 (docs/issues.md, removed; see git history)
