# T332: The parts list's solid booster under a C-1 sends it 600 km up, and its crew die of 30 g coming down

Found by the career smoke test (`cargo smoke career_casual`) on 2026-11-20, once T331 gave the C-1 a
reaction wheel and the first rocket climbed straight.

The first rocket of a Casual career: a C-1 with its parachute, a decoupler, the solid booster from
the parts list (fitted to the decoupler: 1.9 m wide, three times that long, 23 t of grain, 400 kN)
and four fins. 27.8 t on the pad, 1.3 times its weight. With SAS holding it straight it burns for
150 s, is 89 degrees up at burnout 118 km high, and coasts to 606 km. Coming back down vertically
at 2.9 km/s the capsule pulls 30 g: "Crew lost: 30 g was too much". The parachute still opens and
the capsule lands at 7.8 m/s, so the science (19.2) and the contract are earned with a dead crew.

The booster is sized as a tank is (`fitting::ASPECT`, 3 times its width), not for what it lifts:
some 3.5 km/s under a 1.4 t capsule, where a first suborbital hop wants under 2 (Mercury-Redstone
went to 187 km and pulled 11 g). It also makes the first contracts (10 km, 30 km, space) one flight.

## To decide (ask the user)

- A solid booster put under a part is sized for what it lifts: a burn that gives the stack above a
  first hop's speed (about 1.5 to 2 km/s), so shorter under a capsule; or
- the parts list's booster keeps its size and its thrust is set so it burns longer and lower; or
- the crew's g limit is looked at again (30 g is about where people die; 11 g is a real ballistic
  entry), or the tutorial says how big a booster a first flight wants.

## Done when

- `career_casual` flies the booster as the parts list gives it and its crew come home alive (the
  `known` on T332 there becomes a pass).

## Decided and built (2026-11-20)

The user said continue after the options were offered, so the recommended one was taken: a solid
booster put under a craft (a node facing down) is sized for what it lifts (`fitting::boost`,
`fitting::lifted`, `Against::lifts`): grain for `HOP_DELTA_V` (2 km/s of the whole craft, ideal) and
thrust for `HOP_TWR` (2.5 at sea level), as wide as the node or narrower until it is three times as
long as it is wide. One beside a hull, or as the root, is sized as before. The editor and the smoke
test's builder pass what the craft weighs.

Under a C-1 it comes out 0.90 m by 2.40 m at 110 kN: 4.2 t on the pad, 2.4 g. The air and gravity
take three quarters of its speed (a light rocket with a blunt nose), so it burns out at 490 m/s
and 14 km and coasts to 22 km; the crew come home, 11.2 science, a node. A longer booster, typed
in, takes it to space. `career_casual` checks the crew come home alive.
