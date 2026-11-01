# T166: Tutorial layer: guided steps over a mission

A tutorial is a mission whose goals are steps: each shows a hint, points at the control that does it, and waits for the player to do it.

## Scope

- A tutorial record: ordered steps, each with text, an optional UI element to highlight, and a done condition from the mission goals (T165).
- Key names in the text are written as action names and shown as the player's current binding, so rebinding does not break a tutorial.
- A hint after the player has been idle on a step; skip a step, skip the tutorial, replay it from the menu.
- Text lives in data, so it can be localised (T130) and mods can add tutorials.
- Nothing pauses the simulation unless a step asks for it.

## Done when

- A test tutorial of three steps advances headless, driven by scripted inputs.
- The highlight and the hint show in the running game, and a rebound key changes the text.
- `docs/modding.md` describes the tutorial records.

## Done

- `osp_sim::tutorial`: a `tutorials` record (`scenario`, `name`, `description`, `steps`); a step has `text`, `hint` (after `hint_after` s, 20 by default), `highlight` (a node or layout id) and what it waits for: `when` (a mission condition), `ui` (a UI data condition, judged by the game) or neither (the player presses Continue). `Universe` owns the `Tutorial` and looks at it after every tick, so it runs headless; a restart ends it. A step naming a body, site, vessel, goal or SAS mode that is not there stops it starting.
- Mission conditions grew the controls: `Throttle`, `Sas`, `Engines`, `Pitch`, `Plan`, and `Goal` (a goal of the scenario is met; tutorial steps only).
- `osp_ui`: `UiHighlight` names a node or layout id and every node with it gets a pulsing teal outline.
- `osp_game::tutorial`: the `tutorial` window (left dock) with the step, hint and buttons (Continue, Skip step, End tutorial), `tutorial.*` data, `tutorial.next` and `tutorial.skip`. `[action]` in the text becomes the keys bound to the action now. The main menu has a Tutorials tab (`game.start_tutorial:<id>`, which also replays one), and `OSP_TUTORIAL=<id>` starts one at launch.
- The base game ships `controls` (6 steps from the pad: throttle, stage, navball, SAS, climb to 1 km, the map). `map_button` is an id on the toolbar's map button.
- Tests: three steps advancing with scripted inputs, the hint timing, a step done by the game and a manual one, giving up, a step waiting for a mission goal, bad references, record defaults (`osp_sim::tutorial`); each new condition passing and failing (`osp_sim::mission`); the outline coming and going (`osp_ui`); keys shown as bound and rebound, and that every shipped tutorial starts, names bound keys and points at ids that exist (`osp_game::tutorial`); the real `controls` tutorial played headless (`the_controls_tutorial_plays_through`, smoke).
- Seen in the running game (`OSP_TUTORIAL=controls OSP_CAPTURE_TUTORIAL=1`, `OSP_CAPTURE_MENU_TUTORIAL=1`): the step with the navball outlined, the hint after 20 s in amber with the player's own keys, Continue, the map button outlined, the finish; the Tutorials tab and a tutorial started from it. With the throttle key temporarily rebound from Z to U the step said "press U" (reverted).
- `docs/modding.md` has a Tutorials section, the new conditions, the `tutorial.*` and `menu.tutorials` keys and the actions.

## Open

- `controls` is only the first minute of T167, not the first-launch tutorial: T167 extends or replaces it with pitch-over, the burn executor and warp.
- The hint comes after time on the step, not time without input.
- A highlight outlines the whole node, so a layout (the navball) is outlined with its empty margins. Give a finer node an `id` to point at it.
- Nothing says a step was done (no notice, no sound; sound is T125), and progress is not saved, so the menu does not mark a tutorial as played.
- Text is in the records, so it can be localised (T130), but nothing does it yet.
- The buttons were not clicked by hand (the capture fires `tutorial.next`), and a tutorial was not played through with the keyboard.

## Where it comes from

- Planning of 2026-11-01.
