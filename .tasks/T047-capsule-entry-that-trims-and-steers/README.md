# T047: Capsule entry that trims and steers

The command module tumbles on the way in, flips from heat shield first to apex first at about 100 km, and pulls 69 g (Apollo 11: 6.5 g). The pod is a uniform frustum with no RCS and no centre of mass off the axis.

## Scope

- Pods get a real mass distribution (centre of mass moves with what is inside) and their own RCS (the record data is there).
- A blunt-body hypersonic pressure model that gives trim and lift.
- A bank command that steers the lift, and an entry autopilot tool for the player.

## Done when

- The Apollo entry peaks at 7 g or under and lands within 50 km of its target; the smoke test's known issue 12 flips to a pass.

## Where it comes from

- docs/systems-plan.md (step 22)
- old issue list, issue 12 (docs/issues.md, removed; see git history)
