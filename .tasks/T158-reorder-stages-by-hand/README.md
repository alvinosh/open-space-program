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
- Still open: trying it with the pointer by hand, up/down buttons, the right-click menu (add above or below, split one off), undo.
