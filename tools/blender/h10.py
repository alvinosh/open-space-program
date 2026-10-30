"""H-10: expander-cycle hydrolox upper-stage engine (RL10 class).

Length 2.3 m, exit diameter 1.17 m, envelope radius 0.6 m. Long slender bell built from brazed
tubes (radial ripple), small chamber, compact geared turbopump on one side near the top.
Origin = gimbal pivot at the top centre; nozzle exit plane at z = -2.3.
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

NAME, OUT = "h10", "models/engines/h10.glb"
LENGTH, RE, ENVELOPE = 2.3, 0.585, 0.6
RC, Z_INJ, RT, Z_T = 0.13, -0.30, 0.064, -0.58
TUBES = 90

sc, coll = osp.new_scene("osp_" + NAME)
m = osp.material
mats = {
    "tubes": m("H10_TubeWallNozzle_BrightSteel", "#cfcfcc", 1.0, 0.22),
    "tubes_in": m("H10_TubeWallNozzle_HeatTintedInner", "#8f877c", 0.85, 0.42),
    "steel": m("H10_Injector_Steel", "#b8b8b6", 1.0, 0.3),
    "dark": m("H10_GimbalMount_DarkSteel", "#46494e", 0.85, 0.45),
    "pump": m("H10_Turbopump_Aluminium", "#c4c1ba", 1.0, 0.36),
    "lines": m("H10_Lines_Steel", "#a9aaac", 1.0, 0.26),
    "act": m("H10_Actuator_Silver", "#d2d3d4", 1.0, 0.25),
    "black": m("H10_Actuator_Black", "#1c1c1e", 0.2, 0.55),
}
objs = []

prof = osp.nozzle_contour(RC, Z_INJ, RT, Z_T, RE, -LENGTH, 30.0, 30.0, 9.0, n_bell=11, n_throat=2,
                          r_fillet=0.05, bell_bias=1.25)


def tube_ripple(r, z, theta):
    """Crest/valley alternation, depth ~15% of the local tube width."""
    depth = 0.15 * math.tau * r / TUBES
    return 0.5 * depth * math.cos(TUBES * theta)


bell = osp.Geo().add(osp.lathe(prof, 2 * TUBES, ripple=tube_ripple))
nozzle = osp.make_object("H10_TubeWallNozzle", bell, [mats["tubes"], mats["tubes_in"]], coll, 45,
                         solidify=dict(thickness=0.006, material_offset=1), concave_angle=12)
objs.append(nozzle)

# Manifolds: hydrogen inlet manifold partway down the bell, exit ring, chamber-top outlet manifold.
g = osp.Geo()
z_fm = -0.86
r_fm = osp.interp_r(prof, z_fm) + 0.022
g.add(osp.torus(r_fm, 0.026, 56, 8, z_fm))
g.add(osp.lathe(osp.band(prof, -LENGTH + 0.036, 0.03, 0.008, inset=0.003), TUBES, closed=True))
g.add(osp.torus(RC + 0.016, 0.02, 48, 8, Z_INJ - 0.025))
objs.append(osp.make_object("H10_Manifolds", g, [mats["steel"]], coll, 40))

# Gimbal mount on top and the thrust cone down to the injector.
g = osp.Geo()
g.add(osp.lathe([(0.0, -0.025), (0.092, -0.025), (0.1, -0.018), (0.1, -0.005), (0.094, 0.0),
                 (0.0, 0.0)], 40))
g.add(osp.box((0.12, 0.12, 0.07), (0, 0, -0.06), bevel=0.01, bevel_segs=2))
for axis in ((1, 0, 0), (0, 1, 0)):
    a = Vector(axis)
    g.add(osp.cyl_between(Vector((0, 0, -0.06)) - a * 0.085, Vector((0, 0, -0.06)) + a * 0.085, 0.02, 16))
g.add(osp.lathe([(0.0, -0.22), (0.155, -0.22), (0.15, -0.205), (0.06, -0.1), (0.045, -0.09),
                 (0.0, -0.09)], 40))
objs.append(osp.make_object("H10_GimbalMount", g, [mats["dark"]], coll, 40))

# Injector / LOX dome closing the top of the tube bundle.
g = osp.Geo()
g.add(osp.lathe([(0.0, Z_INJ - 0.006), (0.15, Z_INJ - 0.006), (0.15, Z_INJ + 0.016),
                 (0.138, Z_INJ + 0.026), (0.12, Z_INJ + 0.05), (0.09, Z_INJ + 0.07),
                 (0.0, Z_INJ + 0.08)], 48))
for k in range(12):
    a = math.tau * k / 12
    g.add(osp.cylinder(0.007, Z_INJ + 0.012, Z_INJ + 0.024, 6),
          matrix=Matrix.Translation((0.143 * math.cos(a), 0.143 * math.sin(a), 0)))
objs.append(osp.make_object("H10_Injector", g, [mats["steel"]], coll, 40))

# Turbopump at +X: turbine over a two-stage fuel pump, LOX pump geared alongside.
g = osp.Geo()
TP = Vector((0.27, 0.0, 0.0))
tp = Matrix.Translation(TP)
g.add(osp.lathe([(0.0, -0.27), (0.07, -0.27), (0.07, -0.16), (0.055, -0.14), (0.0, -0.14)], 28), matrix=tp)
g.add(osp.torus(0.068, 0.022, 28, 8, -0.205), matrix=tp)
g.add(osp.lathe([(0.0, -0.47), (0.045, -0.47), (0.06, -0.455), (0.08, -0.44), (0.08, -0.29),
                 (0.06, -0.27), (0.0, -0.27)], 28), matrix=tp)
g.add(osp.torus(0.078, 0.026, 28, 8, -0.33), matrix=tp)
g.add(osp.torus(0.078, 0.024, 28, 8, -0.41), matrix=tp)
lox = Vector((0.2, 0.17, 0.0))
g.add(osp.lathe([(0.0, -0.38), (0.04, -0.38), (0.055, -0.36), (0.055, -0.24), (0.045, -0.225),
                 (0.0, -0.225)], 24), matrix=Matrix.Translation(lox))
g.add(osp.torus(0.055, 0.018, 24, 8, -0.3), matrix=Matrix.Translation(lox))
gear_c = (TP + lox) / 2 + Vector((0, 0, -0.235))
g.add(osp.box(((lox - TP).length + 0.06, 0.07, 0.06), bevel=0.01, bevel_segs=1),
      matrix=Matrix.Translation(gear_c) @ Matrix.Rotation(math.atan2(lox.y - TP.y, lox.x - TP.x), 4, "Z"))
# Propellant inlets rising to stage flanges: fuel beside the pump volute, LOX on the LOX pump.
for (x, y), r_in, z0 in (((TP.x + 0.11, -0.08), 0.04, -0.36), ((lox.x, lox.y), 0.035, -0.24)):
    g.add(osp.lathe([(0.0, z0), (r_in, z0), (r_in, -0.075), (r_in + 0.012, -0.07), (r_in + 0.012, -0.045),
                     (0.0, -0.045)], 20), matrix=Matrix.Translation((x, y, 0)))
objs.append(osp.make_object("H10_Turbopump", g, [mats["pump"]], coll, 40))

# Lines: fuel pump -> nozzle inlet manifold, jacket outlet -> turbine, turbine exhaust -> injector,
# LOX pump -> injector.
g = osp.Geo()
a_fm = math.radians(-25)
fuel = osp.fillet([TP + Vector((-0.04, -0.06, -0.41)), TP + Vector((-0.02, -0.13, -0.45)),
                   Vector((0.24, -0.14, -0.62)),
                   Vector((r_fm * math.cos(a_fm), r_fm * math.sin(a_fm), z_fm + 0.01))], 0.06, 6)
g.add(osp.sweep(fuel, 0.026, 14, False, False))
g.add(osp.sweep(osp.fillet([Vector((0.125, -0.07, Z_INJ - 0.025)), Vector((0.17, -0.08, -0.29)),
                            Vector((0.19, -0.05, -0.22)), TP + Vector((-0.02, -0.02, -0.2))], 0.04, 5),
                0.022, 12, False, False))
g.add(osp.sweep(osp.fillet([TP + Vector((0.0, 0.0, -0.15)), TP + Vector((0.0, 0.0, -0.1)),
                            Vector((0.17, 0.0, -0.1)), Vector((0.15, 0.0, -0.2))], 0.035, 5),
                0.022, 12, False, False))
g.add(osp.sweep(osp.fillet([lox + Vector((-0.03, -0.03, -0.3)), Vector((0.14, 0.12, -0.3)),
                            Vector((0.11, 0.09, -0.24)), Vector((0.06, 0.05, -0.23))], 0.04, 5),
                0.024, 12, False, False))
objs.append(osp.make_object("H10_Lines", g, [mats["lines"]], coll, 40))

# Two small gimbal actuators.
g = osp.Geo()
for az in (135, 225):
    a = math.radians(az)
    c, s = math.cos(a), math.sin(a)
    top, bot = Vector((0.26 * c, 0.26 * s, -0.022)), Vector((0.155 * c, 0.155 * s, -0.46))
    d = (bot - top).normalized()
    side = d.cross(Vector((0, 0, 1))).normalized()
    g.add(osp.cyl_between(top + d * 0.025, top + d * 0.27, 0.026, 14), 0)
    g.add(osp.cyl_between(top + d * 0.25, bot - d * 0.025, 0.012, 10), 1)
    for p in (top + d * 0.01, bot - d * 0.01):
        g.add(osp.cyl_between(p - side * 0.018, p + side * 0.018, 0.017, 10), 0)
    g.add(osp.box((0.05, 0.04, 0.04), bevel=0.006, bevel_segs=1), 1,
          matrix=Matrix.Translation((0.135 * c, 0.135 * s, -0.46)) @ Matrix.Rotation(a, 4, "Z"))
objs.append(osp.make_object("H10_Actuators", g, [mats["act"], mats["black"]], coll, 40))

path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
osp.report(sc, objs, path, [
    ("length (-zmin)", -lo[2], LENGTH),
    ("top z (pivot at 0)", hi[2], 0.0),
    ("exit diameter (tube crests)", 2 * osp.max_radius_near(sc, nozzle, -LENGTH), 2 * RE),
])
print(f"  envelope radius {osp.max_radius(sc, objs):.4f} (limit {ENVELOPE})")
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=-14)

DEBUG_VIEWS = [("below", -65, -40), ("above", 30, 140), ("top", 20, -40, ((-0.6, -0.6, -0.7), (0.6, 0.6, 0.0)))]
