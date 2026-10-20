"""C-1: one-person capsule (Mercury/Vostok-era class).

Height 2.9 m, base diameter 1.9 m. Origin = centre of the base: the lowest point of the heat
shield is at z = 0 and the top of the antenna section at z = 2.9.
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

NAME, OUT = "c1", "models/pods/c1.glb"
HEIGHT, R_BASE = 2.9, 0.95
Z_CONE0, R_CONE0 = 0.11, 0.925     # cone base (sits in the heat shield)
Z_CONE1, R_CONE1 = 2.1, 0.375      # cone top = parachute/antenna section radius
ROWS, BEADS = 6, 64

sc, coll = osp.new_scene("osp_" + NAME)
m = osp.material
mats = {
    "shield": m("C1_HeatShield_Ablative", "#3d2b1f", 0.0, 0.88),
    "shingle": m("C1_Shingles_DarkInconel", "#2b2b2e", 0.6, 0.5),
    "neck": m("C1_RecoverySection_Dark", "#333437", 0.55, 0.45),
    "frame": m("C1_WindowFrame_Steel", "#55575a", 0.85, 0.4),
    "glass": m("C1_Window_Glass", "#0b1016", 0.0, 0.06),
    "thruster": m("C1_Thruster_Steel", "#9c9c9a", 1.0, 0.35),
}
objs = []


def r_cone(z):
    return R_CONE0 + (R_CONE1 - R_CONE0) * (z - Z_CONE0) / (Z_CONE1 - Z_CONE0)


HALF_ANGLE = math.atan((R_CONE0 - R_CONE1) / (Z_CONE1 - Z_CONE0))

# Heat shield: shallow spherical cap with a rounded edge.
rs = (0.88 ** 2 + 0.062 ** 2) / (2 * 0.062)
cap = [(r, rs - math.sqrt(rs * rs - r * r)) for r in (0.0, 0.2, 0.4, 0.6, 0.75, 0.88)]
shield = cap + [(0.915, 0.068), (0.94, 0.078), (R_BASE, 0.095), (R_BASE, 0.118), (0.0, 0.118)]
objs.append(osp.make_object("C1_HeatShield", osp.Geo().add(osp.lathe(shield, 96)), [mats["shield"]],
                            coll, 40))

# Conical pressure section: overlapping shingle rows with longitudinal stiffening beads.
prof = [(R_CONE0 - 0.01, Z_CONE0 - 0.015)]
h = (Z_CONE1 - Z_CONE0) / ROWS
for k in range(ROWS):
    za, zb = Z_CONE0 + k * h, Z_CONE0 + (k + 1) * h
    prof += [(r_cone(za) + 0.009, za + 0.002), (r_cone(zb - 0.004), zb - 0.004)]
prof += [(R_CONE1 - 0.01, Z_CONE1 + 0.01)]


def beads(r, z, theta):
    return 0.5 * 0.08 * math.tau * r / BEADS * math.cos(BEADS * theta)


objs.append(osp.make_object("C1_ConicalSection", osp.Geo().add(osp.lathe(prof, 2 * BEADS, ripple=beads)),
                            [mats["shingle"]], coll, 40, concave_angle=12))

# Cylindrical parachute (recovery) section and antenna canister.
neck = [(0.0, Z_CONE1 - 0.02), (R_CONE1, Z_CONE1 - 0.02), (R_CONE1, 2.49), (R_CONE1 - 0.012, 2.5),
        (R_CONE1 - 0.012, 2.52), (0.362, 2.53), (0.362, 2.875), (0.35, 2.894), (0.33, HEIGHT),
        (0.21, HEIGHT), (0.2, HEIGHT - 0.008), (0.16, HEIGHT - 0.008), (0.15, HEIGHT), (0.0, HEIGHT)]
g = osp.Geo().add(osp.lathe(neck, 96, ripple=lambda r, z, t: 0.5 * 0.0035 * math.cos(48 * t)
                            if 0.3 < r and Z_CONE1 < z < HEIGHT - 0.01 else 0.0))
g.add(osp.lathe([(0.355, Z_CONE1 - 0.03), (0.395, Z_CONE1 - 0.03), (0.395, Z_CONE1 + 0.012),
                 (0.355, Z_CONE1 + 0.012)], 64, closed=True))
objs.append(osp.make_object("C1_RecoverySection", g, [mats["neck"]], coll, 40, concave_angle=8))

# One trapezoidal window on the upper cone, facing +X.
zc = 1.5
normal = Vector((math.cos(HALF_ANGLE), 0.0, math.sin(HALF_ANGLE)))
mw = osp.surface_frame((r_cone(zc), 0.0, zc), normal, (0, 0, 1))
outer = [(-0.13, -0.15), (0.13, -0.15), (0.095, 0.15), (-0.095, 0.15)]
inner = [(-0.1, -0.12), (0.1, -0.12), (0.068, 0.12), (-0.068, 0.12)]
g = osp.Geo().add(osp.frame(outer, inner, -0.035, 0.028), 0, mw)
g.add(osp.prism([(-0.105, -0.125), (0.105, -0.125), (0.072, 0.125), (-0.072, 0.125)], -0.03, 0.018), 1, mw)
objs.append(osp.make_object("C1_Window", g, [mats["frame"], mats["glass"]], coll, 30))

# Attitude thrusters: four pitch/yaw nozzles at the top of the cone, two roll nozzles near the base.
noz = [(0.012, -0.03), (0.012, 0.0), (0.008, 0.012), (0.014, 0.025), (0.022, 0.055)]
g = osp.Geo()
for k in range(4):
    a = math.radians(45 + 90 * k)
    radial = Vector((math.cos(a), math.sin(a), 0.0))
    p = radial * r_cone(1.95) + Vector((0, 0, 1.95))
    g.add(osp.lathe(noz, 16), matrix=osp.align_z(p, radial))
for a_deg, sign in ((90, 1), (270, -1)):
    a = math.radians(a_deg)
    radial = Vector((math.cos(a), math.sin(a), 0.0))
    tangent = Vector((-math.sin(a), math.cos(a), 0.0)) * sign
    p = radial * (r_cone(0.32) + 0.01) + Vector((0, 0, 0.32))
    g.add(osp.lathe(noz, 16), matrix=osp.align_z(p, tangent))
    g.add(osp.box((0.06, 0.05, 0.05), bevel=0.008, bevel_segs=1), 0,
          matrix=Matrix.Translation(p - tangent * 0.025 - radial * 0.005) @ Matrix.Rotation(a, 4, "Z"))
objs.append(osp.make_object("C1_Thrusters", g, [mats["thruster"]], coll, 40,
                            solidify=dict(thickness=0.0015)))

path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
osp.report(sc, objs, path, [
    ("height (zmax)", hi[2], HEIGHT),
    ("lowest point (zmin = 0)", lo[2], 0.0),
    ("base diameter", hi[0] - lo[0], 2 * R_BASE),
    ("antenna section diameter", 2 * R_CONE1, 0.75),
])
print(f"  cone half-angle {math.degrees(HALF_ANGLE):.1f} deg")
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=18)

DEBUG_VIEWS = [("below", -40, 60), ("window", 10, 0, ((0.2, -0.4, 1.1), (0.8, 0.4, 2.0))), ("top", 50, 140)]
