"""K-1 Booster: sea-level gas-generator kerolox engine (Merlin-1D class).

Length 2.5 m, exit diameter 0.90 m, envelope radius 0.47 m (nine of these cluster at ~0.92 m).
Origin = gimbal pivot at the top centre; nozzle exit plane at z = -2.5.
"""
import importlib
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)
import osp_common as osp
import k1_common as k1
importlib.reload(osp)
importlib.reload(k1)
from mathutils import Vector

NAME, OUT = "k1_booster", "models/engines/k1_booster.glb"
LENGTH, RE, ENVELOPE = 2.5, 0.45, 0.47

sc, coll = osp.new_scene("osp_" + NAME)
mats = k1.materials()
prof = k1.chamber_and_bell(RE, -LENGTH, theta_n=33.0, theta_e=6.0, n_bell=16)
objs = k1.build_powerhead(coll, mats, "K1B", prof)

nozzle = osp.make_object("K1B_Nozzle", osp.Geo().add(osp.lathe(prof, 72)),
                         [mats["bronze"], mats["bronze_in"]], coll, 40, solidify=k1.REGEN_SOLIDIFY)
bands = osp.Geo()
for z in (-1.2, -1.45, -1.7, -1.95, -2.2):
    bands.add(osp.lathe(osp.band(prof, z, 0.028, 0.006), 72, closed=True))
bands.add(osp.lathe(osp.band(prof, -2.44, 0.045, 0.006), 72, closed=True))  # exit stiffener
objs += [nozzle, osp.make_object("K1B_NozzleBands", bands, [mats["band"]], coll, 40)]

# Gas-generator exhaust: turbine outlet down beside the bell to a small flared outlet.
o = k1.TURBINE_OUTLET
duct = osp.fillet([o, o + Vector((0, 0, -0.3)), Vector((0.385, 0.0, -1.36))], 0.12, 8)
flare = [Vector((0.385, 0.0, -1.36 - 0.14 * k / 6)) for k in range(1, 7)]
radii = [0.055] * len(duct) + [0.055 + 0.02 * (k / 6) ** 2 for k in range(1, 7)]
g = osp.Geo().add(osp.sweep(duct + flare, radii, 18, cap0=True, cap1=False))
objs.append(osp.make_object("K1B_GGExhaust", g, [mats["hot"]], coll, 40,
                            solidify=dict(thickness=0.004)))
g = osp.Geo().add(osp.cyl_between(Vector((0.385, 0, -1.30)), Vector((0.385, 0, -1.325)), 0.064, 18))
g.add(osp.cyl_between(o + Vector((0, 0, -0.06)), o + Vector((0, 0, -0.085)), 0.064, 18))
objs.append(osp.make_object("K1B_GGExhaustFlanges", g, [mats["hot"]], coll, 40))

path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
osp.report(sc, objs, path, [
    ("length (-zmin)", -lo[2], LENGTH),
    ("top z (pivot at 0)", hi[2], 0.0),
    ("exit diameter", 2 * osp.max_radius_near(sc, nozzle, -LENGTH), 2 * RE),
])
print(f"  envelope radius {osp.max_radius(sc, objs):.4f} (limit {ENVELOPE})")
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=-14)
