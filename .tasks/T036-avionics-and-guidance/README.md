# T036: Avionics and guidance

A procedural avionics unit sized from the mass it controls and the tech level (RP-1's formulas; Near-Earth levels work only close to Earth). Without a powered unit a vessel has only the limited control of a science core: no SAS, no gimbal command, no burn executor, no staging sequencer.

## Scope

- The instrument unit as a ring with an avionics unit.
- Sensors (gyros, star trackers, sun sensors) as small procedural parts, perfect at first.
- Sequencer triggers a unit can arm: an acceleration limit, a propellant level, a time.

## Done when

- The Saturn V cannot steer through the S-IC without its instrument unit, and the unit's mass is the sizer's number, not a flat mass.

## Done (2026-11-02)

- `avionics` component and sizer: RP-1's formulas from what it controls and a level. The Saturn V's instrument unit is 2007 kg from 2950 t at the 1959 level, against about 2 t, 0.91 m tall under the S-IVB's ring, on two 350 Ah batteries of its own that it draws down at 3.5 kW. The Saturn IB has one too, and the CM a computer for 50 t (its power is on the CM's load).
- `Propulsion::controlled`: what the powered units control, added up, against the vessel's mass; a level for near the Earth works only in the Earth's sphere of influence. Out of control means no SAS, no gimbal, no burn executor, no triggers.
- `CraftDef.triggers`: a time, a propellant level or an acceleration limit fires activations by itself. The Saturn V's centre and outboard cutoffs are triggers now, not sequences: Apollo 11 leaves 21 t in the S-IC at cutoff.
- `the_saturn_v_cannot_steer_without_its_instrument_unit` flies the stack without it: SAS goes off, no engine swings.

## Still open

- A vessel with no unit at all still flies as it always has, so that no craft breaks: 43 of the 45 craft have none (T087).
- The stage key is not gated by control, and a Near-Earth level on a vessel in lunar orbit counts as near the Earth, since the Moon is in the Earth's sphere of influence.
- Sensors (gyros, star trackers, sun sensors) are not parts yet: attitude is known perfectly.

## Where it comes from

- docs/systems-plan.md (step 11, Avionics and guidance)
- old issue list, issue 13 (docs/issues.md, removed; see git history)
