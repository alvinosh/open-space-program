# T169: Scenario and tutorial: Mars orbiter probe

An uncrewed probe to Mars, and the first scenario where the ship has to stay alive on its own: power, a link home and its own avionics. The transfer itself is already possible (the planner solves Earth to Mars, about 3.6 km/s from low Earth orbit and about 2.65 km/s relative on arrival; rails warp goes to 100 000 times and automatic warp to 10^8).

## Scope

- Craft: a probe built by hand with solar arrays, a battery, an avionics unit, a dish and a stage with the delta-v for departure, a correction and capture.
- Goals: leave Earth's sphere of influence on a Mars transfer, a correction that sets the periapsis, capture into a Mars orbit in a given range, and keep the probe powered and in contact through it.
- The part that makes it a game: the light delay is minutes at Mars and the probe goes behind the planet, so the capture burn has to be uploaded and armed before the blackout (T038), with the arrays toward the Sun and the dish toward Earth.
- Steps: target Mars, the transfer planner (F4), create the burn, Aim Pe, warp, the correction, upload the capture burn before the blackout, Circ Pe, confirm contact.
- It must be playable in minutes, not hours: the warp steps in the tutorial matter.

## Done when

- A script with player-level controls completes it, including the blackout.
- The tutorial runs start to finish in the running game.

## Notes

- Crewed Mars waits for the lander (T172) and a much larger stack.
- Mars is a smooth ball until T110, which an orbiter does not mind.

## Where it comes from

- Planning of 2026-11-01. Revised the same day: power, avionics and comms are core, not depth.
