# T246: New game asks: Casual, Realistic or Custom

A mode is a named set of mods, a record any mod can add to. New game offers Casual, Realistic and
Custom (the Mods list), and the save keeps the result (T245).

## Done when

- A new game on each mode has the right parts, and the choice is visible in the Load list.

## Done so far

Built 2026-11-09, after the user said the mods should be selected when creating a new game. New game
opens a screen (`layouts/menu/new_game.json5`) with the modes (`game_modes` records), the mods one by
one, and Start; the title menu's Mods row is gone. Casual is the default for a player who has not
chosen yet, the last choice is remembered, and the Load list says each save's mode or Custom.
`OSP_CAPTURE_MODS=1` shows it. Scripted runs and tests still have every mod its manifest leaves on.
