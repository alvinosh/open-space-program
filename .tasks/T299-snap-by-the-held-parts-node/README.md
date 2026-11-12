# T299: A part in hand snaps when its own node comes near a free node

Said by the user on 2026-11-12: "currently to attach a part to a node the mouse needs to be at the
node, however i would prefer the nodes node to be at the node for it to clip".

Today what is in hand goes onto a free node when the pointer comes within some pixels of the node's
ring. The part is drawn at the pointer by its middle, so its own node (the bottom of a tank) is far
from the ring when it snaps. In KSP it snaps when the held part's node comes near the free node on the
screen.

## Scope

- While what is in hand floats at the pointer, find its free nodes on the screen; it snaps to a free
  node of the craft when one of its own nodes that faces the other way comes within the snap
  distance of it. The nearest pair wins.
- The pointer keeps its place on the part through the snap (no jump of the part to the pointer).
- Surface attachment stays by the point under the pointer.

## Done when

- In a capture, a tank held by its middle goes onto a pod's bottom node when the tank's top node, not
  the pointer, reaches the ring.
