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

## Done

- `osp_sim::mission`: scenario records take `goals`, `fail` and `ordered`. A condition is one of `Altitude`, `Speed`, `Orbit`, `Near`, `Landed`, `Recovered`, `Propellant`, `Destroyed`, `Elapsed`; a goal may be held for a time (`hold`) and stays met once it is. `Universe` owns the `Mission` and looks at it after every physics tick and every rails jump, so it is judged with no renderer and under time warp. Pass when every goal is met, fail the moment a `fail` entry holds; either stops the clock and the stats (time, delta-v used, peak g, highest altitude). A goal naming a site, body or vessel that is not there stops the scenario starting, with the reason.
- `osp_game::mission`: publishes `mission.*`, drains the mission's events into a notice (teal for a goal or a pass, amber for a failure) and opens the `mission_results` window when it ends. `mission_goals` (left dock) opens when a scenario with goals starts.
- The `pad` scenario has three goals (leave the pad, pass the Karman line, reach a stable orbit) and a `fail` on `Destroyed`.
- Tests: one per condition that passes and one that fails, hold, order, fail and freeze, rails, stats, bad references, the record syntax (`osp_sim::mission`); the notice wording (`osp_game::mission`). Smoke mission `first_orbit` flies the Pathfinder from the pad with a pitch program, the stage key, RCS ullage and the throttle, and all three goals are met (451 s, 6.2 g).
- Seen in the running game with `OSP_CAPTURE_MISSION=1 OSP_SCENARIO=pad`: the goals window, the tick and "Goal complete" notice, the results window. A temporary `Elapsed` fail on the pad was also run to see the real end: "Mission failed" with its reason, the window opening by itself, the stats stopped (reverted).
- `docs/modding.md` has a Missions section and the `mission.*` keys.

## Open

- The smoke tests' `within` checks are still their own code; they are not goals yet. A condition type is shared only in the sense that the same questions can now be asked either way.
- Not modelled, so not conditions: battery charge (T035), splashing down (T048), crew lost or alive (T049). `Recovered` means landed in one piece with the peak g under the limit.
- Delta-v used is thrust over mass summed over physics ticks, so it includes what gravity and drag took; it does not count RCS or anything on rails.
- A `hold` under heavy time warp is counted in the jump sizes, which are coarse.
- The goals window uses the left dock and shows the same list the results do; no progress numbers yet (how high you are against the goal). T166 will want the current goal highlighted (`current`, ordered missions only) and a hint.
- Not tried by hand (the capture scripted the launch).

## Where it comes from

- Planning of 2026-11-01: the core game needs goals, tutorials and scenarios people want to play.
