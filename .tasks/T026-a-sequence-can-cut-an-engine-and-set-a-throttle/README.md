# T026: A sequence can cut an engine and set a throttle limit

A sequence lights engines, releases decouplers, jettisons fairings and switches RCS on, and nothing else. The real S-IC cut its centre F-1 at 2 min 15 s to hold 3.9 g; the sim's stack reaches 5.1 g at burnout.

## Scope

- New activations `cutoff` and `throttle` in `staging::Activation`: cut an engine or group, set a throttle limit per engine or group.
- The Saturn V's sequences cut the centre F-1 at 2 min 15 s.
- The staging window shows them.

## Done when

- The mission smoke test's S-IC peak acceleration is 4 g or under, and its 'known issue' finding for it flips to a pass.

## Notes

- T029 generalises this into actions on any component.

## Where it comes from

- docs/systems-plan.md (step 1)
- old issue list, issue 9 (docs/issues.md, removed; see git history)
