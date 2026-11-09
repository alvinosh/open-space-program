# T250: Decide which ship systems are optional, and how a mod switches one

Power, comms, life support, heating, structural failure, aerodynamics and failures (T050) are code,
not fields on a part. "Realistic X" for those needs a rule the simulation reads, likely a `rules`
record that mods patch, stored in the save.

## Done when

- A written list of which systems are in the casual game, which are mods and what casual does
  instead (for example, no power use at all or a simple battery).
- The mechanism is chosen, and a task exists for each system.
