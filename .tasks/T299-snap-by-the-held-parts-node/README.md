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

## Done (2026-11-13)

Asked again by the user on 2026-11-13: "you need to do the task where the parts clip when the nodes
overlap, and not when the cursor is over the node".

- While something is in hand, its own free nodes are found on the screen as it is drawn at the
  pointer (`OwnNode`, `own_nodes`): the in-hand group built by itself as the renderer builds it,
  placed by the middle the renderer measures (`PartInHand::middle`, set by
  `osp_render::part_pictures::draw_part_in_hand`, hidden or not). It goes on a free node when one of
  its own nodes facing the other way (cosine under -0.5) comes within 24 px of that node's ring;
  once on, it stays until that node is 36 px off, so it does not flicker at the edge. The nearest
  pair wins, and it goes on by that node.
- The pointer stays where it is through the snap; the preview shows the part on the craft.
- The pointer on the ring still puts on what goes on by its side (a booster on a radial
  decoupler's node: it has no node of its own to bring) and anything the renderer has not drawn
  yet. Surfaces are by the point under the pointer, as before.
- Hints: "Let go with one of its nodes on a ring, or on the side of a part".
- Captures: `EditorPointAtNode` now puts the scripted pointer where the in-hand part's node that
  goes on the ring is on it (`Placing::bring_to_node`, moved until it is, since a node's offset from
  the pointer changes with perspective), so every editor capture now places by the node: the
  plane's tank goes on the capsule with the pointer 140 px from the ring. `OSP_CAPTURE_EDITOR_BUILD`,
  `_CRAFT` (new shot `plane_tank_preview.png`), `_FIT`, `_LOOSE`, `_MOVE`, `_DRAG` and
  `OSP_CAPTURE_CLUSTER` pass.

## Not checked

- With a real mouse.

