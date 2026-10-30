"""K-19: single-chamber oxygen-rich staged-combustion kerolox engine (RD-191 class).

Length 4.0 m, exit diameter 1.45 m, envelope radius 0.9 m. Long regeneratively cooled bell with a
smooth dark-bronze outer jacket, cooling manifolds and stiffening bands. The main turbopump stands
beside the chamber at +X (turbine on top, LOX pump, fuel pump below) with the preburner next to the
turbine; the hot oxygen-rich turbine exhaust arches over in a big duct (with a bellows) into the
top of the mixing head. LOX and fuel boost pumps sit on either side with their stage inlets on
top, dense plumbing in between, a gimbal bearing on top and two gimbal actuators.
Origin = gimbal pivot at the top centre; nozzle exit plane at z = -4.0.
"""
import importlib
import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)
import osp_common as osp
import k5_k19_kit as kit
importlib.reload(osp)
importlib.reload(kit)
from mathutils import Matrix, Vector

NAME, OUT = "k19", "models/engines/k19.glb"
LENGTH, RE, ENVELOPE = 4.0, 0.725, 0.9
RC, Z_INJ, RT, Z_T = 0.19, -1.08, 0.118, -1.55
TPX = 0.54                                 # main turbopump axis, azimuth 0 (+X)

sc, coll = osp.new_scene("osp_" + NAME)
m = osp.material
mats = {
    "bronze": m("K19_RegenNozzle_OxidisedBronze", "#3a2b20", 0.75, 0.45),
    "bronze_in": m("K19_RegenNozzle_SootedInner", "#352f2a", 0.4, 0.62),
    "band": m("K19_NozzleBands_OxidisedBronze", "#4b3828", 0.8, 0.4),
    "manifold": m("K19_CoolantManifolds_Steel", "#8d8c88", 0.95, 0.34),
    "head": m("K19_MixingHead_Steel", "#a9a9a6", 1.0, 0.3),
    "duct": m("K19_HotGasDuct_HeatTinted", "#8a8278", 0.9, 0.4),
    "bellows": m("K19_DuctBellows_Steel", "#b8b8b6", 1.0, 0.28),
    "tpa": m("K19_Turbopump_Steel", "#9fa0a2", 1.0, 0.34),
    "boost": m("K19_BoostPumps_Aluminium", "#c4c1ba", 1.0, 0.36),
    "lines": m("K19_PropellantLines_Steel", "#a8a9ab", 1.0, 0.27),
    "small": m("K19_ControlLines_Copper", "#a0674a", 1.0, 0.35),
    "valves": m("K19_Valves_Aluminium", "#cfccc5", 1.0, 0.33),
    "dark": m("K19_GimbalBearing_DarkSteel", "#45484d", 0.85, 0.45),
    "act": m("K19_Actuator_Silver", "#d2d3d4", 1.0, 0.25),
    "black": m("K19_Actuator_Black", "#1c1c1e", 0.2, 0.55),
}
objs = []


def cyl(rho, deg, z):
    a = math.radians(deg)
    return Vector((rho * math.cos(a), rho * math.sin(a), z))


prof = osp.nozzle_contour(RC, Z_INJ, RT, Z_T, RE, -LENGTH, 30.0, 32.0, 6.0, n_bell=40, n_throat=3)


def wall(z):
    return osp.interp_r(prof, z)


# -------------------------------------------------------------------------- regen nozzle
bell_z = [-LENGTH + (Z_T - 0.06 + LENGTH) * (k / 13) ** 1.25 for k in range(14)]
lathe_z = [-LENGTH, -LENGTH + 0.006] + bell_z[1:] + [Z_T - 0.025, Z_T, Z_T + 0.04, Z_T + 0.09, Z_T + 0.15,
                                                    Z_T + 0.2, -1.3, Z_INJ]
noz_prof = [(wall(z), z) for z in lathe_z]
nozzle = osp.make_object("K19_RegenNozzle", osp.Geo().add(osp.lathe(noz_prof, 72)),
                         [mats["bronze"], mats["bronze_in"]], coll, 40,
                         solidify=dict(thickness=0.008, material_offset=1))
