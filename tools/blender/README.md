# Blender part generators

Scripts that build the core part models in Blender 5.2 and export them as `.glb` files under
`assets/core/models/`. Each script rebuilds its own scene (`osp_<name>`), exports, and prints a
check of the result. Other scenes are left alone.

| Script | Output |
| --- | --- |
| `k1_booster.py` | `models/engines/k1_booster.glb` |
| `k1_vacuum.py` | `models/engines/k1_vacuum.glb` |
| `h10.py` | `models/engines/h10.glb` |
| `a10.py` | `models/engines/a10.glb` |
| `h25.py` | `models/engines/h25.glb` |
| `h2.py` | `models/engines/h2.glb` |
| `m2.py` | `models/engines/m2.glb` |
| `m2_vacuum.py` | `models/engines/m2_vacuum.glb` |
| `k5.py` | `models/engines/k5.glb` |
| `k19.py` | `models/engines/k19.glb` |
| `a87.py` | `models/engines/a87.glb` |
| `k0.py` | `models/engines/k0.glb` |
| `c1.py` | `models/pods/c1.glb` |
| `c4.py` | `models/pods/c4.glb` |
| `cold_gas_quad.py` | `models/rcs/cold_gas_quad.glb` |

`osp_common.py` has the shared helpers (lathe/sweep primitives, nozzle contours, materials, export,
checks, preview renders). `k1_common.py` is the K-1 powerhead shared by both K-1 engines,
`m2_common.py` the M-2 powerhead shared by the M-2 and M-2 Vacuum, and `k5_k19_kit.py` holds
helpers the K-5 and K-19 share (open bands, collars, bolt circles, bellows, debug views).

## Running

With Blender running and the Blender-MCP addon listening on `localhost:9876`, `bl.py` sends code to
it and prints the output:

```sh
python3 tools/blender/bl.py tools/blender/k1_booster.py                    # rebuild + export
python3 tools/blender/bl.py tools/blender/k1_booster.py --preview /tmp/osp  # also render /tmp/osp/k1_booster.png
python3 tools/blender/bl.py -c "import bpy; print(bpy.app.version_string)"
```

Headless works too (no preview): `blender -b --factory-startup --python tools/blender/h10.py`.

The printed check gives the bounding box in Blender axes, triangle counts (total and per object),
material names and the spec checks (length/height, exit/base diameter, origin), both from the
evaluated objects and read back from the written `.glb`. Engines also print their envelope radius.

## Conventions

- Metres, real scale. Blender is Z-up; export uses `export_yup=True`, so Blender +Z is the game's
  nose direction.
- Engines: origin = gimbal pivot at the top centre. The nozzle axis runs down -Z and the exit plane is
  at exactly z = -length. Everything stays inside the envelope radius about Z.
- Pods: origin = centre of the base. Lowest point of the heat shield at z = 0, top at z = height.
- RCS block: origin = centre of the mounting face against the hull (x = 0), body extends to +X,
  +Z is the rocket's nose, ±Y is around the hull.
- Materials: Principled BSDF with solid values only (no textures). Names are
  `<Part>_<Component>_<Finish>`. Detail comes from geometry.
- Budgets: engines ≤ 25k triangles, pods ≤ 20k, RCS block ≤ 2k.
- Geometry is generated in world space and every object keeps an identity transform.
- Nozzle bells are lathed profiles (chamber, converging section, throat, Rao bell) made into
  open thin-walled shells with a live Solidify modifier. A short vertical lip keeps the exit plane
  exact after Solidify.
- Every object ends with an "OSP Smooth by Angle" geometry-nodes modifier: smooth shading with
  sharp edges above an angle. It comes after Solidify, because Solidify's rim edges would
  otherwise stay smooth. Tube-wall and shingle ripples also get their concave grooves marked sharp.
- Export: only the model's mesh objects are selected, with `use_selection`, `use_active_scene`
  and `export_apply`. The preview camera and lights sit in a separate `<name>_preview_rig`
  collection and are not exported.
- Blender 5.2 crashes if the glTF exporter runs under `bpy.context.temp_override(view_layer=...)`.
  `osp_common.context()` only overrides the window and scene.

## KSP model converter

`convert_mu.py` turns KSP `.mu` part models (the Realism Overhaul repos under
`scripts/raw_space_data/repositories`) into `.glb` files. It runs inside Blender with the vendored
`vendor/io_object_mu` reader; the addon's material, texture and animation code is replaced, and its
skinned-mesh import falls back to the unskinned mesh when the addon's armature code fails.
`convert_mu_batch.py` (system `python3`) splits a job list over parallel headless Blenders.

