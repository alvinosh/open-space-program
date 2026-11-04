# T187: Menu shell: screen stack, Back and Esc, Title and Pause

One way of moving between screens, and the two roots in the new look.

## Scope

- `Menu` holds a stack of screen ids in place of `page` and `play`; the `Play` enum goes. Actions `menu.open:<id>` and `menu.back`; data `menu.screen`, `menu.can_back`. Showing the top screen goes through `LayoutVisibility`, by group `menu`.
- Esc is `menu.back`, following the order in T185. The stack is empty each time the menu opens.
- `layouts/menu/title.json5` and `pause.json5`, with `#title_items` and `#pause_items` for mods. Tutorials on Title only. Pause keeps the vessel entries above the game entries.
- The other screens were ported in this step too (Load, Vessels, the list of craft, Tutorials, Settings): moving the old markup into the new frame and rewriting it later was more work than porting it, and `main_menu.json5` is gone. What stays for T189 is listed there. The list of craft stays a menu screen until T190.
- Scripts (`PLAY_SCRIPT`, `MENU_SCRIPT`, `SETTINGS_SCRIPT`, `GRAPHICS_SCRIPT`, `MENU_TUTORIAL_SCRIPT`) move to the new actions, with an `ExpectScreen` check. `docs/modding.md` lines on `menu.page` and `menu.play` are replaced.
- `OSP_CAPTURE_MENUS=1` captures every screen, for review, and walks them with Esc and Back in turn so a difference stops the run.
- Esc closes an open popup before it opens the menu (`LayoutVisibility::dismiss_popups`).
- `game.exit` is here without its question, so Tutorials can be reached from a game. T188 adds the question.

## Done when

- Esc and Back agree on every screen, in a test.
- `every_menu_screen_in_the_game_follows_the_rules` (T186) checks real screens: it passes with none today, so it should also insist Title and Pause exist.
- The user has seen captures of the real screens and accepts them.