objs.append(nozzle)

g = osp.Geo()
for z, h, proud in ((-1.86, 0.03, 0.012), (-2.12, 0.03, 0.012), (-3.08, 0.04, 0.014), (-3.5, 0.04, 0.014),
                    (-LENGTH + 0.07, 0.058, 0.02)):
    g.add(kit.open_band(prof, z, h, proud, 0.008, 56))
objs.append(osp.make_object("K19_NozzleBands", g, [mats["band"]], coll, 40))

# Coolant manifolds: fuel inlet ring on the bell, film-cooling belt at the throat, collector at the
# top of the chamber; feed stubs on the inlet ring.
g = osp.Geo()
Z_FM = -2.55
R_FM = wall(Z_FM) + 0.012 + 0.042
g.add(osp.torus(R_FM, 0.042, 48, 8, Z_FM))
g.add(kit.open_band(prof, Z_FM + 0.05, 0.1, 0.012, 0.008, 56))
g.add(osp.torus(wall(Z_T - 0.06) + 0.04, 0.032, 36, 6, Z_T - 0.06))
g.add(osp.torus(RC + 0.03, 0.03, 36, 6, Z_INJ - 0.05))
g.add(kit.open_band(prof, -1.27, 0.06, 0.03, 0.006, 36))                              # actuator ring
for az in (135, 225):
    g.add(osp.box((0.15, 0.07, 0.05), bevel=0.008, bevel_segs=1),
          matrix=Matrix.Translation(cyl(0.27, az, -1.3)) @ Matrix.Rotation(math.radians(az), 4, "Z"))
for k in range(4):
    a = math.radians(45 + 90 * k)
    c = cyl(R_FM + 0.035, 45 + 90 * k, Z_FM)
    g.add(osp.cyl_between(c, c + Vector((math.cos(a), math.sin(a), 0)) * 0.04, 0.022, 8))
objs.append(osp.make_object("K19_CoolantManifolds", g, [mats["manifold"]], coll, 40))

# ------------------------------------------------------------- mixing head and gimbal bearing
g = osp.Geo()
head = [(0.0, Z_INJ - 0.012), (0.2, Z_INJ - 0.012), (0.255, Z_INJ - 0.01), (0.255, Z_INJ + 0.04),
        (0.225, Z_INJ + 0.045), (0.225, -0.86)]
head += [(0.225 * math.cos(math.radians(a)), -0.86 + 0.15 * math.sin(math.radians(a))) for a in (20, 40, 60, 75, 90)]
g.add(osp.lathe(head, 48))
kit.bolt_circle(g, 0, Vector((0, 0, Z_INJ + 0.04)), (0, 0, 1), 0.24, 18, 0.008, 0.0, 0.018, 4)
objs.append(osp.make_object("K19_MixingHead", g, [mats["head"]], coll, 40))

g = osp.Geo()
g.add(osp.lathe([(0.0, -0.045), (0.13, -0.045), (0.14, -0.035), (0.14, -0.008), (0.132, 0.0), (0.0, 0.0)], 32))
g.add(osp.torus(0.15, 0.028, 32, 8, -0.11))                                      # gimbal ring
g.add(osp.box((0.15, 0.15, 0.1), (0, 0, -0.11), bevel=0.015, bevel_segs=2))
for axis in ((1, 0, 0), (0, 1, 0)):
    a = Vector(axis)
    g.add(osp.cyl_between(Vector((0, 0, -0.11)) - a * 0.18, Vector((0, 0, -0.11)) + a * 0.18, 0.026, 12))
g.add(osp.lathe([(0.0, -0.22), (0.11, -0.22), (0.1, -0.19), (0.07, -0.16), (0.0, -0.155)], 28))
for az in (90, 180, 270):                                                       # thrust legs to the head
    top, bot = cyl(0.07, az, -0.2), cyl(0.215, az, -0.87)
    g.add(osp.cyl_between(top, bot, 0.03, 10))
    g.add(osp.box((0.07, 0.07, 0.06)), matrix=Matrix.Translation(bot + Vector((0, 0, 0.01)))
          @ Matrix.Rotation(math.radians(az), 4, "Z"))
