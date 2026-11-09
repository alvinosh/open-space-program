# T248: A Realistic Engines mod: ullage, ignitions, throttle floors, start-up

In the base game an engine lights whenever asked, as often as asked, and throttles to zero.
`realistic_engines` puts `ignitions`, `requires_ullage`, `min_throttle` and the slow `response` back
with patches. It needs no simulation change.

Decided 2026-11-09: in the base game engines start instantly, with no start-up at all. Solids keep
their single ignition and no throttle, since that is what a solid is. The `realism` mod's engines
carry these fields themselves and depend on this mod.

For now the mod is on until T246 chooses the defaults, because Realism needs it and the tests load
the mods as their manifests say.

## Done when

- With the mod off, any engine relights in free fall at any throttle.
- With it on, the engines behave as they do today, and the smoke tests pass.
