# T028: Open components with state

A component is a def, a built form, a runtime state and a step. A `Systems` store sits next to `Propulsion`, follows a part through a split, and holds what is not propulsion. A registry replaces the closed list of kinds; a component the physics does not know is a load error that names it.

## Scope

- The four-part component contract and the registry.
- The `Systems` store with state per part, following `release` and (later) `join`.
- Rails events: a system declares when it next needs attention (eclipse in or out, battery empty), as `next_trajectory_event` does.
- A hook so the smoke report can chart a system's state.

## Done when

- A battery's charge stays with its part through a stage separation.
- The editor's property panel shows its parameters with no editor code for it.

## Done (2026-11-02)

- `crates/osp_sim/src/systems.rs`: the `System` trait (parameters, state, step, next event), `KINDS` (the list a new kind joins with one line) and `Systems`, the state part by part. `Propulsion.systems` sits next to the tanks, goes with a part through `release` and is dropped with a part that tears off.
- Battery and crew are the first two, and their rows of the placement matrix are green (25 red cells down to 1, the pod that T015's fit check will refuse).
- Rails: `Universe::advance_rails` stops at `Systems::next_event` and steps every system by the time it jumped.
- The smoke report charts whatever `Systems::totals` holds (`system_<kind>_<reading>` columns), with no code for any of them; the Saturn V's page has battery and crew charts.
- The part window lists `self.systems[]` with no section for any kind. The editor's property panel (T019) reads the same `parameters()`: a test does what it will do, knowing no battery.
- A component nobody knows is an error at load that names it (the craft format is a closed enum in serde; a test holds it).

## Still open

- The kinds are Rust types: a mod cannot add one until code mods exist (T136). `Join` (T039) has to merge two stores.
- Nothing draws on a battery yet (T034), so no event fires in play yet; the warp stop is tested only through the code path it shares with a trajectory event.

## Where it comes from

- docs/systems-plan.md (step 3, The build core)