g.add(osp.torus(0.222, 0.02, 40, 6, -0.875))
for az in (135, 225):                                                            # actuator brackets
    rot = Matrix.Rotation(math.radians(az), 4, "Z")
    g.add(osp.box((0.36, 0.06, 0.07), bevel=0.008, bevel_segs=1), matrix=rot @ Matrix.Translation((0.29, 0, -0.04)))
    g.add(osp.box((0.06, 0.1, 0.06)), matrix=rot @ Matrix.Translation((0.46, 0, -0.095)))
objs.append(osp.make_object("K19_GimbalBearing", g, [mats["dark"]], coll, 40))

# --------------------------------------------------------------------------- main turbopump
g = osp.Geo()
tp = Matrix.Translation((TPX, 0.0, 0.0))
g.add(osp.lathe([(0.0, -0.86), (0.12, -0.86), (0.18, -0.82), (0.2, -0.76), (0.2, -0.6), (0.17, -0.555),
                 (0.125, -0.53), (0.0, -0.53)], 28), matrix=tp)                    # turbine
g.add(osp.torus(0.2, 0.055, 28, 8, -0.69), matrix=tp)                              # turbine inlet torus
g.add(osp.lathe([(0.0, -1.2), (0.12, -1.2), (0.17, -1.16), (0.17, -0.92), (0.14, -0.87), (0.0, -0.86)],
                28), matrix=tp)                                                     # LOX pump
g.add(osp.torus(0.17, 0.075, 28, 8, -1.03), matrix=tp)                             # LOX volute
g.add(osp.lathe([(0.0, -1.56), (0.07, -1.56), (0.12, -1.52), (0.14, -1.46), (0.14, -1.26), (0.12, -1.2),
                 (0.0, -1.2)], 24), matrix=tp)                                      # fuel pump
g.add(osp.torus(0.14, 0.055, 24, 8, -1.37), matrix=tp)                             # fuel volute
for z, r in ((-0.86, 0.18), (-1.2, 0.16)):                                          # housing flanges
    g.add(osp.lathe([(r - 0.01, z - 0.014), (r + 0.024, z - 0.014), (r + 0.024, z + 0.014), (r - 0.01, z + 0.014)],
                    28), matrix=tp)
# Preburner beside the turbine, hot gas straight into the turbine inlet torus.
PB = cyl(0.33, 80, 0.0) + Vector((TPX, 0, 0))
pb = Matrix.Translation(PB)
g.add(osp.lathe([(0.0, -1.18), (0.045, -1.18), (0.078, -1.15), (0.085, -1.1), (0.085, -0.75), (0.07, -0.69),
                 (0.04, -0.655), (0.0, -0.65)], 20), matrix=pb)
g.add(osp.lathe([(0.08, -0.93), (0.1, -0.93), (0.1, -0.9), (0.08, -0.9)], 20), matrix=pb)
g.add(osp.cyl_between(PB + Vector((0, 0, -0.71)), cyl(0.22, 80, -0.71) + Vector((TPX, 0, 0)), 0.045, 12,
                      caps=False))
objs.append(osp.make_object("K19_Turbopump", g, [mats["tpa"]], coll, 40))

# Hot oxygen-rich gas duct: turbine outlet, up and over, down into the mixing head through a bellows.
R_GD = 0.12
raw = [Vector((TPX, 0, -0.56)), Vector((TPX, 0, -0.36)), Vector((0.46, 0, -0.26)), Vector((0.32, 0, -0.28)),
       Vector((0.18, 0, -0.5)), Vector((0.07, 0, -0.79))]
duct = osp.fillet(raw, 0.12, 6)
g = osp.Geo().add(osp.sweep(duct, R_GD, 18, False, False))
objs.append(osp.make_object("K19_HotGasDuct", g, [mats["duct"]], coll, 40))
g = osp.Geo()
d_leg = (raw[5] - raw[4]).normalized()
g.add(kit.bellows(raw[4] + d_leg * 0.05, d_leg, R_GD, 0.13, 4, 0.016, 18))
for p, d in ((raw[4] + d_leg * 0.035, d_leg), (raw[4] + d_leg * 0.195, d_leg), (Vector((TPX, 0, -0.51)), (0, 0, 1))):
    g.add(kit.collar(p, d, R_GD, R_GD + 0.028, 0.03, 18))
