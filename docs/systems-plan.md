# Spacecraft systems and procedural parts

Plan from 2026-10-30, after the Saturn V smoke test (`docs/smoke-tests.md`) flew Apollo 11 and
listed what the sim does not do. The goal is that a mission like it flies on the systems the
real one used (power, avionics, communications, docking, a lander on legs, a heat shield,
parachutes, a crew that has to breathe), and that every one of those parts is built from a
requirement by a procedural sizer, like the tanks already are, so a player can build the
Saturn V, the Shuttle or a Dragon and get the right mass and performance without a flat number
anywhere.

Written for: you, and the later sessions that build each step. Each step is one turn: build it,
verify it, report back. Sizes (S, M, L, XL) are defined in `.tasks/README.md`. This covers
electricity, parachutes, docking ports, oceans, crash tolerance and the launch escape
tower (the early feature checklist is now in `.tasks/`), and ties them to the part model of
`docs/vessel-overhaul.md`. The build editor (`docs/build-editor-plan.md`) comes first, and
steps 3 to 7 finish the build core right after it, so that every part type after them is something
a player can place.

## Where it stands

From the smoke test's report, and a look at the code:

| System | Today |
| --- | --- |
| Tanks, engines, RCS, decouplers, fairings, wings, wheels | Simulated and procedural (storage walls from membrane theory, gear from a record) |
| Batteries | A number on the part. `parts/gather.rs` drops the component when the tree is built. Nothing draws or recharges it |
| Fuel cells, solar panels, RTGs | Fuel cells are plain boxes. `solar_panels` (11 tech levels) is loaded and never read |
| Avionics | `avionics` (37 levels, with mass and power formulas) is loaded and never read; any vessel flies without one |
| Antennas, ground stations, links | Nothing at all |
| Docking | Nothing. `PartTree` and `Assembly` can release a joint and split; nothing can join two vessels |
| Landing legs | `GearLegDef` is a wheeled strut. The Moon has no terrain (a smooth ball) |
| Heat, heat shields | No temperature anywhere. `MaterialDef` has density and strength, no heat properties |
| Parachutes, the sea | Nothing. A capsule hits the ground |
| Crew | A seat count. Pod records carry food, oxygen and water, and nothing reads them |
| How parts reach the physics | `PartTree::from_craft` (`parts/gather.rs`) sorts every part into one of a closed set of kinds (`PartKind`: hull, tank, engine, RCS, wheel, pod, fins, wing, gear, shell, fairing half) by its single functional component, and refuses the rest ("a stage has no part like it"). An engine, an RCS block and a pod must stand upright; a battery, crew or container component is ignored |
| Joints that move | `Assembly` has `Hinge` and `Slide` joints and surface mounts (`u`, `v`, standoff). Nothing drives them, and mass, aero and contact do not follow their travel |
| Sequences | `Activation` is four things: light an engine, release a joint, jettison a fairing, switch RCS on |
| Placing parts | `CraftDef::place` puts an assembly on a connector, and `fit_fairings` is a build tool. There is no general place, validate, pattern or undo; the editor is Phase 3 |
| Failures | `EngineDef::reliability` is loaded and only `rated_burn_time` is read |
| Command cutoff, accurate burns | A sequence cannot cut an engine (issue 9); the executor overshoots by 0.9 m/s (issue 10) |

Data for several of these already came in from RP-1 and RealFuels (battery, solar and avionics
tech levels, pod contents, engine ignition costs), so some steps are mostly wiring.

## Principles

