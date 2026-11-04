# T194: Saturn V: the depth the gate left out

The Apollo gate (T180) was closed with every system working and its values rough, as the user asked on 2026-11-04 ("dont focus on having the resulting values accurate"). This is what that left, from those tasks' notes.

## Heat

- Sun, planet and internal heat, conduction between parts, radiators and heaters. Today a lump comes back to 290 K by itself.
- S-IVB boil-off.
- A temperature limit for each material, and radiative heating at lunar-return speeds.
- The ablator's mass coming off, charring, and sizing it from the heat load.

## The way down and home

- The entry's peak is about 8.4 g against 6.5 g, and it aims for no landing point.
- A touchdown point to pick, and the Moon's relief (T042).
- Sizing legs from mass and slope, and legs that unfold over time.
- Floating by buoyancy from the shape, moved by the waves. Reefing. Recovery.

## Crew

- Cabin pressure, CO2, a temperature limit. People and not a count.

## Build

- The lander's parts are copied into `saturn_v.json5`: an assembly for both.
- The S-II and S-IVB are too long, which the tower shows: the stack is about 118 m against 110.6 m.
- Automatic abort triggers.
- The stage key after docking fires the next sequence on the flown vessel's own list, which is the service module's jettison.
