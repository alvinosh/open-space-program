# T251: A smaller Solar System for casual games

Said on 2026-11-09: a smaller system could work. It replaces "real Solar System only" (T076) for the
casual game. These are the real planets made smaller, not a fictional system.

At a tenth of the radius with the same surface gravity, orbital speed is about 2.5 km/s and reaching
orbit about 3.4 km/s, against 9.4 km/s now. Masses, atmosphere heights, orbits and terrain heights
have to scale with it.

## Ask first

- Whether the base game is the small system with a Real Scale mod on top, which fits T243.
- The scale, after flying a casual rocket on it.

## What is in the way

Mods can be swapped while the game runs, but the solar system cannot. `Universe.system` and the terrain
in `WorldData` are built once at startup, and `Universe::restore` keeps the system that is there. A
casual and a realistic save side by side need the system and terrain rebuilt on a new game and on
loading a save whose scale differs.

## Done when

- A new game on the small system reaches orbit on a stock rocket, and the terrain, sky and map look
  right at that scale.
- Saves of both scales load in one session.
- `cargo smoke` still passes at real scale.
