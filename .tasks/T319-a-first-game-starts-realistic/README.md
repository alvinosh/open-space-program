# T319: A first game starts in Realistic mode

Found by the playtests (2026-11-16): with an empty user folder the New game screen has Realistic lit
and every Realism mod on, so a new player who clicks Start sandbox gets the RP-1 parts list
(15D13, 457M Hall Effect, A-1, Aerobee...) and no K-1 Booster on screen. The playtests pick Casual
by hand.

The plan since 2026-11-09 is that the base game is Casual and realism comes from mods. To decide:
should a first game default to Casual?

## Answer and outcome (2026-11-16)

The user: yes, a first game should be Casual. It already was (`default: true` on the Casual mode in
`assets/core/game_modes/stock.json5`). The playtests saw Realistic because the settings file ignored
`OSP_USER_DIR`: they read the player's own settings, where Realistic was chosen, and their Start
wrote Casual back into them (put back the same day). `settings_file()` now keeps the settings in
`OSP_USER_DIR` as saves and windows already were, and the playtests check that a first game shows
Casual lit (`mods.modes[0].on`) instead of picking it.
