# T337: A tank with a cluster of engines round its end is not sized for them

Found by the Casual career smoke test (T333) on 2026-11-20. One engine put on a tank's bottom node
makes the tank as the parts list gave it hold that engine's burn, or as much as the engine can lift
(`fitting::size_for_engines`). Engines put round a tank's end with symmetry, by their own nodes
along the axis (T300: three K-1s under a 3 m tank), go on by a surface mount, which that sizing does
not look at, so the tank stays as the list made it: a 3 m by 5.7 m tank under the Casual orbiter's
three K-1s, which burn it in 38 s. The smoke test types the length in (`LOWER_TANK`, 18 m: 132 t on
the pad).

## Done when

- Engines put round a tank's end are summed and the tank sized for all of them, as for one on its
  node, and `career_casual_orbit` drops `LOWER_TANK`.

## 2026-11-21

The Sparrow rebuild: nine K-0s under a 1.3 m tank (one on its node, eight round its end) left the tank
sized for the first, at a thrust to weight of 7.8 on the pad, and the eight round it hang outside the
tank's edge (no engine plate, the tank does not widen).
