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
