# T182: Sky and haze from each body's air, with multiple scattering

The sky was single scattering with a constant standing in for the rest, on a sphere mesh, Earth only, and only the terrain got haze. Now:

- `osp_sim::scattering`: a body's air from its data (`scattering`, see docs/modding.md), with tables for transmittance, all higher orders of scattering (Hillaire 2020) and sky irradiance.
- `shaders/sky.wesl`: one fullscreen pass per body in sight, after the opaque scene, from the depth buffer. Everything opaque sits in the same haze, and the planet's glow from space is the same code as the sky from the ground.
- Earth, Mars (dust, butterscotch day, blue glow round the Sun) and Venus. The terrain's and the light probe's sky light come from the same tables instead of fits.
- Stars fade by the brightness of the sky in front of them; the Sun is an HDR disc with a glare; the view tonemaps with Khronos PBR Neutral, which keeps the hue of a sunset.

## Done when

- The march is within 3% of a fine integral everywhere you can look (a test holds it), and the shipped bodies' skies are in range (a test holds it).
- No banding or shimmer in captures, with MSAA, TAA, FXAA and render scale, and on GPUs without dual-source blending.

## Left for later

- Plumes and trails are transparent and drawn after the pass, so they get no haze: T118.
- Sunlight is the same at every planet (`RAW_SUNLIGHT`); Mars should get 43% of it, with the exposure making up for it.

## Where it comes from

- The user's request of 2026-11-03 to match and beat the reference mods; this is the sky part of Scatterer (T067).
