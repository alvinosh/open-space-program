# T253: Ship systems made robust: fuel, power, comms, heat, life support

Said on 2026-11-09: these systems are the priority and have to be very robust: fuel lines for
asparagus staging, aerodynamic heating, comms for unmanned probes with different antennas and relays
that differ in size and power, and much more realistic life support in Realistic and a simple one in
Casual. It goes before the smaller Solar System (T251) and the casual rockets (T249).

## Where each stood on 2026-11-09, from the code

- Fuel: propellant flows along the part tree, and across a decoupler only with crossfeed on. Every
  tank reached drains in proportion. No fuel lines, no direction, no drain order. Asparagus cannot be
  built.
- Power: batteries, solar arrays (angle, eclipse, ageing), fuel cells, RTGs, load shedding and separate
  networks are real. The player cannot place any of them.
- Comms: a real link budget, line of sight, light delay, a crew or a link to command. One band, one
  relay hop, no catalogue of antennas, and they cannot be placed.
- Heat: Sutton-Graves on the surfaces facing the flow, shadowing, ablation. Each part is one lump, only
  hulls and cabins can fail, one 1200 K limit for everything, no conduction, no engine or solar heat.
- Life support: three daily consumables. Out of oxygen, everyone dies after five minutes. Food and water
  never matter. No CO2, cabin air, recycling or gradual effects.
