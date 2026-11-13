# T294: Facilities: launch pad and builder levels, upgraded with money

Chosen by the user on 2026-11-12: in both modes a career upgrades its launch facility and its builder
for bigger and heavier rockets, paid with money. The launch pad caps the rocket's mass at liftoff; the
builder caps its height, its width and its part count. Each level's limits and price are records, one
set for Casual and one for Realistic, which mods patch. The sandbox has no limits.

## Done when

- A craft over the pad's mass or the builder's size is refused at launch, and says by how much (test).
- Upgrading costs the level's price and raises the limits; a save keeps the levels (test).
- The hangar shows each facility's level, limits and the next upgrade.

## Done (2026-11-13)

- `facilities` records (`FacilityDef`, `FacilityLevelDef`): levels with a `price` and the limits they
  set, `mass` (from the pad only), `height`, `width`, `parts`. Casual in the base game
  (`assets/core/facilities/stock.json5`: pad 30, 180, 700, 3,000 t for 0, 60,000, 250,000, 900,000;
  builder 30/50/80/130 m tall, 6/10/16/30 m wide, 40/100/250/any parts for 0, 40,000, 150,000,
  600,000), by the base game's craft (the Sounder 27 t on the first pad, the Talon and Titan on the
  second, the Pathfinder on the third, the Colossus on the last). Realistic patched by the Realism mod
  (`patches/facilities.json5`: from the V-2 to the Saturn V and beyond, 15 t and 20 m to 6,000 t and
  125 m, 300,000 to 20,000,000). Prices are guesses until contracts (T295) and T301 set the pace.
- `Career::facilities` (levels, saved), `level`, `upgrade` (pays the next level's price),
  `refuses` (each limit a craft is over, by how much), `CraftSize`, `Universe::craft_size` (mass with
  propellant, height and width from the box round what it collides with, parts).
- The editor refuses a career's launch over its facilities ("Too big to launch. Launch pad: liftoff
  mass 92.8 t, 62.8 t over its 30.0 t") after the tech tree check. Its panel shows each facility's
  level, limits and an upgrade button with the next level and price (`career.upgrade:<id>`,
  `career.facilities`, `career.notice`; `debug.funds:<amount>` for captures).
- Tests: `career::tests::a_craft_over_the_facilities_is_refused_saying_by_how_much`,
  `an_upgrade_costs_its_price_and_raises_the_limits` (and back from a save),
  `universe::tests::a_craft_is_measured_as_it_stands`,
  `tests/tech_trees.rs` `facilities_grow_level_by_level_and_start_with_the_first_rockets` (both
  modes). Capture: `OSP_CAPTURE_FACILITIES` (a 30 m tank refused, the pad upgraded, then launched).

## Left out

- The runway and orbit launches have no mass limit (only the pad's), and launching to orbit is
  still offered in a career.
- The old notice stays under the panel until the next launch try.
