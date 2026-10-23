# Kitten Space Agency parity plan

Target: everything Kitten Space Agency (KSA) has shipped in its public builds (163 changelogs,
August 2025 to October 2026), then the features it has confirmed but not built yet. Sources are
the community wiki's [build pages](https://kittenspaceagency.wiki.gg/wiki/Version_history) and
[planned features](https://kittenspaceagency.wiki.gg/wiki/Planned_features) page; KSA has no
official roadmap, so the list is a snapshot of what it does, not of what it promises.

Phases are ordered by what depends on what, not by what is most fun. Sizes are relative:
S (days), M (a week or two), L (several weeks), XL (months). Tick items off as they land.

## Already at or past KSA

Keep these; parity work must not regress them.

- Exact patched conics in universal variables, with SOI changes, atmosphere entry and terrain
  approach solved to the tick, identical at any warp (KSA: flight plans re-solved at encounters).
- Integer time (µs since J2000, ±292 000 years), hierarchical frames, floating origin at the
  vessel, Encke integration for powered flight.
- IAU rotation models for every body; real launch-site coordinates.
- Exact torque-free rotation for any rigid body (Euler–Poinsot, closed form at any warp);
  SAS that holds exactly on rails.
- RO-style engines (pressure-dependent Isp, ignitions, ullage, throttle floors, spool-up
  curves, throttle slew and tail-off), RCS couples, engine gimbal.
- Procedural tanks with real wall thickness, burst and buckling, and fins that tear off.
- Earth from real data at higher resolution than KSA's (30 m elevation and 10 m land cover
  around the Cape), single-scattering atmosphere, trees by land-cover class.
- Data-driven UI and keybindings, mods with layered records and patches, live reload (F5).

## Phase 1: flight planning (L)

Everything here runs on today's vessel model, is testable headless in `osp_sim`, and is what
makes the map view useful. It also gives later phases their test harness (burns to fly).

- [x] Targets: any body or vessel; distance, relative velocity, relative inclination.
- [x] Orbit markers: Ap/Pe on every patch, AN/DN against the target, closest approach with
      distance and time, hover readouts.
- [ ] Same-time marker on the target's orbit; SOI bubbles at closest approach.
- [x] Closest-approach solver between two trajectories: range rate sampled finer than the
      shortest period, each minimum refined to the microsecond.
- [ ] MOID filter so small moons are not skipped on interplanetary legs.
- [x] Burns (maneuver nodes): time plus prograde/normal/radial Δv, prediction through every
      planned burn, drag handles, fine-tune steps, delete; click the trajectory to add one;
      circularise at Ap/Pe.
