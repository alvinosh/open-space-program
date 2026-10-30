# T164: Rockets that fly: fix the craft that barely work

Raised in play on 2026-10-30: many of the rockets barely work and more should fly the way they did. Fix the craft files by hand (not the Realism generator), one craft at a time, until each flies its own sequences to its mission, and back each with a test as `tests/sounding_rockets.rs` does.

## Order

1. Find out which fly: fly every craft of `assets/core/craft` and `mods/realism/craft` through its own sequences from its own launch site (a script, as `fly()` in `sounding_rockets.rs`) and list what reaches its apogee or orbit, what stalls on the pad, what tumbles, what breaks up. The list becomes one task per broken craft under this epic.
2. Fix them from the list, worst and best known first (Saturn IB, Falcon 9 and Soyuz are the usual suspects), one per turn.

## Done when

- Each craft is either flying its mission in a test or has a task that says why not.
