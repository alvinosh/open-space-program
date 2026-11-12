# T264: Cabins: volume, pressure, air and leaks

Realistic only. A crewed part's cabin has a volume from its shape and holds air: oxygen, nitrogen and
CO2 as partial pressures. Breathing turns oxygen into CO2 in it; scrubbers take CO2 out; a regulator
tops oxygen and nitrogen up from tanks to a set pressure. A cabin can be let down and filled again,
and leaks through a hole of some size (damage now, failures with T050). People in a cabin without
pressure are lost unless suited. Docked cabins share air through a tunnel; a cabin can be shut off,
which takes its volume and its people's room with it. Casual: off.

## Done when

- Breathing without a scrubber raises CO2 until the crew weakens (test).
- A leak lowers the pressure at the rate its hole gives, and the crew is lost when it is too low (test).
- The Spacecraft window shows each cabin's pressure, oxygen and CO2.

## 2026-11-10

T258 built the cabin this grows from (`systems/cabin.rs`): a store on each part with life support
that the resource solver reaches, with its air (so far only what is breathed out, CO2, up to the
profile's `cabin` kg per m³) and its lockers. The volume is the part's `volume` or the profile's room
per seat.

## Done (2026-11-12)

- The Realistic profile gives cabins `air` (`mods/realistic_life_support/life_support/profiles.json5`):
  `CabinOxygen` kept at 21 kPa from `LqdOxygen`, `CabinNitrogen` at 80 kPa from `Nitrogen`, and CO2 up
  to 8 kPa, at 294 K, as partial pressures from what there is of each in the cabin's volume. A cabin
  starts full. People breathe `CabinOxygen` out of it and breathe CO2 into it; lockers carry
  `LqdOxygen` for the days they carry. Casual cabins have no air.
- A regulator tops each gas up from the tanks the part reaches, filling an empty cabin in 60 s and
  giving no more than that against a leak (`Cabin::regulator`, lots on the resource solver).
- Leaks: `System::breach` makes a hole (m²) in a cabin; the air goes out by choked flow to vacuum, a
  share of what there is a second (`Cabin::leak_rate`: 1 cm² empties a 2 m³ cabin with a time constant
  of 126 s). The solver works such a loss out exactly with everything else going in and out over a
  step of any length (`System::decay`, `Store::leak`), and lets a maker fill what leaks besides the
  room, so a regulator holds a small leak exactly and its supply runs down at the hole's rate.
- Let down and filled again: the cabin's actions `cabin_let_down` (opens a 10 cm² valve, stops the
  regulator) and `cabin_fill`.
- `limits` in the profile: under 6.3 kPa of pressure people last 90 s, under 10 kPa of oxygen an hour,
  over 2 kPa of CO2 half a day. Out of a limit they go short of it like a need, by stages, at the
  crew's resilience; time warp stops where the air crosses a limit (`System::next_event_at`, from the
  inflows a dry run of the solver gives, exact for e^-kt with steady inflows).
- Suits: the crew actions `suits_on` and `suits_off`. Suited, the limits do not touch them, they
  breathe what tops the air up and their CO2 goes into the suits.
- The Spacecraft window shows each cabin's pressure, oxygen and CO2, and whether it is leaking or let
  down; a cabin's part window its pressure and each gas. The Supplies tank in the parts list holds
  `LqdOxygen` for what is breathed from the air.
- The solver had a flaw this showed up: a lot that wants far more than it can use (a regulator asking
  for a minute's refill of a full cabin) took everything from lower-priority lots sharing its inputs on
  the first pass, and shares only ever fell, so they never got it back. It now starts from a settled
  guess (`settle_on`) where each lot asks each pool for what the other pools let it have, and then
  cuts back as before, which still guarantees nothing goes out of bounds.
- Tests (crew.rs): without a scrubber the CO2 passes 2 kPa, the crew weakens, then dies 300 s after
  8 kPa, to the millisecond at the highest warp and a minute at a time; a 1 cm² hole lets the
  pressure down at e^-kt and the crew is lost 90 s after 6.3 kPa, the same at the highest warp and a
  second at a time; a 1 mm² hole is held, the pressure unchanged and the nitrogen in the lockers used
  at exactly the hole's rate, and once it is gone the oxygen is still held; let down with suits on
  the crew are well and breathe the lockers' oxygen, filled it is back at sea level in the minute,
  without suits they are lost.
- 777 tests across the sim, game and mods crates. `cargo smoke saturn_v`: 143 passed, 0 gaps, every
  finding as before, all three crew home with 43 kg of oxygen, 124 kg of water and 19 kg of food left.

## Left out

- Docked cabins do not share air, a hatch cannot shut one off, and a cabin's volume is still the
  part's `volume` or the rules' room per seat, not its shape's: T286.
- Holes only come from `System::breach`, which nothing in the game calls yet: failures (T050) and
  damage will. The cabin's valve is the only way to let air out in play.
- Air goes to vacuum: a cabin let down or holed in an atmosphere still empties.
- Suits are for everyone in a cabin at once, and draw on the cabin's supply, not suits of their own.
- The cabin's temperature is the profile's figure, not the part's heat (T257).
