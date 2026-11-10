# T263: Crew who are people: a name each, and health that goes by stages

Each person aboard has a name and a condition. For each need (air, water, food, and later CO2,
pressure, dose and stress) they go from fine to short, weak, critical and dead, with times from the
rules' profile, and come back if given what they lack. One person can die while the others live. Saved
with the vessel; the Spacecraft window lists them. Today the crew are a count and everyone is lost at
once, five minutes after the oxygen runs out.

## Done when

- Without water a crew goes through the stages, and the first is lost at the profile's time and not
  before, at any warp (test).
- Given water back before the end, they recover (test).
- Names come from a list a mod can replace. The roster and recovery (T103) build on this.

## Done (2026-11-10)

- Each crew holds `people` (`systems::Person`): a name, a resilience of 1 to 1.2 from a hash of
  the name, and how long, in effect, they have gone without each need. For each need with a `lasts`
  they go fine, short, weak (a third of the way, `weak`), critical (two thirds, `critical`) and die at
  `lasts` times their resilience, one at a time. With part of what they need they decline in
  proportion; with all of it they recover as fast as they declined. Each change is said by name
  ("Ada Berg is weak from lack of water", "... has died for want of water", "... has recovered").
- Time warp stops at each stage: `System::next_event_given` gets the shares the tanks give now, from
  a dry run of the solver in `Propulsion::next_system_event`.
- Names: a new `crew_names` collection (`assets/core/crew_names/stock.json5`, first and last names a
  mod can add to or replace), the same name for the same seat of the same part; a craft can name its
  crew (`crew: { names: [...] }`). Boarding moves the person and how they are, and says who moved.
- The Realistic profile now gives water 3 days and food 30 days, besides 5 minutes for air, and
  calls them air, water and food.
- The Spacecraft window lists the people, and how anyone not well is doing.
- Tests (crew.rs): the stages and each death on time, at the highest warp and a minute at a time;
  recovery; half the water, half the pace; names from the list or the craft; boarding moves the
  person. The crew's save round trip now covers the people.
- 774 tests pass across the sim, game, mods and smoke crates, every smoke mission among them.
  `cargo smoke saturn_v`: 143 passed, 0 gaps, every sample and finding as before; the only change in
  its log is that "Someone moved across" now names who.

## Left out

- The same seat of the same part gets the same name on every launch; the roster (T103) gives each
  person a life of their own across flights.
- Acceleration still loses everyone in a cabin at once.
- Recovering does not stop time warp; nothing hangs on it.
- A save from before has its crews named "Crew member 1" and so on.
- CO2, pressure, dose and stress come as needs or conditions with T258, T264, T267 and T266.
