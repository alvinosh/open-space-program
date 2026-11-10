# T258: Life support: realistic and casual

Realistic: oxygen, carbon dioxide and its scrubbers, water, food, cabin air and temperature, recycling,
waste, and people who weaken before they die, one at a time. Casual: one simple supply, as a rule (T250).

Supplies have to be placeable from the parts list too (T255 left that here).

## Done when

- A long flight without a scrubber is lost to CO2, and one with recycling lasts longer, in a test.
- In Casual a crew needs only the one supply.

## Planned 2026-11-10 (T260)

Part of the T260 project. It now builds on the rules (T261), the resource solver (T262) and crew
who are people (T263), so it does no tank drawing or dying of its own. Casual: one supplies resource
in every crewed part. Realistic: oxygen, water and food in, CO2 into the cabin air, waste water and
waste out, scrubber canisters as a process. Recycling is T265 and cabin pressure T264, so the
CO2 test here can use a cabin of fixed volume.

## Done (2026-11-10)

- Casual: the base game's rules now name a `casual` profile (`assets/core/life_support/casual.json5`):
  one `Supplies` resource (new, in the base game's resources), 1 kg a person a day, 15 days without
  it. Pods carry days of supplies in their lockers (`supplies` on a pod record: the C-1 14 days, the
  C-4 30). Nothing else: no air, no scrubber.
- A cabin (`systems/cabin.rs`) on every part with life support: its air, which takes so much of what
  is breathed out (the profile's `cabin`, kg per m³, over its volume: `room_per_seat` or the part's
  `volume`), and its lockers, full of `days` of what its seats need and its fitted processes burn,
  plus what `carries` says. The resource solver reaches a part's cabin for anything on that part
  (`System::stored`, `set_stored`). What the lockers carry is in the part's mass as built.
- Realistic (`mods/realistic_life_support`): breathing makes CO2 into the cabin's air, drinking makes
  waste water and eating waste (dumped when there is nowhere to keep them). Every cabin is fitted
  with a LiOH scrubber a seat (up to 1.2 kg of CO2 a day each, 20 W), and carries the LiOH for its
  days. With the air full, nobody can breathe out, and they go short of air. A `water_recovery`
  process turns waste water back into 85 % water. New resources CO2, WasteWater, Waste, LiOH.
- The solver now runs a step's lots together instead of one after another: a scrubber takes what is
  breathed out in the same step, a recycler's water is there to drink, and what may be dumped is
  there to take before it is. Each resource's takers and makers are cut back, lowest priority first,
  until the bins balance at the end of the step; with steady rates they stay in bounds all through.
- The Saturn V's command and lunar modules carry LiOH (35 and 8 kg) in the craft data.
- The parts list has a Supplies tank holding a day's worth of each need in proportion, insulated when
  one of them is cryogenic. The Spacecraft window shows each supply with the days it lasts, and how
  fouled the cabin air is.
- Tests: a cabin without LiOH is lost to CO2 when its 0.3 kg of room fills (0.3 days for one person),
  to the second at the highest warp, and one with LiOH lives ten days with clean air; water recovery
  keeps 9 kg of water going past five days where without it runs dry in three; lockers carry days of
  each need and the scrubber's LiOH; in Casual a capsule's one person needs only Supplies, 14 kg of
  it, from the lockers (tests/base_game.rs). The solver's day-in-one-step test now includes the crew's
  scrubbers.
- 778 tests across the sim, game, mods and smoke crates, every smoke mission among them; two old
  base-game tests changed to say the base game now has Supplies and casual life support.
  `cargo smoke saturn_v`: 143 passed, 0 gaps, all three crew home, 43 kg oxygen, 124 kg water (the
  scrubbers make some) and 19 kg food left.

## Left out

- What lockers carry does not come off the part's mass as it is used.
- Cabin pressure, oxygen and nitrogen in the air, leaks: T264. The cabin's air holds only what is
  breathed out, and a cabin's volume is a figure, not its shape's.
- Sabatier, electrolysis for air and greenhouses: T265. The scrubber and water recovery are here.
- The realism mod's other capsules carry no supplies of their own (their pod records have none).
