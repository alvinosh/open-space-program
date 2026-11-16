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

Confirmed by the `two_stages` playtest (2026-11-16): after capsule, decoupler, tank and engine the
engine is off the bottom of the screen and its ring is not drawn, so the player must zoom out with the
wheel before going on (`Play::Wheel` in `crates/osp_game/src/playtest/plays.rs`; take it out once
this is done).

## Done (2026-11-16)

`Showing::follow`, from `Editor::settle` (which every change to the craft goes through): when the
craft has grown and some corner of the box round it is out of sight (more than 25° off the middle of
the view, the camera being 60° high), the view eases out (`Showing::ease`, most of the way in half
a second) to look at the middle of the craft from far enough to see it all, still looking the same
way. Not if the player has moved the view since the craft last changed; any move of theirs also
stops an easing. The editor counts as busy while the view eases, so playtests wait for it.

`two_stages` no longer turns the wheel to see the engine's ring: the whole 28 m two-stager stays in
sight while it is built.
