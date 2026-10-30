# T050: Failures and reliability

The faults store (per-part health, wear against rated time, causes: over temperature, over load, over use, chance), `EngineDef::reliability` read in full (only `rated_burn_time` is read today), a seeded generator per vessel so a smoke test repeats.

## Done when

- An engine past its rated burn time fails by its record, and the same seed gives the same failure.

## Where it comes from

- docs/systems-plan.md (step 25)
