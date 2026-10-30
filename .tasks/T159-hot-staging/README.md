# T159: Hot staging

The sim already lets an upper stage fire through a vented interstage (`Propulsion::clear`), but a sequence cannot express hot staging. Since T161 a sequence fires its activations in the order listed, so light-then-separate fits in one sequence, but both happen in the same tick, not when the engine is up to thrust.

## Scope

- A sequence step can wait: a delay in seconds or until an engine reaches a share of its thrust. This is a small, specific piece, not T029.
- The default sequences for a stack with a vented interstage light the upper engines, then release the lower stage after the delay.
- The staging window shows the delay on the row (T155).
- The Saturn V's S-IC/S-II separation uses it (the ullage rockets and the S-II lighting, see T150 for the S-IVB).
- Crossing between the two: an engine that lights while still joined must not damage the stage below (a heat or force model is later; for now a clear exit is the only rule).

## Done when

- A two-stage test rocket with a vented interstage reaches orbit lighting the upper stage 1 s before release; the same with a closed interstage is refused or warned in the window.
