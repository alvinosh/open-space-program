# Hand-made craft, kept for the simulation's tests

The game's craft were written by hand until 2026-11-21, when they were replaced by craft built in
the builder (the `rebuild_*` playtests, `crates/osp_game/src/playtest/rebuilds.rs`). The unit tests
of `osp_sim` that were written against the hand-made ones (a glider's dolly, a satellite's arrays, a
fairing's halves, the stages of the Pathfinder) still read them from here. The game does not ship
them.
