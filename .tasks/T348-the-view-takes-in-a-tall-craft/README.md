# T348: The view takes in a tall craft

Found on 2026-11-21 rebuilding the stock craft in the builder, as a player would (the playtests `rebuild_*`, crates/osp_game/src/playtest/rebuilds.rs), after the user said "I still hate the builder, i cant even mirror fairings into a payload".

Building the 68 m Pathfinder, the view kept the bottom of the booster in sight and left the
payload and upper stage off the top of the screen. T308 keeps a growing craft in sight; it does not
fit a craft taller than the view.

## Done when

- After a part goes on, the whole craft fits the view unless the player has zoomed in.
