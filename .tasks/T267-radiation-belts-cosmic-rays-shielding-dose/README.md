# T267: Radiation: belts, cosmic rays, shielding and dose

Realistic only. A dose rate that comes with the place and time, next to sunlight in `Environment`:
galactic rays (less inside a magnetosphere and on a surface), the belts of bodies whose records have
them (Earth's inner and outer belts, Jupiter's), and the ground. A cabin's walls shield it; dose builds
up per person, and past the profile's limits they sicken, slowly. Parts count dose too, for T050.
Shown in mSv/h. Casual: off. T116 keeps the aurora and the overlay.

## Done when

- A vessel parked in Earth's belts takes far more than one in low orbit, and one on the Moon's
  surface less than one in lunar orbit (test).
- Thicker shielding lowers the dose (test).
- The dose over a coast is the same at any warp, belt crossings included (test).

## Done (2026-11-12)

- `crate::radiation`: the dose rate at a point, by kind (cosmic, trapped, surface), Sv/s before any
  shielding. Cosmic rays come from the root body's record, less the share of the sky the body below
  fills and the share its magnetic field keeps out (falling with the square of the distance). Belts
  follow the field lines of a dipole on the body's spin axis (L = r / cos²λ), a smooth bump from
  `inner` to `outer` with its `rate` at `peak`, falling off the magnetic equator, and nothing below
  `floor`, where the air takes the particles. A body's ground gives off `surface`, by how much of the
  sky it fills. The air above a point counts as shielding (its pressure over gravity, kg/m²).
- The figures are in the core bodies' records (`radiation`): the Sun's cosmic rays (0.06 mSv/h),
  Earth's field and two belts, Jupiter's, and what the Moon's and Mars's ground give off. Unshielded:
  low orbit 0.37 mSv a day, deep space 1.4, the heart of the inner belt 1,430, lunar orbit 1.1 and
  the Moon's surface 0.96.
- Rules `radiation` names a `radiation` record; the base game's is `null`, the new Realistic
  Radiation mod (`mods/realistic_radiation`) sets `realistic` and is in the Realistic game mode. Its
  record says how much shielding cuts each kind to 1/e (cosmic 400 kg/m², trapped 20, surface 100)
  and the doses at which people are sick, weak, critical and die (1, 2, 4 and 6 Sv, times their
  resilience).
- A part's walls shield whoever sits in it: its dry mass over the area of its skin
  (`Geometry::skin_area`: walls and skins, or a solid's surface where a subpart has neither; the
  stand-in shape's for a model part). The Saturn V's command module comes to 122 kg/m², the lunar
  module 27. Each person keeps their dose (`Person::dose`), which goes with them when
  they board another part and does not go away.
- Over a coast the dose is integrated along the orbit, a panel at a time (each a fiftieth of the way
  from the body, 1 s to 10 min, Simpson's rule), so one step and many come to the same. Landed or
  under power, the rate where it is times the step. Time warp stops where anyone aboard reaches their
  next stage (`radiation::time_to_dose`, the same sums a step to it does).
- The Spacecraft window shows the rate outside and inside the crew's part (mSv/h) and each person's
  dose; someone sick shows as "(short: radiation sickness)".
- The Saturn V smoke reports the most anyone took as an info line: 10.5 mSv over the mission, with
  143 passed, 0 gaps and every other finding as before. That is about twice what Apollo 11's crew
  took (1.8 mGy absorbed, a few mSv): the cosmic rays are a solar-minimum figure and 1969 was near
  solar maximum.
- 782 tests across the sim, game and mods crates. Tests: rates (belts far above low orbit and off the equator less, the Moon's ground below its orbit,
  the air's column at sea level); shielding cuts the belts most; a day of a transfer orbit through
  both belts in one step and in 1,440 minutes agrees to 1e-6, and the dose is found where the sums
  reach it; a capsule in the inner belt takes over a hundred times low orbit's dose and four times
  the wall mass cuts it a hundredfold; parked there a person is sick at 1 Sv and weak at 2, times
  their resilience, warp stopping there at the highest warp as an hour at a time.

## Left out

- Parts do not count dose yet: T050 will say what failures need, and what shields a part.
- The field is a dipole on the spin axis: no tilt or offset, so no South Atlantic Anomaly (T116), and
  the magnetotail does not shield the Moon. Solar storms are T269.
- Other parts of the vessel do not shield a crew; only their own part's walls and the air do.
- Dose is not shown in the parts list or the editor.
- Cosmic rays do not follow the solar cycle; T269 (solar storms) is where the Sun's activity comes in.
