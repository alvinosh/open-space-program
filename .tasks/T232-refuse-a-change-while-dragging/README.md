# T232: Refuse a change while dragging, not after

While a handle, arrow or ring is dragged the craft is rebuilt and looks fine. On release the change
is checked, and a refused one snaps back with the reason under the Launch button, across the
screen from the pointer.

## Scope

- Check while dragging (the rebuild already runs on its own thread): the handle stops at the
  limit, or turns red with the reason beside the pointer, before it is let go.

## Done when

- A drag past what the rules allow shows why before release, and release keeps the last allowed
  size or place.

## Where it comes from

From the editor review of 2026-11-07: the user asked for everything wrong with the editor from a UX point of view, then for each finding to be filed as a task. The epic, T221, lists them all.
