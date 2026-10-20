"""K-1 Vacuum: the K-1 powerhead with a vacuum nozzle (MVac class).

Length 4.0 m, exit diameter 3.0 m, envelope radius 1.52 m. Regeneratively cooled nozzle down to
z = -1.3, then a thin radiatively cooled niobium extension to the exit at z = -4.0, with a turbine
exhaust manifold torus at the joint and a stiffening ring near the exit.
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

NAME, OUT = "k1_vacuum", "models/engines/k1_vacuum.glb"
LENGTH, RE, ENVELOPE = 4.0, 1.5, 1.52
Z_JOINT = -1.3

sc, coll = osp.new_scene("osp_" + NAME)
mats = k1.materials()
mats["nb_out"] = osp.material("K1V_NiobiumExtension_Outer", "#2c2d2f", 0.55, 0.72)
mats["nb_in"] = osp.material("K1V_NiobiumExtension_Inner", "#4d4f52", 0.55, 0.62)
mats["ring"] = osp.material("K1V_StiffeningRing_Gunmetal", "#3c3e42", 0.8, 0.5)

prof = k1.chamber_and_bell(RE, -LENGTH, theta_n=38.0, theta_e=12.0, n_bell=20)
ext, regen = osp.split_profile(prof, Z_JOINT)
r_joint = regen[0][0]
objs = k1.build_powerhead(coll, mats, "K1V", prof)

objs.append(osp.make_object("K1V_RegenNozzle", osp.Geo().add(osp.lathe(regen, 64)),
                            [mats["bronze"], mats["bronze_in"]], coll, 40, solidify=k1.REGEN_SOLIDIFY))
bands = osp.Geo()
bands.add(osp.lathe(osp.band(prof, -1.12, 0.026, 0.006), 64, closed=True))
bands.add(osp.lathe(osp.band(prof, Z_JOINT + 0.05, 0.05, 0.012), 64, closed=True))  # regen flange
objs.append(osp.make_object("K1V_RegenBands", bands, [mats["band"]], coll, 40))

extension = osp.make_object("K1V_NozzleExtension", osp.Geo().add(osp.lathe(ext, 80)),
                            [mats["nb_out"], mats["nb_in"]], coll, 50,
                            solidify=dict(thickness=0.006, material_offset=1))
rings = osp.Geo()
rings.add(osp.lathe(osp.band(prof, Z_JOINT - 0.005, 0.04, 0.012), 80, closed=True))  # extension flange
rings.add(osp.lathe(osp.band(prof, -3.68, 0.07, 0.022), 80, closed=True))           # stiffener near exit
objs += [extension, osp.make_object("K1V_ExtensionRings", rings, [mats["ring"]], coll, 40)]

# Turbine exhaust: duct from the turbine down into a torus manifold around the joint, which
# dumps the gas into the extension as a film-cooling layer.
r_man = 0.055
R_man = r_joint + 0.012 + r_man
g = osp.Geo().add(osp.torus(R_man, r_man, 64, 10, Z_JOINT + 0.01))
o = k1.TURBINE_OUTLET
duct = osp.fillet([o, o + Vector((0, 0, -0.2)), Vector((R_man, 0, -1.16)),
                   Vector((R_man, 0, Z_JOINT + 0.03))], 0.1, 8)
g.add(osp.sweep(duct, 0.05, 18, cap0=False, cap1=False))
g.add(osp.cyl_between(o + Vector((0, 0, -0.06)), o + Vector((0, 0, -0.085)), 0.062, 18))
objs.append(osp.make_object("K1V_ExhaustManifold", g, [mats["hot"]], coll, 40))

path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
osp.report(sc, objs, path, [
    ("length (-zmin)", -lo[2], LENGTH),
    ("top z (pivot at 0)", hi[2], 0.0),
    ("exit diameter", 2 * osp.max_radius_near(sc, extension, -LENGTH), 2 * RE),
    ("joint diameter (~0.75)", 2 * r_joint, 0.75),
])
print(f"  envelope radius {osp.max_radius(sc, objs):.4f} (limit {ENVELOPE})")
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=-14)
