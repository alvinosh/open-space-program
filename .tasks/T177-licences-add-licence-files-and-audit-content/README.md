# T177: Licences: add licence files and audit what ships

I am not a lawyer; a written answer from each author beats this reading.

## Scope

- There is no LICENSE file at the repo root, though `Cargo.toml` says `MIT OR Apache-2.0`. Add `LICENSE-MIT` and `LICENSE-APACHE`.
- `mods/realism` comes from Realism Overhaul family sources: CC-BY-NC-ND (ROEngines), CC-BY-NC-SA (RP-1, TestFlight, Bluedog_DB, Community Resource Pack), CC-BY-SA (RO, ROCapsules, ROTanks, RealFuels) and Tundra Exploration's art, used with the author's permission. The 398 tracked records ship in every release (`release.yml` copies `mods`). That is fine in spirit for a free game if the licence texts and credits travel with them: add them to the mod. The converted models are gitignored and do not ship; keep it that way (ND).
- Managed servers (T176) are a product. They should run `core` only, or content with written commercial permission. Ask Tundra's author whether the permission covers that, and the RP-1 maintainers about donations next to NC content.
- Check what in `assets/core` came from a commercial game's files: `scripts/convert_ksp_engine_configs.py` reads KSP configs. Real engine figures are facts; another game's part names, configs and models are not ours.
- Terrain data (NASA, NOAA, ESA, Copernicus) is free with attribution; confirm the Copernicus DEM terms before any paid use. Barlow is OFL.

## Done when

- The licence files and the mod's credits are in, and each item above is answered or has a note on who to ask.

## Where it comes from

- Planning of 2026-11-01.
