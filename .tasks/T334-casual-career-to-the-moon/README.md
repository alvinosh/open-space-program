# T334: Smoke: a Casual career from orbit to a crew on the Moon and back

Part of T301. On from T333's first orbit: a crewed orbit and return, a Moon flyby, a Moon orbit, a
landing, and a crew landed on the Moon and brought home, each craft built from what the career has
unlocked by then and paid for with what it has earned.

## Done when

- The Casual career runs from the first launch to a crew back from the Moon with no gaps, each
  contract met, reported per flight as T333 does, with the flights and game time in total.

## Progress (2026-11-20)

`crates/osp_smoke/src/career_moon.rs`, mission `career_casual_moon`: T333's six flights, then
flight 7, a crew in orbit and home. The orbiter with instruments goes to 165 by 893 km, coasts to
the highest point, the A-10 burns back until the lowest point is 30 km, the capsule lets go and
falls through the air retrograde, and splashes down at 7.8 m/s with its crew; the capsule peaked at
785 K of its 1,200 K. A crew in orbit pays 120,000. After 7 flights: 14.1 science, 273,000, three
nodes. 134 checks, no gaps.

What the Moon will need, found on the way:
- No heat shield: a bare C-1 coming back from the Moon (11 km/s, periapsis 40 to 55 km) peaks at
  960 to 970 K of its 1,200 K and 8 to 14 g, and its crew splash down alive; at 65 km it skips out.
- No landing legs in the Casual parts list: whether a lander can stand on its engine or tank is
  still to try.
- Science: 14 in hand after the first orbit, and Earth's thermometer and barometer sets are nearly
  all done; the next useful nodes cost 45 (Advanced Rocketry, Uncrewed Craft, which needs Flight
  Control and Electrics first). Money is plenty (273,000: the pad's third level is 250,000). The
  Moon's sets are worth much more (its space high 6 against Earth's 1.4).
- A Moon flyby with what General Rocketry gives needs three stages (K-1 cluster, a K-1, an A-10 for
  the translunar burn) and a free return aimed at a 40 to 55 km periapsis.

## Progress (2026-11-20, later)

Flight 8, round the Moon and home, passes (168 checks, no gaps). The career buys the pad's third
level (250,000) after flight 7. The Moon Flyer, 220 t and 44 m: four K-1s under a 3 m by 24 m tank,
a K-1 under a 3 m by 7 m tank, an A-10 stage with the instruments, a C-1. On the pad the planner's
launch time for the Moon is waited for (6.3 h; due east), the K-1 stage's last propellant is let go
in orbit, and the A-10 flies the planner's 2,881 m/s departure. A 31 m/s correction puts it 502 km
over the Moon; just inside the Moon's sphere a 225 m/s burn aims the way home at a 45 km Earth
periapsis, which needs no more correcting. 1,053 K of 1,200 K and 11.8 g coming in, 7.8 m/s
splashdown. Fly by the Moon pays 300,000, and telemetry sends 33.9 science from round the Moon.
After 8 flights, 12.7 game days: 47.9 science, 323,000, three nodes.

Found on the way, in the smoke harness (fixed): the ascent pilot left its last rotation input
set, which the game takes as the pilot steering, so SAS let go every tick after a climb.

Next: a Moon orbit, a landing (no legs: whether a lander stands on its engine is still to try), a
crew landed and home.
