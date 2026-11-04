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
