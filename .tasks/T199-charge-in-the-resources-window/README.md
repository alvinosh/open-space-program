# T199: Charge in the Resources window

The user could not tell when the batteries ran out and the avionics stopped.

## Done when

- The Resources window shows what is left and how long it lasts, and says so when it is gone.

## Done

- A `Charge` row per stage with batteries, in kWh or Wh, and a `POWER` line: how long the first network's batteries last at what it draws now, `Not running down`, `Batteries flat`, `Loads off` or `Out of power`.
- `Low power` alert with under 5 minutes left, flat batteries or any load off. `Out of power` now fires for avionics without power, which before only showed `No control`.
- `Systems::power_state` has a test that the time it gives is when the store does run out, and a lunar module on the pad bore that out: 32d 19h at the start, 8d 16h 47m left after 24.09 days.
- `OSP_CAPTURE_POWER=1` runs a lunar module's batteries out. A Saturn V was run the same way to see `Out of power` with `No control`.

## Left out

- The time is to the batteries going flat. Fuel cells then keep the highest priority loads going until their tanks run out, which it does not count.
