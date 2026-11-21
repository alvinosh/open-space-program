# T340: A tank as the list gave it does not grow to absurd lengths

Found on 2026-11-21 rebuilding the stock craft in the builder, as a player would (the playtests `rebuild_*`, crates/osp_game/src/playtest/rebuilds.rs), after the user said "I still hate the builder, i cant even mirror fairings into a payload".

A tank still as the parts list gave it grows to feed its engine for a whole burn, keeping its
width. The Titan rebuild's first-stage tank, 0.6 m wide (see the width bug), grew to 218 m long
under an A-87, with nothing said.

## Done when

- A tank sized for its engine keeps a sane length to width (it widens, or stops and says the
  engine wants a wider tank).
