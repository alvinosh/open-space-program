# T305: A career's first flights can bring their science home

Found while building T284. Science is earned only by data and samples that reach the ground, sent by
radio or recovered. In both trees the first node has no way to do either:

- There is no parachute in the parts list at all (T048 made the component; only the Saturn V's
  capsule carries any), so a first rocket's capsule hits the ground too fast to be recovered.
- Casual: the antenna is in Communication (45 science, after Electrics). The pad's readings can be
  recovered, but the thermometer and barometer there are worth 4 science at most, under the first
  nodes' 5.
- Realistic: RP-1's start has instruments but no capsule, probe core, battery or antenna (those come
  in 1950 to 1956), so a sounding rocket's readings are lost with it. RP-1 sends them by the sounding
  rocket core's telemetry.

## To do

- A parachute in the parts list, on top of a capsule or on a side, sized for what it brings down,
  in the first node of both trees.
- Realistic: a way for the first sounding rockets to get their data home. To decide: a small
  telemetry radio in the start node (with its own battery), the whip antenna and a battery moved
  into the start node, or a parachute for the instrument section (the V-2s' film came back that way).

## Done when

- A Casual career's first rocket with a thermometer and a barometer comes home and earns enough for a
  first node (test or capture).
- A Realistic career's first sounding rocket earns science.

## Done (2026-11-13)

Decided by the user for Realistic: the way closest to RP-1, a sounding rocket's telemetry.

- Parachute in the parts list (Systems), in the first node of both trees: `procedural: { parachute:
  { diameter, deploy_altitude } }` (`ParachuteRequirement`), a drum packed as big and heavy as its
  canopy (0.115 kg/m² and 2 kg; an Apollo main, 25.4 m, about 61 kg). It starts at 24 m and 3 km,
  which brings a C-1 down at about 8 m/s. A craft with no sequences of its own arms its parachutes
  in a last sequence.
- Telemetry unit in the parts list (Systems), in the first node of both trees: 1 W at 512 bit/s
  (RP-1's Science-Core sends on a 1 W omni) on its own 40 kJ battery. `antenna` `telemetry: true`:
  it only sends; it hears no commands, so a vessel with nothing else flies with or without a link,
  a probe core cannot be reached through it, and no burn is uploaded over it.
- The Realism mod's historical ranges each have a small station for telemetry
  (`mods/realism/ground_stations/ranges.json5`: White Sands, Peenemünde, Baikonur, Wallops,
  Vandenberg, Kourou).
- The C-1 and C-4 capsules have batteries (10 and 20 kWh; Mercury flew on about 13.5): without them
  instruments had no power off the pad in Casual.
- Tests: `tests/first_flight.rs`. Casual: the suborbital hop's capsule with a thermometer, a barometer
  and the parts list's parachute climbs to 27 km, splashes down at 7.8 m/s, is recovered with 10 kB
  of readings and earns 12.6 science (the first nodes cost 5). Realistic: a WAC Corporal from White
  Sands with the telemetry unit sends 7.9 science to the range's station before it is lost (RP-1's
  first nodes cost 1 to 4). `sizing` (the parachute against an Apollo main, and a soft landing),
  `comms::tests::policy::telemetry_sends_data_but_carries_no_commands`, `tests/tech_trees.rs` (both in
  both first nodes). Capture: `OSP_CAPTURE_SCIENCE` puts the parachute on the capsule's nose in the
  editor (`science_chute_preview.png`, `science_on_craft.png`).

## Left out

- The pacing is not tuned: a WAC Corporal earning 7.9 may be quick for Realistic (T301 tunes it).
- The parachute's canopy was not seen opening in a capture (T048's drawing is used as it was).
- The smoke tests were not run after the C-4 got a battery (the Pathfinder Crew flies one).
