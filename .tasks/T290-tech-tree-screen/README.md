# T290: The tech tree screen

A screen with the game's tree laid out left to right by tier (Casual) or year (Realistic), lines from
each node to what it builds on, each node locked, ready or unlocked, its cost and its parts. Unlock a
ready node with science. Reached from the hangar and the editor.

## Done when

- A science game can unlock a node from the screen, and the parts list then shows its parts.
- Both trees read at a glance: a scripted-pointer capture of each.

## Done (2026-11-12)

- `layouts/tech_tree.json5` over everything (z 80): a panel with the picked node, and the tree, the
  `tech_tree` widget (`osp_game::tree_screen`). Cards and lines are layout nodes made on the fly
  (`spawn_node`), so they take the theme and their clicks are actions (`tree.pick:<node>`). Cards
  say unlocked (lit), ready (accent edge) or locked (sunk, dim), with cost (and year in a tree by
  years) and what they unlock; lines are elbows from each node to what builds on it, lit from what
  is unlocked; a tree by years has its years over the columns (`ScienceTree::column_labels`).
- Drag looks round, the wheel zooms about the pointer; −, Fit and + in the corner. It opens fitted
  (all of it if it reads at 70% or more, else its start at the left).
- The panel: title, tier or year and cost, description, where it stands, Unlocks (parts by name,
  then engine configurations, tank types, batteries, avionics: `ScienceTree::records`), Bigger
  parts, Builds on and Leads to (click to go there), and Unlock in the footer, or why not.
- Opened from the editor bar (a new tree icon) and a Tech tree row in the pause menu, in a career
  only (`career.active`). Esc and Back close it first: the menu and the editor leave them to it.
- A new career is now kept from New game (`CareerTree::pending`) and carried into the game at its
  first launch, so the tree works before the first launch too. `debug.science:<n>` gives science.
- Fixed on the way: scroll bars were drawn over everything (`GlobalZIndex(999)`), through the tree
  and through questions (900); now only over what they scroll (`ZIndex(1)`).
- Tests: `tests/tech_trees.rs` (what a node leads to, year columns), `tree_screen` (every node has
  a place of its own, the fit). Captures: `OSP_CAPTURE_CAREER` opens the tree from the editor bar,
  picks Basic Rocketry, gets 12 science, unlocks it (7 left), Esc, and the parts list then has the
  A-10 with 31 parts locked (33 before); `OSP_CAPTURE_CAREER_REALISTIC` shows RP-1's tree at its
  start and zoomed out.

## Left out

- Science is only given by `debug.science` until it is earned (T284).
- No search or filter in the tree, and no line routing round cards in RP-1's busiest years.
