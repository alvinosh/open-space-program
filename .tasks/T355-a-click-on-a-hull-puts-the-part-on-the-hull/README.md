# T355: A click on a hull puts the part on the hull, not on a free node near it

Found on 2026-11-21 rebuilding the stock satellite in the builder: a solar panel clicked onto the
bus snapped to the battery's free node a few pixels away (T299 snaps a part in hand by its own node
from 24 px) and was refused, "node `top` is 1.02 m across, too big for `battery`'s `bottom` of
0.16 m". The player pointed at the hull and got a refusal about a part they were not aiming at. The
rebuild puts the battery on after the arrays.

## Done when

- A part in hand snaps to a free node only where it fits there; otherwise it goes on the surface
  under the pointer.
