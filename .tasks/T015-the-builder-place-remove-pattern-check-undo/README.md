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

## Where it comes from

- docs/build-editor-plan.md (E3)
