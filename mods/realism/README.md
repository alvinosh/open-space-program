# Realism: RO/RP-1 parts and history

Real engines, capsules, tanks, batteries and the RP-1 tech tree, imported from the Realism
Overhaul family of KSP mods, plus historical rockets and their launches. The records are
generated; edit the importers, not the files:

```sh
python3 scripts/realism/import_realism.py                 # records + model job lists
python3 tools/blender/convert_mu_batch.py scripts/realism/out/model_jobs.json \
    scripts/realism/out/model_results.json --skip-existing  # .mu -> .glb (engines, capsules)
python3 scripts/realism/import_realism.py                 # again: sizes measured from the models
cargo build -p osp_sim --example craft_report --release
python3 scripts/realism/historical_craft.py               # craft, launch sites, scenarios
```

`scripts/realism/out/library_jobs.json` converts every other `.mu` in the snapshot (tanks,
fairings, solar panels, structure) into `models/library/` the same way, at their own origin
and scale with textures up to 1024 px.

Models (glTF binary, metres, Y up, textures embedded as PNG): `models/engines/` (221, origin
at the top node, nozzle down), `models/pods/` (33, origin at the base centre) and
`models/library/<mod>/<path>.glb` (931). The 50 `.mu` files that did not convert are effect
helpers with no meshes.

The sources are the repositories in `scripts/raw_space_data/repositories` (KSP-RO's
RealismOverhaul, ROEngines, ROCapsules, ROTanks, ROSolar, RealFuels, RP-1, TestFlight;
Bluedog_DB; TundraExploration; Community Resource Pack). `scripts/realism/out/report.json`
lists what was imported and what was skipped and why.

## Collections

Every record has the game's physics fields first, then catalogue fields the game loads into
`CatalogInfo` (`title`, `manufacturer`, `description`, `year`, `tech`, `used_on`,
`source`) and extra source data it ignores for now.

| Collection | Records | Source |
| --- | --- | --- |
| `engines` | One per RO engine config and subconfig (AJ10-37, F-1-1.52M, RD-107-8D74PS, ...), grouped by family file | RO `Engine_Configs`, dated by RP-1 |
| `engine_families` | Each family's configs, parts and model | RO, ROEngines |
| `air_breathing_engines` | Jet, ramjet and propeller configs with their AJE cycle parameters | RO |
| `pods` | Capsules and cockpits: mass with batteries, consumables and crew; stored electricity; RCS | ROCapsules, RO, Bluedog_DB, Tundra |
| `resources` | Every propellant and fluid the imported parts use (densities as stored) | Community Resource Pack, RealFuels, RO |
| `tank_types` | RealFuels construction types by era (steel fuselage to stir-welded stainless) and `Solid Grain` | RO `RO_RealFuels.cfg`, RP-1 `ResourceTechs.cfg` |
| `materials` | Alloys the era tank types are built from | history |
| `batteries` | RealFuels Battery-I to IV (silver-zinc) and later chemistries | RO, RP-1, history |
| `avionics` | RP-1 procedural avionics tech levels (mass and power per controlled tonne) | RP-1 |
| `solar_panels` | ROSolar cell tech levels (W/m², kg/m², degradation) | ROSolar |
| `tech_nodes` | The RP-1 tech tree: 330 nodes with parents, layout and the year each was reached | RP-1 `RP0TechTree.cfg`, `TreeYears.csv` |
| `craft`, `scenarios`, `launch_sites` | Historical rockets from the V-2 to Starship, launched at their real pads and times | `historical_craft.py` |

### Engines

```json5
{
  id: "AJ10-37",            // the RO config name; craft list engines by it
  mass: 84.0,               // kg (RO origMass × massMult)
  thrust_vac: 33800.0,      // N
  isp_vac: 271.0, isp_sl: 122.0,
  min_throttle: 1.0,        // minThrust / maxThrust
  ignitions: 1,             // null: unlimited
  requires_ullage: true,
  gimbal_range: 4.25,       // degrees
  propellants: { UDMH: 0.263, IWFNA: 0.737 },  // mass ratios (RealFuels gives volumes)
  length: 1.9, exit_diameter: 0.48,            // m, measured from the model when there is one
  model: "models/engines/ROE-AJ10.glb",        // origin at the top node, nozzle down
  family: "AJ10_Early", kind: "liquid",        // liquid, solid or electric
  pressure_fed: true,
  pressurant: { Helium: 1.0 },                 // drawn but not burned
  ignition_resource: { ElectricCharge: 0.2 },
  electric_power: 0.0,                         // kW at full thrust (electric thrusters)
  reliability: { ignition_start: 0.825, ignition_end: 0.965, cycle_start: 0.75,
                 cycle_end: 0.917, rated_burn_time: 115, tested_burn_time: 150 },
  thrust_curve: [[fraction, multiplier], ...], // solids
  year: 1956, tech: "orbitalRocketry1956", dated_by: "rp1_parts_browser",
  used_on: "Vanguard", source: "KSP-RO/RealismOverhaul/.../AJ10_Early_Config.cfg",
}
```

Solid motors' `mass` is the whole inert motor (case, insulation, nozzle), as in RO, so their
grain goes in a `Solid Grain` tank, which adds next to nothing.

## Dates

A record is available from the start of its `year`, or, without one, from its tech node's
(`osp_sim::tech`). `dated_by` says where the year came from:

- `rp1_parts_browser`: RP-1's Parts Browser data, the historical year of that config or part.
- `rp1_tree`: the year of the RP-1 tech node that unlocks it.
- `family`: the earliest dated config of the same engine.
- `history`: set by hand where RP-1 has no date, or where its date is a gameplay one. RP-1
  clamps everything before its 1951 career start to 1950 and places some foreign engines
  at their American equivalent's year (Viking 2 in 1958, not 1979). Those keep RP-1's year as
  `rp1_year`.

When a scenario starts, the game warns (Mods window, F2) about parts its craft use before
they existed. `cargo run -p osp_sim --example craft_report --release -- <craft or scenario>`
prints a craft's stages and the same check.

## Historical craft

The figures are rounded public numbers for each stage: propellant, dry mass, diameter and
length. Tanks are sized to hold the propellant, and `structure_mass` is calibrated so each
stage's dry mass matches. The game stacks stages, so parallel staging is approximated: R-7
strap-ons, Atlas's booster package and strap-on solids are a bottom stage carrying the
boosters' propellant plus what the core burns meanwhile, with a copy of the core engine that
drops with them. Each craft's comment says how it is built.

## Licences

The data and models keep their mods' licences (from the CKAN metadata in the snapshot):

| Source | Licence |
| --- | --- |
| RealismOverhaul, ROCapsules, ROTanks, RealFuels | CC-BY-SA |
| ROSolar | CC-BY-SA 4.0 |
| ROEngines (most engine models) | CC-BY-NC-ND: no commercial use, no changes beyond format conversion |
| RP-1, TestFlight, Bluedog_DB, Community Resource Pack | CC-BY-NC-SA 4.0 |
| Tundra Exploration | configs CC-BY-NC-SA 4.0; models and textures All Rights Reserved |

Tundra Exploration's models are used with its author's permission, recorded in
`scripts/realism/sources.py` (`PERMITTED_ROOTS`); other restricted sources are left out of
the conversion unless permitted there. Most of the rest is non-commercial: check before
distributing anything built from this mod.
