"""K-1 powerhead shared by the K-1 Booster and K-1 Vacuum generators.

Gas-generator kerolox powerhead (Merlin-1D class): gimbal mount on top, LOX injector dome,
regeneratively cooled chamber, a turbopump beside the chamber at +X, propellant lines and two
gimbal actuators. Each engine script supplies its own bell and turbine-exhaust routing.
"""
import math

from mathutils import Matrix, Vector

import osp_common as osp

# Chamber / throat shared by both variants (metres, nozzle axis along -Z).
RC, Z_INJ = 0.19, -0.35          # chamber radius, injector face
RT, Z_T = 0.125, -0.95           # throat radius and height
CONV_DEG = 30.0
WALL = 0.008                     # regen wall thickness (Solidify)
REGEN_SOLIDIFY = dict(thickness=WALL, material_offset=1)  # slot 0 outside, slot 1 inside

TP_X = 0.33                      # turbopump axis distance from engine axis (at azimuth 0)
TP_R = 0.11
TURBINE_OUTLET = Vector((TP_X, 0.0, -0.80))


def materials():
    m = osp.material
    return {
        "bronze": m("K1_RegenNozzle_OxidisedBronze", "#2f251e", 0.75, 0.48),
        "bronze_in": m("K1_RegenNozzle_SootedInner", "#3a332d", 0.45, 0.6),
        "band": m("K1_NozzleBand_DarkBronze", "#4a3a2c", 0.85, 0.4),
        "steel": m("K1_Injector_Steel", "#b4b4b1", 1.0, 0.3),
        "alu": m("K1_Turbopump_Aluminium", "#cdc9c1", 1.0, 0.38),
        "lines": m("K1_PropellantLines_Steel", "#a3a4a6", 1.0, 0.26),
        "dark": m("K1_GimbalMount_DarkSteel", "#43464b", 0.85, 0.45),
        "act": m("K1_Actuator_Silver", "#d2d3d4", 1.0, 0.25),
        "black": m("K1_Actuator_Black", "#1c1c1e", 0.2, 0.55),
        "hot": m("K1_TurbineExhaust_HeatTinted", "#6e5c4a", 0.85, 0.5),
    }


def chamber_and_bell(re, z_e, theta_n, theta_e, n_bell):
    """Full inner contour exit -> injector, closed at the injector face."""
    prof = osp.nozzle_contour(RC, Z_INJ, RT, Z_T, re, z_e, CONV_DEG, theta_n, theta_e,
                              n_bell=n_bell, r_fillet=0.06)
    return prof + [(0.0, Z_INJ)]


def _actuator(geo, top, bot, body, rod):
    top, bot = Vector(top), Vector(bot)
    d = bot - top
    length, u = d.length, d.normalized()
    side = u.cross(Vector((0, 0, 1))).normalized()
    geo.add(osp.cyl_between(top + u * 0.03, top + u * 0.6 * length, 0.036, 16), body)
    geo.add(osp.cyl_between(top + u * 0.6 * length, top + u * 0.64 * length, 0.028, 16), body)
    geo.add(osp.cyl_between(top + u * 0.62 * length, bot - u * 0.03, 0.016, 12), rod)
    geo.add(osp.box((0.05, 0.05, 0.07), top + u * 0.25 * length - side.cross(u) * 0.035, bevel=0.006,
                    bevel_segs=1), rod)
    for p in (top + u * 0.012, bot - u * 0.012):
        geo.add(osp.cyl_between(p - side * 0.022, p + side * 0.022, 0.021, 12), body)


