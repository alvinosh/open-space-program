"""Cold-gas RCS quad block.

Housing 0.30 m (Z) x 0.24 m (Y) x 0.14 m deep (+X) on a dark mounting plate. Origin = centre of
the mounting face that sits against the hull (x = 0); +X points away from the hull, +Z is the
rocket's nose. Four conical nozzles point +Z, -Z, +Y and -Y.
"""
import importlib
import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)
import osp_common as osp
importlib.reload(osp)
from mathutils import Matrix, Vector

NAME, OUT = "cold_gas_quad", "models/rcs/cold_gas_quad.glb"
SZ, SY, DEPTH = 0.30, 0.24, 0.14
PLATE = 0.01
NOZZLE_LEN, NOZZLE_EXIT_R = 0.06, 0.025

sc, coll = osp.new_scene("osp_" + NAME)
m = osp.material
mats = {
    "housing": m("RCS_Housing_LightGrey", "#c9cacc", 0.15, 0.5),
    "plate": m("RCS_MountingPlate_Dark", "#2d2e30", 0.6, 0.5),
    "steel": m("RCS_Nozzle_Steel", "#a8a8a6", 1.0, 0.3),
}
objs = []

# Mounting plate (slightly larger than the housing) with four bolt heads.
g = osp.Geo().add(osp.box((PLATE, SY + 0.02, SZ + 0.02), (PLATE / 2, 0, 0), bevel=0.003, bevel_segs=1))
for y in (-0.125, 0.125):
    for z in (-0.155, 0.155):
        g.add(osp.cylinder(0.0035, PLATE - 0.001, PLATE + 0.004, 6), 1,
              matrix=Matrix.Translation((0, y, z)) @ Matrix.Rotation(math.pi / 2, 4, "Y"))
objs.append(osp.make_object("RCS_MountingPlate", g, [mats["plate"], mats["steel"]], coll, 40))

# Bevelled housing; its back face is buried in the plate.
xc = 0.5 * (DEPTH + 0.004)
housing = osp.make_object("RCS_Housing", osp.Geo().add(
    osp.box((DEPTH - 0.004, SY, SZ), (xc, 0, 0), bevel=0.016, bevel_segs=2)), [mats["housing"]], coll, 40)
objs.append(housing)

# Nozzle bosses on the four side faces.
mounts = [((xc, 0, SZ / 2), (0, 0, 1)), ((xc, 0, -SZ / 2), (0, 0, -1)),
          ((xc, SY / 2, 0), (0, 1, 0)), ((xc, -SY / 2, 0), (0, -1, 0))]
g = osp.Geo()
for p, d in mounts:
    g.add(osp.lathe([(0.0, -0.004), (0.018, -0.004), (0.018, 0.004), (0.015, 0.007), (0.0, 0.007)], 16),
          matrix=osp.align_z(p, d))
objs.append(osp.make_object("RCS_NozzleBosses", g, [mats["housing"]], coll, 40))

# Thin-walled conical nozzles: throat just above the boss, exit 0.06 m from the housing face.
noz = [(0.009, 0.0), (0.009, 0.007), (0.0055, 0.013), (0.011, 0.024), (NOZZLE_EXIT_R, NOZZLE_LEN)]
g = osp.Geo()
for p, d in mounts:
    g.add(osp.lathe(noz, 16), matrix=osp.align_z(p, d))
objs.append(osp.make_object("RCS_Nozzles", g, [mats["steel"]], coll, 40,
                            solidify=dict(thickness=0.0015)))

path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
hlo, hhi, _ = osp.evaluated_stats(sc, [housing])
osp.report(sc, objs, path, [
    ("mounting face x (min x = 0)", lo[0], 0.0),
    ("housing depth (+X)", hhi[0], DEPTH),
    ("housing Y", hhi[1] - hlo[1], SY),
    ("housing Z", hhi[2] - hlo[2], SZ),
    ("nozzle tip z (housing + 0.06)", hi[2], SZ / 2 + NOZZLE_LEN),
])
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=20, azimuth=-35)

DEBUG_VIEWS = [("back", 10, 150), ("below", -40, 30)]
