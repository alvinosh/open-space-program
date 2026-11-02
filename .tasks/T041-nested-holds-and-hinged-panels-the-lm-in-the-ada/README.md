# T041: Nested holds and hinged panels (the LM in the adapter)

The lunar module sits inside the adapter, which holds it on releasable joints, and the adapter's panels open on actuated joints. Today 'a stage cannot let go from inside another'.

## Scope

- A release that is not at the end of the stack.
- The container component already marks inside surfaces and nodes.

## Done when

- The smoke test does transposition and extraction with the real lunar module, from the S-IVB.

## Notes

- 2026-11-02: moved up for the Moon mission. The adapter panels may simply let go until actuated joints (T030) exist.

- 2026-11-02: from T039, what a plain decoupler sheds loses its sequences (`Propulsion::from_released`; only a vessel letting go of a join keeps them, by `Sequences::take_parts`). The CSM separating from the adapter sheds the S-IVB with the lander on it, so the lander's own sequences must survive that. A release in a stack that a join turned round already goes the right way and leaves in its own frame.

## Where it comes from

- docs/systems-plan.md (step 16)
- old issue list, issue 8 (docs/issues.md, removed; see git history)
