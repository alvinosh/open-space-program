# T198: Wind effects on a rocket

On the climb a rocket wore grey feathers up its whole length from about 400 m/s, and at night they glowed on a black hull. They were the re-entry effect's white Mach look, which had not been looked at on a long rocket since its rework (T183).

## Done when

- A climb by day and at night shows nothing on the hull, a falling rocket does not glow at night, and an entry looks as it did.

## Done

- `osp_sim::plasma::entry_look`: nothing shows climbing, unless it is as hot as the way out of a skipping entry. Heating on a real climb to orbit is held under that by a test, for small and big rockets.
- A Mach cone's white is vapour, so it shows by the light on it (`vapour_light`, `extent.w` in the shader), and its light on what is around goes with it.
- `exposure` in `shaders/plasma.wesl` no longer counts open air beside the outline as the air reaching a point: that marked the wall of a long hull as reached all the way back, and strips came off the length of it.
- `OSP_CAPTURE_ASCENT=1` and `OSP_CAPTURE_NIGHT=1` with `OSP_SCENARIO=pad` for the climb. `OSP_CAPTURE_FALLING=1` with a scenario starting fast in the air for the rest, checked with a temporary mod: a falling Pathfinder by day and at night, and the Apollo CM at 7 km/s at 65 km, which came out the same as before.

## Left out

- A faint fringe on the upper hull of a rocket falling by day: real shoulders in its outline do catch the air.
- A vapour cone around Mach 1 on the climb. Real rockets have one for a few seconds; this took away what was there rather than adding it.
