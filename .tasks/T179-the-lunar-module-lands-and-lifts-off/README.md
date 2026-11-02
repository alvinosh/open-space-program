# T179: The lunar module flies by itself: lands on the Moon and lifts off

The Saturn V carries a dead 15 t drum where the lunar module should be. Before it can ride on the stack it has to fly: a craft of its own, with a descent stage on legs, an ascent stage with two seats, the real engines and propellant, starting in lunar orbit.

## Scope

- `mods/realism/craft/lunar_module.json5`, built by hand.
- A leg's foot: the gear contact, without rolling.
- A scenario that starts it in lunar orbit, and a smoke mission that flies it with a player's controls.

## Done when

- The smoke mission lands at under 3 m/s, standing within 12 degrees of upright, lets the descent stage stay on the ground and flies the ascent stage back to orbit.
- The craft weighs within 1% of Apollo 11's 15,103 kg.

## Done

- `mods/realism/craft/lunar_module.json5`, by hand: the ascent stage on its own model (cabin, two seats, LMAE, its tanks and four R-4D quads) on an octagonal descent stage in foil (LMDE-H, four tanks, batteries), on four legs. It weighs 15.0 t, split as Apollo 11's was.
- `foot: true` on a gear leg: it grips instead of rolling, and is drawn as a pad. A part with a `model` shape of its own is now drawn (only templates' models were). A craft with legs on a site with no pad stands on them, and gravity settles propellant on the ground whatever the last tick did.
- Scenarios `apollo11_landing` (lunar orbit) and `apollo11_liftoff` (at Tranquility Base, `sequences_done: 1` so the first press lifts off). The smoke mission `lunar_module` lands, stays 21.6 h, leaves the descent stage and reaches orbit.

## Still open

- The landing is flown by the smoke script's steering law. A player has only the throttle and SAS: the descent tool is T044, and nobody has landed it by hand.
- The Moon is a smooth ball (T042), so the slope is never tested, and the legs have no crush core yet (T043).
- It is not on the Saturn V: that needs T039, T040, T041 and T044.

