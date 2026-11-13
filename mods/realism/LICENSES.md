# Licences and credits

The game's code and its own assets are `MIT OR Apache-2.0` (`LICENSE-MIT` and `LICENSE-APACHE`
at the repository root). This folder is different. Most of its records are generated from the
Realism Overhaul family of mods for KSP, and each record keeps the licence of the source it
came from. The mod is a collection of records, not one work under one licence, and what is
below applies to the collection, not the root licences.

I am not a lawyer. Where it matters, ask the author.

## Sources

Every generated record has a `source` field with the file it came from. The texts are in
[`licenses/`](licenses/); the Creative Commons legal codes are at the links.

| Source | Authors | Licence | Used for |
| --- | --- | --- | --- |
| [RealismOverhaul](https://github.com/KSP-RO/RealismOverhaul) | NathanKell and the RO contributors | [CC BY-SA](https://creativecommons.org/licenses/by-sa/4.0/) | `engines`, `engine_families`, `air_breathing_engines`, `pods`, `resources`, `tank_types`, `batteries`: figures, names, descriptions, the TestFlight-style reliability blocks |
| [RealFuels](https://github.com/NathanKell/ModularFuelSystem) | ialdabaoth, NathanKell, taniwha, Starwaster, blowfish | CC BY-SA | `resources`, `tank_types`, `batteries` |
| [ROCapsules](https://github.com/KSP-RO/ROCapsules) | pap1723 and KSP-RO | CC BY-SA | `pods` |
| [ROSolar](https://github.com/KSP-RO/ROSolar) | pap1723 and KSP-RO | [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) | `solar_panels` |
| [RP-1](https://github.com/KSP-RO/RP-1) | RP-1 contributors | [CC BY-NC-SA 4.0](licenses/CC-BY-NC-SA-4.0.md), [its notice](licenses/RP-1.md) | `tech_nodes`, `avionics`, the dates on engines and pods (`year`, `tech`, `dated_by`), the science figures in `patches/science.json5` (experiments, and where each body's situations change and what science there is worth) |
| [TestFlight](https://github.com/KSP-RO/TestFlight) | Agathorn and KSP-RO | CC BY-NC-SA 4.0 | the model the `reliability` figures follow |
| [Bluedog Design Bureau](https://github.com/CobaltWolf/Bluedog-Design-Bureau) | CobaltWolf, Jso, Zorg, Invaderchaos | CC BY-NC-SA 4.0 | some `pods` |
| [Community Resource Pack](https://github.com/UmbraSpaceIndustries/CommunityResourcePack) | RoverDude | CC BY-NC-SA 4.0 | `resources` |
| [Tundra Exploration](https://github.com/damonvv/TundraExploration) | damonvv and contributors | configs CC BY-NC-SA 4.0; models and textures All Rights Reserved (see [its notice](licenses/TundraExploration.md)) | two `pods` (the configs only) |
| [ROEngines](https://github.com/KSP-RO/ROEngines) | pap1723 and KSP-RO | [CC BY-NC-ND 4.0](licenses/ROEngines.md) | engine models only (see below); the records hold numbers measured from them |
| [ROTanks](https://github.com/KSP-RO/ROTanks) | pap1723 and KSP-RO | CC BY-SA in the CKAN metadata, MIT in the repository ([text](licenses/ROTanks.md)) | tank models only (see below) |

Not from any of these: `craft`, `scenarios`, `launch_sites`, `assemblies`, `liveries`,
`patches`, `rcs/peroxide.json5` and `decals` are written by hand from public figures and are
the project's (`MIT OR Apache-2.0`). So are the `materials`, whose values are common
engineering numbers.

## Models

The converted models (`models/`, glTF binary) are not in the repository: `*.glb` is gitignored
and the release workflow does not make them. Keep it that way:

- ROEngines is CC BY-NC-ND. The licence allows no changes and no commercial use, and a
  conversion to another format is a change we should not distribute.
- Tundra Exploration's models and textures are All Rights Reserved: no redistribution and no
  re-use without the author's permission, and we do not have it. Nothing of them may be in the
  repository or a release. The two Crew Dragon `pods` records (`Rodan ...`) still name
  converted files (`models/pods/TE_18_DRAGONV2_POD*.glb`); those exist only on a machine that
  converted them, and a game without them draws a plain shape. Only the configs (CC BY-NC-SA)
  are used here.
- ROTanks, ROCapsules, ROSolar and the rest of the library follow their own licences above.

Anyone who runs the importer and converts the models gets files they may not pass on.

## What follows from it

- **Non-commercial.** RP-1, TestFlight, Bluedog, the Community Resource Pack and Tundra's
  configs are NC. Do not sell this mod, or put it behind a payment, a subscription or ads.
  The game is free, and donations do not unlock anything.
- **Share-alike.** If you adapt records from an SA source, share the result under the same
  licence (or, for BY-SA, a compatible one) and say what you changed.
- **Credit.** Keep this file and `licenses/` with the mod, wherever the mod goes, and keep the
  `source` fields.
- **Servers.** A paid or managed server should not run this mod without written permission
  from the authors of the NC sources. It runs `core`, which is the project's own.
