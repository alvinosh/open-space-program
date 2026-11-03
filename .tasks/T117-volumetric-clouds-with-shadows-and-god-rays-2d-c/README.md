# T117: Volumetric clouds with shadows and god rays; 2D cloud layers

Volumetric clouds with shadows and god rays, and 2D cloud layers with normal maps.

Reference: EnvironmentalVisualEnhancements (T067): 2D cloud layers, cloud shadows and city lights.

## Done when

- Everything in the scope list works, and what can run headless has a test.

## Done

On 2026-11-03 the ray-marched volumes went: thresholded noise showed its pattern on the way up, and the user asked for clouds that look the same as EnvironmentalVisualEnhancements, with it as a direct guide. So it now works as that does, and uses its textures (MIT, `assets/core/clouds/LICENSE.txt`):

- `osp_sim::clouds::CloudMap` reads its two-texture cube of cloud imagery. Packs of it stack layers of a map for more cloud, and so does this: a body's map and a turned copy (`map_turns`, `stacked`), since `fair` covers 12% of the sphere and two copies about 30%. Earth's `seed` is picked for broken cloud over the Cape.
- `shaders/clouds.wesl` is its Cloud shader on a sphere found by the ray: map shaken by UV noise, tiling detail, near fade, rim fade. Sheets are lit through the tables of `osp_sim::scattering` (T182). Earth is ten times Kerbin, so the detail has a second, finer copy. A thin high sheet is the same map turned another way.
- `crate::cloud_puffs` and `shaders/cloud_puffs.wesl` are its volume particles, on a mesh of quads in place of a geometry shader: a hex grid on a lattice fixed to the planet, fading in as the sheet fades out.
- The sim's weather takes its cover from the map, so shadows on the ground match. Bodies without a map of their own show the shared one where their weather says.

Checked in stills from the pad, under, in and over the sheet, and at 16, 39, 92 and 400 km, plus Mars and Venus. Reading the map adds about half a second to starting up. Not checked: watched in motion (the grid stepping along, flying through at speed) and a whole globe from far away. The whole pattern stands still; only the detail and the shake move. `docs/screenshots/look_pad.jpg` shows the old clouds.

## Where it comes from

- Early feature checklist (since removed), Phase 8
