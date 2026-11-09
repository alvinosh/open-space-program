# T252: A part in hand is drawn at the pointer

Said on 2026-11-09: dragging from the parts list only showed the list's tile, with no version of
the part to show where it would go.

While it has nowhere to go, what is in hand is drawn at the pointer as the hangar draws it: a part
from the list, the first part of an empty craft, or a part dragged off the craft with what it holds.
Over the hangar the tile is hidden. Near a node or a surface the part shows where it would go, as
before.

## Done when

- `OSP_CAPTURE_EDITOR_DRAG=1` shows the part at the pointer in each case, and on the craft at a node.
