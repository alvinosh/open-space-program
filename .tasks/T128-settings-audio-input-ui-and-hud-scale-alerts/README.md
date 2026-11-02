# T128: Settings: audio, input, UI and HUD scale; alerts

What is left of the Esc menu and settings (graphics exist): audio, input, UI and HUD scale, and alerts.

## Done when

- Everything in the scope list works, and what can run headless has a test.

## Done

- Sound: the menu's Sound page sets how loud everything is, and the engines, the wind and the effects on their own.
- Input: the Controls page lists every key with a `label` in the `keybindings` records. Click one, press the new key, Esc to leave it, Reset all to go back; a key on two actions says so. What the player chooses is kept in the settings file and put over the records whenever they load, so a mod can still change the rest.
- Scale: the Interface page scales the menu and the HUD together (`UiScale`, 75 to 150%). Popups, map labels and body markers place themselves in UI units, so they stay on what they name.
- Alerts (`alerts.rs`): chips over the flight view for ground coming up fast, a structure near its limit, high g on a crew, no power, no control, low propellant and no signal, for as long as each lasts, with a tone as one appears. Each can be turned off on the Interface page, which also has warp stopping at signal changes (T037's `warp_stops_at_links`, which nothing could turn on before).
- The settings file keeps its old flat graphics, so a file from before this still loads.
- Tests: `settings_file_tests`, `alerts::tests`. `OSP_CAPTURE_SETTINGS=1` captures the pages and the flight view and map at 125%.

## Dropped

- A HUD scale apart from the UI scale. `UiScale` is one figure for all of it, a separate one would have to scale each dock about its own corner, and nothing yet needs them to differ.

## Still open

- Rebinding has not been pressed with a real keyboard: the logic is tested but a capture has no key presses. No mouse sensitivity or axis settings yet.

## Where it comes from

- Early feature checklist (since removed), Phase 10
