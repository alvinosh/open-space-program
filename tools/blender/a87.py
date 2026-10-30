"""A-87: pump-fed hypergolic booster thrust chamber (LR-87 class, one chamber of the Titan II
first-stage engine).

Length 3.2 m, exit diameter 1.30 m, envelope radius 0.85 m. Long regeneratively cooled tube-wall
bell with prominent hat-section stiffening bands, a gear-driven turbopump (two pumps on a
gearbox, turbine underneath) with its gas generator beside the chamber at +X, and the turbine
exhaust pipe running down beside the bell to an outlet just above the exit, with the oxidiser
pressurisation heat exchanger on it. Fuel enters the tubes through a manifold below the throat.
Origin = gimbal pivot at the top centre; nozzle exit plane at z = -3.2.
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

NAME, OUT = "a87", "models/engines/a87.glb"
LENGTH, RE, ENVELOPE = 3.2, 0.65, 0.85
RC, Z_INJ, RT, Z_T = 0.31, -0.62, 0.23, -1.20
TUBES = 72
WALL = 0.010
BAND_SEGS = 48

sc, coll = osp.new_scene("osp_" + NAME)
m = osp.material
mats = {
    "tubes": m("A87_TubeWallNozzle_DarkSteel", "#55585c", 0.9, 0.38),
    "tubes_in": m("A87_TubeWallNozzle_HeatTintedInner", "#7b7368", 0.85, 0.45),
    "bands": m("A87_HatBands_Steel", "#7f8285", 0.95, 0.33),
    "dome": m("A87_InjectorDome_DarkSteel", "#4b4e52", 0.9, 0.4),
    "dark": m("A87_GimbalMount_DarkSteel", "#3d4045", 0.85, 0.45),
    "tpa": m("A87_Turbopump_Gunmetal", "#5c5f63", 0.9, 0.42),
    "gg": m("A87_GasGenerator_Inconel", "#8b857b", 0.9, 0.4),
    "lines": m("A87_PropellantDucts_Steel", "#8f9194", 1.0, 0.3),
    "bellows": m("A87_DuctBellows_Steel", "#b3b4b5", 1.0, 0.25),
    "hot": m("A87_TurbineExhaust_HeatTinted", "#6b5f53", 0.85, 0.5),
    "soot": m("A87_TurbineExhaust_SootedInner", "#2a2725", 0.3, 0.8),
    "act": m("A87_Actuator_Silver", "#cfd0d1", 1.0, 0.25),
    "black": m("A87_Actuator_Black", "#1c1c1e", 0.2, 0.55),
}
objs = []

prof = osp.nozzle_contour(RC, Z_INJ, RT, Z_T, RE, -LENGTH, 30.0, 32.0, 5.0, n_bell=8, n_throat=2,
                          r_fillet=0.12, bell_bias=1.2)


def tube_depth(r):
    return 0.14 * math.tau * r / TUBES


def tube_ripple(r, z, theta):
    """Tube crests sit on the contour (so the exit diameter is exact), valleys below it."""
    return 0.5 * tube_depth(r) * (math.cos(TUBES * theta) - 1.0)


nozzle = osp.make_object("A87_TubeWallNozzle", osp.Geo().add(osp.lathe(prof, 2 * TUBES, ripple=tube_ripple)),
                         [mats["tubes"], mats["tubes_in"]], coll, 45,
                         solidify=dict(thickness=WALL, material_offset=1), concave_angle=12)
objs.append(nozzle)


def r_at(z):
    return osp.interp_r(prof, z)


def hat_band(z_c, h=0.09, proud=0.024, flange=0.016, t=0.005, step=0.008):
    """Hat-section band riding on the tube crests: thin flanges top and bottom, raised crown.
    The open inner side is buried in the tube wall (between the valleys and the inner skin)."""
    zb, zt = z_c - h / 2, z_c + h / 2
    zs = (zb, zb + flange, zb + flange + step, zt - flange - step, zt - flange, zt)
    off = (t, t, proud, proud, t, t)
    pts = [(r_at(zb) - tube_depth(r_at(zb)) - 0.0012, zb)]
    pts += [(r_at(z) + o, z) for z, o in zip(zs, off)]
    pts.append((r_at(zt) - tube_depth(r_at(zt)) - 0.0012, zt))
    return osp.lathe(pts, BAND_SEGS)


# Hat bands down the bell, a band on the chamber carrying the actuator lugs, and the exit ring.
Z_LUG = -0.86
g = osp.Geo()
for z in (-1.50, -1.79, -2.08, -2.37, -2.66, -2.95):
    g.add(hat_band(z))
g.add(hat_band(Z_LUG, h=0.075, proud=0.026))
zb, zt = -LENGTH + 0.004, -LENGTH + 0.07
g.add(osp.lathe([(r_at(zb) - tube_depth(RE) - 0.0012, zb), (RE + 0.016, zb), (RE + 0.016, zt - 0.012),
                 (r_at(zt) + 0.006, zt), (r_at(zt) - tube_depth(RE) - 0.0012, zt)], 48))
for k in range(2):  # actuator lugs on the chamber band (actuators at 135 and 225 deg)
    a = math.radians(135 + 90 * k)
    g.add(osp.box((0.07, 0.08, 0.1), bevel=0.008, bevel_segs=1),
          matrix=Matrix.Translation((0.35 * math.cos(a), 0.35 * math.sin(a), Z_LUG)) @ Matrix.Rotation(a, 4, "Z"))
bands = osp.make_object("A87_HatBands", g, [mats["bands"]], coll, 40)
objs.append(bands)

# Injector and oxidiser dome closing the top of the tube bundle, with its bolted flange.
g = osp.Geo()
dome = [(0.0, Z_INJ - 0.01), (0.348, Z_INJ - 0.01), (0.348, Z_INJ + 0.032), (0.322, Z_INJ + 0.042)]
for k in range(1, 7):
    a = math.radians(90 * k / 6)
    dome.append((0.322 * math.cos(a), Z_INJ + 0.042 + 0.17 * math.sin(a)))
g.add(osp.lathe(dome, 40))
for k in range(12):
    a = math.tau * (k + 0.5) / 12
    g.add(osp.cylinder(0.011, Z_INJ + 0.03, Z_INJ + 0.05, 6),
          matrix=Matrix.Translation((0.336 * math.cos(a), 0.336 * math.sin(a), 0)))
objs.append(osp.make_object("A87_InjectorDome", g, [mats["dome"]], coll, 40))

# Gimbal mount: pivot plate at z = 0, gimbal block with trunnions, thrust cone onto the dome.
g = osp.Geo()
g.add(osp.lathe([(0.0, -0.04), (0.19, -0.04), (0.2, -0.03), (0.2, -0.008), (0.19, 0.0), (0.0, 0.0)], 36))
g.add(osp.box((0.2, 0.2, 0.11), (0, 0, -0.1), bevel=0.014, bevel_segs=2))
for axis in ((1, 0, 0), (0, 1, 0)):
    a = Vector(axis)
    g.add(osp.cyl_between(Vector((0, 0, -0.1)) - a * 0.145, Vector((0, 0, -0.1)) + a * 0.145, 0.034, 20))
for k in range(6):
    a = math.tau * (k + 0.5) / 6
    g.add(osp.cylinder(0.014, -0.052, -0.04, 6),
          matrix=Matrix.Translation((0.155 * math.cos(a), 0.155 * math.sin(a), 0)))
g.add(osp.lathe([(0.0, -0.47), (0.2, -0.47), (0.19, -0.44), (0.11, -0.2), (0.09, -0.155), (0.0, -0.155)], 36))
objs.append(osp.make_object("A87_GimbalMount", g, [mats["dark"]], coll, 40))

# Turbopump at +X: gearbox, oxidiser pump (+Y) and fuel pump (-Y) on top with axial inlets,
# turbine with its inlet manifold underneath, solid-propellant start cartridge on the turbine.
OX = Vector((0.57, 0.17, 0.0))
FU = Vector((0.57, -0.17, 0.0))
TB = Vector((0.58, 0.0, 0.0))
g = osp.Geo()
g.add(osp.box((0.19, 0.5, 0.12), (0.565, 0.0, -0.47), bevel=0.03, bevel_segs=2))
for c, r_p in ((OX, 0.088), (FU, 0.08)):
    t = Matrix.Translation(c)
    g.add(osp.lathe([(0.0, -0.415), (r_p + 0.012, -0.415), (r_p + 0.012, -0.395), (r_p, -0.385),
                     (r_p, -0.3), (r_p - 0.02, -0.285), (0.064, -0.255), (0.064, -0.24), (0.0, -0.24)], 24),
          matrix=t)
    g.add(osp.torus(r_p + 0.004, 0.04, 24, 6, -0.345), matrix=t)          # pump volute
g.add(osp.lathe([(0.0, -0.80), (0.075, -0.80), (0.085, -0.76), (0.12, -0.72), (0.12, -0.57),
                 (0.09, -0.545), (0.09, -0.53), (0.0, -0.53)], 28), matrix=Matrix.Translation(TB))
g.add(osp.torus(0.122, 0.034, 28, 6, -0.635), matrix=Matrix.Translation(TB))  # turbine inlet manifold
g.add(osp.cyl_between(TB + Vector((0.06, -0.15, -0.66)), TB + Vector((0.13, -0.27, -0.66)), 0.034, 14))
g.add(osp.cyl_between(TB + Vector((0.12, -0.25, -0.66)), TB + Vector((0.14, -0.285, -0.66)), 0.042, 14))
objs.append(osp.make_object("A87_Turbopump", g, [mats["tpa"]], coll, 40))

# Gas generator beside the turbine (+Y), hot-gas duct into the turbine manifold.
GG = Vector((0.44, 0.27, 0.0))
g = osp.Geo()
g.add(osp.lathe([(0.0, -0.86), (0.03, -0.86), (0.045, -0.845), (0.052, -0.82), (0.052, -0.66),
                 (0.06, -0.655), (0.06, -0.635), (0.045, -0.63), (0.03, -0.61), (0.0, -0.605)], 20),
      matrix=Matrix.Translation(GG))
gg_out = GG + Vector((0.04, -0.035, -0.70))
g.add(osp.sweep(osp.fillet([gg_out, gg_out + Vector((0.035, -0.04, 0.0)),
                            TB + Vector((-0.06, 0.104, -0.635))], 0.04, 4), 0.026, 12, False, False))
objs.append(osp.make_object("A87_GasGenerator", g, [mats["gg"]], coll, 40))

# Propellant ducts: suction lines up to the stage flanges (with gimbal bellows), oxidiser
# discharge into the dome, fuel discharge down to the regen inlet manifold, GG feed lines.
Z_FM = -1.40
r_fm = r_at(Z_FM) + 0.048
g = osp.Geo()
gb = osp.Geo()
for c, r_d in ((OX, 0.066), (FU, 0.058)):
    t = Matrix.Translation(c)
    g.add(osp.lathe([(0.0, -0.25), (r_d, -0.25), (r_d, -0.07), (r_d + 0.02, -0.065), (r_d + 0.02, -0.035),
                     (0.0, -0.035)], 20), matrix=t)
    g.add(osp.lathe([(r_d + 0.004, -0.21), (r_d + 0.016, -0.21), (r_d + 0.016, -0.19),
                     (r_d + 0.004, -0.19)], 20, closed=True), matrix=t)
    bel = [(r_d - 0.002, -0.165)]
    for k in range(4):
        z = -0.165 + 0.02 * k
        bel += [(r_d + 0.014, z + 0.01), (r_d + 0.003, z + 0.02)]
    bel[-1] = (r_d - 0.002, -0.085)
    gb.add(osp.lathe(bel, 20), matrix=t)
# Oxidiser discharge: volute -> dome side inlet.
a_ox = math.radians(32)
ox_end = Vector((0.27 * math.cos(a_ox), 0.27 * math.sin(a_ox), -0.535))
ox = osp.fillet([OX + Vector((-0.02, 0.122, -0.345)), OX + Vector((-0.17, 0.14, -0.345)),
                 ox_end + Vector((0.08, 0.04, 0.04)), ox_end], 0.07, 4)
g.add(osp.sweep(ox, 0.05, 14, False, False))
g.add(osp.lathe([(0.0, -0.012), (0.066, -0.012), (0.066, 0.012), (0.0, 0.012)], 20),
      matrix=osp.align_z(ox[len(ox) // 2], Vector(ox[len(ox) // 2 + 1]) - Vector(ox[len(ox) // 2 - 1])))
# Fuel discharge: volute -> down beside the chamber -> regen inlet manifold below the throat.
a_fm = math.radians(-38)
fm_in = Vector((r_fm * math.cos(a_fm), r_fm * math.sin(a_fm), Z_FM + 0.02))
fuel = osp.fillet([FU + Vector((-0.02, -0.118, -0.345)), FU + Vector((-0.08, -0.17, -0.40)),
                   Vector((0.46, -0.34, -0.95)), fm_in + Vector((0.03, -0.03, 0.12)), fm_in], 0.1, 4)
g.add(osp.sweep(fuel, 0.046, 14, False, False))
g.add(osp.torus(r_fm, 0.042, 48, 6, Z_FM))
# GG feed lines (oxidiser from the ox volute, fuel from the fuel discharge).
g.add(osp.sweep(osp.fillet([OX + Vector((0.05, 0.11, -0.37)), OX + Vector((-0.05, 0.13, -0.56)),
                            GG + Vector((0.0, 0.0, -0.6))], 0.04, 4), 0.012, 8, False, False))
g.add(osp.sweep(osp.fillet([Vector((0.47, -0.33, -0.86)), Vector((0.36, -0.1, -0.88)),
                            Vector((0.36, 0.2, -0.78)), GG + Vector((-0.04, -0.02, -0.66))], 0.05, 4),
                0.012, 8, False, False))
objs.append(osp.make_object("A87_PropellantDucts", g, [mats["lines"]], coll, 40))
objs.append(osp.make_object("A87_DuctBellows", gb, [mats["bellows"]], coll, 30))

# Turbine exhaust pipe: turbine outlet down beside the bell (clear of the hat bands), turning
# vertical for the last stretch to a short flared outlet pointing straight down (-Z) just above
# the exit ring, with the oxidiser heat exchanger on it. The outlet is an open end: the game
# draws the turbine exhaust plume from it (EXHAUST_OUTLET: mouth centre, inner diameter).
R_EX = 0.072
EX_X = 0.75
P0, P1 = Vector((0.58, 0.0, -0.79)), Vector((0.585, 0.0, -1.06))
P2, P3 = Vector((EX_X, 0.0, -2.70)), Vector((EX_X, 0.0, -3.03))
duct = osp.fillet([P0, P1, P2, P3], 1.5, 2)
for k in range(61):  # pipe clear of the bell and its hat bands
    p = P1.lerp(P2, k / 30) if k <= 30 else P2.lerp(P3, (k - 30) / 30)
    assert p.xy.length - R_EX - 0.01 > r_at(p.z) + 0.024, (p, r_at(p.z))
assert EX_X - (R_EX - 0.004) > RE + 0.016 + 0.01  # plume clear of the exit ring
g = osp.Geo().add(osp.sweep(duct, R_EX, 18, cap0=True, cap1=False))
down = Vector((0, 0, -1))
FLARE = 0.07
g.add(osp.lathe([(R_EX, 0.0), (R_EX + 0.003, 0.025), (R_EX + 0.009, 0.045), (R_EX + 0.016, 0.06),
                 (R_EX + 0.016, FLARE), (R_EX + 0.006, FLARE)], 18), matrix=osp.align_z(P3, down))
g.add(osp.cyl_between(P0 + Vector((0, 0, -0.03)), P0 + Vector((0, 0, -0.055)), R_EX + 0.016, 18))
d = (P2 - P1).normalized()
hx_c = P1 + d * 0.62
g.add(osp.lathe([(R_EX, -0.24), (0.1, -0.2), (0.106, -0.19), (0.106, 0.19), (0.1, 0.2), (R_EX, 0.24)], 18),
      matrix=osp.align_z(hx_c, d))
for s in (-0.16, 0.16):
    g.add(osp.cyl_between(hx_c + d * s + Vector((0, 0.0, 0)), hx_c + d * s + Vector((0, 0.13, 0)), 0.016, 8))
g.add(osp.sweep(osp.fillet([hx_c + d * -0.16 + Vector((0, 0.12, 0)), Vector((0.66, 0.2, -0.95)),
                            Vector((0.72, 0.3, -0.6)), Vector((0.72, 0.3, -0.06))], 0.08, 3), 0.014, 8, False, True))
g.add(osp.cylinder(0.026, -0.075, -0.05, 10), matrix=Matrix.Translation((0.72, 0.3, 0)))
objs.append(osp.make_object("A87_TurbineExhaust", g, [mats["hot"]], coll, 40))
# Sooted bore inside the outlet (the dark interior seen from below), closed well up the pipe.
g = osp.Geo().add(osp.lathe([(R_EX + 0.006, FLARE), (R_EX - 0.004, 0.0), (R_EX - 0.004, -0.22), (0.0, -0.22)], 18),
                  matrix=osp.align_z(P3, down))
objs.append(osp.make_object("A87_TurbineExhaustOutlet", g, [mats["soot"]], coll, 40))
EXHAUST_OUTLET = (P3 + down * FLARE, 2 * (R_EX + 0.006), 2 * (R_EX - 0.004))

# Two gimbal actuators (pitch/yaw, 90 deg apart, opposite the turbopump) down to the chamber lugs.
g = osp.Geo()
for az in (135, 225):
    a = math.radians(az)
    c, s = math.cos(a), math.sin(a)
    top, bot = Vector((0.47 * c, 0.47 * s, -0.04)), Vector((0.375 * c, 0.375 * s, Z_LUG))
    u = (bot - top).normalized()
    length = (bot - top).length
    side = u.cross(Vector((0, 0, 1))).normalized()
    g.add(osp.cyl_between(top + u * 0.04, top + u * 0.56 * length, 0.046, 18), 0)
    g.add(osp.cyl_between(top + u * 0.56 * length, top + u * 0.6 * length, 0.036, 18), 0)
    g.add(osp.cyl_between(top + u * 0.58 * length, bot - u * 0.03, 0.02, 12), 1)
    g.add(osp.box((0.06, 0.06, 0.1), top + u * 0.24 * length - side.cross(u) * 0.05, bevel=0.008,
                  bevel_segs=1), 1)
    for p in (top + u * 0.015, bot - u * 0.015):
        g.add(osp.cyl_between(p - side * 0.03, p + side * 0.03, 0.026, 12), 0)
    g.add(osp.box((0.09, 0.08, 0.035), (0.47 * c, 0.47 * s, -0.0175), bevel=0.006, bevel_segs=1), 1)
objs.append(osp.make_object("A87_Actuators", g, [mats["act"], mats["black"]], coll, 40))

path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
osp.report(sc, objs, path, [
    ("length (-zmin)", -lo[2], LENGTH),
    ("top z (pivot at 0)", hi[2], 0.0),
    ("exit diameter (tube crests)", 2 * osp.max_radius_near(sc, nozzle, -LENGTH), 2 * RE),
])
print(f"  envelope radius {osp.max_radius(sc, objs):.4f} (limit {ENVELOPE})")
c_ex, d_mouth, d_bore = EXHAUST_OUTLET
print(f"  turbine exhaust outlet: mouth centre ({c_ex.x:.4f}, {c_ex.y:.4f}, {c_ex.z:.4f}), "
      f"inner diameter {d_mouth:.4f} at the mouth ({d_bore:.4f} bore), pointing (0, 0, -1)")
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=-14)

DEBUG_VIEWS = [("below", -65, -40), ("side", 0, 0), ("top", 20, -40, ((-0.85, -0.85, -1.4), (0.85, 0.85, 0.0)))]
