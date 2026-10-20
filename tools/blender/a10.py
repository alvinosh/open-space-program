"""A-10: pressure-fed hypergolic upper-stage engine (AJ10 class).

Length 2.7 m, exit diameter 1.5 m, envelope radius 0.76 m. Bulky valve/injector head with two
propellant inlet lines, a short ablative chamber in a steel case, and a large radiatively cooled
columbium nozzle extension. No turbopump. Origin = gimbal pivot; exit plane at z = -2.7.
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

NAME, OUT = "a10", "models/engines/a10.glb"
LENGTH, RE, ENVELOPE = 2.7, 0.75, 0.76
RC, Z_INJ, RT, Z_T = 0.15, -0.42, 0.095, -0.82
Z_FLANGE = -1.05          # ablative chamber / columbium extension joint
R_CASE = 0.195

sc, coll = osp.new_scene("osp_" + NAME)
m = osp.material
mats = {
    "valve": m("A10_ValveHead_Silver", "#cacac8", 1.0, 0.28),
    "case": m("A10_ChamberCase_Steel", "#8e9092", 0.9, 0.38),
    "char": m("A10_AblativeLiner_Charred", "#1f1d1c", 0.0, 0.85),
    "cb_out": m("A10_ColumbiumExtension_Outer", "#2b2b2d", 0.5, 0.7),
    "cb_in": m("A10_ColumbiumExtension_Inner", "#4d4c4a", 0.5, 0.6),
    "flange": m("A10_Flange_Steel", "#76787b", 0.9, 0.4),
    "dark": m("A10_GimbalMount_DarkSteel", "#46494e", 0.85, 0.45),
    "lines": m("A10_PropellantLines_Steel", "#a9aaac", 1.0, 0.26),
    "act": m("A10_Actuator_Silver", "#d2d3d4", 1.0, 0.25),
    "black": m("A10_Actuator_Black", "#1c1c1e", 0.2, 0.55),
}
objs = []

prof = osp.nozzle_contour(RC, Z_INJ, RT, Z_T, RE, -LENGTH, 30.0, 32.0, 8.0, n_bell=18, r_fillet=0.05)
ext, liner = osp.split_profile(prof, Z_FLANGE)
r_fl = liner[0][0]

# Ablative chamber: closed solid, steel case outside, charred liner (the nozzle contour) inside.
outer = [(r_fl, Z_FLANGE), (r_fl + 0.05, Z_FLANGE), (r_fl + 0.05, Z_FLANGE + 0.03),
         (r_fl + 0.022, Z_FLANGE + 0.03), (0.152, Z_T + 0.02), (0.168, Z_T + 0.1),
         (R_CASE, -0.7), (R_CASE, Z_INJ)]
for r_o, z in outer[4:]:
    assert r_o > osp.interp_r(liner, z) + 0.02, z
inner = list(reversed(liner))[:-1]  # injector face edge down to the flange
segs = 64
verts, faces = osp.lathe(outer + inner, segs, closed=True)
seg_mat = [0] * (len(outer) - 1) + [1] * (len(inner) + 1)
chamber = osp.Geo().add((verts, faces), [seg_mat[k // segs] for k in range(len(faces))])
objs.append(osp.make_object("A10_AblativeChamber", chamber, [mats["case"], mats["char"]], coll, 40))

extension = osp.make_object("A10_NozzleExtension", osp.Geo().add(osp.lathe(ext, 80)),
                            [mats["cb_out"], mats["cb_in"]], coll, 50,
                            solidify=dict(thickness=0.005, material_offset=1))
objs.append(extension)

g = osp.Geo()
g.add(osp.lathe(osp.band(prof, Z_FLANGE, 0.025, 0.045, inset=0.003), 80, closed=True))
g.add(osp.lathe(osp.band(prof, -1.95, 0.04, 0.012, inset=0.003), 80, closed=True))
g.add(osp.lathe(osp.band(prof, -2.6, 0.045, 0.014, inset=0.003), 80, closed=True))
for k in range(24):
    a = math.tau * (k + 0.5) / 24
    g.add(osp.cylinder(0.008, Z_FLANGE - 0.035, Z_FLANGE + 0.04, 6),
          matrix=Matrix.Translation(((r_fl + 0.032) * math.cos(a), (r_fl + 0.032) * math.sin(a), 0)))
objs.append(osp.make_object("A10_FlangesAndRings", g, [mats["flange"]], coll, 40))

# Valve / injector head: injector flange and dome, valve body, two ball-valve actuators.
g = osp.Geo()
g.add(osp.lathe([(0.0, Z_INJ - 0.006), (0.218, Z_INJ - 0.006), (0.218, Z_INJ + 0.03),
                 (0.2, Z_INJ + 0.04), (0.2, Z_INJ + 0.06)] +
                [(0.2 * math.cos(math.radians(a)), Z_INJ + 0.06 + 0.1 * math.sin(math.radians(a)))
                 for a in (25, 50, 70, 90)], 56))
for k in range(16):
    a = math.tau * k / 16
    g.add(osp.cylinder(0.009, Z_INJ + 0.025, Z_INJ + 0.042, 6),
          matrix=Matrix.Translation((0.208 * math.cos(a), 0.208 * math.sin(a), 0)))
g.add(osp.box((0.36, 0.2, 0.15), (0, 0, -0.27), bevel=0.025, bevel_segs=2))
for x in (-0.095, 0.095):
    g.add(osp.lathe([(0.0, -0.17), (0.04, -0.17), (0.055, -0.155), (0.055, 0.135), (0.04, 0.15),
                     (0.0, 0.15)], 24), matrix=Matrix.Translation((x, 0, -0.235)) @ Matrix.Rotation(
        -math.pi / 2, 4, "X"))
    g.add(osp.box((0.07, 0.07, 0.05), (x, -0.12, -0.17), bevel=0.01, bevel_segs=1))
objs.append(osp.make_object("A10_ValveHead", g, [mats["valve"]], coll, 40))

# Gimbal mount on top of the valve body.
g = osp.Geo()
g.add(osp.lathe([(0.0, -0.025), (0.095, -0.025), (0.105, -0.017), (0.105, -0.006), (0.098, 0.0),
                 (0.0, 0.0)], 40))
g.add(osp.box((0.13, 0.13, 0.075), (0, 0, -0.062), bevel=0.01, bevel_segs=2))
for axis in ((1, 0, 0), (0, 1, 0)):
    a = Vector(axis)
    g.add(osp.cyl_between(Vector((0, 0, -0.062)) - a * 0.09, Vector((0, 0, -0.062)) + a * 0.09, 0.02, 16))
g.add(osp.lathe([(0.0, -0.205), (0.11, -0.205), (0.075, -0.13), (0.05, -0.095), (0.0, -0.095)], 32))
objs.append(osp.make_object("A10_GimbalMount", g, [mats["dark"]], coll, 40))

# Two propellant inlet lines (oxidiser +X, fuel -X) from the valve body up to stage flanges.
g = osp.Geo()
for sx, rad in ((1, 0.04), (-1, 0.036)):
    path = osp.fillet([Vector((sx * 0.15, 0, -0.28)), Vector((sx * 0.29, 0, -0.28)),
                       Vector((sx * 0.31, 0, -0.06))], 0.07, 8)
    g.add(osp.sweep(path, rad, 16, False, True))
    g.add(osp.lathe([(0.0, -0.06), (rad + 0.018, -0.06), (rad + 0.018, -0.035), (0.0, -0.035)], 20),
          matrix=Matrix.Translation((sx * 0.31, 0, 0)))
    g.add(osp.lathe([(0.0, -0.012), (rad + 0.012, -0.012), (rad + 0.012, 0.012), (0.0, 0.012)], 20),
          matrix=osp.align_z(Vector((sx * 0.2, 0, -0.28)), Vector((sx, 0, 0))))
objs.append(osp.make_object("A10_PropellantLines", g, [mats["lines"]], coll, 40))

# Two gimbal actuators, 90 degrees apart, down to lugs on the chamber case.
g = osp.Geo()
for az in (45, 135):
    a = math.radians(az)
    c, s = math.cos(a), math.sin(a)
    top, bot = Vector((0.32 * c, 0.32 * s, -0.025)), Vector((0.235 * c, 0.235 * s, -0.62))
    d = (bot - top).normalized()
    side = d.cross(Vector((0, 0, 1))).normalized()
    g.add(osp.cyl_between(top + d * 0.03, top + d * 0.36, 0.034, 16), 0)
    g.add(osp.cyl_between(top + d * 0.34, bot - d * 0.03, 0.015, 12), 1)
    g.add(osp.box((0.05, 0.05, 0.07), top + d * 0.16 - side.cross(d) * 0.04, bevel=0.006, bevel_segs=1), 1)
    for p in (top + d * 0.012, bot - d * 0.012):
        g.add(osp.cyl_between(p - side * 0.02, p + side * 0.02, 0.02, 12), 0)
    g.add(osp.box((0.06, 0.05, 0.05), bevel=0.006, bevel_segs=1), 1,
          matrix=Matrix.Translation((0.21 * c, 0.21 * s, -0.62)) @ Matrix.Rotation(a, 4, "Z"))
objs.append(osp.make_object("A10_Actuators", g, [mats["act"], mats["black"]], coll, 40))

path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
osp.report(sc, objs, path, [
    ("length (-zmin)", -lo[2], LENGTH),
    ("top z (pivot at 0)", hi[2], 0.0),
    ("exit diameter", 2 * osp.max_radius_near(sc, extension, -LENGTH), 2 * RE),
])
print(f"  envelope radius {osp.max_radius(sc, objs):.4f} (limit {ENVELOPE})")
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=-14)

DEBUG_VIEWS = [("below", -65, -40), ("above", 30, 140), ("top", 15, -40, ((-0.5, -0.5, -1.2), (0.5, 0.5, 0.0)))]
