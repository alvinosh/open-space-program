# T045: Thermal model

A lumped thermal node per part, with materials that have heat properties, environment and aerodynamic heating, radiators, heaters and boil-off.

## Scope

- Temperature, heat capacity from mass and material; conduction through joints; radiation (emissivity, a coarse view factor from the voxels); Sun, planet infrared and albedo; internal heat from every load.
- Aerodynamic heating: convective from nose radius, density and speed; radiative at lunar-return speeds.
- `MaterialDef` gains specific heat, conductivity, emissivity and a temperature limit.
- Procedural radiator (area from heat to reject), heater, insulation by tank type (boil-off rate, which decides how long the S-IVB can wait).
- A part over its temperature fails by its material.

## Done when

- The S-IVB's boil-off over two orbits and the CM's entry temperatures are in the real range.

## Where it comes from

- docs/systems-plan.md (step 20, Heat and the heat shield)
- old issue list, issue 13 (docs/issues.md, removed; see git history)
