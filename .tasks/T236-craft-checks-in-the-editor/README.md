# T236: Craft checks in the editor

The editor never says a craft will not work. `builder::warnings` already finds an engine nothing
feeds and an engine firing into a part that stays on, and nothing shows them.

## Scope

- A checks list in the vessel column: `builder::warnings`, TWR under 1 on the pad, nothing to
  control it, decouplers in no stage, and unstable in flight once T205 has the centres.
- A click on a check picks the part it is about. Launch looks different while any fail, and still
  launches.

## Done when

- Each check shows for a craft that has the problem, and none for the shipped craft that fly.

## Where it comes from

From the editor review of 2026-11-07: the user asked for everything wrong with the editor from a UX point of view, then for each finding to be filed as a task. The epic, T221, lists them all.

## 2026-11-21

From the stock craft rebuild: in a Realistic game a probe-core craft with no antenna cannot be
staged, one with no battery loses its link at liftoff, and one heavier than its avionics are rated
for cannot be flown. The editor said nothing about any of them (T353).

Also from the rebuild: tanks as the parts list gives them are built for 2.5 g ("Built for"), and the
rebuilt Pathfinder pulls 4 g near booster burnout, where a tank burst in the first-orbit smoke test.
The rebuilds now type 5 g into every launcher tank; the editor should say what g a craft pulls and
which tanks will not take it.
