# T117: Volumetric clouds with shadows and god rays; 2D cloud layers

Volumetric clouds with shadows and god rays, and 2D cloud layers with normal maps.

Reference: EnvironmentalVisualEnhancements (T067): 2D cloud layers, cloud shadows and city lights. Go further than it and raymarch real volumes, lit through the tables of `osp_sim::scattering` (T182).

## Done when

- Everything in the scope list works, and what can run headless has a test.

## Done

`osp_sim::clouds` (weather map from `clouds` in the body's data, tiling noise) and `crate::clouds` with `shaders/clouds*.wesl`: a half-resolution march with temporal reprojection and a depth-aware composite, a flat sheet from far away and past where steps run out, thin high cloud, shadows on the ground and in the sky's air, diffuse light under overcast. A `clouds` graphics setting. The weather map does not change with time: only the fine structure moves. Cloud decks seen from just above are the weakest view.

Two flaws seen from about 12,000 km, not yet looked into: a faint regular grid over the sea, which was there before the FFT waves went in, and scalloped edges on some clouds where the sheet reads the weather map's coarse levels.

## Where it comes from

- Early feature checklist (since removed), Phase 8
