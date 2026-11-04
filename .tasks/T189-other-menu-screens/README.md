# T189: Vessels, Save, Load, Tutorials and Settings screens

What is left after T187, which ported the lists and settings and deleted `main_menu.json5`.

## Scope

- Save: "New save", then the saves. Replaces the two rows Pause has for now, Save game and Save as a new game.
- `slider` widget in `osp_ui`, value, min, max, step and an action. T019 needs it too. Used for the four volumes, which are `setting_choice` lines for now.
- Vessels: the row of the vessel being flown stops being clickable. It is marked already.

## Done when

- Saving to a new or an existing save works by the scripts, and the volumes work with a drag.
- The capture of every screen shows one look, by eye.
