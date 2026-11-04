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

Reworked on 2026-11-04, after the user said the Apollo capsule's effect looked bad: a flat blue plate ahead of the shield, a blotchy shield, brown smears and white needles. Now closer to Firefly's recipe, still our own code:

- The trail is up to 1400 wide, faint strips that only add light, off the outline itself. Their streaks come from a tiling noise texture read by the fragment's angle around the flow, so strips over the same place agree. Their hot core falls off by the metre, not by share of length, or long and short strips make bars.
- The shock is no longer the hull pushed along its normals, which on a flat shield can only be a flat copy. `osp_sim::plasma::Shock` builds it from the airstream map (a cone back from each texel, stood off furthest in the middle, smoothed) and it is drawn as two layers, hot gas and shock front, with no edge. It scales with the hull's breadth, so a long rocket going sideways gets one the size of its width.
- The shield is smooth, hottest at the shoulder, and a dull dark red at low heat. Colours run warm to pink to violet, as dim orange is brown. Sparks are few, short and slower. A second light in the wake lights the flanks. The fire is seen with at most `MOST_EXPOSURE`, or the night side's opened-up exposure burns it out to white.

Checked in stills on the Apollo CM: side, rear and front at 65 km, side at 85 km and 38 km and at night, and four frames 0.25 s apart. The user called it good enough. Left: not watched in motion or through a whole entry; not looked at on a long rocket or another body since the rework (the shock has tests for a long hull); some bars still show on the CM's skirt; at low speed (38 km, 2 km/s) the white Mach look is patchy at the rim; the white Mach cone on the way up is barely visible; particles are the strips only, with no smoke or chunks; and no haze or clouds in front of it (same gap as T118).

## Where it comes from

- The user's request of 2026-11-03. Reference: Firefly (T067); the method is Leah Lindner's paper on atmospheric entry VFX.
