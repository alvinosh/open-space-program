# T119: Ocean: FFT spectrum, foam, refraction, underwater

What is left of the ocean (wind waves, roughness, sky reflection and shallows are done): an FFT (JONSWAP) spectrum, foam, refraction, underwater.

Reference: the ocean of Scatterer (T067), after Tessendorf's FFT waves and Bruneton's ocean lighting.

## Done when

- Everything in the scope list works, and what can run headless has a test.

## State

Done: FFT in three cascades (`osp_sim::ocean`, `crate::ocean`), filtered slopes and roughness, reflected clouds. Foam is computed and drawn but has not been looked at closely.

Left: the water's surface isn't displaced (normals only, so the horizon line stays flat), refraction of the seabed is still the old colour blend, and underwater can't be seen, since the camera stays above the sea.

## Where it comes from

- Early feature checklist (since removed), Phase 8
