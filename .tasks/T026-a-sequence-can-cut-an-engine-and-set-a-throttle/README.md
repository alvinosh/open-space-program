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

## Done (2026-11-02)

- `cutoff` and `throttle` are actions like any other (T029): by part or by section in a craft's sequences, by hand from the part window (Cut off on a lit engine), with icons of their own on the staging tile. A cut engine falls off its curve and can be lit again if it has ignitions left. A thrust limit multiplies what the throttle asks, held at the engine's minimum.
- The Saturn V's sequences cut the centre F-1 at 2 min 15 s, then the outboard ones with 1% of the propellant left (the real ones went on the oxygen's low-level sensors). The smoke flight's S-IC peak is 3.98 g, Apollo 11's 3.9 g, and the finding is a pass that the flight is held to (3 to 4 g).
- The smoke pilot's pitch program ends its turn at 150 km, not 160 km: with less thrust late in the burn the stack climbed to 86 km by cutoff (Apollo 11: 67 km). It is now 83 km at 2.74 km/s (Apollo 11: 2.76).

## Where it comes from

- docs/systems-plan.md (step 1)
- old issue list, issue 9 (docs/issues.md, removed; see git history)
