"""M-2 powerhead shared by the M-2 and M-2 Vacuum generators.

Full-flow staged-combustion methalox powerhead (Raptor 2 class). A compact gimbal mount sits on
a short thrust cone on top of the main injector dome. The oxygen turbopump (+X) and the fuel
turbopump (-X) are mounted high on either side of the dome, each with its preburner hanging
below the turbine. Each turbine's spiral exhaust scroll runs straight into a big hot-gas duct
that curves round the -Y side into the head: the oxygen-rich one into the dome shoulder, the
fuel-rich one into the toroidal manifold around the dome base. The gimbal actuators and main
valves sit on the +Y side, and there are only a few external lines. Each engine script supplies
its own throat and bell.
"""
import math

from mathutils import Matrix, Vector

import osp_common as osp

# Chamber shared by both variants (metres, nozzle axis along -Z). The throat radius is the
# engine's own: it sets the area ratio for the given exit.
RC, Z_INJ = 0.21, -0.74          # chamber radius, injector face
Z_T = -1.16                      # throat height
CONV_DEG = 35.0
WALL = 0.01                      # regen wall thickness (Solidify)
SEGS = 72                        # nozzle / chamber lathe segments

TP_X = 0.50                      # turbopump axes at (+-TP_X, 0): oxygen +X, fuel -X
Z_VOL_OX, Z_VOL_F = -0.24, -0.285  # pump discharge volutes
Z_TURB = -0.43                   # turbine exhaust scroll / hot-gas duct height
Z_PB_TOP, Z_PB_BOT = -0.52, -0.93  # preburner flange top and dome bottom
R_DOME, Z_DOME_TOP = 0.29, -0.30
Z_DOME_EQ = Z_INJ + 0.1          # where the dome's crown starts
R_FRM, r_FRM, Z_FRM = 0.31, 0.066, Z_INJ + 0.115  # fuel-rich hot-gas manifold torus
Z_LOX_X = -0.86                  # LOX crossover height


def valve_centre(s):
    """Main valve body on the +Y side of the preburner at x = s * TP_X."""
    return Vector((s * (TP_X + 0.07), 0.2, -0.7))


def materials():
    m = osp.material
    return {
        "bell": m("M2_RegenNozzle_HeatTintedSteel", "#4a3f37", 0.9, 0.3),
        "bell_in": m("M2_RegenNozzle_HeatTintedInner", "#5b5048", 0.8, 0.45),
        "chamber": m("M2_Chamber_Steel", "#a6a6a2", 1.0, 0.3),
        "chamber_in": m("M2_ChamberLiner_Copper", "#8a5a3c", 1.0, 0.4),
        "band": m("M2_ChamberJacket_Inconel", "#9a9790", 1.0, 0.32),
        "dome": m("M2_InjectorDome_Steel", "#bcbbb7", 1.0, 0.26),
        "tp": m("M2_Turbopump_Inconel", "#c7c4bc", 1.0, 0.3),
        "pb": m("M2_Preburner_Inconel", "#aeaba3", 1.0, 0.34),
        "duct": m("M2_HotGasDuct_HeatTinted", "#a39482", 0.95, 0.32),
        "valve": m("M2_Valve_Aluminium", "#cfccc5", 1.0, 0.38),
        "lines": m("M2_Lines_Steel", "#a9aaac", 1.0, 0.26),
        "gimbal": m("M2_GimbalMount_Steel", "#8c8f94", 1.0, 0.36),
        "act": m("M2_Actuator_Silver", "#d2d3d4", 1.0, 0.25),
        "black": m("M2_Actuator_Black", "#1c1c1e", 0.2, 0.55),
    }


def chamber_and_bell(rt, re, z_e, theta_n, theta_e, n_bell, bell_bias=1.0):
    """Inner contour exit -> injector face (open at the top; the dome closes it)."""
    return osp.nozzle_contour(RC, Z_INJ, rt, Z_T, re, z_e, CONV_DEG, theta_n, theta_e,
                              n_bell=n_bell, n_throat=2, r_fillet=0.07, bell_bias=bell_bias)


def segment_slots(profile, segs, slot_of_z):
    """Per-face material slots for a lathe of `profile`: slot_of_z(mid-height of each row)."""
    slots = []
    for (r0, z0), (r1, z1) in zip(profile, profile[1:]):
        slots += [slot_of_z(0.5 * (z0 + z1))] * segs
    return slots