- [ ] Orbit right-click menu (warp here, burn here, match target plane).
- [x] Burn executor (KSA's burn gauge: AUTO, WARP, DELETE): point at the burn, wait for
      alignment, settle propellant with RCS, start at T − half the burn, throttle down near
      the end, respect minimum throttle, stop when dry, then hold. (G-limits still to do.)
- [ ] Flight plan window: every patch (start, end, transition, elements), encounters, burns.
- [x] Warp to Ap, Pe, SOI change and burn (with margin); smooth automatic warp rate.
- [ ] Warp to any point on the orbit; warp levels up to 100 years per second (engines cannot
      start above 30×).
- [x] SAS and navball: burn, target and anti-target modes and markers; target frame
      (relative velocity as prograde/retrograde, so retrograde kills relative velocity).
- [ ] Navball frames STAR and OVEL; roll target (normal, inverted, any); autopilot profiles
      (strict, balanced, relaxed); RCS direct and pulse modes.
- [x] Danger arcs: trajectory drawn red inside the atmosphere or below the highest terrain,
      surface-fixed impact marker (vacuum impact; drag not included).
- [x] Transfer planner: Lambert transfers, porkchop of Δv against departure and flight time,
      one synodic period (or half the target's period across planes, with a per-orbit
      departure search), impact transfers left out, Hohmann-based flight times, create
      burn; interplanetary ejection burns; aim for a periapsis at the target.
- [ ] Transfer planner extras: preview of the picked transfer, time-of-flight sliders,
      transfers starting from solar orbit, out-of-plane ejection steering.
- [ ] Rendezvous panel: relative velocity, distance, closest approach, target autopilot modes.
- [ ] Engine control readouts: Δv, burn time, surface TWR at ambient pressure.

## Phase 2: vehicles made of parts (XL)

The keystone. A vessel is a stack of stages today; KSA's editor, docking, decouplers, debris,
radial boosters and part damage all need a tree of parts.

- [x] Part tree with connectors (fixed and surface attach, typed capabilities: fluid, power,
      solid motor case, decoupler joint); vehicle splits into new vehicles when a joint opens.
      (Craft files still list stages: each stage becomes a hull with its parts mounted on
      it, hanging from a decoupler under the stage above, and staging releases joints on
      the tree. The solid motor case capability waits for solid motors.)
- [x] Per-part mass properties with full inertia tensors (analytic shapes: cylinder, sphere,
      dome, cone, frustum, cuboid, ogive); assembly frame vs body frame; principal axes.
      (Shapes so far: point, rod, plate, solid cylinder, tube, half tube, solid and hollow
      sphere, frustum and cone; domes, boxes and ogives wait for parts shaped like them.
      The centre of mass is still kept on the axis: a part to one side counts towards its
      height and the inertia, not its sideways offset.)
- [x] Torque-free rotation for asymmetric bodies (Euler-Poinsot), alongside the symmetric top.
      (One closed form for both: Jacobi elliptic rates and the turn about the angular
      momentum from the integral of the third kind. Under torque, the full tensor.)
- [ ] Port procedural tanks, fins, engines, RCS, wheels and pods to parts; existing craft
      files become part trees. (Started: they are parts on the tree, but the physics still
      runs on the stages gathered from it, and craft files cannot describe a tree yet.)
- [ ] Propellant graph per engine with flow rules (KSA has four), drain order, crossfeed.
- [ ] Decouplers (stack and radial), interstages and fairings, size adapters. (Done for
      stacks: a decoupler at every stage joint and jettisonable fairings, as activations;
      radial decouplers and size adapters wait for the part tree.)
- [x] Activation sequences separate from resource groups; drag and drop in flight
      (activations per stage, not per part, until the part tree).
- [x] Debris vessels from jettisoned parts (stages and fairing halves; crashed debris is
      removed, and rails warp leaves behind debris that needs physics).
- [x] Vessel switching (`]`/`[`), several vessels per scenario, Universe Manifest window
      (all objects, target, fly).
- [x] Manifest filters, follow, recover, destroy, clear debris (follow: the camera looks at
      a vessel, or a body in map view, and distances are from it; no hierarchy dropdowns
      or column choice yet).
- [ ] Control-module requirement and "control from here".
- [ ] Part highlighting and part action windows in flight (pinnable).
- [ ] Multi-vessel physics: nearby vessels share a solver; separate islands in parallel.

## Phase 3: editor and saves (L)

- [ ] Vehicle editor: parts catalogue with thumbnails, tags and size filter; click to place,
      symmetry (mirror and radial n), 90°/15°/5° rotation, duplicate, delete, reroot, greybox;
      orthographic and free cameras; connect highlighting; tooltips.
- [ ] Per-sequence Δv and TWR (vacuum and atmosphere), from a fuel-flow time simulation.
- [ ] Tank fill and refill, uniform part scaling 0.5–2× with physical rescaling.
- [ ] Vehicle files saved from the editor; launch from file.
- [ ] Game save and load (vessels, time, plans, crew, terrain changes).
- [ ] Start menu: system, vehicle, location, game type (sandbox, testing); spawn landed or in
      orbit anywhere.

## Phase 4: contact physics (L)

- [ ] Colliders per part (box, capsule, cylinder, sphere; animated, e.g. legs), compound
      centre of mass; rigid-body contact with friction and rolling resistance.
- [ ] Vehicle-terrain and vehicle-vehicle collision; landing legs.
- [ ] Docking ports: alignment readout, target port, per-port camera viewport, latching by
      contact and kinetic energy, undock, off-centre ports.
- [ ] Crash tolerance per part, part destruction, debris shedding, g and dynamic-pressure
      breakup with warnings; dents; explosions with shrapnel and shockwave.
- [ ] Oceans as physics: buoyancy, splashdown, drag, floating and seabed states; aerostats.

## Phase 5: propulsion and systems breadth (L)

- [ ] Thermodynamic engine model (combustor, De Laval nozzle, flow separation) behind the
      current data-driven engines; Engine Designer tool.
- [ ] Solid motors: stackable segments with grain geometry that shapes thrust; separation
      motors; APCP and double-base propellants.
- [ ] Substances with phases (gas, liquid, solid); tanks in moles; methalox and monoprop sets.
- [ ] Electricity: batteries, solar panels with sun tracking, generators, consumers, circuits.
- [ ] Fuel ports, player-defined fuel lines and hoses, pumping that costs power, tank lockout.
- [ ] Parachutes: drogues and mains, arm/deploy/disreef/cut, reefing, max g, radial chutes.
- [ ] Vessel lights (switches, U toggles all); launch escape tower.

## Phase 6: kittens (XL)

- [ ] Crew roster (stats, memorial), seats, fill seats, launch without crew warning, recovery.
- [ ] EVA: doors, jetpack (MMU), chase camera, walk/run/jump, low-g gaits, swimming, ladders,
      grab, board, let go.
- [ ] IVA camera and seat swap; kitten portraits.
- [ ] Kitten models, fur, expressions, animations; moddable kittens.

## Phase 7: worlds (L, then XL for interstellar)

- [ ] KSA's 25-body Solar System: add Phobos, Deimos; Io, Europa, Ganymede, Callisto and six
      small Jovian moons; Saturn (volumetric rings with Keplerian rock meshes, E and G rings)
      with Pan, Mimas, Enceladus, Tethys, Dione, Rhea, Titan, Iapetus; Uranus and five moons;
      Neptune and Triton; Pluto and Charon. JPL Horizons elements, equatorial or ecliptic.
- [ ] Dense variant (KSA: 3 186 bodies): Ceres, Vesta, Pallas, Hygiea, Eros, dwarf planets,
      comets ('Oumuamua, Halley, Hale-Bopp, Borisov, 3I/ATLAS) and thousands of asteroids.
- [ ] System presets at start: full, dense, Earth and Moon, Earth only.
- [ ] Airless-body terrain: procedural eroded craters, real Moon and Mars elevation.
- [ ] Biomes on every body; ground clutter (rocks with collision, grass) that persists.
- [ ] KSA's five pads: LC-39A (have), Vandenberg LC-4, Starbase OLM A, Kourou ELA-4,
      Māhia LC-1A.
- [ ] Interstellar: multi-star systems (barycentric pairs), star crossings in flight plans,
      Alpha Centauri, Barnard's Star, Tau Ceti; 128-bit nanosecond time; interstellar warp;
      ramjet, scoop, magnetic trap tank, fusion and thermal drives; interstellar planner.
- [ ] Space weather (announced 2026-10-20): stellar wind, cosmic rays, trapped belts, tilted
      offset dipole magnetospheres, radiation gauge in mSv/h, aurora, magnetosphere overlay.

## Phase 8: rendering (XL)

- [ ] Volumetric clouds with shadows and god rays; 2D cloud layers with normal maps.
- [ ] Physics-driven plumes: expansion with pressure, shock diamonds, refraction, crossflow
      bending, merging, persistent trails with shadows. (Done: ray-marched plume per engine
      from its thrust level and exit/ambient pressure, shock diamonds, ground splash, plume
      light, bloom, bending from the gimballed nozzle onto a smoothed axis that trails the
      airflow. Still to do: refraction, merging, trails, pad smoke.)
- [ ] FFT ocean (JONSWAP, foam, refraction, underwater).
- [ ] GPU particles (launch debris), engine heat glow, thin-film colour on hot metal.
      (Done: heat-tint oxide films on nozzles that shift colour with the viewing angle, and
      procedural paint, composite, metal and char finishes on every vessel surface. Still
      to do: glow while firing, particles.)
- [ ] Distant bodies and vessels as sprites, glints, Milky Way, Sun surface, Hapke lighting.
- [ ] Atmosphere reflections on vessels, point-light shadows, ambient occlusion, better AA.
      (Done: sky light, a per-frame cubemap of the sky and ground at the vessel that lights
      and is reflected by every PBR surface near it; the terrain gets its own skylight. TAA
      and screen-space ambient occlusion on High and Ultra. Still to do: reflections of
      nearby objects (Bevy's SSR needs deferred rendering, which the terrain, haze and sky
      don't use yet), point-light shadows.)
- [ ] Secondary viewports (docking camera, picture in picture).
- [ ] Ray-traced IVA (Bevy Solari is the likely route).

## Phase 9: audio (M)

- [ ] Engine sounds driven by throttle and distance, clusters; wind; layered sounds.
- [ ] Biome ambience; IVA muffling; capsule rattle; channels and DSP in data.
- [ ] Music player with playlists, faders and music per SOI.

## Phase 10: interface (M)

- [ ] Esc menu, settings (graphics, audio, input, UI and HUD scale), alerts.
- [ ] Gamepad and mouse-button bindings.
- [ ] Localisation.
- [ ] Ground track window (next three orbits, terminator, subsolar point, grid); Target
      Tracking Window (LVLH along-track/altitude and along-track/cross-track plots, closest
      approach before and after burns, add and drag burns, sunrise/sunset); celestial info
      (gravity, dynamic pressure); map grid; camera modes (orbit, parent, stars, chase,
      surface).
- [ ] Console with commands (KSA has about 70: target, warp to next, save vehicle, refill,
      celestial info, debug views) and on-boot command lists.
- [ ] In-game HUD editor and saved HUD layouts; windows that detach into their own OS window.
- [ ] Screenshots key; version number and update check.

## Phase 11: modding (M)

- [ ] Code mods (KSA: C# injection with Harmony). Here: a scripting layer; decide WASM or an
      embedded language.
- [ ] Shader overrides from mods; part creation tooling (glTF to record helper).

## After parity: KSA's confirmed but unbuilt features

Multiplayer; science and career; life support; recolouring parts; ground anchoring;
terramechanics; interactive IVA instruments; trajectories plotted per reference frame;
colonies (semi-confirmed). KSA has ruled out n-body physics and caves.

## Decisions to make

- Rigid-body contact for Phase 4: Avian or Rapier, or our own solver on top of the current
  integrator (needs to keep the exact-on-rails guarantees).
- Real Solar System only, or also a fictional system like KSA's planned one.
- Scripting runtime for Phase 11.
