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

First version: `osp_sim::plasma` (heating, hull map), `crate::plasma`, `shaders/plasma.wesl`. Shock, sheath, wake and sparks follow the hull from any angle. Left: close up at night it still blows out to white; it has not been watched in motion or through a whole entry; and it has no haze or clouds in front of it (same gap as T118).

## Where it comes from

- The user's request of 2026-11-03. Reference: Firefly (T067); the method is Leah Lindner's paper on atmospheric entry VFX.
