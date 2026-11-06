# T202: Quicksave and quickload

Asked for on 2026-11-06: quick save and quick load as in Kerbal Space Program, kept as saves within saves.

## Done when

- F5 in flight makes a quicksave that belongs to the game's save, without changing the save itself.
- Holding F9 carries on from the game's newest quicksave; a tap does nothing.
- The Load screen shows each save's quicksaves under it, to load or delete; deleting a save takes its quicksaves.

## Done

- A save's quicksaves live in a folder named after it: `saves/game_1.save` and `saves/game_1/quicksave_<n>.save`, called `game_1/quicksave_<n>` in the actions. Each F5 adds a new one, `Quicksave <n>`, so earlier ones are kept. In a game with no save yet, F5 starts the save first.
- `game.quicksave` (F5) and `game.quickload` (F9, `hold: true`): the held key loads after 1 s and once per press; the same action from a button loads at once. Flight only, not in the menu, the editor or a tutorial. What happened shows under the altimeter for 3 s (`saves.flash`).
- Loading a quicksave keeps the game going to its save, which is not up to date, so leaving asks to save as before. Like any load it forgets the revert point.
- Load screen: the quicksaves sit under their save, stepped in (`menu_nested`), newest first, each with Load and Delete. The delete question for a save says how many quicksaves go with it.
- Action names are checked to be a save or a quicksave in one, so `game.delete_save` cannot reach outside the saves folder.
- Keys moved to make room: mod reload F5 to F7, collider view F9 to O.
- Unit tests in `saves.rs` and `dialog.rs`. Scripted capture: `OSP_CAPTURE_QUICKSAVE=1` with `OSP_USER_DIR=<empty folder>` presses the real keys.

## Left out

- Quicksaves have no names of their own (no text box in the menu yet) and there is no cap on how many a save keeps.
- Saving over another game's save, or Save as, leaves quicksaves where they were: those of the save written over stay with it.
- No quickload screen of its own (KSP has one on Alt+F9): the Load screen lists them.
