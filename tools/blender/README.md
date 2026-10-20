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
| `c1.py` | `models/pods/c1.glb` |
| `c4.py` | `models/pods/c4.glb` |
| `cold_gas_quad.py` | `models/rcs/cold_gas_quad.glb` |

`osp_common.py` has the shared helpers (lathe/sweep primitives, nozzle contours, materials, export,
checks, preview renders). `k1_common.py` is the K-1 powerhead shared by both K-1 engines.

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
