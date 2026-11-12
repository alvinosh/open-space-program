# T272: Experiments that run over time

An experiment is a record: how long it runs, how much data it makes a second or what sample, its power,
and where it works (which bodies; landed, in the air, low or high orbit; an altitude band; sunlight).
On a part it runs while switched on and its conditions hold, pauses when not, and keeps going while
the player flies something else. Casual: minutes. Realistic: hours to days.

## Done when

- At any warp an experiment makes its data in its time, and pauses when it leaves where it works
  (test).
- A thermometer, a barometer, a magnetometer, a camera and a surface sampler are in the parts list.

## Done (2026-11-12)

- `experiments` records (`ExperimentDef`): duration, `size` (bytes) or `sample` (kg), power, value
  (for T284), situations, bodies, sunlight, tech. `instruments` records (`InstrumentDef`): a box of a
  mass and size carrying experiments, listed as `instrument:<id>` in the parts list's new Science
  group (a flask tab).
- `osp_sim::science`: situations as KSP's (landed, splashed, flying low and high, space low and high)
  by each body's `science.flying_high` and `science.space_high` (defaults 0.18 of the air's height and
  a third of the radius), a vessel's `Place`, and subjects `<experiment>@<body>.<situation>`.
- The `experiment` system: switched on (from the start, or the part window), somewhere it works and
  powered (shed first), it collects a whole set per subject in its duration, then rests until it is
  somewhere new. The vessel tells its systems where it is before each step (`System::situate`); time
  warp stops where a set is done (`next_event`) and where a coast crosses space low and high
  (`next_radius_crossing`), so it is exact at any warp. Collected shares are saved with the vessel.
- Content: thermometer, barometer, magnetometer, camera, surface sampler. Casual: a minute to ten.
  Realistic (`mods/realism/patches/science.json5`): RP-1's figures (ten minutes and 75 B for
  temperature and pressure, a month for the magnetometer, a film camera whose 2 kg of film is a sample,
  an 80 g surface sample in five minutes) and RP-1's body thresholds (Earth 40 km and 35,786 km).
  Trees: Casual first node has the thermometer and barometer, then camera, magnetometer, sampler;
  RP-1 has the first three at its start, the magnetometer at Satellite Era Science, the sampler at
  Surface Science.
- Tests: `systems::experiment` (a set takes its time at any warp and warp stops where it is done; on
  an orbit from 300 km to 8000 km the share low and high is the same at the highest warp as in five
  second steps; what it says and wants), `science` (subjects), `tests/tech_trees.rs` (45 Casual parts,
  the sounding rocket's instruments at the start). Capture: `OSP_CAPTURE_SCIENCE` puts a thermometer
  and a barometer on a tank under a C-1, launches, and opens the thermometer's window on the pad:
  13.5% after 81 s of a 600 s set, landed on Earth.

## Left out

- What is collected stays on the experiment until drives (T273) and sending (T274); science is not
  earned yet (T284). Samples are counted in kg there for now.
- The window shows the readings, not the place in words (`Experiment::status` has it).
- Found on the way: warp above 4x holds still on the pad (T302), and nothing goes on a capsule's
  side (T303).