objs.append(osp.make_object("K19_DuctBellows", g, [mats["bellows"]], coll, 40))

# ------------------------------------------------------------------------------ boost pumps
g = osp.Geo()
BL, BF = cyl(0.6, -100, 0.0), cyl(0.6, 108, 0.0)            # LOX (-Y) and fuel (+Y) boost pumps
for c, s in ((BL, 1.15), (BF, 1.0)):
    mt = Matrix.Translation(c)
    g.add(osp.lathe([(0.0, -0.54), (0.05, -0.54), (0.09 * s, -0.51), (0.1 * s, -0.47), (0.1 * s, -0.38),
                     (0.13 * s, -0.36), (0.13 * s, -0.22), (0.1 * s, -0.19), (0.1 * s, -0.1),
                     (0.13 * s, -0.1), (0.13 * s, -0.065), (0.0, -0.065)], 24), matrix=mt)
    g.add(osp.torus(0.13 * s, 0.05 * s, 24, 8, -0.29), matrix=mt)
    kit.bolt_circle(g, 0, c + Vector((0, 0, -0.065)), (0, 0, 1), 0.115 * s, 8, 0.008, -0.004, 0.012, 4)
objs.append(osp.make_object("K19_BoostPumps", g, [mats["boost"]], coll, 40))

# ------------------------------------------------------------------------- propellant lines
g = osp.Geo()
lines = [
    # LOX boost pump -> main LOX pump inlet (below the turbine).
    ([BL + Vector((0.13, 0.08, -0.3)), Vector((0.27, -0.48, -0.46)), Vector((0.52, -0.42, -0.78)),
      Vector((TPX, -0.27, -0.93)), Vector((TPX, -0.15, -0.93))], 0.07, 0.1),
    # Fuel boost pump -> main fuel pump inlet (axial, from below).
    ([BF + Vector((0.1, -0.05, -0.3)), Vector((0.12, 0.52, -0.6)), Vector((0.32, 0.48, -1.3)),
      Vector((0.45, 0.28, -1.68)), Vector((TPX, 0.0, -1.7)), Vector((TPX, 0.0, -1.54))], 0.06, 0.09),
    # LOX volute -> preburner (the oxygen-rich preburner takes almost all of the LOX).
    ([cyl(0.17, 60, -1.03) + Vector((TPX, 0, 0)), Vector((0.69, 0.2, -1.16)), PB + Vector((0, 0, -1.3)),
      PB + Vector((0, 0, -1.16))], 0.045, 0.06),
    # Fuel volute -> nozzle coolant inlet ring.
    ([cyl(0.14, -120, -1.37) + Vector((TPX, 0, 0)), Vector((0.42, -0.22, -1.47)), Vector((0.45, -0.23, -2.0)),
      cyl(R_FM, -26.6, -2.42), cyl(R_FM, -26.6, Z_FM)], 0.055, 0.12),
]
for path, r, fr in lines:
    g.add(osp.sweep(osp.fillet(path, fr, 4), r, 12, False, False))
objs.append(osp.make_object("K19_PropellantLines", g, [mats["lines"]], coll, 40))

