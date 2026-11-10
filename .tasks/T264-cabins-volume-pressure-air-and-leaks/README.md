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
