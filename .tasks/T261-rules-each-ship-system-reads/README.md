# T261: The rules each ship system reads, set by Casual and Realistic

Each optional system reads its setting from the `game` rules record: off, or the id of a profile
record that holds its numbers (a person's needs, how long they last without, how fast data goes).
The base game's record is Casual; the Realistic mods patch it. A save keeps its mods (T245), so it
keeps its rules. What the settings are is T250's.

## Done when

- `RulesDef` has an entry for each system in T250's list, and the profiles are records a mod can add
  or patch, both described in `docs/modding.md`.
- A test loads the base game and the Realistic set and gets each system's two settings; a save made
  under one loads under it.
- A system that is off costs nothing each tick and shows nothing in the part window.

## Done (2026-11-10)

The mechanism, with life support as the first system on it. Each later system adds its own rules
entry, profile collection and Realistic mod in its own task (T250), so no entry exists for a system
before its code does.

- `RulesDef::life_support` names a record of the new `life_support` collection (`LifeSupportProfileDef`:
  `needs`, each a `resource`, `daily` kg and `lasts` s). The base game's rules say `null`; the new
  Realistic Life Support mod (`mods/realistic_life_support`, needs Realistic Fuels for `LqdOxygen`, brings
  `Water` and `Food`) sets `"realistic"` with 0.84 kg oxygen (lasts 5 min), 3 kg water and 0.6 kg food a
  day. The Realistic game mode turns it on. `PartCatalog::life_support_needs` looks it up.
- A part's `life_support: {}` has no figures of its own any more. Built under rules with none, the
  component is left off the part, so there is nothing to show and nothing in the step. The crew read
  their needs from it, any number of them; `System::take_consumption` and `consuming` give owned names.
- A save keeps its mods (T245) and each crew keeps its needs in its saved state. A save from before
  has crews with no needs, who use nothing.
- Tests: `the_rules_say_what_people_need`, `a_crew_keeps_what_it_needs_through_a_save` (crew.rs),
  `life_support_comes_from_the_rules_of_the_mods_on` (tests/base_game.rs). `cargo smoke saturn_v`:
  143 passed, 0 gaps, every sample and finding the same as before (43 kg oxygen, 112 kg water, 19 kg
  food left at splashdown).
- The game-mode test now says Realism alone is no longer the Realistic mode: the parts mod does not
  bring the ship systems' mods.
