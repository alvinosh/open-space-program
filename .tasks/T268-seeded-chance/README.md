# T268: Seeded chance: one generator per save and per vessel

Failures and storms need chance that repeats: one generator for the world and one per vessel,
seeded, saved, and drawn from at event times only, so the same save gives the same failures and
storms whatever the warp.

## Done when

- The same seed gives the same events at real time and at the highest warp (test).
- The generators' state comes back from a save.
