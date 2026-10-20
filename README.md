# Phymi

A realism-focused spaceflight game in Rust + [Bevy](https://bevyengine.org) 0.20, in the spirit of
KSP with Realism Overhaul / Real Solar System: real-scale planets, real engine performance,
limited ignitions and ullage. Built to be modded.

```sh
cargo run -p phymi_bake --release           # once: download and bake Earth's terrain (see below)
cargo run                                   # launch from the Cape
PHYMI_SCENARIO=leo cargo run                # start in low Earth orbit
cargo run --features dev                    # faster incremental builds (Bevy dynamic linking)
cargo test --workspace                      # physics, mod loader and UI tests (no GPU needed)
PHYMI_CAPTURE=/tmp/shots cargo run          # scripted run that saves screenshots, then exits
```

For debugging the view: `PHYMI_CAMERA=<yaw°>,<pitch°>,<distance m>` sets the starting
camera, `PHYMI_START_TIME=<s since J2000>` the time of day, and `PHYMI_FPS=1` logs frame
times. With `PHYMI_CAPTURE`, `PHYMI_CAPTURE_QUICK=1` takes a single still, and
`PHYMI_CAPTURE_AT=20,30` launches and takes stills at those times.

The toolchain is pinned in `rust-toolchain.toml` (Bevy 0.20 needs Rust 1.97.1+).

## Controls

Default bindings (all rebindable in `assets/core/keybindings/default.json5`):

| Key | Action |
| --- | --- |
| Shift / Ctrl | Throttle up / down |
| Z / X | Full throttle / cut |
| W S / A D / Q E | Pitch / yaw / roll |
| Space | Stage (first press arms the booster) |
| `,` `.` `/` | Time warp down / up / stop |
| M | Map view with predicted trajectory |
| Right-drag, wheel | Orbit / zoom camera |
| F1 / F2 / F3 | Help / mods & diagnostics / flight HUD |
| F5 | Reload mods and UI |
| F12 | Data inspector (every value the UI can bind to) |

## Layout

```
crates/
  phymi_sim/     Simulation core: f64, SI units, no Bevy. Tested headless.
  phymi_mods/    Mod loader: manifests, load order, layered records, patches. No Bevy.
  phymi_terrain/ Planet surface data: cube-sphere tiles, land-cover classes, tile packs. No Bevy.
  phymi_bake/    Tool that downloads public Earth datasets and bakes them into tiles.
  phymi_ui/      Data-driven UI on bevy_ui: layouts, themes, bindings, actions.
  phymi_game/    The Bevy app wiring it all together (rendering, camera, input, publishing).
assets/core/     The base game, itself a mod: bodies, parts, craft, launch sites, scenarios, UI.
mods/           Installed mods (example_mod shows every mechanism).
docs/modding.md Modding guide and reference.
```

## Earth

Earth is built from real data: NASA Blue Marble imagery, NOAA ETOPO 2022 elevation and
bathymetry, ESA land cover and NASA Black Marble night lights, with 30 m elevation and 10 m
land cover around Cape Canaveral. `cargo run -p phymi_bake --release` downloads the sources
(about 900 MB, cached in `.bake-cache/`) and bakes them into `assets/core/terrain/earth/`
(about 190 MB, not checked in); without it Earth is a smooth sphere.

- **Terrain** is a quadtree cube-sphere whose chunks match the baked tiles. Chunks are built
  on worker threads from the same height function the simulation collides with, so you land
  on the ground you see.
- **Surface shading** uses the imagery from afar. Where the land cover is far more detailed
  (around the Cape) it colours the ground by class: forest, marsh, scrub, beach, lagoon.
  Near the camera it adds procedural detail, ripples on water, sun glint and night-side
  city lights.
- **Atmosphere**: single-scattering sky (Rayleigh, Mie, ozone) seen from the ground and from
  orbit, aerial perspective on the terrain, and sunlight reddened and shadowed by the air
  and the planet. Exposure adapts between day and night.
- **Vegetation**: trees and bushes placed from the land cover: pines, broadleaf trees,
  palms, mangroves and scrub, depending on class and latitude.
- **Launch sites** (`launch_sites` records) reshape the terrain with a concrete pad on an
  embankment and roads. Each pad gets a launch mount, a lattice service tower with an
  access arm, floodlights, a service building, a water tower and propellant tanks. The
  stock site sits at LC-39A on Cape Canaveral.

Credits and licences for the datasets are in `assets/core/terrain/README.md`.

## Physics foundation

Designed to be correct at every scale, from a rocket on the pad to Jupiter, and to give
the same result at any time warp.

- **Integer time.** `SimTime` counts microseconds since J2000, so there is no drift, and
  differences between times are exact for centuries.
- **Hierarchical frames.** Every state is relative to its parent body (patched conics), in
  inertial J2000-ecliptic axes. Frame conversions go through the lowest common ancestor, so
  a state near the Moon is never routed through 1.5×10¹¹ m heliocentric coordinates.
  Rendering uses the same idea: a floating origin at the active vessel, and f64 offsets for
  everything else.
- **Exact coasting.** Gravity-only motion is an analytic conic in universal variables: one
  formulation for elliptic, parabolic, hyperbolic and radial orbits, with a bracketed
  solver that always converges. A coasting vessel is evaluated exactly at any time, so 1x and
  100 000x warp follow the same trajectory.
- **Events solved to the tick.** SOI exit, atmosphere entry and terrain approach are found
  analytically (grazing passes included). SOI entry into moving moons uses a conservative-
  advancement search that cannot step over an encounter. Results don't depend on step size,
  so warp can't skip a Moon encounter, and the map-view prediction uses the same code.
- **Encke integration** for powered and atmospheric flight: only the deviation from a
  reference conic is integrated, so precision doesn't degrade far from the frame origin.
- **Fixed physics ticks** (20 ms) with render-time extrapolation, so results don't depend
  on frame rate.
- **Real orientations.** IAU pole and prime-meridian models for every body, so launch
  sites are at their true latitude and longitude.
- **RO-style engines.** Pressure-dependent Isp, limited ignitions, ullage, throttle limits.

The test suite checks these properties directly: bit-identical results between physics
ticks and rails, Moon transfers identical across warp rates, a century of warp without orbit
drift, exact atmosphere-entry ticks, near-parabolic and radial Kepler cases, and frame
round-trips to well under a millimetre.

## Modding

Everything in `assets/core` (planets, parts, craft, scenarios, UI layouts, theme,
keybindings) is a record that mods can add, replace or patch field by field, with
ModuleManager-style targeting, filters and passes. The UI is declarative JSON5 with live data
bindings, so mods can add windows, buttons and HUD lines, or restyle the game, without code.
Problems are reported in game (F2) instead of crashing. See [docs/modding.md](docs/modding.md).

## Roadmap

- Scripting layer for mods (actions, part modules, autopilots) on top of the record and action system
- Debris vessels from jettisoned stages; vessel switching
- Torque / moment-of-inertia attitude, engine gimbal, SAS
- Maneuver nodes on the patched-conic predictor
- Terrain data for the Moon and planets; more high-resolution regions on Earth
- Clouds, multiple scattering, aero heating
- Part-based craft editor; save/load
