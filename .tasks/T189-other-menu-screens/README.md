# T189: Vessels, Save, Load, Tutorials and Settings screens

What is left after T187, which ported the lists and settings and deleted `main_menu.json5`.

## Scope

- Save: "New save", then the saves. Replaces the two rows Pause has for now, Save game and Save as a new game.
- `slider` widget in `osp_ui`, value, min, max, step and an action. T019 needs it too. Used for the four volumes, which are `setting_choice` lines for now.
- Vessels: the row of the vessel being flown stops being clickable. It is marked already.

## Done when

- Saving to a new or an existing save works by the scripts, and the volumes work with a drag.
- The capture of every screen shows one look, by eye.

## Done

- `layouts/menu/save.json5`: New save (with the name it will get, `menu.next_save`), then the saves, the one this game goes to marked (`item.current`). That one is saved over at once (`ask.overwrite` fires `game.save_to`), any other after "Save over <name>?". A save from this screen goes back to Pause, which says what was saved. Pause has one Save game row now, with the save's name beside it.
- If a save cannot be written, the game goes on going to the save it went to (before, Save as a new game forgot it first).
- `slider` widget in `osp_ui` (`value`, `min`, `max`, `step`, `action`), firing `<action>:<value>` once for each step it is pressed or dragged to; `setting_slider` builds a settings line from it with the value beside it. The four volumes use it, in steps of 5%. The menu rules test counts it as a control.
- Vessels: the row of the vessel flown is `menu_row_current`, no action, looks selected under the pointer too.
- `OSP_CAPTURE_SAVE=1` with `OSP_USER_DIR=<empty folder>`: new saves, its own save at once, another's with Cancel and with Save over it, each checked on disk and on `menu.save_name`; then a volume dragged to 50, 25, past the end to 0, and 75. `UiPointAlong` puts the scripted pointer part way along a node. `PLAY`, `LEAVING`, `REVERT`, `MENUS`, `CLEAR` and `MENU_TUTORIAL` still pass.

## Left out

- The slider takes no keys or wheel. The editor's sizers (T019) can use it as it is.
