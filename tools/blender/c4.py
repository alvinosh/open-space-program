"""C-4: four-person capsule (modern Dragon/Orion class).

Height 3.9 m, base diameter 3.7 m (sits on a 3.7 m rocket). Origin = centre of the base: the
lowest point of the heat shield is at z = 0 and the top of the nose cap at z = 3.9.
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

NAME, OUT = "c4", "models/pods/c4.glb"
HEIGHT, R_BASE = 3.9, 1.85
Z_SIDE0, R_SIDE0 = 0.272, 1.76     # sidewall starts on top of the dark edge band
Z_SIDE1, R_SIDE1 = 3.3, 0.9        # ~1.8 m diameter at the top of the sidewall
SEGS = 96                          # 3.75 deg per segment; pockets are whole segments
WIN_Z, POD_Z = (2.1, 2.42), (2.72, 3.08)

sc, coll = osp.new_scene("osp_" + NAME)
m = osp.material
mats = {
    "shield": m("C4_HeatShield_Ablative", "#2c2724", 0.0, 0.85),
    "band": m("C4_ShieldEdgeBand_Black", "#151517", 0.1, 0.6),
    "white": m("C4_Sidewall_White", "#e8e8e4", 0.0, 0.42),
    "pod": m("C4_ThrusterPod_DarkGrey", "#3a3b3e", 0.3, 0.6),
    "frame": m("C4_WindowFrame_Grey", "#5c5e61", 0.4, 0.5),
    "glass": m("C4_Window_Glass", "#0b1016", 0.0, 0.05),
    "cap": m("C4_NoseCap_White", "#ededea", 0.0, 0.4),
    "ring": m("C4_DockingRing_Aluminium", "#b8b8b5", 1.0, 0.35),
    "thruster": m("C4_Thruster_Inconel", "#6e665d", 0.9, 0.45),
}
objs = []
TAN = (R_SIDE0 - R_SIDE1) / (Z_SIDE1 - Z_SIDE0)
HALF_ANGLE = math.atan(TAN)


def r_side(z):
    return R_SIDE0 - (z - Z_SIDE0) * TAN


# Heat shield (shallow spherical cap) and the dark edge band above it, one lathe.
rs = (1.7 ** 2 + 0.095 ** 2) / (2 * 0.095)
prof = [(r, rs - math.sqrt(rs * rs - r * r)) for r in (0.0, 0.4, 0.8, 1.15, 1.45, 1.7)]
prof += [(1.77, 0.104), (1.815, 0.115), (1.842, 0.127), (R_BASE, 0.14)]
n_shield = len(prof) - 1
prof += [(R_BASE, 0.252), (1.83, 0.272), (R_SIDE0, Z_SIDE0), (0.0, Z_SIDE0)]
verts, faces = osp.lathe(prof, SEGS)
fm = [0 if k // SEGS < n_shield else 1 for k in range(len(faces))]
objs.append(osp.make_object("C4_HeatShield", osp.Geo().add((verts, faces), fm),
                            [mats["shield"], mats["band"]], coll, 40))

# White sidewall (truncated cone) with panel grooves, window and thruster-pod pockets, top deck.
zs = [Z_SIDE0 - 0.01, Z_SIDE0 + 0.03, 0.7, 0.996, 1.0, 1.006, 1.01, 1.4, 1.896, 1.9, 1.906, 1.91,
      WIN_Z[0], WIN_Z[1], 2.57, POD_Z[0], POD_Z[1], Z_SIDE1]
side = []
for z in zs:
    groove = 0.006 if z in (1.0, 1.006, 1.9, 1.906) else 0.0
    side.append((r_side(z) - groove, z))
side += [(R_SIDE1 - 0.016, Z_SIDE1 + 0.014), (0.0, Z_SIDE1 + 0.014)]
verts, faces = osp.lathe(side, SEGS)
row_w, row_p = zs.index(WIN_Z[0]), zs.index(POD_Z[0])


def pocket(row, az_deg, width):
    j0 = round(az_deg / 360 * SEGS) - width // 2
    return [row * SEGS + (j0 + j) % SEGS for j in range(width)]


pockets = [(pocket(row_w, az, 4), 0.03, 2, 1, (0.025, 1)) for az in (45, 135, 225, 315)]
pockets += [(pocket(row_p, az, 8), 0.1, 3, 3) for az in (0, 90, 180, 270)]
verts, faces, fm = osp.recess((verts, faces), [0] * len(faces), pockets)
objs.append(osp.make_object("C4_Sidewall", osp.Geo().add((verts, faces), fm),
                            [mats["white"], mats["frame"], mats["glass"], mats["pod"]], coll, 40))

# Docking ring under the hinged nose cap; hinge on the +Y side.
g = osp.Geo()
g.add(osp.lathe([(0.58, Z_SIDE1), (0.68, Z_SIDE1), (0.68, 3.4), (0.705, 3.41), (0.705, 3.45),
                 (0.58, 3.45)], 64, closed=True))
for k in range(12):
    a = math.tau * (k + 0.5) / 12
    g.add(osp.box((0.05, 0.03, 0.05), bevel=0.006, bevel_segs=1),
          matrix=Matrix.Translation((0.69 * math.cos(a), 0.69 * math.sin(a), 3.375)) @ Matrix.Rotation(a, 4, "Z"))
objs.append(osp.make_object("C4_DockingRing", g, [mats["ring"]], coll, 40))

cap = [(0.0, 3.45), (0.8, 3.45), (0.825, 3.465), (0.825, 3.5)]
cap += [(0.825 * math.cos(math.radians(a)), 3.5 + 0.4 * math.sin(math.radians(a)))
        for a in (12, 24, 36, 48, 60, 70, 80, 90)]
g = osp.Geo().add(osp.lathe(cap, 72))
hinge = Vector((0.0, 0.84, 3.4))
g.add(osp.box((0.16, 0.06, 0.16), (0.0, 0.83, 3.41), bevel=0.012, bevel_segs=1))
g.add(osp.cyl_between(hinge + Vector((-0.11, 0.035, 0)), hinge + Vector((0.11, 0.035, 0)), 0.025, 12))
for x in (-0.06, 0.06):
    g.add(osp.box((0.03, 0.05, 0.12), (x, 0.855, 3.36), bevel=0.006, bevel_segs=1))
objs.append(osp.make_object("C4_NoseCap", g, [mats["cap"]], coll, 40))

# Two small nozzles in each thruster pod, canted out of the pocket.
noz = [(0.022, -0.03), (0.022, 0.0), (0.014, 0.02), (0.024, 0.045), (0.042, 0.095)]
g = osp.Geo()
zc = sum(POD_Z) / 2
for az in (0, 90, 180, 270):
    a = math.radians(az)
    radial = Vector((math.cos(a), math.sin(a), 0.0))
    tangent = Vector((-math.sin(a), math.cos(a), 0.0))
    normal = radial * math.cos(HALF_ANGLE) + Vector((0, 0, math.sin(HALF_ANGLE)))
    floor = radial * r_side(zc) + Vector((0, 0, zc)) - normal * 0.1
    for side_off, cant in ((-0.11, Vector((0, 0, 0.35))), (0.11, tangent * 0.45)):
        p = floor + tangent * side_off
        g.add(osp.lathe(noz, 16), matrix=osp.align_z(p, normal + cant))
objs.append(osp.make_object("C4_Thrusters", g, [mats["thruster"]], coll, 40,
                            solidify=dict(thickness=0.002)))

path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
osp.report(sc, objs, path, [
    ("height (zmax)", hi[2], HEIGHT),
    ("lowest point (zmin = 0)", lo[2], 0.0),
    ("base diameter", hi[0] - lo[0], 2 * R_BASE),
    ("sidewall top diameter", 2 * R_SIDE1, 1.8),
])
print(f"  sidewall half-angle {math.degrees(HALF_ANGLE):.1f} deg")
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=18)

DEBUG_VIEWS = [("below", -40, 60), ("pods", 15, 20, ((0.3, -0.6, 2.0), (1.4, 0.6, 3.9))), ("top", 55, 140)]
