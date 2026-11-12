# T292: Tank sizes and tank types unlocked by the tree

Said by the user on 2026-11-12: "Tank sizes should be locked, in realism mode the tank types should be
locked, in casual modes there will only be one tank type."

A node can raise the widest tank (and procedural part) a career can build; the editor offers no wider,
and a craft with a wider one says so. In Realistic, tank types are also locked by the nodes their
records name (they already have `tech`). In Casual there is one tank type (T249), so only the size is
locked.

## Done when

- A new career builds tanks only up to the roots' width, and a node that raises it lets wider ones be
  built (test, both modes).
- In Realistic a locked tank type is not offered (test).

## Done (2026-11-12)

- A `tech_nodes` record may give `sizes: { diameter, length }`, m, as Procedural Parts' tech limits:
  a career builds procedural parts (tanks, tubes, nose cones: any width or length the shape has) up
  to the biggest of its unlocked nodes. `ScienceTree::allowed` gives the sizes and the tank types a
  set of unlocked nodes allows (`builder::Allowed`); a tree whose nodes limit a size and whose roots
  do not is a content problem.
- The builder holds one `Allowed` (`Builder::allow`): a resize or a field that makes a part bigger
  than allowed, or picks a locked tank type, is refused with the reason ("the tech tree allows a
  diameter of 2 m so far"); a part past a limit already may stay so or shrink. A new part is brought
  within it (`Allowed::bring_within`): sizes cut back, a locked tank type swapped for the most
  advanced allowed one that holds the same and is as highly pressurised.
- The editor keeps the builder's `Allowed` from the career (`follow_the_career`), the size handles
  stop at the limit, the part in hand is drawn as it will go on, and the tank type choice offers only
  unlocked types. A launch is refused for a part too big or of a locked type, saying which
  (`Career::locked_parts`, "Tank (diameter 3 m, 2 m so far)").
- Casual: 2 m from the start, then 3 m (General Rocketry), 4 m (Advanced), 6 m (Heavy), 10 m (Very
  Heavy). Tank types are not locked (Casual is to have one, T249).
- Realistic: RP-1 has no size limits (it limits rockets by launch complex), so the materials nodes
  carry the widest stage of their years: 1.7 m from the start (the V-2, whose engine is a root),
  1.8 (1950), 2.7 (1952), 3.1 (1956), 5.6 (1961), 6.7 (1964), 13 m (1967, the Saturn V and N1), 20 m
  (2029). The base game's seven tank types get RP-1 nodes for the tanks they stand for
  (`mods/realism/patches/tech.json5`); Solid Grain is a motor's and not locked.
- The refusal label of the part settings moved under the part's name: it was below the settings and
  off screen for a tank.
- Tests: `shaping` (limits), `builder` (refusals and bringing within), `career` (launch check),
  `tests/tech_trees.rs` (both trees: start widths, what raises them, every Realistic tank type in a
  node, a new tank brought within). Captures: `OSP_CAPTURE_CAREER` ends with a 3 m tank refused in a
  Casual career; `OSP_CAPTURE_CAREER_REALISTIC` shows a new tank at 1.7 m of Al Gridded Tank with
  only the five early types offered, and Stiffened Al 2219 refused.

## Left out

- Lengths are not limited by either tree (the builder facility's height will be, T294).
- Procedural decouplers are sized to what they join and not checked themselves.
- The tree screen (T290) is to show what each node widens.
