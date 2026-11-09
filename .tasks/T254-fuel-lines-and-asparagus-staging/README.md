# T254: Fuel lines and asparagus staging

A fuel line is a small part put on the tank it drains, naming the part it feeds. Flow is one way.
Engines draw from what feeds them, the furthest upstream first, so the outer boosters empty first and
fall away with their engines still fed from the core's point of view full.

## Done when

- A craft with pairs of boosters feeding inwards keeps the core full until the last pair is gone, in a test.
- A line can be placed in the editor, with symmetry, and is drawn.
- Letting go of the source or the target ends the feed, and saved games keep it.
- The delta-v readout counts the feed.

## Done so far

Built 2026-11-09. `fuel_line: { to: <part id> }` is a component on a small part. Propellant goes one way,
across decouplers, and each engine drains what is furthest up its lines first (`Propulsion::plumb` and
`draw`). In the parts list under Structure: put it on a tank, then click what it feeds, with symmetry
copies each feeding the nearest copy. Drawn as a pipe. The delta-v readout works it through as it burns.

Left: a tank fed only through a line does not fill itself from the engines it feeds (`properties::mixes`).
Re-aiming a placed line means taking it off and putting on another.