def build_powerhead(coll, mats, prefix, profile):
    """Everything above/around the chamber (profile: the nozzle contour, for the fuel manifold).
    Returns the list of created objects."""
    objs = []

    # Gimbal mount: top plate at z = 0 (the pivot), gimbal block with trunnions, thrust post.
    g = osp.Geo()
    g.add(osp.lathe([(0.0, -0.032), (0.105, -0.032), (0.115, -0.024), (0.115, -0.006),
                     (0.107, 0.0), (0.0, 0.0)], 48))
    g.add(osp.box((0.15, 0.15, 0.075), (0, 0, -0.07), bevel=0.012, bevel_segs=2))
    for axis in ((1, 0, 0), (0, 1, 0)):
        a = Vector(axis)
        g.add(osp.cyl_between(Vector((0, 0, -0.07)) - a * 0.105, Vector((0, 0, -0.07)) + a * 0.105,
                              0.024, 20))
    for ang in range(4):
        a = math.radians(45 + 90 * ang)
        g.add(osp.cylinder(0.011, -0.04, -0.032, 6), matrix=Matrix.Translation(
            (0.085 * math.cos(a), 0.085 * math.sin(a), 0)))
    g.add(osp.lathe([(0.0, -0.215), (0.095, -0.215), (0.07, -0.13), (0.055, -0.1), (0.0, -0.1)], 32))
    objs.append(osp.make_object(prefix + "_GimbalMount", g, [mats["dark"]], coll, 40))

    # LOX dome / injector head on top of the chamber, with the fuel manifold ring around it.
    g = osp.Geo()
    dome = [(0.0, Z_INJ - 0.004), (0.212, Z_INJ - 0.004), (0.212, Z_INJ + 0.022), (0.196, Z_INJ + 0.022)]
    for k in range(1, 9):
        a = math.radians(90 * k / 8)
        dome.append((0.196 * math.cos(a), Z_INJ + 0.022 + 0.14 * math.sin(a)))
    g.add(osp.lathe(dome, 56))
    g.add(osp.torus(0.205, 0.02, 48, 8, Z_INJ - 0.03))
    for ang in range(12):  # injector flange bolts
        a = math.tau / 12 * ang
        g.add(osp.cylinder(0.008, Z_INJ + 0.02, Z_INJ + 0.032, 6),
              matrix=Matrix.Translation((0.204 * math.cos(a), 0.204 * math.sin(a), 0)))
    objs.append(osp.make_object(prefix + "_InjectorDome", g, [mats["steel"]], coll, 40))

    # Turbopump at +X: pump (with volute) over turbine (with inlet manifold), two inlets on top.
    g = osp.Geo()
    tp = Matrix.Translation((TP_X, 0.0, 0.0))
    g.add(osp.lathe([(0.0, -0.53), (0.075, -0.53), (0.075, -0.515), (TP_R, -0.505),
                     (TP_R, -0.335), (0.098, -0.31), (0.07, -0.298), (0.0, -0.298)], 32), matrix=tp)
    g.add(osp.torus(0.098, 0.034, 32, 10, -0.43), matrix=tp)
    g.add(osp.lathe([(0.0, -0.81), (0.055, -0.81), (0.06, -0.79), (TP_R, -0.745), (TP_R, -0.56),
                     (0.085, -0.54), (0.072, -0.525), (0.0, -0.525)], 32), matrix=tp)
    g.add(osp.torus(0.1, 0.03, 32, 8, -0.6), matrix=tp)
    g.add(osp.lathe([(0.0, -0.305), (0.06, -0.305), (0.06, -0.085), (0.074, -0.08),
                     (0.074, -0.05), (0.0, -0.05)], 32), matrix=tp)                       # LOX inlet
    fuel_in = Matrix.Translation((TP_X + 0.02, -0.135, 0.0))
    g.add(osp.lathe([(0.0, -0.4), (0.045, -0.4), (0.045, -0.1), (0.056, -0.095),
                     (0.056, -0.07), (0.0, -0.07)], 24), matrix=fuel_in)                 # fuel inlet
    gg = Vector((TP_X - 0.02, 0.135, -0.66))                                             # gas generator
    g.add(osp.lathe([(0.0, -0.09), (0.03, -0.09), (0.038, -0.07), (0.038, 0.05), (0.03, 0.07),
                     (0.0, 0.07)], 20), matrix=Matrix.Translation(gg))
    g.add(osp.sweep(osp.fillet([gg + Vector((0, 0, -0.06)), gg + Vector((0, -0.02, -0.1)),
                                Vector((TP_X, 0.05, -0.7))], 0.04), 0.018, 10, False, False))
    objs.append(osp.make_object(prefix + "_Turbopump", g, [mats["alu"]], coll, 40))

    # Propellant lines: LOX pump -> dome, fuel pump -> nozzle manifold (regen inlet).
    g = osp.Geo()
    lox = osp.fillet([Vector((TP_X - 0.07, 0.06, -0.43)), Vector((0.232, 0.065, -0.43)),
                      Vector((0.245, 0.065, -0.27)), Vector((0.12, 0.06, -0.27))], 0.06, 6)
    g.add(osp.sweep(lox, 0.038, 16, False, False))
    for p, d in ((Vector((0.238, 0.065, -0.36)), (0, 0, 1)), (Vector((0.175, 0.062, -0.27)), (-1, 0, 0))):
        g.add(osp.lathe([(0.0, -0.01), (0.05, -0.01), (0.05, 0.01), (0.0, 0.01)], 20),
              matrix=osp.align_z(p, d))
    z_fm = -1.04
    r_fm = osp.interp_r(profile, z_fm) + 0.03
    fuel = osp.fillet([Vector((TP_X - 0.03, -0.09, -0.43)), Vector((0.24, -0.15, -0.5)),
                       Vector((0.215, -0.15, -0.9)), Vector((r_fm * 0.82, -r_fm * 0.57, z_fm + 0.01))], 0.08, 6)
    g.add(osp.sweep(fuel, 0.032, 14, False, False))
    g.add(osp.torus(r_fm, 0.028, 48, 8, z_fm))
    objs.append(osp.make_object(prefix + "_PropellantLines", g, [mats["lines"]], coll, 40))

    # Two gimbal actuators (pitch/yaw, 90 deg apart, opposite the turbopump).
    g = osp.Geo()
    for az in (135, 225):
        a = math.radians(az)
        c, s = math.cos(a), math.sin(a)
        top = Vector((0.3 * c, 0.3 * s, -0.025))
        bot = Vector((0.225 * c, 0.225 * s, -0.63))
        _actuator(g, top, bot, 0, 1)
        g.add(osp.box((0.07, 0.05, 0.05), bevel=0.008, bevel_segs=1), 1,
              matrix=Matrix.Translation((0.2 * c, 0.2 * s, -0.63)) @ Matrix.Rotation(a, 4, "Z"))
    objs.append(osp.make_object(prefix + "_Actuators", g, [mats["act"], mats["black"]], coll, 40))
    return objs

