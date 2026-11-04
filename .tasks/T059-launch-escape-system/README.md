# T059: Launch escape system

A tower with a solid motor and a decoupler, fired as an action by an abort trigger. Needs the solid motor's thrust curve (exists on engine records).

It goes on the Saturn V too: the user asked where it was on 2026-11-04, and a completed Saturn V has one, so this moved from `later` into the Apollo gate (T180). No Apollo special case: the tower is a part like any other, and any craft can carry one.

## Done when

- An abort on the pad and at max Q pulls a capsule clear of the booster in the smoke test.
- The Saturn V carries the tower and its boost cover, and lets them go after the S-II lights, as Apollo 11 did.

## Notes

- 2026-11-04: the stack is 109.7 m without a tower, and the real one stood 110.6 m with its 10 m tower: the S-II and S-IVB as built are longer than the real ones, by about 9 m between them. The tower will make that show. It is only the build's lengths, and has nothing to do with this task, so it needs a decision of its own.

## Done

- `abort` on a craft, with `vessel.abort` on Backspace and in the Spacecraft window. The tower is parts on the capsule's apex: a cover, legs, an escape motor and a jettison motor. It goes 30 s after the S-II lights.
- `tests/saturn_v.rs` aborts on the pad, down to landing with the crew alive, and at max Q.
- A stage on top of its holder is not under it, so a tower's motors fire.

## Left out

The abort is only the player's call: nothing triggers it. No canards or pitch motor, the motor's thrust line is set through the centre of mass instead. The S-II and S-IVB are too long, so the stack with its tower is taller than the real 110.6 m.

## Where it comes from

- docs/systems-plan.md (palette table, step 25)
