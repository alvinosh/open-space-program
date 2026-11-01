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

## Where it comes from

- Planning of 2026-11-01.