1. A procedural part is a requirement and a design. The craft file (or the editor's palette)
   stores what the part must do ("5.9 kWh", "land 5.6 t at 9 m/s", "a 0.8 m S-band dish"); a
   sizer turns it into dimensions, mass and performance from physics and the tech level, and
   runs again whenever the data changes. A flat `structure: { mass }` for hardware that has a
   sizer is a bug the build audit reports.
2. Geometry first. A sizer reads the part's geometry where it can: a battery fills a solid, a
   heat shield covers the outer surface that faces the flow (the voxel aero already knows what
   is enclosed), a leg's pad area comes from the ground pressure, a cabin is a pressure vessel
   sized the way a tank is. A requirement that cannot be met is a build error with a reason,
   never a part that quietly underperforms.
3. Tech level comes from the date. Sizers take the record of the tech level the game's year
   unlocks (`batteries`, `solar_panels`, `avionics`, `tank_types`, `materials`, and new
   `ablators`, `canopies`, `antennas`), as tanks and engines already do (`tech.rs`).
4. One oracle per sizer: a real part it must reproduce within a stated tolerance (the Apollo
   CM's batteries, its main parachutes, the LM's legs). The oracle is a unit test, and the
   Apollo 11 smoke test is the oracle for the whole.
5. Every system has a rails mode. A three-day coast must stay cheap and exact: power, thermal
   and links are closed-form between events (eclipse in or out, battery empty, a link opening,
   a limit crossed), found the way `next_trajectory_event` finds an SOI change.
6. Nothing special-cases a vehicle. A system works on any part tree, so the Soyuz, Dragon and
   Shuttle get it for free. Build-time tools (symmetry, assemblies, the fairing tool) write
   plain parts, as in the overhaul plan.
7. Deterministic. Failures use a seeded generator per vessel, so a smoke test repeats.
8. The editor, the generators and the tests build through one API (`docs/build-editor-plan.md`, E3). A smoke test that
   rebuilds the Saturn V by placing parts is the proof that a player can.
9. A component is open. Adding one (a parachute, a dish, a heat shield) means writing its def,
   built form, state, step and actions in one place; no other file lists the kinds. A component
   the physics does not know is a load error naming it, never silently ignored.
10. Each step ends by turning the smoke test's "not modelled" finding into checks with real
   figures, and by adding telemetry for it to the report (state of charge, temperature, link
   margin).

## The build core

You asked whether a player will be able to pick a heat shield, a parachute or landing legs and
attach them radially or on a surface. The vessel overhaul built the part model for that
(geometry, connectors, surface and node joints, components, assemblies). Reading the code for
this plan shows the physics does not yet stand on it. Fix this before any system, or each new
part type becomes another special case in `gather.rs`.

What is general today, and what is not:

- General: a part's geometry, its connectors, joints at a node or on a surface at (u, v) with a
  standoff, hinge and slide motions, components on any shape, assemblies, templates, paint
  layers, sizing of tanks from membrane theory, voxel aero, mass and inertia in 3D, a resource
  graph for propellant, per-part runtime state for engines, tanks and RCS.
- Closed: the physics reads a craft through `PartTree::from_craft`, which sorts each part into
  one of eleven kinds by the single functional component it has, and refuses a part with two
  ("a stage has no part like it"), or an engine, RCS block or pod that is not upright. New
  components are not refused; they are ignored (batteries are). So a heat shield, a parachute or
  an antenna has nowhere to go.
- Missing: what an actuated joint does (stowed, deploying, deployed, locked, how fast, on what
  power) and what follows it (aero shape, mass, contact); actions beyond four on a sequence;
  parts that wrap another part's surface (an ablator, insulation, solar cells, a radiator); a
  way to place, check and size a part that the editor, the generators and the tests all use.

What the core must do, as the player sees it:

- Place any part at a node, on a surface (radial, at a ring, inside a container, under a
  fairing) at any orientation, and get the right mass, centre of mass, aero and joint loads.
- Pick a procedural part, set its requirement, see its size, mass and fit, and have the
  craft file keep the requirement.
- Place n of something round the axis, or a mirrored pair, as a build-time operation that
  writes plain parts.
- Get told when a part does not fit (overlaps, too big for its node, outside the fairing, a view
  blocked, a joint that cannot carry its load or what it must pass).
- Fire a part's actions from a sequence or a key, whatever the part is: deploy, arm, cut,
  jettison, undock, light, cut off.

The steps for it are E2 and E3 of `docs/build-editor-plan.md` (physics on any part at any pose,
and the builder), which come with the editor, and steps 3 to 7 below, which come right after it.
The Saturn V's golden figures must not move through E2.

## Smoke tests for the build core

These are step E1 of `docs/build-editor-plan.md`, and grow with it: a placement matrix of every
component against every way to place it, sizer checks (each against its oracle, more requirement
gives more mass, a later tech level never more, an impossible requirement an error with its
reason), reference builds (the Saturn V, a Falcon 9 with a Dragon, the Shuttle stack, a Soyuz, a
lander, a capsule, each placed from the palette with no flat mass), and property tests with a
seeded generator. They are written first and fail first, so the order of the steps is the order
of the red cells turning green.

## The shared systems pieces

Needed by the systems after the core.

- Environment. At a vessel's position and time: the direction and strength of sunlight, eclipse
  by any body (umbra and penumbra), planet albedo and infrared, whether another point is hidden
  behind a body (for links), and later the sea and ground. Cached each tick, and evented on
  rails.
- Networks. The `power` capability on connectors already exists and nothing uses it. Power and
  data become networks over the joints that carry them, solved per tick (and in closed form on
  rails): sources, storage, loads with priorities, brownout shedding the lowest first. The fluid
  network (`Propulsion::reachable`) stays as it is and docking ports plug into all three.
- Join. `Assembly::release` and `split` exist; the other direction does not. A join takes two
  vessels and a pair of connectors, merges the part trees, re-roots, adds mass and inertia,
  conserves linear and angular momentum, merges the runtime state and sequences, and removes
  one vessel from the universe. Release then restores both. Docking, berthing and the editor all
  use it.
- Faults. Per-part health, wear (burn time against rated time), failure causes (over
  temperature, over load, over use, chance) and what a failure does. Without it every system
  works perfectly.

## The systems

Each says what the real vehicle does, what the player sets, what the sim models, and what the
smoke test then checks.

### Electrical power

Real: the CSM ran on three fuel cells (about 1.4 kW each, hydrogen and oxygen from the service
module's cryogenic tanks, water out) and three entry batteries; the LM on silver-zinc batteries
that the descent and ascent stages carried; satellites on solar arrays that track the Sun and
lose power in eclipse; deep space on RTGs.

- Parts, all procedural: battery (volume and tech give capacity and mass, primary or
  rechargeable, depth of discharge, cold derate), fuel cell (rated power gives mass and
  reactant flow, makes water), solar array (area, tracking, degradation, power from distance
  and incidence), RTG (isotope mass gives power, decaying), power converter and bus.
- Loads: avionics, transmitters, cabin, heaters, and the vessel's own actuators (engine ignition
  by its `ignition_resource`, gimbals, RCS valves, reaction wheels). Brownout switches off what
  it must: no ignition, no SAS, no link.
- Smoke: the Saturn V's power budget from launch to splashdown, with the fuel cells burning real
  hydrogen, and the CM running on batteries for entry.

### Avionics and guidance

Real: the Saturn V's instrument unit (a 6.6 m ring, about 2 t) flew the stages, and the CSM
computer flew the rest. RP-1 sizes avionics from the mass they control, in tech levels.

- A procedural avionics unit (mass and power from the controlled mass and the level; Near-Earth
  levels work only close to the Earth, Deep Space ones anywhere). Without a powered unit a
  vessel has only the limited control of a science core: no SAS, no gimbal command, no burn
  executor, no staging sequencer.
- Sensors that the attitude depends on (gyros, star trackers, sun sensors) as small procedural
  parts, with accuracy by level; start with perfect ones.
- The sequencer gains triggers a unit can arm: an acceleration limit, a propellant level, a
  time. This is where the S-IC's centre engine cutoff lives (see step 1).
- Smoke: the Saturn V cannot steer through the S-IC without its instrument unit, and the unit's
  mass comes out as the sizer's number, not a flat 15 t in a drum.

### Communications

Real: S-band through the Deep Space Network and the Honeysuckle Creek and Parkes dishes; the
signal was lost behind the Moon at lunar orbit insertion and in the plasma of entry.

- Parts: antenna (procedural: dish diameter, band, power, gain from both, mass from the
  diameter; omni, patch, steered dish with a slew rate), transponder.
- Ground stations as records on bodies (like launch sites): Goldstone, Madrid, Canberra,
  Honeysuckle, Parkes, Houston, and so on.
- A deterministic link budget (transmit power, both gains, free-space loss, a noise floor, data
  rate, margin), light delay, hidden behind a body or not (Environment), relay through another
  vessel.
- What it changes: an uncrewed vessel is controllable only with a link, or with its own avionics
  running a plan it was given; a crewed one has autonomy. Burn plans are uploaded and arrive
  late. The UI shows link margin and the next acquisition and loss, and warp can stop at them.
- Smoke: acquisition and loss of signal along Apollo 11, the loss behind the Moon, the delay
  at the Moon (1.3 s).

### Docking

Real: the CSM separated from the adapter, turned round, docked probe to drogue with the LM and
pulled it out of the S-IVB; later the LM docked with the CSM in lunar orbit.

- Join (above) for the physics, and a procedural docking port (ring diameter, type: probe and
  drogue, androgynous, berthing; mass from diameter and load; what it passes: fluid, power,
  data, crew).
- Last metres: a capture sequence (contact, soft capture on a spring, retract, hard latches)
  which needs contact between port rings, so the first real collider; alignment readout,
  closing speed, the target port, a camera per port, SAS `target port`.
- Undock with a spring impulse, as a decoupler does.
- Nested holds (issue 8): the LM sits inside the adapter, which holds it on releasable joints;
  hinged adapter panels open. A container already marks inside surfaces and nodes, and a
  decoupler can release an inside joint; the missing piece is a release that is not at the
  stack's end.
- Smoke: transposition and extraction on the way out, the LM and CSM docking in lunar orbit.

### Landing legs and the lunar module

Real: four legs with crushable honeycomb struts sized to take 3 m/s, footpads spread to the
regolith's bearing strength; a descent stage with a throttleable engine, which stays as the
ascent stage's launch pad.

- Moon terrain, baked from public lunar elevation data (`osp_bake` is Earth only; the sim takes
  height sources per body already), a surface record (bearing strength, friction), and landing
  sites as launch-site records.
- A procedural leg: strut length and stroke from the vehicle's mass, touchdown speed and the
  tallest slope it must stand on; crush core that absorbs energy once; a footpad whose area
  comes from the ground pressure; deployed and stowed (the aero shape changes). It generalises
  `GearLegDef` and `gear.rs`'s contact solver (point contacts with substeps) from a wheel to a
  foot. Tip-over comes out of the contact solver, not a rule.
- The lunar module as parts: a procedural cabin shell with crew, descent tanks, the LMDE and
  LMAE (records exist), RCS quads, legs, and staging on the ground.
- A player tool for the descent: hold a descent rate, hold a hover height, pick a touchdown
  point; the smoke test flies with it, as a player would.
- Smoke: land at under 3 m/s on a slope under 12°, stay, lift off from the descent stage,
  rendezvous and dock.

### Heat and the heat shield

Real: the CM's ablator (a few centimetres, thicker at the stagnation point) took about 11 km/s
off a lunar return; the S-IVB's tanks boiled off hydrogen between its burns; solar panels and
the cabin needed radiators and heaters.

- Thermal model: a lumped node per part (temperature, heat capacity from mass and material),
  conduction through joints, radiation (emissivity, a coarse view factor from the voxels),
  environment flux (Sun, planet infrared and albedo), internal heat from every load, and
  aerodynamic heating (convective heating from nose radius, density and speed, and radiative
  heating at lunar-return speeds). `MaterialDef` gains specific heat, conductivity, emissivity
  and a temperature limit.
- Parts, procedural: radiator (area from the heat to reject), heater, insulation by tank type
  (boil-off rate, which decides how long the S-IVB can wait), and the heat shield: an ablator
  record, a thickness from the heat load the trajectory gives, mass from area and thickness, a
  recession per tick, charring, and a failure when it is gone. It covers the outer surface that
  faces the flow (voxels) and takes its nose radius from the geometry's curvature.
- A part over its temperature fails by its material: melts, yields, ignites, or loses its
  function.
- Smoke: the S-IVB's propellant after two orbits, the CM's entry temperatures and ablator left,
  and what the same entry does to a half-thickness shield.

### Capsule entry, parachutes and the sea

Real: the CM's centre of mass sat off the axis, so it trimmed at 157° and the crew rolled it to
steer the lift (6.5 g peak); two drogues at 7 km, three 25 m mains at 3 km, landing at about
9 m/s in the Pacific.

- Pods need a real mass distribution (so their centre of mass moves with the parts inside),
  their own RCS (the record data is there), a blunt-body hypersonic pressure model that gives
  trim and lift, and a bank command that steers the lift (issue 12), with an entry autopilot
  tool for the player.
- A procedural parachute: canopy area from the vehicle mass and the descent speed it must
  reach, a type (ringsail, ribbon, cruciform) for drag and opening shock, pack mass from area
  and material; arm, deploy by altitude or dynamic pressure, reefing and disreef, cut; drag
  along the velocity, opening load that breaks a part or a chute if too fast. Drogues and mains
  are the same part with different requirements. Retro rockets and airbags use the same
  triggers.
- The sea as physics: buoyancy from the displaced volume (the geometry integration exists),
  splash loads, floating attitude and flotation bags, and recovery.
- Smoke: the CM enters at 6.5 g or less, opens its chutes, lands at 9 m/s and floats upright.

### Crew and life support

Real: three crew for eight days on oxygen, water and food, a lithium hydroxide scrubber for
carbon dioxide, a cabin at 5 psi oxygen; 6.5 g in entry.

- Crew in seats, with mass and metabolic load; consumables as resources in tanks (the pod records
  list food, oxygen and water); cabin pressure and gas from a pressure vessel sized the way a
  tank is; a procedural scrubber and a procedural life-support module sized from crew and
  days. Crew move between vessels through a docked tunnel.
- Limits that kill: acceleration (by axis and duration), pressure, oxygen, temperature. The
  report says why a crew was lost.
- Radiation is left out until the rest works.

## The palette of procedural parts

What a player picks in the editor, and what it is sized from. Tanks, fins, fairings and
interstages exist already.

| Part | Requirement | Sized from | Oracle |
| --- | --- | --- | --- |
| Battery | energy, rechargeable | volume or energy, tech energy density, cold derate | CM batteries (the pod record's 21 390 kJ) |
| Fuel cell | power, hours | power, reactant flow, tech | CSM fuel cell, about 1.4 kW |
| Solar array | power at 1 AU, tracking | area, tech watts per m², mass per m² | an ISS or Skylab array |
| RTG | power at end of life | isotope mass, decay | Voyager's RTGs |
| Avionics unit | the mass it controls, level | RP-1 mass and power formulas | the Saturn V's instrument unit, about 2 t |
| Antenna | band, gain or range, data rate | dish diameter, power, mass per area | the CSM's four 0.8 m dishes |
| Radiator | heat to reject, temperature | area, emissivity, mass per m² | a CSM radiator |
| Heat shield | peak flux and heat load, shape | ablator thickness, area, density | CM AVCOAT, a few cm |
| Parachute | mass, final speed, type | canopy area, drag, pack density | CM mains, 3 × 25 m, 9 m/s |
| Landing leg | mass, touchdown speed, slope | strut length, stroke, crush force, pad area | LM leg, 3 m/s |
| Docking port | diameter, type, load | ring size, latch count, mass | Apollo probe and drogue, 0.8 m tunnel |
| Cabin | crew, days, pressure | pressure vessel wall, scrubber, consumables | CM and LM cabins |
| Capsule shell | volume, ballistic coefficient | revolved walls, centre of mass off the axis | CM outline and trim |
| Reaction wheel, CMG | torque, momentum | rotor size, mass | a Skylab CMG |
| RCS block | thrust, impulse, propellant | thrusters by record, tank sized | the SM quad (exists as a record) |
| Launch escape system | thrust, burn time | a solid motor and a decoupler | the Apollo tower |
| Grid fin, airbrake | area, hinge rate | planform, mass | Falcon 9's grid fins |

## Steps

Milestone 0, what the smoke test already measured:

1. (S) A sequence can cut an engine, and set a throttle limit per engine or group
   (`staging::Activation`): `cutoff` and `throttle`. Issue 9. The Saturn V's centre F-1 goes at
   2 min 15 s. Done when the smoke test's S-IC peak acceleration is 4 g or under. Step 4
   generalises it.
2. (S) The burn executor counts its RCS ullage push and allows RCS-only burns. Issue 10. Done
   when a planned 22.5 m/s delivers 22.5 within 0.1, and the smoke test's trim helper can go.

Milestone 1, the build editor, and the Saturn V built in it. Its steps are E1 to E13 in
`docs/build-editor-plan.md`: the build smoke tests, physics on any part at any pose, the builder, the
editor itself, and a smoke test that builds the Saturn V in it and flies Apollo 11. Nothing below
starts before E13 passes, so that every system after it adds parts a player can place and test.

Milestone 2, the systems side of the build core. The editor takes the parts it has today;
these make parts of any kind possible, and each adds to what the editor can place:

3. (L) Open components with state. A component is a def, a built form, a runtime state and a
   step; a `Systems` store sits next to `Propulsion`, follows a part through a split, and holds
   what is not propulsion. A registry replaces the closed list of kinds, and a component the
   physics does not know is a load error that names it. Done when a battery's charge stays with
   its part through a stage separation, and the editor's property panel shows its parameters
   with no editor code for it.
4. (M) Actions and sequences. A component declares its actions (deploy, arm, cut, jettison,
   undock, cutoff, throttle); a sequence entry, a key binding and a staging icon are a part and
   an action. Done when `Activation` is not a list of four, step 1's cutoff is one action among
   them, and the editor's sequence window shows an action it has never heard of.
5. (L) Actuated joints. The `Hinge` and `Slide` motions get a state (stowed, moving, deployed,
   locked), a speed, limits, a power cost and an action to start them; the aero shape (the
   voxels rebuild), the centre of mass and the contacts follow the travel. Done when a leg or a
   panel deploys on an action and the vessel's drag and centre of mass change with it.
6. (M) Surface layers. A layer on a wall (an ablator, insulation, solar cells, a radiator panel)
   with its own thickness, material and area, drawn like paint and weighed from area, thickness
   and density. A wall can carry more than one. Done when a layer's mass is area times thickness
   times density, and it sits under a fairing without showing.
7. (M) Sizers. The `procedural` field in craft files, the sizer trait, the oracle harness, a
   palette entry for each sizer (so its requirement is a slider in the editor), and an inventory
   of every flat mass in the shipped craft. Done when the battery sizer reproduces the CM's
   21 390 kJ within 10% in mass and the editor's palette lists it with a slider.

Milestone 3, shared pieces:

8. (M) The environment: Sun, eclipse, albedo, infrared, and what is hidden behind a body. Done
    when the eclipse in the 185 km parking orbit matches the geometry (at most about 35 of 88
    min).
9. (M) Networks: power and data over joints, with priorities and brownout, in closed form on
    rails. Done when a test craft with a source, a battery and two loads sheds the low priority
    one first.

Milestone 4, power and avionics:

10. (L) Electrical power: the battery, fuel cell, solar array and RTG sizers, the loads, brownout
    effects, the fuel cell burning real reactants (the service module gains its hydrogen and
    oxygen tanks, which the craft lacks today). Done when the Saturn V smoke test reports a
    power budget with a margin at splashdown.
11. (M) Avionics: the sizer, the control it gates, the sequencer's triggers, the sensors
    (perfect at first). Done when the Saturn V cannot steer without its instrument unit and the
    unit's mass is the sizer's.

Milestone 5, communications:

12. (M) Ground stations and links: the records, the antenna sizer, the link budget, delay,
    occultation, relay. Done when the smoke test reports acquisition and loss along Apollo 11,
    including the loss behind the Moon.
13. (S) Command and data policy: uncrewed needs a link, uploaded plans arrive late, a comms
    window, warp stops at loss and acquisition.

Milestone 6, docking:

14. (L) Join: the merge, the runtime state, the sequences, momentum, and release restoring both.
    Done when a property test joins and releases any two craft without changing either.
15. (L) Docking ports: the sizer, contact between rings, capture and latching, the alignment
    readout and camera, undock; placed with the builder like any part. Done when the smoke
    test docks the CSM with something and pulls away.
16. (M) Nested holds and hinged panels (issue 8): the LM inside the adapter on releasable
    joints, the adapter panels opening on actuated joints (step 5). Done when the smoke test
    does transposition and extraction with the real LM, from the S-IVB.

Milestone 7, the lander:

17. (M) The Moon's terrain from public elevation data, a surface record, landing sites. Done when
    the Moon has relief at the Sea of Tranquility and a vessel lands on it.
18. (L) Procedural legs and foot contact: the sizer, crush, deploy (step 5), the contact solver
    and tip-over. Done when four legs go on a descent stage with one radial pattern from the
    builder, a lander touches down at 1 m/s unharmed, crushes its struts at 5 and breaks at 12.
19. (L) The lunar module from procedural parts, the descent tool, and staging on the ground
    (`mods/realism/craft/lunar_module.json5`, replacing the drum in the Saturn V). Done when the
    smoke test lands, stays, lifts off, meets the CSM and docks. Needs steps 10, 15, 16 and 18.

Milestone 8, the return:

20. (L) Thermal: the node model, material heat data, environment and aero heating, radiators,
    heaters, boil-off, limits. Done when the S-IVB's boil-off over two orbits and the CM's
    entry temperatures are in the real range.
21. (M) The heat shield: an ablator layer (step 6) sized from the heat load, recession and
    failure. Done when the Apollo entry survives and a half-thickness shield burns through.
22. (M) Capsule entry: mass distribution, pod RCS, a blunt-body pressure model, bank steering,
    the entry tool (issue 12). Done when the entry peaks at 7 g or under and lands within 50 km
    of its target.
23. (L) Parachutes and the sea: the sizer, deployment as an action, opening loads, buoyancy,
    flotation (issue 11). Done when the CM lands at about 9 m/s and floats upright.

Milestone 9, crew:

24. (M) Crew and life support: seats and loads, consumables, the cabin, the scrubber, limits that
    kill, crew transfer. Done when the smoke test reports consumables left at splashdown.

Milestone 10, accuracy:

25. (M) Failures and reliability: the faults store, `EngineDef::reliability` read in full, part
    failures from temperature and load, a seeded generator, the launch escape system.
26. (M) Structure: joint strength and breakup, crash tolerance per part, g and dynamic pressure
    limits, using the loads `structure::joint_loads` already computes.
27. (L) Rebuild the Saturn V from procedural parts only, in the editor (the recipe of E10 with no
    flat mass or hand-shaped part left; the build audit fails on one), within 3% of the
    hand-built mass and delta-v, and the Apollo 11 smoke test flies the whole mission from that
    craft, landing included, with no bridged step. This is the acceptance of everything above.
28. Then one mission at a time: Falcon 9 and Dragon (propulsive landing, grid fins, the sea,
    docking), the Shuttle (a winged entry with tiles, payload bay doors), Soyuz (docking,
    parachutes and soft-landing rockets), each as a smoke test whose gaps become the next steps.

Every step from 8 on is done when its parts are in the editor's palette with their sliders, the
Saturn V recipe places them, and the mission test checks what they do (`docs/build-editor-plan.md`,
"What each system adds after that").

The shortest road to the Moon landing is the editor (E1 to E13), then 3 to 7, 8 to 10, 14 to 16 and
17 to 19; the lander needs power to run. Steps 1 and 2 are small and make every later number
honest, so they go first. The return (20 to 23) can wait for the landing, or come before it if you
would rather see Apollo come home right first.

## Choices made

- An energy bus with priorities, not circuits. Breakers, fuses and wiring come later if wanted;
  the network is where they would go.
- Lumped thermal nodes per part, not a mesh. Part scale is what a player sees and edits; the
  nose radius and the exposed area come from the geometry.
- A deterministic link budget with no fading or noise. Stations are data.
- A docked pair is one rigid vessel. Flexing and vibration are out.
- A craft stores the requirement, not the sized result, so a data fix improves every craft.
- Parts are not a closed list. The physics reads each part's geometry, pose and components
  directly; a new system is a new component, not a new case in the loader.
- Coatings (an ablator, insulation, solar cells) are layers on a wall. Things with a shape of
  their own (a dish, a leg, a panel on a hinge, a parachute pack) are parts. The builder treats
  both the same way when it checks the fit.
- Radiation, weather, wind and slosh are out of this plan. Add them where a smoke test shows
  they matter.

## Tracker

Each step is a task in `.tasks/`, with its dependencies (which are what the steps really need, not
only the order above). Steps 1 and 2 are T026 and T027; 3 to 7 are T028 to T032; 8 and 9 are T033
and T034; 10 and 11 are T035 and T036; 12 and 13 are T037 and T038; 14 to 16 are T039 to T041; 17 to
19 are T042 to T044; 20 to 23 are T045 to T048; 24 is T049; 25 and 26 are T050 and T051; 27 is T052;
the three missions of step 28 are T053 to T055. The palette parts without a step of their own are
T056 to T059. The editor's steps are T013 to T025 (`docs/build-editor-plan.md`). Status lives in
`.tasks/`, not here.