```sh
Blender -b --factory-startup --python tools/blender/convert_mu.py -- jobs.json results.json
python3 tools/blender/convert_mu_batch.py jobs.json results.json --workers 6 --skip-existing
```

The batch driver runs chunks of a few jobs per Blender. A job that crashes or hangs Blender
(`--timeout`, default 300 s per job) is marked failed and the rest of its chunk reruns.
`--skip-existing` skips jobs whose `.glb` is newer than its `.mu` and texture files and whose job
is unchanged (`job_hash`), keeping their previous results. Logs go to `<results>.logs/`. The exit
status is 0 even when jobs fail; check `ok`. `$BLENDER` or `--blender` picks the executable.

Job (one per part, the input is a JSON array):

```json
{"id": "ROE-AJ10-137", "output": "/abs/out/ROE-AJ10-137.glb", "kind": "engine",
 "models": [{"path": "/abs/.../bluedog_Apollo_Block2_SPS.mu", "position": [0, 0, 0],
             "rotation": [0, 0, 0], "scale": [1, 1, 1], "textures": {"oldBase": "/abs/new/base"}}],
 "rescale": 1.5646, "origin": [0, 0.6608, 0], "hide_transforms": ["Mesh_TwoBall"],
 "max_texture": 2048}
```

- `models` are the PART's `MODEL` nodes in KSP part space (Unity, Y up), before `rescale`.
  `rotation` is Unity Euler degrees. A model's own root transform is replaced, as KSP does.
  Optional `parent` names a transform of an earlier model to attach to.
- `textures` is the `texture = old, new` map: base names, case-insensitive, `new` an absolute
  path without extension. Textures are otherwise found next to the `.mu` (.dds .png .tga .mbm .jpg).
- `origin` (pre-rescale part space) becomes the glTF origin: the top node for engines, the base
  centre for pods.
- `hide_transforms` drops those transforms with their children. A whole model can be hidden by
  its KSP clone name, `<GameData url>(Clone)`, as B9PartSwitch configs do.
- Optional: `keep_transforms` (names exempt from the effect-name filter) and `report_transforms`
  (extra transform names to report).
- Optional `pose_animations`: animation clips to leave the model as they end (each transform a
  clip moves takes its last key), e.g. the Tundra Dragon's nose shroud, saved open, closed for
  flight. Every other clip is dropped and the model keeps the pose it was saved in.

Output: one mesh object with an identity transform, glTF Y up, metres, after `rescale` and
`origin`. Colliders are dropped, and so are effect meshes: Particles/Additive/DepthMask/unlit-alpha
shaders, alpha materials without a texture (runtime flag decals), and objects named like
`*glow*`, `*plume*`, `*flare*` (not `flared`) or `*fx*`. Materials are Principled BSDF:
`_MainTex` x `_Color` as base colour, `_BumpMap` as the normal map (DXT5nm converted to RGB),
no emission, metallic 0, roughness 0.55 (down to 0.35 for shiny specular shaders; Unity Standard
uses its metallic/gloss values). Cutoff shaders export as MASK, translucent ones as BLEND, unless
the texture has no alpha. Back faces are culled. DDS textures are flipped upright, everything is
downscaled to `max_texture` and embedded as PNG. Tiling/offset is baked into the UVs. A mesh
that makes a model over 12 m across more than five times bigger than the rest of it is dropped
as broken (meshes under bones the addon can't pose), and if `hide_transforms` would hide every
mesh it is ignored.

Result (one per job, same order):

- `id`, `ok`, `error` (and `traceback`), `output`, `bytes`, `seconds`, `warnings` (dropped
  subtrees with mesh counts, missing textures, importer fallbacks).
- `triangles`, `vertices`, `bbox_min`, `bbox_max`, `height`, `max_diameter`, `top_diameter`
  (top 6% of the height), `bottom_diameter` (bottom 6%). Engines also get `length` (origin to the
  lowest point) and `exit_diameter` (lowest 4% of the length). Diameters are 2 x the largest
  distance from the Y axis. All in glTF axes and metres.
- `transforms`: `{name: {position, direction, up, count, instances: [{position, direction, up,
  hidden}]}}` for `*thrustTransform*`, `*rcsTransform*`, `*Transform`, `node_*` and
  `report_transforms`. The top-level values are the first instance, in hierarchy order.
  `direction` is the transform's Unity +Z (engine thrust transforms point along the exhaust),
  `up` its Unity +Y (ModuleRCS thrusts along Y unless `useZaxis`). `hidden` means under
  `hide_transforms`.
- `textures` (glTF image names), `texture_details` (source, role, source and output size, PNG
  bytes), `materials` (name, alpha mode), `meshes` (kept mesh names), `sources` (files read).
- From the driver: `job_hash`, and `skipped` for jobs kept by `--skip-existing`.
