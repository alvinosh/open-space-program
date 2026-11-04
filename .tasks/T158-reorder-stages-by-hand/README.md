# T158: Reordering stages by hand works and shows where it lands

You cannot swap stages around in practice. The sim can move activations and reorder sequences (`staging.rs`), and the layout wires it to dragging a number or an icon, but it has never been tried by hand (T060) and the drop target is not shown; dropping on a row is the only affordance.

## Scope

- Try the drag and drop with the pointer and fix what fails.
- Show the drop: a line between rows where a sequence will land, a highlight on the row an icon joins.
- Up and down buttons on a row as an alternative to dragging.
- Right-click on a row: add a sequence above or below, move all, split one off (from T060).
- An undo for the last move.

## Done when

- A scripted-pointer capture test swaps two sequences and splits an engine out of a group; the same on the live window (not only the model) is checked by hand once and noted here.

## Done so far (with the T160 redesign)

- The whole row is the drag handle, a tile can be dragged onto a row or onto the strip between two rows (`staging.drop_at`, it lights up under the pointer) to put it at exactly that place, and the + takes a dropped tile as a new last sequence. Tested at the action level (`controls::tests`), not with the pointer.
- `OSP_SCENARIO=apollo11 OSP_CAPTURE_STAGING_POINTER=1` drives the staging window through the same picking the mouse goes through, on the live window, with its engines burning so the data under it changes every frame. It swaps two rows, undoes that, splits one J-2 out of a group onto a strip, moves a row with its arrow and adds a sequence from a row's menu, checking the sequences after each and failing if they differ.
- That found two bugs: the strips between rows had no width, so nothing could ever be dropped on them, and the arrow glyphs were not in the font. Both fixed; tooltips also stay away during a drag.
- Up and down arrows on a row, a right-click menu on a row (`staging_menu.json5`: add above or below, move, merge with the one below, split one off), an undo beside the + for the last change, gone once anything has fired or fallen away.
- The script stands in for the pointer, so the feel of a drag and the row menu opening beside a real cursor were left to a check by hand (a capture has no cursor, so the menu opens at the top left). The user closed this task on 2026-11-04.
