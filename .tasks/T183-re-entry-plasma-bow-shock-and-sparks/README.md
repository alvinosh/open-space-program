# T183: Re-entry: plasma, bow shock and sparks

Nothing shows on entry yet. A vessel coming in at 11 km/s should wear a sheath of plasma shaped by its own hull, with a bow shock ahead, a wake streaming behind and sparks off the heat shield.

## Scope

- Driven by what the sim already knows (dynamic pressure, Mach, heating), not by altitude. The colour comes from the temperature and the body's air.
- The hull's shape from the vessel's own meshes, seen along the airflow, so a capsule and a shuttle differ without per-part data.
- Bow shock in front of the hull, wake behind it, particles; drawn with the transparent pass, and hazed like plumes (T118).
- Light from the plasma on the hull and the ground.

## Done when

- Captures of an Apollo-speed entry from outside, in the wake and from far below, by day and at night, with no popping as the effect comes and goes.
- What can run headless has a test.

## State

Rebuilt on 2026-11-03 after Lindner's paper and Firefly's breakdown of it, from a first version that ray-marched a box and looked nothing like either. `osp_sim::plasma`: heating, the look by speed and density (`entry_look`), points scattered by area over the vessel's own meshes, and the airstream map, a depth map along the flow with how far each texel is inside the hull's outline. `crate::plasma` and `shaders/plasma.wesl`: one mesh of strips the vertex shader shapes, as wgpu has no geometry shaders. Layers: hot hull, wake, closing streaks, trail, shock veil, sparks, and the forward bow dome. Streaks come off the outline the air sees, so a flat heat shield's rim counts.

Checked in stills: Apollo-sized capsule by day and at night from four sides, a long rocket, at 95, 65 and 32 km, and on Venus. Left: not watched in motion or through a whole entry; the white Mach cone on the way up is barely visible; particles are the strips only, with no smoke or chunks; and no haze or clouds in front of it (same gap as T118).

## Where it comes from

- The user's request of 2026-11-03. Reference: Firefly (T067); the method is Leah Lindner's paper on atmospheric entry VFX.
