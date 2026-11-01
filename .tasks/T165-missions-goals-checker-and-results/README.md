# T165: Missions: goals, a checker and a results screen

Today a scenario is a start state and nothing else: there is nothing to do and no way to win. A mission is a scenario plus goals, and the game says when they are met. Tutorials (T166) and every scenario after them are built on this.

## Scope

- A scenario record can list `goals`, in order or in parallel, and `fail` conditions (crew lost, vessel destroyed, out of time).
- Conditions are evaluated on the simulation state every tick, with no renderer: an orbit (body, apoapsis and periapsis ranges, inclination), a height or speed reached, distance and relative speed to a target held for a time, landed within a radius of a site, recovered (landed or splashed down with the crew alive and the peak g under a limit), propellant or battery left.
- A goals panel in flight, a short notice when one is met, and a results screen at the end (goals, time, delta-v used, peak g).
- Goals are records, so mods can add them.
- The smoke tests' checks and a mission's goals are the same thing: one condition type, two uses.

## Done when

- A scenario with three goals is passed by a headless script that flies it with player-level controls.
- Each condition type has a test that passes and one that fails.
- The goals panel and results screen show in the running game.
- `docs/modding.md` describes the goal records.

## Where it comes from

- Planning of 2026-11-01: the core game needs goals, tutorials and scenarios people want to play.
