# T090: Game save and load

Save and load vessels, time, plans, crew and terrain changes.

## Done when

- Everything in the scope list works, and what can run headless has a test.

## Done

- `osp_sim::save`: a save is a line of JSON saying what it is (for the list), then the universe's `Snapshot` in MessagePack: vessels whole with their parts, what they burned and what fell away, the clock, plans and their executor, crew and every other system (each saves under its name in `systems::KINDS`), targets, the mission, the tutorial, soft captures.
- `tests/save.rs` flies a game, saves it, loads it into another universe and flies both on: every bit is the same, on the pad, burning, with debris, in orbit under the executor, at a rendezvous, and for every craft in the game. What cannot be read (another version, cut short, other bodies in the solar system) is refused and changes nothing.
- `osp_game::saves`: the files, in `saves/` in `OSP_USER_DIR` or beside the settings file, written in one step. A launch saves the game; Save and Save as new are on the menu.

## Notes

- There are no terrain changes in the game to save yet: nothing changes the ground. Whatever does will belong in `Snapshot`.
- A save holds bodies by number, so one made among other bodies is refused rather than remapped. A mod that adds a body makes older saves unreadable until that is done.
- A new field in what is saved needs `#[serde(default)]`, or `FORMAT` bumped, or older saves stop reading: there are no migrations yet.
- Saves are named `Game <n>`: the UI has no text input to name one yet.

## Where it comes from

- Early feature checklist (since removed), Phase 3
