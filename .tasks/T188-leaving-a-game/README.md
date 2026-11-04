# T188: Leaving a game: one dialog, exit to main menu, Continue

Nothing is thrown away without asking.

## Scope

- `Dialog` resource and `dialog` layout: title, text, buttons with actions. Esc is Cancel.
- Leaving a game asks to save first (Save and continue, Don't save, Cancel): New game, clicking a save to load, Exit to main menu, Quit. Skipped when the game's time equals that of its last save.
- `game.exit`: back to the default scenario standing still with no game in progress, as at startup, forgetting the current save.
- Continue on Title loads the newest save.
- Delete, overwrite, destroying a vessel (`vessel.destroy`) and clearing a pad or runway (`editor.clear`) ask through the same dialog; the "Sure?" and "Destroy X?" buttons go.

## Done when

- A scripted run covers each way out: Save and continue writes the save, Don't save leaves it, Cancel leaves the game running.
- Exit to main menu then New game starts clean, and Tutorials is reachable again.

## Done

- `dialog.rs` and `layouts/dialog.json5`: one question at a time, over the flight, the menu and the editor, with a shade that takes the pointer. Esc and Back close it and nothing else. A menu test holds it to the type scale and controls.
- `ask.leave:<action>` is on New game, loading a save, Exit to main menu and Quit: Save and continue, Don't save, Cancel. If the save fails the question stays and says why, so the game is not lost.
- Skipped when the save holds the game as it is now. This compares a hash of the snapshot (`save::fingerprint`), not the clock as first planned, because a vessel can be destroyed with the game paused and the clock would not show it. A running tutorial never asks, as it has no Save.
- Continue on Title (`game.continue`, `menu.can_continue`, `menu.continue_line`) loads the newest save this game can read.
- Deleting a save, destroying a vessel and clearing a pad or runway ask through the same dialog (`ask.delete_save`, `ask.destroy`, `ask.clear`); the "Sure?" buttons, `item.deleting`, `item.destroying` and `editor.clearing` are gone.
- `OSP_CAPTURE_LEAVING=1` with `OSP_USER_DIR=<empty folder>` runs each way out and checks the saves after each. `OSP_CAPTURE_CLEAR`, `OSP_CAPTURE_PLAY` and `OSP_CAPTURE_MENUS` pass with the new actions.

## Left out

- Overwriting a save asks when the Save screen exists to overwrite one (T189).
- Closing the window from the desktop does not ask.
