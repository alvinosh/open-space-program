# T306: A new part takes its size from what it goes on

Said by the user on 2026-11-13: the editor is "horrible", focus on making it better: "better placing,
better optimization, more intelligent building, ie when you add a tank it tries to guess the size
based on what its being attached to".

Before this, a new procedural part was only as wide as the node it went on (`palette::fit`). That
went wrong in the common cases:

- An engine's top node is as wide as its nozzle (a K-0's is 0.3 m, a realism engine's a third of its
  nozzle), so a tank put on an engine came out 0.3 m x 0.9 m. Building from the engine up was refused
  anyway: the node rule said a 2.8 m tank was "too big for" the engine's 0.9 m top, though the same
  joint built top down passed.
- A tank, tube or decoupler put on a hull's side was as wide as the whole hull.
- A tank on a radial decoupler's node was not sized at all (2 m whatever the decoupler).
- A structural tube kept its 2 m top whatever its bottom was fitted to: an adapter.

## Done (2026-11-13)

- `osp_sim::fitting` replaces `palette::fit`. `fit(part, &Against, catalog)`:
  - On a node: as wide as the node, as before. A tube is as wide at both ends.
  - On an engine's mount (`fitting::feeding`: the front of a liquid engine with no tank of its own),
    a cylinder tank holds what the engine burns in 150 s (an engine whose sea-level Isp is at least
    70% of its vacuum Isp) or 350 s (a vacuum engine), at full thrust. The amount is measured on
    the tank as built and filled with the engine's propellants: two test builds, about 1.4 ms.
    Length about 3x the width, no narrower than the nozzle, widths rounded to 5 cm (10 cm over 2 m),
    within the career's limits (narrower and longer when the width is capped). A K-0 gets
    0.85 x 2.4 m, a K-1 Booster 2.8 x 8.1 m, a K-1 Vacuum 3.7 x 10.4 m, an H-10 2.3 x 6.6 m.
  - On a hull's side: a tank, tube or decoupler is half as wide as the hull there (a booster beside
    its core). Fins and wings are still sized for the whole hull. The hull's width is now taken from
    its shape at that point, not from where the pointer's ray met its facets.
  - By its side on a node facing across (a booster on a radial decoupler): as wide as the node.
- `builder::check_nodes`: an engine's front takes a part of any width (`fitting::engine_front`), so a
  tank on an engine is as good as the engine under the tank, whichever holds the other. Other nodes
  keep the 2.5x rule.
- Tests: `fitting::tests` (widths, tube ends, beside, tanks fed for 150/350 s on a K-0, K-1 Booster
  and H-10 to within 10%, career limits, only a liquid engine's front fed),
  `builder::tests::an_engine_s_front_holds_a_tank_of_any_width` (and a tank still may not hang from
  a probe core).
- Capture `OSP_CAPTURE_EDITOR_FIT=1`: a K-1 Booster as the root, a tank on its mount (2.8 x 8.1 m,
  kerolox), a tube on the tank (2.8 m both ends), two tanks beside it with symmetry 2 (1.4 m).
  `OSP_CAPTURE_EDITOR_CRAFT` now checks its radial boosters and their nose cones are 1 m beside the
  2 m core. `OSP_CAPTURE_EDITOR_BUILD` and `OSP_CAPTURE_CLUSTER` still pass.

## Guesses to confirm

- The burn times (150 s and 350 s) and half the hull's width beside it.
- A tank put under a pod or another tank is still 3x its width long: nothing yet says what engine
  will be under it (T309).
