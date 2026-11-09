# T244: Turn mods on and off on a Mods screen

Before this, every mod in `mods/` always loaded unless its manifest said otherwise.

Mods on the title screen lists every installed mod. Turning one on brings what it depends on and
turning one off takes what depends on it. The choice is kept in `settings.json`, and the manifest's
`enabled` is now only the starting state. Changes load when the screen is left, without a restart.
Scripted runs ignore the file.

## Done when

- A mod can be turned off and its parts and craft are gone from the editor, and the choice is still
  there after a restart. `OSP_CAPTURE_MODS=1` shows it.
