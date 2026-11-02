# T027: The burn executor counts its RCS ullage push

The executor pushes the stack forward on RCS for 3 s before an engine that needs ullage lights, and only counts delta-v from ignition, so every burn overshoots by about 0.9 m/s (measured: planned 22.53 and 1.68 m/s, changed by 23.39 and 2.52 m/s). It also made the return miss the entry corridor.

## Scope

- Count the push in `autopilot::BurnExecutor::account` (or plan for it).
- Allow RCS-only burns.
- Remove the smoke test's RCS trim helper (`planning::rcs_trim`) once burns are accurate.

## Done when

- A planned 22.5 m/s delivers 22.5 within 0.1 m/s, and the mission smoke test's 'executor does not deliver the planned delta-v' finding flips to a pass.

## Done (2026-11-02)

- What an ullage push gives while the executor waits is part of the burn: what is left to do at ignition is the burn less the push.
- A burn that would take the engine under 2 s at full thrust, or with no engine to light, is flown on the RCS, starting half its push ahead of the burn's time (`Phase::Pushing`).
- The smoke test's `rcs_trim` helper is gone. Its finding is a pass the flight is held to: the first correction delivers 0.05 m/s over, the return correction 0.001 m/s.

## Where it comes from

- docs/systems-plan.md (step 2)
- old issue list, issue 10 (docs/issues.md, removed; see git history)
