# T154: Solid boosters that look and burn like solids

A solid motor is drawn with the same engine bell as a liquid engine and gets the same plume. It should be recognisable at a glance.

## Scope

- Its own plume look (T153 added a first `Solid` look, selected by `kinds: ["solid"]`; tune it here against real footage): bright orange-white core, a thick grey-white smoke trail with alumina, little or no shock diamonds, no turbine exhaust.
- Its own model: a case with a short, wide, fixed or lightly gimballed nozzle, not a turbopump bell. A procedural fallback when a config has no model.
- The staging icon is distinct from a liquid engine (T155).
- A solid cannot be throttled or shut down; the engine readout says so.

## Done when

- A Saturn V and a Shuttle-style stack are told apart from a liquid stack in a screenshot at 100 m, by the nozzle and the plume.
- T098 is not needed for this; the thrust curve the importer gives (`thrust_curve`) is already data.

## Done

- The plume stays where T153 left it (a heavy flame, no clean diamonds), and a solid now leaves a column of smoke (`exhaust_trail.rs`, `shaders/exhaust_trail.wesl`, `trail` on a `plumes` record): anchored to the ground and turning with the planet, staying after the motor is gone. Its width and depth come from mass flow over airspeed and the air's density, so it is opaque low down, a wide veil at 40 km and gone by 80 km, with no altitude table. It thins as it spreads over minutes.
- A solid without a model of its own is drawn with a straight cone from a wide throat and a short housing above it, no longer than its exit is wide (`EngineDef::nozzle_length`; the rest of its `length` is sunk in the case).
- A lit solid ignores a cutoff, offers none in its part window, and that window says it burns until its grain is gone instead of giving a throttle range.

## Still open

- No ground cloud: the column starts wide off the pad, but the exhaust does not run along the ground and billow up the way it does at a real launch.
- Liquids leave no trail (`trail: 0` on their looks); a kerosene look could take a small share of soot.
