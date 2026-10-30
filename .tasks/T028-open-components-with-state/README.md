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

## Where it comes from

- docs/systems-plan.md (step 3, The build core)
