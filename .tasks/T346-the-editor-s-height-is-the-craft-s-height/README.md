# T346: The editor's height is the craft's height

Found on 2026-11-21 rebuilding the stock craft in the builder, as a player would (the playtests `rebuild_*`, crates/osp_game/src/playtest/rebuilds.rs), after the user said "I still hate the builder, i cant even mirror fairings into a payload".

The vessel summary's Height reads 0 m for a payload being built (a probe core and a 1.6 m
tube) and 5 m for the two-stage Sparrow (21 parts, about 20 m tall), and 218 m once for the Titan.

## Done when

- Height is the craft's extent along its axis, whatever it is built from.
