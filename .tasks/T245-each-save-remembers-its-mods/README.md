# T245: Each save remembers its mods

A casual game and a realistic game hold different parts, so the mods belong to the save. Today they
are one global set, and a save is a snapshot that does not name them. A save whose mod is off still
loads, but the mod's models are not found.

Record the mods in the save's header. Loading a save switches to them, and says so if one is not
installed.

## Done when

- A save made with a mod on, loaded with it off, comes back with the mod and its models.
- A save needing a mod that is not installed says which one, in the Load list.

## Done so far

Built 2026-11-09. The save's header lists its mods. Loading switches to them before anything is
drawn, the first screen and a new game's editor go back to the Mods screen's choices, and a save from
before this keeps whatever is loaded. `OSP_CAPTURE_SAVE_MODS=1` shows it.
