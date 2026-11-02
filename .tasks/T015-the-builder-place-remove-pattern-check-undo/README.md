# T015: The builder: place, remove, pattern, check, undo

`osp_sim::builder`: operations on a draft craft that write plain parts. The one API the editor, the craft generators and the tests all use.

## Scope

- Place a part at a node, on a surface at (u, v) with a standoff, inside a container, or radially, at any orientation.
- Remove, move, rotate, reroot; patterns (n round the axis) and mirror, written as plain parts with nothing remembering the pattern.
- Attach rules and fit checks: overlap, node size and type, inside the fairing or container, a view or path kept clear, a joint able to carry what the part passes and the load (`structure::joint_loads`).
- Each operation returns what changed, or refuses with the rule that stops it.
- An undo log.

## Done when

- The Saturn V is rebuilt by calls to the builder and weighs within 1% of the hand-built craft.
- A place then a remove restores the craft exactly.
- A pattern of n equals n placements by hand; mirroring twice is the identity.
- A seeded random walk of valid operations never makes an invalid craft.
- The matrix cell Pod × Inside a container (a 1.9 m capsule put in a 1 m container, which today sticks out through the wall and is not refused) is grey: refused with the fit rule.

## Done so far

- `crates/osp_sim/src/builder.rs` is written and compiles: place, remove, move, rotate, orient, reroot, pattern, mirror, undo and redo, the rules (join, node size, overlap, fit inside a container, fit in a fairing) and `warnings` (engines nothing feeds, exhaust into something that stays on). `Geometry::solids` gives it what fills space.
- None of its operations has a test yet, and nothing calls it but the one test below.

## Still open

- `tests/builder.rs` runs the rules over every shipped craft, and is `#[ignore]`d because it fails: 283 refusals, nearly all from the overlap rule. A stage's hull against the tanks it holds (`upper_stage` and `upper_stage_tank_1`) is the most common, a pod against its capsule next, then small reaction wheels against tanks. The next step is to work out why a hull has anything that fills space, skip pairs where one holds the other, and decide by hand whether the wheels are real overlaps to fix in the craft.
- Then everything under Done when: none of it is met yet, and the matrix cell Pod x Inside a container is still red.

## Where it comes from

- docs/build-editor-plan.md (E3)
