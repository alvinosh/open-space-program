# T050: Failures and reliability

The faults store (per-part health, wear against rated time, causes: over temperature, over load, over use, chance), `EngineDef::reliability` read in full (only `rated_burn_time` is read today), a seeded generator per vessel so a smoke test repeats.

## Done when

- An engine past its rated burn time fails by its record, and the same seed gives the same failure.

## Where it comes from

- docs/systems-plan.md (step 25)

## Planned 2026-11-10 (T260)

Moved from M7 into the T260 project, and widened from engines to every part. Realistic only (rules,
T261); Casual has no failures.

- Each kind of part has a mean time between failures; none fail before half of it, and almost all by
  one and a half times it.
- Engines also by ignitions and burn time, from their `reliability` records.
- A failure is a malfunction (works badly or not at all, can be fixed, T270) or critical (gone).
- Quality per part, chosen in the editor: more mass for a longer life.
- Dose past a part's tolerance shortens its life (T267).
- The part window and the monitor say what a failure would take out, where a spare would carry on.
- Chance comes from T268's seeded generators.
