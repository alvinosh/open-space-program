# T049: Crew and life support

Crew in seats with mass and metabolic load; consumables as resources (pod records list food, oxygen and water); a cabin as a pressure vessel sized the way a tank is; a procedural scrubber and life-support module; crew transfer through a docked tunnel; limits that kill.

## Scope

- Acceleration (by axis and duration), pressure, oxygen, temperature; the report says why a crew was lost.
- Radiation is left out until the rest works.

## Done when

- The smoke test reports consumables left at splashdown.

## Open question

This waits for T039 (join two vessels) because crew moving through a docked tunnel is in scope. A single capsule's cabin does not need that. If life support should come earlier, split this in two (the cabin, then the transfer) and move T039 to the transfer half.

## Notes

- 2026-11-02: T040's `docking_port` has no tunnel yet: this adds whether crew can pass, next to `crossfeed`.

## Done

- Crew are counted in seats, and `life_support` has them use oxygen, water and food from the tanks they can reach. No oxygen for 5 minutes, or too much acceleration for too long, loses them. `board` moves people between seats through a docked tunnel. The Spacecraft window shows it.

## Left out

No cabin pressure, CO2 or temperature limit. They are a count and not people yet. T194.

## Where it comes from

- docs/systems-plan.md (step 24)
- old issue list, issue 13 (docs/issues.md, removed; see git history)
