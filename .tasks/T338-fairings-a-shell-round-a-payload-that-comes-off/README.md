# T338: Fairings: a shell round a payload that comes off in pieces

Found on 2026-11-21 rebuilding the stock craft in the builder, as a player would (the playtests `rebuild_*`, crates/osp_game/src/playtest/rebuilds.rs), after the user said "I still hate the builder, i cant even mirror fairings into a payload".

The parts list has no fairing, and a part's settings cannot make one: the hand-made stock
craft had shell halves (`span: [-90, 90]`) with a decoupler on each, but nothing in the builder
makes a half shell or lets a shell go. Seven stock launchers were rebuilt with a nose cone straight
on the payload instead. Mirror symmetry cannot help: it copies whole parts.

## Scope

- A fairing as KSP's procedural fairings: a base on the stack, walls drawn up round what stands on
  it, closing to a nose (ogive, cone), as wide as the base or wider.
- It splits into two or more pieces when staged, which fall away.
- Its size follows the payload: wider or taller payload, wider or taller fairing.

## Done when

- The Sparrow, Titan, Talon, Pathfinder, Heron, Kestrel and Colossus rebuilds put their payloads
  under a fairing, and it comes off in pieces in flight.
