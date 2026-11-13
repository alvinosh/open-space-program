# T284: Science buys the tech tree

Decided in T271 on 2026-11-10: as in KSP's career with Kerbalism. Data that reaches the ground, sent or
recovered, earns science by its experiment's value, where it was done and on which body, less each
time the same is done again. Science unlocks the tree's nodes, and a node's parts show in the parts
list only once it is unlocked. Each save keeps its science and what it has unlocked.

The base game needs a tree of its own for its parts (with T249's short list); Realism has RP-1's nodes
(`tech_nodes`), which today unlock by date. A guess to confirm: a new game can still start with every
part, as a sandbox, and the date rule stays for historical scenarios.

## Done when

- Sending a measurement from Earth orbit earns science, doing it again earns less, and the science
  unlocks a node whose parts then show in the parts list (test).
- A save keeps its science and unlocked nodes.

## 2026-11-10

Confirmed by the user: a new game can still start with every part, as a sandbox.

## 2026-11-12

Pacing asked for by the user (T301): Casual goes sounding rockets, suborbital, orbit, Moon; Realistic
much slower, with many more sounding rockets and uncrewed probes before the first crew. The rewards
are set so each mode's tree is earned at that pace.

## Done (2026-11-13)

- What a set is worth: its experiment's `value` times its body's figure for the situation
  (`bodies` `science.values`, `SituationValues`, 1 where not given; `Place::value`). Worked out where
  the set is made and kept with the data on the drive (`Held::worth`), so whatever comes home carries
  its own worth.
- Less each time (`science::earned`): the first whole set of a subject that reaches the ground earns
  all it is worth, the next half, then a quarter, smoothly for parts of sets, twice it in all.
- `Career::receive` earns it, from data sent (`Universe::deliver`) and from what a recovered vessel
  held (`Universe::recover_vessel`). The recovery notice says the science; a vessel whose drives are
  clear after sending logs "Data sent home: 10.0 science". An experiment's window shows what a set
  where it is is worth.
- Content: Casual's own figures for the seven bodies (Earth 0.25 on the ground to 1.4 high in space,
  the Moon 2.5 to 5, Mars 5 to 10, Venus and Mercury to 12, Jupiter to 15, the Sun 10 low in space);
  Realistic takes RP-1's DataValues (`mods/realism/patches/science.json5`, credited in its LICENSES).
  Casual pacing, roughly: a thermometer and barometer up to space and back is about 21 science (the
  first nodes cost 5, the second tier 18).
- Tests: `science::tests::each_set_of_the_same_earns_half_the_one_before`;
  `comms::tests::sending::data_sent_home_earns_science_less_each_time_and_unlocks_the_tree` (sent from
  Earth orbit: 10, the same again 5 more, then a node whose engine the parts list has); the drive
  test's recovery earns what a set is worth in space near the Earth; `tests/save.rs` keeps what has
  come home through a save.

## Left out

- Found on the way, T305: neither tree's first node can bring a first flight's science home (no
  parachute in the parts list at all; no antenna or battery at Realistic's start). The loop works,
  but a new career cannot get started on it until that is done.
- Not seen in the game: the recovery notice with science and the experiment window's worth (the
  science capture runs every mod, so a career there is on RP-1's tree, where its capsule is locked).
- Repeated subjects all count the same, crewed or not, sent or recovered (KSP gives less for sending).