def bolt_ring(g, center, R, n, r_bolt, z0, z1, slot=0, phase=0.0):
    cx, cy = center
    for k in range(n):
        a = phase + math.tau * k / n
        g.add(osp.lathe([(r_bolt, z0), (r_bolt, z1), (0.0, z1)], 6), slot,   # hex head, base buried
              matrix=Matrix.Translation((cx + R * math.cos(a), cy + R * math.sin(a), 0)))


def spiral(cx, z, Rc, r0, r1, theta_exit, hand, sweep=330.0, n=14):
    """Centreline and tube radii of a spiral volute/scroll round the vertical axis through
    (cx, 0): the tube grows from r0 to r1 and leaves tangentially at theta_exit (degrees).
    hand=+1 runs clockwise seen from above, -1 anticlockwise."""
    pts, radii = [], []
    for k in range(n + 1):
        f = k / n
        th = math.radians(theta_exit + hand * sweep * (1 - f))
        pts.append(Vector((cx + Rc * math.cos(th), Rc * math.sin(th), z)))
        radii.append(r0 + (r1 - r0) * f ** 0.8)
    th = math.radians(theta_exit)
    tangent = Vector((hand * math.sin(th), -hand * math.cos(th), 0.0))
    return pts, radii, tangent


def chamber_details(coll, mats, prefix, profile):
    """Structural jacket round the chamber with a raised rib, the throat jacket, and the regen
    outlet manifold at the top of the chamber."""
    g = osp.Geo()
    r = RC + 0.026
    # Open profiles: the inner faces would be buried in the chamber wall.
    g.add(osp.lathe([(RC - 0.004, -0.965), (RC + 0.006, -0.965), (r, -0.945), (r, -0.9),
                     (r + 0.008, -0.894), (r + 0.008, -0.872), (r, -0.866), (r, -0.815),
                     (r - 0.01, -0.81), (RC - 0.004, -0.81)], 40))
    g.add(osp.lathe(osp.band(profile, Z_T + 0.08, 0.135, 0.018), 40))
    g.add(osp.torus(RC + 0.03, 0.026, 36, 6, Z_INJ - 0.05))
    return osp.make_object(prefix + "_ChamberJacket", g, [mats["band"]], coll, 40)


def _actuator(g, top, bot, body, rod):
    """Electromechanical actuator: screw body with the motor alongside, rod, clevis ends."""
    top, bot = Vector(top), Vector(bot)
    d = bot - top
    length, u = d.length, d.normalized()
    side = u.cross(Vector((0, 0, 1))).normalized()
    out = side.cross(u).normalized()
    if out.dot(Vector((top.x, top.y, 0))) < 0:
        out = -out
    g.add(osp.cyl_between(top + u * 0.03, top + u * 0.56 * length, 0.042, 16), body)
    g.add(osp.cyl_between(top + u * 0.56 * length, top + u * 0.6 * length, 0.032, 16), body)
    g.add(osp.cyl_between(top + u * 0.58 * length, bot - u * 0.03, 0.019, 12), rod)
    g.add(osp.cyl_between(top + u * 0.08 + out * 0.07, top + u * 0.4 * length + out * 0.07, 0.03, 14), rod)
    g.add(osp.box((0.05, 0.05, 0.07), top + u * 0.05 + out * 0.04, bevel=0.006, bevel_segs=1), rod)
    for p in (top + u * 0.012, bot - u * 0.012):
        g.add(osp.cyl_between(p - side * 0.026, p + side * 0.026, 0.023, 12), body)


def _turbopump_profile(ox):
    """(r, z) lathe profile about the pump axis, bottom (turbine flange) to top (inlet flange)."""
    rf, rn = (0.14, 0.1) if ox else (0.122, 0.086)
    prof = [(0.0, Z_PB_TOP), (0.13, Z_PB_TOP), (0.13, -0.505), (0.142, -0.497), (0.16, -0.48),
            (0.16, -0.36), (0.168, -0.355), (0.168, -0.338), (0.125, -0.338), (0.125, -0.325)]
    if ox:
        prof += [(0.15, -0.31), (0.172, -0.28), (0.172, -0.2), (0.16, -0.16), (0.13, -0.135)]
    else:
        prof += [(0.142, -0.315), (0.158, -0.3), (0.158, -0.16), (0.122, -0.132)]
    # Inlet neck with a short bellows, then the inlet flange at the top.
    prof += [(rn, -0.12), (rn + 0.008, -0.108), (rn, -0.096), (rn + 0.008, -0.084), (rn, -0.072),
             (rn, -0.062), (rf, -0.062), (rf, -0.03), (0.0, -0.03)]
    return prof


