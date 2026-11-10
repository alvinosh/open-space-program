# T279: Vessel monitor: every vessel, what runs out when, alerts

One window for every vessel: power, supplies and the time each has left, crew and their condition,
cabin, dose, drives, link and failures, the worst first. Alerts come from the simulation, not from the
window looking: a supply out within a day, a battery low, a drive full, a link lost, a failure. They go
to the alarm clock (T200) and the messages. A vessel's parts can be worked from here only with a link
or a crew (`Vessel::commandable`). Each section shows as its systems arrive.

## Done when

- The player can see which vessel runs out first without flying it.
- An experiment on a probe can be shut from the monitor with a link and not without (test).
