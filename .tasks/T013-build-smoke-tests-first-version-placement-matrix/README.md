# T013: Build smoke tests, first version (placement matrix)

A second smoke suite next to the missions, written first while most of it fails: a placement matrix of every component the sim has against every way to place a part. The red cells are the measured to-do list for the editor's core.

## Scope

- New target `crates/osp_sim/tests/build/`, run by `cargo smoke build`, page at `target/smoke/build/index.html`.
- Rows: storage, engine, RCS, reaction wheel, battery, crew, decoupler, lifting surface, container, gear, and a pod and a part with two components (which T014 names).
- Columns: on a top or bottom node, radially on a surface at (u, v) and standoff, in a ring of 2, 3, 4 or 6, inside a container, inside a fairing, canted 15 and 90 degrees, mirrored.
- Each cell builds a small craft (a 1 m host with the part on it) and checks: it loads or refuses with a reason that names the rule (never silently dropped); mass is the sum of the parts; centre of mass and inertia match an independent sum; outside the skin it is in the aero outline and inside a fairing it is shielded; save and load give the same craft; five seconds on the pad give no NaN and the weight is in the pad load.
- The page draws the matrix as a grid (green, red, grey with the rule named). A red cell carries the task that is meant to fix it; when that task lands the expectation flips and a red cell fails the test.
- Later tasks add sizer checks, reference builds and property tests to the same suite.
- `docs/smoke-tests.md` (The build suite) says how to read the page and what each check means.

## Done when

- The page exists, and every cell that fails names the task that fixes it.

## Where it comes from

- docs/build-editor-plan.md (E1)
- docs/systems-plan.md (Smoke tests for the build core)
