# Baked terrain

Bodies name a dataset directory here with `terrain: "terrain/<name>"` (see
`bodies/solar_system.json5`). The data is generated, not checked in. Bake Earth with:

```sh
cargo run -p osp_bake --release
```

The first run downloads about 900 MB of public datasets into `.bake-cache/` (kept for later
runs); baking takes under a minute and writes about 190 MB to `terrain/earth/`. Without the
data the game still runs, with Earth as a smooth sphere (a warning shows in the Mods window,
F2).

## What is in a dataset

Four layers on a cube-sphere quadtree (`crates/osp_terrain`), each a pack of 129×129
tiles: elevation (bathymetry included), colour, land cover with a water fraction, and night
lights, plus `terrain.meta` (JSON). Globally they go to level 5 (~2.4 km per texel); around
Cape Canaveral elevation goes to level 12 (~19 m) and land cover to level 13 (~10 m).

At runtime the planet shader draws the imagery from afar, switches to land-cover colours
where those are much more detailed (around the Cape), and adds procedural detail near the
camera. Trees are placed from the land cover. Mods can ship their own dataset under the same
path to replace it, or point a body at another directory.

## Sources and credits

| Layer | Dataset | Licence |
| --- | --- | --- |
| Colour | NASA Blue Marble Next Generation, July 2004 (Reto Stöckli, NASA Earth Observatory / GSFC) | NASA imagery, free with credit |
| Night lights | NASA Black Marble 2016 (Joshua Stevens, NASA Earth Observatory; Suomi NPP VIIRS data from Miguel Román, NASA GSFC) | NASA imagery, free with credit |
| Elevation | NOAA NCEI ETOPO 2022, 60 arc-second surface (doi:10.25921/fd45-gt74) | Public domain |
| Land cover | ESA Climate Change Initiative Land Cover, 2015, 300 m (© ESA 2017 - UCLouvain) | Free with acknowledgement |
| Cape elevation | Copernicus DEM GLO-30 | Free with attribution (below) |
| Cape land cover | ESA WorldCover 10 m 2021 v200 | CC BY 4.0 |

Required attributions:

- © ESA WorldCover project 2021 / Contains modified Copernicus Sentinel data (2021)
  processed by ESA WorldCover consortium.
- Produced using Copernicus WorldDEM-30 © DLR e.V. 2010-2014 and © Airbus Defence and
  Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights
  reserved.
- ESA Climate Change Initiative and the Land Cover CCI project.
