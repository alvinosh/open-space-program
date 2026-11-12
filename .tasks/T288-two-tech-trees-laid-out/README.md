# T288: Two tech trees laid out: Casual for the base game's parts, Realistic from RP-1's

The rules name the tree a game uses. Casual's is new, for the base game's 40 parts: a handful of tiers
from a first rocket to the biggest engines, each node a step a player can see. Realistic's is RP-1's
(`mods/realism/tech_nodes`), whose nodes today only unlock by date.

## Scope

- A `tech_trees` record per tree (its name and root) and a rules entry naming the one a game uses.
- A node unlocks the records whose `tech` names it and the parts-list entries it lists itself (the
  procedural parts have no record).
- Every part in each mode's parts list is in exactly one node of its tree, and every node is reached
  from the root, costing more than what it builds on.
- Realistic shows only the nodes that unlock something we have; the empty ones are passed through.
- A preview of both trees to look at before the screen (T290) exists.

## Done when

- A check (test) that every part is placed once, the tree is connected and costs rise along it, in both
  modes.
- Unlocking a node makes its parts available and the next nodes reachable (test).

## Done (2026-11-12)

- `tech_trees` records (`assets/core/tech_trees/stock.json5`, `mods/realism/tech_trees/rp1.json5`):
  a name, the `roots` a science game starts with, and a `layout`. Rules `tech_tree` names the one a
  game uses: the base game `casual`, the Realism mod `rp1` (`mods/realism/patches/tech.json5`).
- `tech_nodes` records can list parts-list entries in `parts` besides the records whose `tech` names
  them. `tech::ScienceTree` (crates/osp_sim/src/tech.rs): the nodes reached from the roots, the node
  of each entry (an engine family by its cheapest configuration), which nodes are shown (those that
  unlock an entry or a record), what each builds on through the nodes passed over, what it costs (at
  least what it builds on), what can be unlocked next, and whether an entry or a record is unlocked.
  `tech::science_tree` picks the rules' tree.
- Casual (`assets/core/tech_nodes/casual.json5`): 19 nodes in seven tiers placing all 40 parts, at 0,
  5, 18, 45, 90, 180 and 400 science. First Steps has the C-1 capsule, tanks, the K-0 to cluster, the
  S-1, fins, nose cones and decouplers; the branches are rocketry (to the M-2 at the end), flight
  control and probes, electrics and communication (to the deep space relay and the RTG), and
  aerodynamics and aviation.
- Realistic: RP-1's 330 nodes, of which 161 unlock something the game has and are shown, by year in
  bands by category (`layout: "years"`), 1942 to 2051, up to 1,000 science. The 93 entries no record
  placed (the procedural parts, the base game's parts by the real engine each stands for, RCS blocks,
  reaction wheels, and some 45 imported engines and 4 capsules RP-1 has no node for) are placed by
  hand in `mods/realism/patches/tech.json5`. RP-1 prices some nodes below what they build on; the
  game charges what that costs.
- `cargo run -p osp_sim --example tech_tree_preview` draws both trees to `target/tech_trees/` (SVG).
- Tests (`crates/osp_sim/tests/tech_trees.rs`): every part of each mode's parts list is in one node,
  every node is reached and costs at least what it builds on (Casual as written, tier by tier);
  unlocking a node opens its parts and what builds on it, and a node that needs two waits for both.
  785 tests across the sim, game and mods crates.

## Guesses to confirm

- Casual's nodes, what is in each and the costs; First Steps lifts a capsule only with the K-0s
  clustered (T249 rebalances the casual parts and may move them).
- In Realistic the base game's engines (K-1 Booster and the rest) stay in the parts list, each in the
  node of the engine it stands for.
- Tank and part sizes are not gated: a procedural tank of any size comes with First Steps.
- No science nodes yet: the instruments come with T272, each in a node of its own.
