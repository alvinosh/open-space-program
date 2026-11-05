# T020: Launch from the editor and from files

New, open, save, save as; the craft list with shipped and user craft; launch the draft or a file; revert to the editor from flight.

## Scope

- Launch picks a site (pad, runway, orbit), as the main menu's 'place then spacecraft' steps already do, and starts the scenario with the draft or a file.
- The menu lists user craft with the shipped ones.
- Revert to the editor from flight (Esc menu), keeping the draft.

## Done when

- You can build, launch, fly, revert, change a tank, and launch again.

## Notes

- Also: vehicle files saved from the editor, and launch from a file. Choosing system, location and game type is the start menu task.

## Done so far

- Launch picks pad, runway or orbit and takes the sequences left in the editor; the Play page lists every craft.
- Revert from flight (pause menu, asks first): `game.revert:launch` to the moment it left, `game.revert:hangar` to the editor with the same craft, staging and place. Everything since goes, saves are not touched. A game loaded from a save has nothing to revert to until its next launch. `OSP_CAPTURE_REVERT=1` with `OSP_USER_DIR=<empty folder>` runs it.

## Still open

- New, open, save and save as; user craft files.

## Where it comes from

- docs/build-editor-plan.md (E8)
