# T075: Decide the scripting runtime for code mods

Code mods need a scripting layer: decide between WASM and an embedded language.

## Decision (2026-11-04)

Neither, for now. Rust is enough and the game adds no other scripting language. Mods stay data, and new behaviour is written in Rust. Written into T136.

## Done when

- The choice and its reason are written into the code mods task, and that task is unblocked.

## Where it comes from

- Early feature checklist (since removed), Decisions to make
