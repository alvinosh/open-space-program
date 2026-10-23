"""K-0: electric-pump-fed kerolox booster engine (Rutherford class).

Length 0.9 m, exit diameter 0.30 m, envelope radius 0.22 m. 3D-printed Inconel chamber and
injector (faceted, ribbed outside), a small regeneratively cooled sea-level bell with printed
cooling-channel ribs, and two brushless electric motors (finned housings, end caps, cable
connectors, harnessed cables) driving the propellant pumps beside the chamber, short lines
through the main valves into the head. Small gimbal and two electromechanical actuators. Origin = gimbal pivot at the top centre; exit plane at z = -0.9.
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

NAME, OUT = "k0", "models/engines/k0.glb"
LENGTH, RE, ENVELOPE = 0.9, 0.15, 0.22
RC, Z_INJ, RT, Z_T = 0.056, -0.29, 0.040, -0.42
Z_SPLIT = -0.45           # printed chamber / regen bell joint, just below the throat
WALL = 0.004              # bell wall (Solidify)
JACKET = 0.008            # printed chamber jacket over the contour
CHANNELS = 72             # regen channels in the bell (two lathe segments each)
MOTOR_R = 0.14            # motor-pump axis distance from the engine axis
MOTOR_AZ = (-45.0, -135.0)  # oxidiser pump, fuel pump

sc, coll = osp.new_scene("osp_" + NAME)
m = osp.material
mats = {
    "bell": m("K0_RegenNozzle_HeatTinted", "#8a847b", 0.9, 0.42),
    "soot": m("K0_RegenNozzle_SootedInner", "#2c2926", 0.4, 0.7),
    "ring": m("K0_NozzleRing_Inconel", "#9c978e", 1.0, 0.45),
    "print": m("K0_PrintedChamber_Inconel", "#a19c93", 1.0, 0.52),
    "inj": m("K0_PrintedInjector_Inconel", "#aaa59c", 1.0, 0.48),
    "gimbal": m("K0_GimbalMount_BlackAnodised", "#1e1f22", 0.35, 0.42),
    "pump": m("K0_PumpHousing_Aluminium", "#c9c7c2", 1.0, 0.34),
    "motor": m("K0_MotorHousing_BlackAnodised", "#202125", 0.35, 0.4),
    "cap": m("K0_MotorEndCap_Aluminium", "#bcbbb7", 1.0, 0.3),
    "conn": m("K0_CableConnector_Silver", "#d0d0ce", 1.0, 0.28),
    "cable": m("K0_MotorCable_Black", "#121214", 0.0, 0.7),
    "lines": m("K0_PropellantLines_Steel", "#b2b3b4", 1.0, 0.26),
    "act": m("K0_Actuator_BlackAnodised", "#1c1d20", 0.35, 0.42),
    "rod": m("K0_ActuatorRod_Silver", "#d4d5d6", 1.0, 0.22),
}
objs = []

prof = osp.nozzle_contour(RC, Z_INJ, RT, Z_T, RE, -LENGTH, 30.0, 36.0, 5.0, n_bell=14, n_throat=3,
                          r_fillet=0.03, lip=0.003, bell_bias=1.3)
bell_prof, upper = osp.split_profile(prof, Z_SPLIT)


def r_at(z):
    return osp.interp_r(prof, z)


def channel_depth(r):
    return 0.09 * math.tau * r / CHANNELS


def channel_ripple(r, z, theta):
    """Printed regen channels: fine ribs with their crests on the contour."""
    return 0.5 * channel_depth(r) * (math.cos(CHANNELS * theta) - 1.0)


bell = osp.make_object("K0_RegenNozzle", osp.Geo().add(osp.lathe(bell_prof, 2 * CHANNELS, ripple=channel_ripple)),
                       [mats["bell"], mats["soot"]], coll, 40,
                       solidify=dict(thickness=WALL, material_offset=1), concave_angle=10)
objs.append(bell)

# Printed rings on the bell: a stiffener partway down and the exit lip ring.
g = osp.Geo()
for z_top, h, proud in ((-0.64, 0.012, 0.004), (-LENGTH + 0.018, 0.015, 0.005)):
    zb = z_top - h
    g.add(osp.lathe([(r_at(zb) - channel_depth(r_at(zb)) - 0.0008, zb), (r_at(zb) + proud, zb),
                     (r_at(z_top) + proud, z_top - 0.003), (r_at(z_top) + 0.001, z_top),
                     (r_at(z_top) - channel_depth(r_at(z_top)) - 0.0008, z_top)], 64))
objs.append(osp.make_object("K0_NozzleRings", g, [mats["ring"]], coll, 30))

# Printed chamber: faceted 24-sided jacket with printed hoop ribs over the converging section
# and chamber, actuator lugs printed on, sooted contour inside. Closed solid.
J = lambda z: r_at(z) + JACKET
outer = [(J(Z_SPLIT), Z_SPLIT)]
z = Z_SPLIT + 0.01
while z + 0.008 < Z_INJ - 0.006:
    outer += [(J(z), z), (J(z) + 0.0035, z + 0.0025), (J(z) + 0.0035, z + 0.0055), (J(z + 0.008), z + 0.008)]
    z += 0.019
outer.append((J(Z_INJ), Z_INJ))
inner = [(r - WALL, zz) for r, zz in reversed(upper)]
segs = 24
verts, faces = osp.lathe(outer + inner, segs, closed=True, phase=math.pi / segs)
seg_mat = [0] * len(outer) + [1] * (len(inner) - 1) + [0]
g = osp.Geo().add((verts, faces), [seg_mat[k // segs] for k in range(len(faces))])
Z_LUG = -0.405
for az in (45, 135):
    a = math.radians(az)
    g.add(osp.box((0.03, 0.02, 0.026), bevel=0.003, bevel_segs=1), 0,
          matrix=Matrix.Translation(((J(Z_LUG) + 0.012) * math.cos(a), (J(Z_LUG) + 0.012) * math.sin(a), Z_LUG))
          @ Matrix.Rotation(a, 4, "Z"))
objs.append(osp.make_object("K0_PrintedChamber", g, [mats["print"], mats["soot"]], coll, 10))

# Printed injector / manifold head: octagonal, faceted, with the injector face at the bottom
# and printed gussets up to the thrust post.
g = osp.Geo()
head = [(0.0, Z_INJ - 0.003), (0.05, Z_INJ - 0.003), (0.05, Z_INJ), (0.074, Z_INJ),
        (0.074, Z_INJ + 0.014), (0.068, Z_INJ + 0.02), (0.068, Z_INJ + 0.058), (0.058, Z_INJ + 0.082),
        (0.04, Z_INJ + 0.104), (0.028, Z_INJ + 0.118), (0.0, Z_INJ + 0.12)]
verts, faces = osp.lathe(head, 8, phase=math.pi / 8)
g.add((verts, faces), [1] * 8 + [0] * (len(faces) - 8))
for k in range(8):
    a = math.tau * (k + 0.5) / 8
    g.add(osp.prism([(0.02, Z_INJ + 0.07), (0.058, Z_INJ + 0.078), (0.03, Z_INJ + 0.15), (0.02, Z_INJ + 0.15)],
                    -0.003, 0.003), 0, matrix=Matrix.Rotation(a, 4, "Z") @ Matrix.Rotation(math.pi / 2, 4, "X"))
for k in range(8):  # printed bolt bosses round the base flange
    a = math.tau * (k + 0.5) / 8
    g.add(osp.cylinder(0.006, Z_INJ + 0.012, Z_INJ + 0.022, 6),
          matrix=Matrix.Translation((0.066 * math.cos(a), 0.066 * math.sin(a), 0)))
objs.append(osp.make_object("K0_PrintedInjector", g, [mats["inj"], mats["soot"]], coll, 10))

# Gimbal mount: pivot plate at z = 0, gimbal block with trunnions, thrust post onto the head.
g = osp.Geo()
g.add(osp.lathe([(0.0, -0.012), (0.045, -0.012), (0.048, -0.009), (0.048, -0.003), (0.045, 0.0),
                 (0.0, 0.0)], 32))
g.add(osp.box((0.05, 0.05, 0.036), (0, 0, -0.03), bevel=0.004, bevel_segs=1))
for axis in ((1, 0, 0), (0, 1, 0)):
    a = Vector(axis)
    g.add(osp.cyl_between(Vector((0, 0, -0.03)) - a * 0.038, Vector((0, 0, -0.03)) + a * 0.038, 0.009, 12))
for k in range(4):
    a = math.radians(45 + 90 * k)
    g.add(osp.cylinder(0.004, -0.016, -0.012, 6), matrix=Matrix.Translation((0.036 * math.cos(a), 0.036 * math.sin(a), 0)))
g.add(osp.lathe([(0.0, Z_INJ + 0.112), (0.03, Z_INJ + 0.112), (0.026, -0.1), (0.02, -0.055), (0.0, -0.055)], 24))
objs.append(osp.make_object("K0_GimbalMount", g, [mats["gimbal"]], coll, 40))

# Motor-pump units: pump (axial inlet up to the stage flange, volute) over a brushless motor
# with a finned housing, end caps and cable connectors on the bottom cap.
FINS = 20
Z_MT, Z_MB = -0.122, -0.33   # motor housing top / bottom


def fin_ripple(r, z, theta):
    j = round(theta / (math.tau / (4 * FINS))) % 4
    return 0.009 if j in (0, 1) else 0.0


pump, motor, cap, conn, cable, lines, valves = (osp.Geo() for _ in range(7))
for idx, az in enumerate(MOTOR_AZ):
    a = math.radians(az)
    u = Vector((math.cos(a), math.sin(a), 0.0))        # radial
    t = Vector((-math.sin(a), math.cos(a), 0.0)) * (1 if idx == 0 else -1)  # tangential, mirrored
    c = u * MOTOR_R
    T = Matrix.Translation(c)
    pump.add(osp.lathe([(0.0, -0.108), (0.03, -0.108), (0.037, -0.1), (0.037, -0.064), (0.028, -0.054),
                        (0.017, -0.05), (0.017, -0.026), (0.026, -0.024), (0.026, -0.01), (0.0, -0.01)], 24),
             matrix=T)
    pump.add(osp.torus(0.035, 0.012, 24, 8, -0.08), matrix=T)
    for k in range(6):
        b = math.tau * k / 6
        pump.add(osp.cylinder(0.003, -0.026, -0.02, 6),
                 matrix=Matrix.Translation(c + Vector((0.021 * math.cos(b), 0.021 * math.sin(b), 0))))
    # Motor: finned housing with caps.
    motor.add(osp.lathe([(0.04, Z_MB - 0.002), (0.04, Z_MT + 0.002)], 4 * FINS, ripple=fin_ripple), matrix=T)
    for zc, sgn in ((Z_MT, 1), (Z_MB, -1)):
        prof_cap = [(0.0, zc - 0.006 * sgn), (0.05, zc - 0.006 * sgn), (0.052, zc), (0.052, zc + 0.01 * sgn),
                    (0.046, zc + 0.016 * sgn), (0.0, zc + 0.016 * sgn)]
        if sgn < 0:
            prof_cap = [(r, zz) for r, zz in reversed(prof_cap)]
        cap.add(osp.lathe(prof_cap, 32), matrix=T)
        face = zc + 0.016 * sgn
        for k in range(6):
            b = math.tau * (k + 0.5) / 6
            cap.add(osp.cylinder(0.0032, min(face, face + 0.003 * sgn), max(face, face + 0.003 * sgn), 6),
                    matrix=Matrix.Translation(c + Vector((0.044 * math.cos(b), 0.044 * math.sin(b), 0))))
    # Connectors on the bottom cap (power and sensor), their cables bundled up the outboard side
    # of the housing in two clamps and on up to the stage.
    zb = Z_MB - 0.016
    for off, rc_, rk in ((-0.009, 0.0105, 0.0065), (0.011, 0.0075, 0.0045)):
        p = c + t * off + u * 0.026
        conn.add(osp.cylinder(rc_, zb - 0.02, zb, 10), matrix=Matrix.Translation(p))
        conn.add(osp.lathe([(0.0, zb - 0.028), (rc_ + 0.002, zb - 0.028), (rc_ + 0.002, zb - 0.02),
                            (0.0, zb - 0.02)], 10), 0, matrix=Matrix.Translation(p))
        q = c + u * 0.063 + t * off * 0.9
        route = [p + Vector((0, 0, zb - 0.028)), p + Vector((0, 0, zb - 0.046)), q + Vector((0, 0, zb - 0.046)),
                 q + Vector((0, 0, -0.07)), q + Vector((0, 0, -0.01))]
        cable.add(osp.sweep(osp.fillet(route, 0.016, 4), rk, 8, False, True))
    for zc, size in ((-0.27, (0.03, 0.04, 0.012)), (-0.17, (0.03, 0.04, 0.012)), (-0.012, (0.02, 0.036, 0.012))):
        cable.add(osp.box(size, bevel=0.002, bevel_segs=1),  # harness clamps, stage bulkhead plug
                  matrix=Matrix.Translation(c + u * (0.058 if zc < -0.1 else 0.063) + t * 0.001 + Vector((0, 0, zc)))
                  @ Matrix.Rotation(a, 4, "Z"))
    # Short line: volute -> over the motor cap -> into the printed head.
    s0 = c - u * 0.033 + Vector((0, 0, -0.08))
    line = osp.fillet([s0 + u * 0.01, u * 0.062 + Vector((0, 0, -0.08)), u * 0.05 + Vector((0, 0, Z_INJ + 0.1))], 0.02, 5)
    lines.add(osp.sweep(line, 0.0105, 12, False, False))
    lines.add(osp.lathe([(0.0, -0.004), (0.016, -0.004), (0.016, 0.004), (0.0, 0.004)], 12),
              matrix=osp.align_z(u * 0.078 + Vector((0, 0, -0.08)), -u))
    # Main propellant valve on the drop into the head, with its small electric operator.
    vc = u * 0.0565 + Vector((0, 0, -0.13))
    valves.add(osp.box((0.026, 0.032, 0.03), bevel=0.003, bevel_segs=1), 0,
               matrix=Matrix.Translation(vc) @ Matrix.Rotation(a, 4, "Z"))
    valves.add(osp.cyl_between(vc + t * 0.014, vc + t * 0.036, 0.0095, 12), 1)
    valves.add(osp.cyl_between(vc + t * 0.036, vc + t * 0.04, 0.006, 8), 0)
objs += [osp.make_object("K0_PumpHousings", pump, [mats["pump"]], coll, 40),
         osp.make_object("K0_MotorHousings", motor, [mats["motor"]], coll, 30),
         osp.make_object("K0_MotorEndCaps", cap, [mats["cap"]], coll, 40),
         osp.make_object("K0_CableConnectors", conn, [mats["conn"]], coll, 40),
         osp.make_object("K0_MotorCables", cable, [mats["cable"]], coll, 50),
         osp.make_object("K0_PropellantLines", lines, [mats["lines"]], coll, 40),
         osp.make_object("K0_MainValves", valves, [mats["inj"], mats["gimbal"]], coll, 40)]

# Two electromechanical actuators (body with a side-mounted motor, rod) down to the chamber lugs.
g = osp.Geo()
for az in (45, 135):
    a = math.radians(az)
    cs, sn = math.cos(a), math.sin(a)
    top = Vector((0.17 * cs, 0.17 * sn, -0.026))
    bot = Vector(((J(Z_LUG) + 0.02) * cs, (J(Z_LUG) + 0.02) * sn, Z_LUG))
    d = (bot - top).normalized()
    length = (bot - top).length
    side = d.cross(Vector((0, 0, 1))).normalized()
    outw = side.cross(d).normalized()
    g.add(osp.cyl_between(top + d * 0.012, top + d * 0.56 * length, 0.016, 16), 0)
    g.add(osp.cyl_between(top + d * 0.56 * length, top + d * 0.6 * length, 0.012, 16), 0)
    g.add(osp.cyl_between(top + d * 0.58 * length, bot - d * 0.01, 0.0065, 10), 1)
    mo = outw * 0.024 if outw.dot(Vector((cs, sn, 0))) > 0 else -outw * 0.024
    g.add(osp.cyl_between(top + d * 0.05 + mo, top + d * 0.36 * length + mo, 0.011, 12), 0)
    g.add(osp.box((0.02, 0.03, 0.02), top + d * 0.08 + mo * 0.5, bevel=0.003, bevel_segs=1), 0)
    for p in (top + d * 0.004, bot - d * 0.004):
        g.add(osp.cyl_between(p - side * 0.012, p + side * 0.012, 0.009, 10), 1)
    g.add(osp.box((0.03, 0.03, 0.02), (0.17 * cs, 0.17 * sn, -0.01), bevel=0.003, bevel_segs=1), 0)
objs.append(osp.make_object("K0_Actuators", g, [mats["act"], mats["rod"]], coll, 40))

path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
osp.report(sc, objs, path, [
    ("length (-zmin)", -lo[2], LENGTH),
    ("top z (pivot at 0)", hi[2], 0.0),
    ("exit diameter (rib crests)", 2 * osp.max_radius_near(sc, bell, -LENGTH), 2 * RE),
])
print(f"  envelope radius {osp.max_radius(sc, objs):.4f} (limit {ENVELOPE})")
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=-14)

DEBUG_VIEWS = [("below", -65, -40), ("side", 5, 90), ("top", 20, -40, ((-0.22, -0.22, -0.45), (0.22, 0.22, 0.0)))]
