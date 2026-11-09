# T243: Casual by default, realism from mods

Said on 2026-11-09: a casual and a realistic mode. Casual is a lot like KSP, with few parts, none
of the trouble with ignitions or ullage, and just liquid fuel and oxidiser. Players turn mods on and
off, so "Realistic Fuels" gives more fuel options, "Realistic Engines" means dealing with ullage,
and so on for a lot of the game's features.

## The shape of it

- The base game (`assets/core`) is the casual game.
- Each piece of realism is a mod shipped with the game, on the Mods screen (T244).
- Casual and Realistic are named sets of those mods, picked for a new game (T246).

Most of it is data the mods already reach: `ignitions`, `requires_ullage`, `min_throttle` and
`response` are fields on each engine, so a patch can add or remove them with no change to the
simulation. Ship systems such as power, comms and life support are not per-record, so they need a
switch (T250).

Decided 2026-11-09: mods are per save (T245), a smaller Solar System could work (T251), and this
work goes first.

## Done when

- A new player gets the casual game without choosing anything, and Realistic gives the game as it
  is today, with the Saturn V still flying.
