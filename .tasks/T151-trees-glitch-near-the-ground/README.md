# T151: Trees glitch near the ground

Logged 2026-10-22: trees glitch near the ground. The camera-orbit load and unload part was fixed that day (hysteresis in `trees.rs`); the overlap and LOD-height suspects were left open. This comes from an earlier note, not the old issue list: check it still happens first.

## Done when

- Reproduce it, then fix it or drop the task.

## Where it comes from

- `crates/osp_game/src/trees.rs`
