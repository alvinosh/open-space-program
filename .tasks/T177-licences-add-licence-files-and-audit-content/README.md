# T177: Licences: add licence files and audit what ships

I am not a lawyer; a written answer from each author beats this reading.

## Scope

- There is no LICENSE file at the repo root, though `Cargo.toml` says `MIT OR Apache-2.0`. Add `LICENSE-MIT` and `LICENSE-APACHE`.
- `mods/realism` comes from Realism Overhaul family sources: CC-BY-NC-ND (ROEngines), CC-BY-NC-SA (RP-1, TestFlight, Bluedog_DB, Community Resource Pack), CC-BY-SA (RO, ROCapsules, ROTanks, RealFuels) and Tundra Exploration's art (All Rights Reserved; there is no permission from the author, see Open). The 398 tracked records ship in every release (`release.yml` copies `mods`). That is fine in spirit for a free game if the licence texts and credits travel with them: add them to the mod. The converted models are gitignored and do not ship; keep it that way (ND).
- Managed servers (T176) are a product. They should run `core` only, or content with written commercial permission. Ask the RP-1 maintainers about donations next to NC content.
- Check what in `assets/core` came from a commercial game's files: `scripts/convert_ksp_engine_configs.py` reads KSP configs. Real engine figures are facts; another game's part names, configs and models are not ours.
- Terrain data (NASA, NOAA, ESA, Copernicus) is free with attribution; confirm the Copernicus DEM terms before any paid use. Barlow is OFL.

## Done when

- The licence files and the mod's credits are in, and each item above is answered or has a note on who to ask.

## Done

- `LICENSE-MIT` and `LICENSE-APACHE` at the root (the Apache text is the standard one; the MIT copyright line reads "Open Space Program contributors"). The root README has a Licence section that lists what is not ours.
- `mods/realism/LICENSES.md`: which record comes from which source, the licence, what that means (non-commercial, share-alike, credit, no paid servers), and the models rule. `mods/realism/licenses/` has the source texts (RP-1, ROEngines, ROTanks, the CC BY-NC-SA 4.0 legal code from the Community Resource Pack, Tundra's licence section). BY-SA and BY-NC-ND are linked.
- Barlow: `assets/core/fonts/OFL.txt` (the OFL asks for the text to travel with the font; before, only the website had it) and a `.gitignore` exception for it.
- Copernicus DEM: read the licence (WorldDEM-30, Articles 4 to 6). Free of charge, reproduction, distribution and adaptation allowed with no limit on purpose, so paid use is fine. It requires the notice (already there) and a liability sentence that was missing; added to `assets/core/terrain/README.md` with the no-endorsement rule. The baked tiles in a release carry these terms.
- Rust dependencies: all 482 external crates in the macOS build are permissive (MIT, Apache, BSD, Zlib, ISC, Unicode); no GPL, LGPL or MPL.
- Commercial game files: nothing found. No KSP part names in `assets/core` (the engines in `stock.json5` are our own K-1, H-10 and so on), the models are built by `tools/blender/*.py`, the icons are SVG. `scripts/` is gitignored, so the KSP converter and its output (`assets/core/imported`) are not in the repo. The Realism mod comes from community mod repositories (CC licences), not from the game's files. The KSP mentions in code and docs are comparisons.
- Data from NASA, NOAA and ESA was already credited in `assets/core/terrain/README.md` and `assets/core/sky/README.md`.

## Open

- Tundra Exploration's models and textures are All Rights Reserved and there is no permission (the earlier notes saying so were wrong; corrected 2026-11-01). Nothing of theirs is tracked. The two Crew Dragon `pods` records still name `models/pods/TE_18_DRAGONV2_POD*.glb`, and the converted files (`mods/realism/models/pods/TE_18_DRAGONV2_POD*.glb`, `mods/realism/models/library/TundraExploration/`) are on the owner's machine only. The importer no longer converts Tundra's art. Either ask damonvv (github.com/damonvv/TundraExploration) in writing, or drop the model fields from those two pods and delete the converted files.
- Ask the RP-1 maintainers (KSP-RO) about donations next to NC content, and about paid managed servers with the Realism mod. Until answered, managed servers run `core` only.
- ROEngines (CC BY-NC-ND) and ROTanks (CKAN says CC BY-SA, the repository has an MIT file): ask pap1723 if converted models are ever to ship.
- `tools/blender/vendor/io_object_mu` is committed as a gitlink (a pointer to github.com/taniwha/io_object_mu, GPL v2) and there is no `.gitmodules`, so a fresh clone gets an empty folder. Add `.gitmodules` or document where to get it.
- The copyright line in `LICENSE-MIT` is a default; change it if the owner wants a name.
- No icon or model carries an origin note. If any SVG icon came from an icon set, name it.
- Check the terms of the ESA CCI land cover and NASA imagery at the source before any paid use; only Copernicus was read in full.

## Where it comes from

- Planning of 2026-11-01.
