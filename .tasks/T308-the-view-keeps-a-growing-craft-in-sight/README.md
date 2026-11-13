# T308: The view keeps a growing craft in sight

Seen in T306's capture on 2026-11-13: the view frames the first part put on (an engine) and stays
there; an 8 m tank put on top runs off the top of the screen, and the player has to zoom out and look
up by hand before putting the next part on its top.

## Scope

- When a placement makes the craft grow past the view, the view eases out (and its target moves) so
  that the craft fits, unless the player has moved the view since the last placement.
- Never moves while a part is in hand or being dragged.

## Done when

- In a capture built from an engine up, every new free node stays on the screen without moving the
  view by hand.
