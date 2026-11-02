# T175: Procedural decoupler and separation motors

A decoupler today is a component on a part with a node, a separation impulse and a crossfeed flag. It has no ring size, no strength and no shock, and a player cannot place one because there is no editor palette yet. Make it a sized part like the tanks, with the small motors that make a stage separation clean.

## Scope

- A decoupler sized from the ring diameter and the load it carries (mass above, preload, bending): its mass, its break load, and its separation impulse from springs or a pyrotechnic charge.
- Axial and radial (for boosters), and one that releases an inside joint (the lander in its adapter).
- Separation motors as small solid motors the sizer picks: retro motors that pull a spent stage away and ullage motors that settle propellant (T150 needs the second).
- The shock at release as a load on both neighbours (feeds the joint strengths of T051), and the interplay with hot staging (T159).
- The requirement fields listed for the editor's palette (T017 makes the entry).

## Done when

- A decoupler placed at a ring of a given diameter carrying the Saturn V's loads is the sized number, not a flat mass, and breaks over its limit.
- The Saturn V's stage separations use sized retro and ullage motors.

## Done (2026-11-02)

- `procedural: { decoupler: { diameter, load, impulse, crossfeed } }`: a ring whose wall carries the load at 1.25 times over, in aluminium at 300 MPa, 2 mm at the thinnest; its mass from that, and `DecouplerDef.strength` what it holds. A joint past its strength breaks and the vessel is lost (`structure::Part::Joint`). All six of the Saturn V's rings are sized, for loads read off the flight at 4 g: 135 kg at the S-IC.
- `procedural: { separation_motor: { impulse, burn_time } }`: one part that is a tank and an engine feeding on it (`SolidMotorDef`, HTPB). The TE-M-424 is the oracle: 102.3 kN for 3.75 s, 153 kg of grain, 104 kg of case, within 10%.
- The Saturn V's ullage motors are sized (they were catalogue records with a grain sphere beside them), and the S-IC has four retro motors that light with the joint letting go. They burn on the debris: a lit solid ignores the throttle and cannot be shut down (part of T154), and one lit as it leaves still starts.
- A section's delta-v and burn time leave out separation motors unless that is all it has (`Propulsion::main_engines_of`).
- The stages' masses are what they were: what the rings and motors weigh came out of each stage's flat mass.

## Still open

- Axial rings only by requirement: a radial pad is still a flat ring, and a decoupler that lets go of an inside joint (the lander in its adapter) has not been tried.
- Shock at release as a load on the neighbours is T051, and a separation with hot staging T159.
- The loads are read off by hand and written in the craft. A sizer that works them out from what stands on the ring belongs to the builder (T015).
- A ring on a stage whose joint it does not hold is read as the stage's (`structure::loads` takes the first with a strength).

## Where it comes from

- Planning of 2026-11-01: raised as missing from the part list.