def build_powerhead(coll, mats, prefix, profile, z_fm, r_fm):
    """Everything above and around the chamber. z_fm, r_fm: the regen inlet manifold on the
    nozzle that the fuel pump discharge line runs down to. Returns the created objects."""
    objs = []

    # Gimbal mount: top plate at z = 0 (the pivot), gimbal cross with trunnions, short thrust
    # cone with gussets down onto the injector dome.
    g = osp.Geo()
    g.add(osp.lathe([(0.0, -0.032), (0.128, -0.032), (0.138, -0.024), (0.138, -0.007),
                     (0.13, 0.0), (0.0, 0.0)], 32))
    bolt_ring(g, (0, 0), 0.11, 8, 0.011, -0.042, -0.032, phase=math.pi / 8)
    g.add(osp.box((0.15, 0.15, 0.07), (0, 0, -0.072), bevel=0.014, bevel_segs=1))
    for axis in ((1, 0, 0), (0, 1, 0)):
        a = Vector(axis)
        c = Vector((0, 0, -0.072))
        g.add(osp.cyl_between(c - a * 0.105, c + a * 0.105, 0.026, 16))
        g.add(osp.cyl_between(c - a * 0.118, c + a * 0.118, 0.016, 10))
    zb = Z_DOME_TOP - 0.025
    g.add(osp.lathe([(0.0, zb), (0.17, zb), (0.17, zb + 0.022), (0.145, zb + 0.032), (0.09, -0.14),
                     (0.075, -0.115), (0.0, -0.115)], 32))
    for k in range(4):
        a = math.radians(45 + 90 * k)
        rot = Matrix.Rotation(a, 4, "Z") @ Matrix.Rotation(math.pi / 2, 4, "X")
        rib = osp.prism([(0.07, -0.13), (0.12, zb + 0.02), (0.168, zb + 0.02), (0.1, -0.13)],
                        -0.008, 0.008)
        g.add(rib, matrix=rot)
    objs.append(osp.make_object(prefix + "_GimbalMount", g, [mats["gimbal"]], coll, 40))

    # Main injector dome: injector flange, superellipse hot-gas dome (oxygen-rich side), the
    # fuel-rich manifold torus round its base, instrumentation bosses.
    g = osp.Geo()
    dome = [(0.0, Z_INJ - 0.004), (0.31, Z_INJ - 0.004), (0.32, Z_INJ + 0.006), (0.32, Z_INJ + 0.04),
            (0.306, Z_INJ + 0.05), (R_DOME, Z_INJ + 0.05)]
    b_r = Z_DOME_TOP - Z_DOME_EQ
    for k in range(7):
        t = math.radians(90 * k / 7)
        dome.append((R_DOME * math.cos(t) ** (2 / 3), Z_DOME_EQ + b_r * math.sin(t) ** (2 / 3)))
    dome.append((0.0, Z_DOME_TOP))
    g.add(osp.lathe(dome, 40))
    bolt_ring(g, (0, 0), 0.307, 16, 0.009, Z_INJ + 0.035, Z_INJ + 0.058)
    g.add(osp.torus(R_FRM, r_FRM, 40, 8, Z_FRM))
    for az, z in ((60, -0.4), (120, -0.4), (-90, -0.36)):
        a = math.radians(az)
        p = Vector((0.2 * math.cos(a), 0.2 * math.sin(a), z))
        g.add(osp.cyl_between(p, p + Vector((math.cos(a), math.sin(a), 0.9)).normalized() * 0.06, 0.014, 8))
    objs.append(osp.make_object(prefix + "_InjectorDome", g, [mats["dome"]], coll, 40))

    # Turbopumps: inlet flange and bellows, pump with its spiral discharge volute (the oxygen
    # pump's leaves for its main valve on +Y, the fuel pump's for the nozzle on -Y), turbine.
    # The fuel pump is a taller two-stage pump with an interstage ring.
    g = osp.Geo()
    for s, ox in ((1, True), (-1, False)):
        x = s * TP_X
        tp = Matrix.Translation((x, 0.0, 0.0))
        g.add(osp.lathe(_turbopump_profile(ox), 24), matrix=tp)
        if ox:
            pts, radii, t = spiral(x, Z_VOL_OX, 0.162, 0.03, 0.068, 100.0, 1)
            tail = [pts[-1] + t * 0.07, Vector((x + 0.1, 0.25, Z_VOL_OX - 0.06)),
                    Vector((x + 0.1, 0.25, -0.58)), valve_centre(s) + Vector((0.0, 0.0, 0.05))]
        else:
            g.add(osp.torus(0.152, 0.042, 28, 6, -0.19), matrix=tp)
            pts, radii, t = spiral(x, Z_VOL_F, 0.158, 0.03, 0.064, -150.0, -1)
            a_in = math.radians(-150)
            tail = [pts[-1] + t * 0.06, Vector((-0.61, -0.28, -0.4)), Vector((-0.58, -0.3, -0.75)),
                    Vector(((r_fm + 0.02) * math.cos(a_in), (r_fm + 0.02) * math.sin(a_in), z_fm + 0.2)),
                    Vector((r_fm * math.cos(a_in), r_fm * math.sin(a_in), z_fm + 0.02))]
        line = osp.fillet(pts[-2:] + tail, 0.12, 4)[1:]
        r_tail = 0.04
        n_tail = len(line) - 1
        g.add(osp.sweep(pts[:-1] + line, radii[:-1] + [radii[-1] + (r_tail - radii[-1]) * min(1.0, 2 * k / n_tail)
                                                      for k in range(len(line))], 12, True, False))
        bolt_ring(g, (x, 0), (0.14 if ox else 0.122) - 0.016, 6, 0.008, -0.03, -0.022)
    objs.append(osp.make_object(prefix + "_Turbopumps", g, [mats["tp"]], coll, 40))

    # Preburners under the turbines, with their main valves (+Y side) and igniters.
    g = osp.Geo()
    for s in (1, -1):
        x = s * TP_X
        pb = Matrix.Translation((x, 0.0, 0.0))
        g.add(osp.lathe([(0.0, Z_PB_BOT), (0.04, Z_PB_BOT), (0.072, Z_PB_BOT + 0.016),
                         (0.096, Z_PB_BOT + 0.05), (0.104, Z_PB_BOT + 0.1), (0.11, -0.57),
                         (0.136, -0.562), (0.136, Z_PB_TOP - 0.002), (0.0, Z_PB_TOP - 0.002)], 22),
              matrix=pb)
        g.add(osp.torus(0.112, 0.022, 18, 5, -0.62), matrix=pb)            # preburner injector manifold
        bolt_ring(g, (x, 0), 0.124, 6, 0.007, -0.57, -0.562, phase=0.2)
        g.add(osp.cylinder(0.016, Z_PB_BOT - 0.04, Z_PB_BOT + 0.01, 8),
              matrix=Matrix.Translation((x + s * 0.03, 0.0, 0)))           # igniter
        vc = valve_centre(s)
        g.add(osp.box((0.1, 0.09, 0.11), vc, bevel=0.012, bevel_segs=1), 1)
        g.add(osp.cyl_between(vc + Vector((0, 0, -0.055)), vc + Vector((0, 0, -0.16)), 0.034, 14), 1)
        g.add(osp.cyl_between(vc + Vector((0, 0, -0.16)), vc + Vector((0, 0, -0.175)), 0.026, 14), 1)
        g.add(osp.cyl_between(vc + Vector((-s * 0.04, -0.03, 0.0)), Vector((x, 0.07, -0.7)), 0.03, 12), 1)
    objs.append(osp.make_object(prefix + "_Preburners", g, [mats["pb"], mats["valve"]], coll, 40))

    # Hot-gas ducts: each turbine's spiral exhaust scroll leaves tangentially (-Y side) and
    # curves into the head. The oxygen-rich duct is the bigger one.
    g = osp.Geo()
    for s, r_d in ((1, 0.1), (-1, 0.088)):
        x = s * TP_X
        pts, radii, t = spiral(x, Z_TURB, 0.15, 0.03, 0.072, -60.0 if s > 0 else -120.0, s)
        a_end = math.radians(-50 if s > 0 else -130)
        if s > 0:
            end = Vector((0.2 * math.cos(a_end), 0.2 * math.sin(a_end), Z_TURB + 0.03))
            mid = Vector((0.4 * math.cos(a_end), 0.4 * math.sin(a_end), Z_TURB))
        else:
            end = Vector((R_FRM * math.cos(a_end), R_FRM * math.sin(a_end), Z_FRM + 0.02))
            mid = Vector((0.4 * math.cos(a_end), 0.4 * math.sin(a_end), Z_TURB - 0.05))
        duct = osp.fillet([pts[-1], pts[-1] + t * 0.12, mid, end], 0.1, 7)[1:]
        n_d = len(duct)
        g.add(osp.sweep(pts + duct, radii + [radii[-1] + (r_d - radii[-1]) * min(1.0, 1.6 * (k + 1) / n_d)
                                             for k in range(n_d)], 14, True, False))
        # Bolted flange where the duct meets the dome / manifold: the last duct point clear of it.
        if s > 0:
            clear = lambda p: p.xy.length > 0.33
        else:
            clear = lambda p: math.hypot(p.xy.length - R_FRM, p.z - Z_FRM) > r_FRM + 0.05
        i = max(k for k in range(1, n_d - 1) if clear(duct[k]))
        g.add(osp.lathe([(r_d - 0.01, -0.012), (r_d + 0.024, -0.012), (r_d + 0.024, 0.012), (r_d - 0.01, 0.012)],
                        24, closed=True), matrix=osp.align_z(duct[i], duct[i + 1] - duct[i - 1]))
        p = pts[-1] + t * 0.05
        g.add(osp.lathe([(radii[-1] - 0.01, -0.012), (radii[-1] + 0.026, -0.012), (radii[-1] + 0.026, 0.012),
                         (radii[-1] - 0.01, 0.012)], 24, closed=True), matrix=osp.align_z(p, t))
    objs.append(osp.make_object(prefix + "_HotGasDucts", g, [mats["duct"]], coll, 40))

    # Lines: the regen inlet manifold the fuel pump feeds, the regen outlet from the chamber-top
    # manifold into the fuel main valve, and a LOX crossover from the oxygen main valve to the
    # fuel preburner under the actuators.
    g = osp.Geo()
    g.add(osp.torus(r_fm, 0.032, 40, 6, z_fm))
    a = math.radians(150)
    g.add(osp.sweep(osp.fillet([Vector(((RC + 0.03) * math.cos(a), (RC + 0.03) * math.sin(a), Z_INJ - 0.05)),
                                Vector((-0.36, 0.2, Z_INJ - 0.05)), Vector((-0.46, 0.2, -0.72)),
                                Vector((-TP_X - 0.03, 0.2, -0.71))], 0.04, 4),
                    0.03, 12, False, False))
    arc = [Vector((0.4 * math.cos(math.radians(a)), 0.4 * math.sin(math.radians(a)), Z_LOX_X))
           for a in range(30, 151, 15)]
    lox = (osp.fillet([Vector((TP_X + 0.02, 0.2, -0.7)), Vector((0.44, 0.21, -0.8)), arc[0]], 0.06, 4)[:-1] + arc +
           osp.fillet([arc[-1], Vector((-0.43, 0.13, Z_LOX_X)), Vector((-TP_X + 0.05, 0.06, -0.82))], 0.05, 3)[1:])
    g.add(osp.sweep(lox, 0.022, 8, False, False))
    objs.append(osp.make_object(prefix + "_Lines", g, [mats["lines"]], coll, 40))

    # Two gimbal actuators on the +Y side, 90 degrees apart, down to lugs under the injector flange.
    g = osp.Geo()
    for az in (45, 135):
        a = math.radians(az)
        c, s = math.cos(a), math.sin(a)
        top = Vector((0.41 * c, 0.41 * s, -0.03))
        bot = Vector((0.37 * c, 0.37 * s, Z_INJ - 0.03))
        _actuator(g, top, bot, 0, 1)
        g.add(osp.box((0.08, 0.06, 0.05), bevel=0.008, bevel_segs=1), 1,
              matrix=Matrix.Translation((0.335 * c, 0.335 * s, Z_INJ - 0.03)) @ Matrix.Rotation(a, 4, "Z"))
    objs.append(osp.make_object(prefix + "_Actuators", g, [mats["act"], mats["black"]], coll, 40))
    return objs