# Small lines: boost turbine drive gas (tapped off the hot-gas duct) and its exhaust into the LOX
# line, the hydraulic drive for the fuel boost turbine, kick-stage fuel to the preburner,
# throat-belt fuel feed, and ignition / purge / control lines to the heads.
g = osp.Geo()
small = [
    ([Vector((TPX - 0.05, -0.1, -0.42)), Vector((0.38, -0.34, -0.38)), Vector((0.02, -0.53, -0.42)),
      BL + Vector((0.07, 0.07, -0.46))], 0.024),
    ([BL + Vector((0.03, 0.0, -0.56)), BL + Vector((0.11, 0.07, -0.64)), Vector((0.22, -0.52, -0.56)),
      Vector((0.28, -0.48, -0.48))], 0.02),
    ([cyl(0.14, 120, -1.37) + Vector((TPX, 0, 0)), Vector((0.38, 0.38, -1.3)), Vector((0.08, 0.66, -0.85)),
      BF + Vector((0.04, 0.0, -0.5))], 0.024),
    ([cyl(0.14, 90, -1.3) + Vector((TPX, 0, 0)), Vector((0.56, 0.24, -1.3)), PB + Vector((-0.06, -0.05, -1.1))], 0.016),
    ([Vector((0.43, -0.22, -1.65)), Vector((0.3, -0.12, -1.66)), cyl(wall(Z_T - 0.06) + 0.055, -20, Z_T - 0.06)], 0.022),
    ([PB + Vector((0.0, 0.0, -0.655)), PB + Vector((0.0, 0.0, -0.55)), Vector((0.36, 0.2, -0.55)),
      cyl(0.17, 30, -0.86)], 0.012),
    ([cyl(0.22, -45, -0.95), cyl(0.3, -50, -0.95), cyl(0.33, -70, -0.6), BL + Vector((0.08, 0.1, -0.18))], 0.012),
    ([cyl(0.22, 160, -0.95), cyl(0.34, 160, -0.98), cyl(0.36, 120, -0.62), BF + Vector((0.06, -0.1, -0.16))], 0.012),
    ([cyl(0.2, 190, -1.3), cyl(0.3, 190, -1.3), cyl(0.3, 200, -0.75), cyl(0.12, 200, -0.7)], 0.012),
]
for path, r in small:
    g.add(osp.sweep(osp.fillet(path, 0.05, 3), r, 6 if r < 0.015 else 8, False, False))
objs.append(osp.make_object("K19_ControlLines", g, [mats["small"]], coll, 40))

# Valves: main fuel throttle, preburner LOX valve, boost-pump discharge valves.
g = osp.Geo()
for c, d, r in ((Vector((0.43, -0.22, -1.72)), (0, 0, 1), 0.055), (Vector((0.65, 0.22, -1.2)), (0.1, 0.5, -0.4), 0.045),
                (Vector((0.37, -0.45, -0.6)), (0.3, 0.1, -0.35), 0.07), (Vector((0.2, 0.5, -0.9)), (0.2, -0.05, -0.7), 0.06)):
    g.add(kit.place(osp.lathe([(r - 0.004, -0.07), (r + 0.025, -0.06), (r + 0.035, -0.02), (r + 0.035, 0.02),
                               (r + 0.025, 0.06), (r - 0.004, 0.07)], 14), osp.align_z(c, d)))
    out = Vector((c.x, c.y, 0)).normalized()
    g.add(osp.box((0.07, 0.07, 0.08), bevel=0.01, bevel_segs=1),
          matrix=Matrix.Translation(c + out * (r + 0.06)) @ Matrix.Rotation(math.atan2(out.y, out.x), 4, "Z"))
objs.append(osp.make_object("K19_Valves", g, [mats["valves"]], coll, 40))

# ---------------------------------------------------------------------------- actuators
g = osp.Geo()
for az in (135, 225):
    top, bot = cyl(0.46, az, -0.13), cyl(0.33, az, -1.3)
    kit.actuator(g, top, bot, 0.045, 0.022, 0, 1, 14)
objs.append(osp.make_object("K19_Actuators", g, [mats["act"], mats["black"]], coll, 40))

path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
osp.report(sc, objs, path, [
    ("length (-zmin)", -lo[2], LENGTH),
    ("top z (pivot at 0)", hi[2], 0.0),
    ("exit diameter", 2 * osp.max_radius_near(sc, nozzle, -LENGTH), 2 * RE),
])
print(f"  area ratio {(RE / RT) ** 2:.1f}")
print(f"  envelope radius {osp.max_radius(sc, objs):.4f} (limit {ENVELOPE})")
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=-14)

DEBUG_VIEWS = [("below", -65, -40), ("above", 30, 140), ("side", 0, 90),
               ("top", 10, -40, ((-0.75, -0.75, -1.8), (0.8, 0.75, 0.0)))]
