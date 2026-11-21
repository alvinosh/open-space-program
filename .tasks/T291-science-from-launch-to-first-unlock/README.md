# T291: Smoke: a science game from the first launch to the first unlock, Casual and Realistic

A script flies the root's parts with an instrument to orbit, gets the data home, earns science, and
unlocks the first node, in both modes; the numbers a player sees are reported.

## Done when

- It passes in Casual and Realistic with no gaps.

## Built (2026-11-20)

`crates/osp_smoke/src/career.rs`, missions `career_casual` and `career_realistic` (`cargo smoke
career_`). Each loads its game mode's mods, starts a career on the mode's tree, takes on what it is
offered, builds a first rocket with the builder from the roots' parts (sized by `fitting` as the
editor does), checks the facilities take it, flies it, and spends what it earned. Both pass with no
gaps.

- Casual: a C-1 with its parachute on a solid booster, four fins, thermometer, barometer and
  telemetry on the booster. First flown with the booster's thrust raised to 800 kN by hand, since
  as the parts list sized it the rocket arced over at 2.7 km (T331, fixed by a reaction wheel in
  the C-1); then the list's booster sent it to 606 km and the crew died coming down (T332, fixed by
  sizing a booster for what it lifts). Now, as the list gives it: 22 km, 11.2 science by telemetry,
  the crew splash down at 7.8 m/s and are recovered; First launch pays 8,000; Aerodynamics (5)
  unlocked, the Builder upgraded (40,000), 18,000 left.
- Realistic: a solid sounding rocket (0.6 m by 4 m, 60 kN), nose cone, four fins, the same three
  instruments. 15.4 km, 2.31 science by telemetry at the link's rate; First rocket pays 20,000;
  Early Solid Rocket Engines (1) unlocked, the Builder upgraded (200,000).

Found and fixed on the way: in a game that sends data all at once (Casual), the vessel's log said
"Data sent home: 0.0 science" every tick while an experiment was collecting (2,400 lines in one
flight); it now says it once the sets under way have gone (`Universe::say_sent`).
