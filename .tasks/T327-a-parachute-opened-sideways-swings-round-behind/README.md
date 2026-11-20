# T327: A parachute opened sideways swings round behind the craft

Said by the user on 2026-11-16: "the parachute does not behave correctly when opening it sideways for
example, it needs to change position dynamically".

## What it does now

- The pull (`systems/parachute.rs`, added in `vessel.rs`) acts at the part the chute is on, straight
  along the air there, from the moment it opens.
- The canopy (`update_canopies`) is drawn each frame straight down the wake from that part, 1.5
  canopies away. It has no place of its own: whichever way the craft moves or turns, it is in the
  wake at once. Opened while the craft goes sideways, or on a part's side, it appears behind with no
  swing, and the craft gets the full pull along the air with no lag.

## What it should do

- The canopy is a thing of its own on its lines: it leaves the pack the way the pack faces, fills,
  and swings round into the wake with its own drag and mass, as a pendulum on the lines.
- The pull acts along the lines, from the canopy to the part, not along the air: a chute that has
  not swung round yet pulls the craft round, and the craft ends up hanging under it.
- A cluster spreads out by itself rather than at fixed turns.
- Drawn where it is, so what is seen is what pulls.

## To ask the user

- What they saw: a radial chute on a capsule's side, a chute opened while flying level, or the
  canopy jumping as the craft turns.

## Done when

- A capsule with a chute on its side, opened at 200 m/s while going level, shows the canopy coming
  out sideways and swinging behind over a second or so, and the capsule turning to hang under it, in
  a capture.
- The Saturn V's command module still comes down at about 9 m/s (the smoke test).
