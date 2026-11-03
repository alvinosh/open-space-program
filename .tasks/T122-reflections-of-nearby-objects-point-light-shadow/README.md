# T122: Reflections of nearby objects, point-light shadows, ambient occlusion

What is left (sky cubemap light, TAA and SSAO are done): reflections of nearby objects (Bevy's SSR needs deferred rendering, which the terrain, haze and sky do not use yet) and point-light shadows.

Reference: Deferred (T067): hierarchical-depth screen-space reflections. The terrain, haze and sky no longer stand in the way of a deferred pass, since haze is now one pass on the depth buffer (T182).

## Done when

- Everything in the scope list works, and what can run headless has a test.

## Where it comes from

- Early feature checklist (since removed), Phase 8
