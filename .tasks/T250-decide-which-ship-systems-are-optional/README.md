# T250: Decide which ship systems are optional, and how a mod switches one

Power, comms, life support, heating, structural failure, aerodynamics and failures (T050) are code,
not fields on a part. "Realistic X" for those needs a rule the simulation reads, likely a `rules`
record that mods patch, stored in the save.

## Done when

- A written list of which systems are in the casual game, which are mods and what casual does
  instead (for example, no power use at all or a simple battery).
- The mechanism is chosen, and a task exists for each system.

## Proposal, 2026-11-10 (T260), to confirm

The mechanism: the `game` rules record gets one entry per system, either off or the id of a profile
record holding that system's numbers, so Casual and Realistic differ in data and not in code. The
base game's record is Casual; each Realistic mod patches its entries and adds its profiles. A save
keeps its mods (T245), and so its rules. T261 builds it.

| System | Casual (base game) | Realistic | Mod |
| --- | --- | --- | --- |
| Life support (T258) | One resource, supplies, so much a person a day. Out of it, people weaken over days and are lost one at a time. | Oxygen, water and food in; CO2, waste water and waste out. Oxygen lasts minutes, water days, food weeks. | Realistic Life Support |
| Cabins (T264) | Off | Volume, pressure, air, leaks | Realistic Life Support |
| Recyclers and greenhouses (T265) | A greenhouse makes supplies | The chemical chains | Realistic Life Support |
| Comfort and stress (T266) | Off | On | Realistic Life Support |
| Radiation (T267) and storms (T269) | Off | On | Radiation |
| Failures (T050) and repairs (T270) | Off | On, with a quality per part | Failures |
| Science (T272 to T275) | Experiments take minutes; data arrives as soon as there is a link | Real durations, drives that fill, data rates | Realistic Science |
| Resources and ISRU (T276, T277) | Ore anywhere on the ground, one converter to liquid fuel and oxidiser | Real deposits and chains | Realistic Resources (needs Realistic Fuels) |
| Automation, monitor, planner (T278 to T280) | On | On | none |
| Power, links, heat | As now | As now | none |
| Avionics | Off, as now | On, as now | Realistic Avionics |

The Realistic game mode turns all of these mods on; Custom lets the player pick.

Also to settle (from T256): a probe with no crew and no antenna can still be commanded today. Proposal:
in Casual a probe core has a small radio of its own that reaches only near its home planet; in
Realistic it needs an antenna.

## Decided 2026-11-10

The user confirmed the table above and the probe core's own small radio in Casual (T282), and made
T260 the priority. Rules entries and mods come with each system's own task; T261 built the first,
life support.
