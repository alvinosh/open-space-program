# T100: Fuel ports, fuel lines and hoses, pumping, tank lockout

Fuel ports, player-defined fuel lines and hoses, pumping that costs power, and tank lockout.

Fuel lines themselves are built (T254, 2026-11-09), and docking ports crossfeed while docked. T086
(propellant flow rules, drain order and crossfeed) was merged here on 2026-11-20, since T254 covers
it: what it still lacked is a drain priority per tank that the player sets, which goes with the
lockout.

## Done when

- Everything in the scope list works, and what can run headless has a test.

## Where it comes from

- Early feature checklist (since removed), Phase 5
