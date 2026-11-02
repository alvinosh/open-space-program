# T087: Control-module requirement and 'control from here'

A vessel needs a control module to be flown, and the player can pick which part it is controlled from.

## Done when

- Everything in the scope list works, and what can run headless has a test.

## Done (2026-11-02)

- A vessel started from a scenario has no control without powered avionics for its mass or a crew (`Propulsion::require_control`, which the debris of it inherits): no SAS, no gimbal, no burn executor, no triggers. A crew flies up to 30 t by hand. `Vessel`s made directly, as most unit tests do, fly as they always have.
- All 40 shipped craft that had no unit got one on their root, 1.2 times their mass, interplanetary, weighing nothing beyond what they already were (flat masses): no figure moved. A test holds every craft to it (`every_craft_can_be_flown`). Spent stages have none, so they no longer answer.
- "Control from here": the part window of a part with avionics or a crew offers it (`Action::CONTROL`). The player's axes, SAS and the navball use that part's axes (`Vessel::control_attitude`); a burn still points the engines.

## Still open

- Those units are a placeholder for the sizer's number on all but the Saturn V and IB: the mass they stand for is inside each craft's flat structure.
- The flight camera and the map's markers still use the vessel's own nose, and the control frame was only tried in tests.

## Where it comes from

- Early feature checklist (since removed), Phase 2
