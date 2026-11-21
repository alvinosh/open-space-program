# T342: A hollow tube can be picked by clicking it

Found on 2026-11-21 rebuilding the stock craft in the builder, as a player would (the playtests `rebuild_*`, crates/osp_game/src/playtest/rebuilds.rs), after the user said "I still hate the builder, i cant even mirror fairings into a payload".

A click on the middle of a hollow tube (Structural) picks what is inside or behind it: on the
Comet, clicking the fuselage tube picked the capsule through its open end. Only a click right on the
tube's wall picks it.

## Done when

- A click anywhere inside a tube's outline picks the tube, unless a part in front of it is hit.
