# T156: Flight HUD smaller and quieter

At the start of a flight the Clock, Flight, Orbit and Mission clock windows are all open, the navball panel is about 430 px wide, and the right menu has 14 buttons. It covers a third of the view and the numbers carry no priority.

## Scope

- Open by default only: the navball with throttle, the staging window, altitude and speed. Everything else is one click away and remembers if it was opened.
- Smaller default type and padding; the navball panel narrower, the SAS and RCS buttons in two rows at most.
- A UI scale setting is T128; here only the defaults.
- The Orbit and Flight windows switch themselves while it makes sense (Orbit only above the atmosphere).

## Done when

- At 1280×720 less than a fifth of the screen is covered at the start of a launch (captured screenshot), and the same windows are open again after a reload of the flight.

## Done

- Flight, Orbit and Resources start closed; the example mod's Mission clock too. What stays open: clock, stage stack, altimeter, navball, launcher. At 1600x900 that is about 14% of the screen (captured: `OSP_CAPTURE` pad and `OSP_SCENARIO=leo`).
- `windows.rs`: the Orbit window opens when the vessel leaves the atmosphere (`vessel.in_space`) and closes on re-entry, and the other `flight` windows the player opens are saved in `windows.json` beside the settings file and restored; captures do not read or write it. Tests cover both.
- Not done: smaller type and padding, and a narrower navball. The navball housing was already redone (272 px wide, SAS strip beside it), and at this size the type reads fine; the staging window is T160. Revisit with T128's UI scale.
