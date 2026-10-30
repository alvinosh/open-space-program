"""H-25: staged-combustion hydrolox main engine (RS-25 / SSME class).

Length 4.27 m, exit diameter 2.30 m (area ratio ~69), envelope radius 1.25 m.
Powerhead: gimbal bearing on the main injector LOX dome, the hot-gas manifold around the main
injector with the fuel (+X) and oxidiser (-X) preburners domed on top of it, and the high-pressure
fuel and oxidiser turbopumps hanging below it on either side of the short main combustion chamber.
Low-pressure pumps near the top feed the high-pressure pumps through big ducts. The tube-wall bell
joins the chamber at area ratio 5; the steerhorn duct carries coolant from the main fuel valve down
and around the bell to the coolant inlet manifold, and three drop lines take it on down to the aft
manifold at the exit. Origin = gimbal pivot at the top centre; exit plane at z = -4.27.
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

NAME, OUT = "h25", "models/engines/h25.glb"
LENGTH, RE, ENVELOPE = 4.27, 1.15, 1.25
RT = RE / math.sqrt(69.0)
RC, Z_INJ, Z_T = 0.235, -0.80, -1.16
Z_JOINT = -1.43            # main combustion chamber / nozzle joint (area ratio ~5)
TUBES = 72
AMP = 0.5 * 0.16 * math.tau / TUBES   # tube crest height, as a fraction of the local radius
FTP = Vector((0.72, 0.0, 0.0))        # high-pressure fuel turbopump axis
OTP = Vector((-0.70, 0.0, 0.0))       # high-pressure oxidiser turbopump axis

sc, coll = osp.new_scene("osp_" + NAME)
m = osp.material
mats = {
    "tubes": m("H25_TubeWallNozzle_Inconel", "#cdc9c1", 1.0, 0.3),
    "tubes_in": m("H25_TubeWallNozzle_HeatTintedInner", "#8f877c", 0.85, 0.42),
    "band": m("H25_HatBands_Inconel", "#a39f97", 1.0, 0.36),
    "manifold": m("H25_CoolantManifolds_Steel", "#b6b6b3", 1.0, 0.3),
    "mcc": m("H25_CombustionChamber_Steel", "#aeadaa", 1.0, 0.34),
    "liner": m("H25_ChamberLiner_OxidisedCopper", "#6a4a36", 0.9, 0.42),
    "face": m("H25_InjectorFace_Sooted", "#2a2826", 0.3, 0.75),
    "hgm": m("H25_HotGasManifold_Inconel", "#c4c1ba", 1.0, 0.36),
    "preburner": m("H25_Preburner_Inconel", "#bab6ad", 1.0, 0.33),
    "pump": m("H25_Turbopump_Steel", "#b3b2ae", 1.0, 0.32),
    "ducts": m("H25_PropellantDucts_Steel", "#a6a7a9", 1.0, 0.27),
    "valve": m("H25_Valves_Aluminium", "#cbc8c1", 1.0, 0.38),
    "dark": m("H25_GimbalBearing_DarkSteel", "#46494e", 0.85, 0.45),
    "act": m("H25_Actuator_Silver", "#d2d3d4", 1.0, 0.25),
    "black": m("H25_Actuator_Black", "#1c1c1e", 0.2, 0.55),
}
objs = []

prof = osp.nozzle_contour(RC, Z_INJ, RT, Z_T, RE, -LENGTH, 30.0, 38.0, 5.5, n_bell=12, n_throat=2,
                          r_fillet=0.08, bell_bias=1.3)
bell, chamber = osp.split_profile(prof, Z_JOINT)
r_joint = bell[-1][0]


def r_bell(z):
    return osp.interp_r(bell, z)


def cyl_pt(az_deg, r, z):
    a = math.radians(az_deg)
    return Vector((r * math.cos(a), r * math.sin(a), z))


def tube_ripple(r, z, theta):
    """Brazed tube bundle: crest/valley alternation; the exit lip ring stays round."""
    if z < -LENGTH + 0.003:
        return 0.0
    return AMP * r * math.cos(TUBES * theta)


def disc(p, d, r, h, segs=12):
    """Flange disc of radius r and thickness h centred at p, axis d."""
    verts, faces = osp.lathe([(0.0, -h / 2), (r, -h / 2), (r, h / 2), (0.0, h / 2)], segs)
    mtx = osp.align_z(p, d)
    return [tuple(mtx @ Vector(v)) for v in verts], faces


def bolts(geo, center, r, n, z0, z1, mat=0, rb=0.009):
    for k in range(n):
        a = math.tau * (k + 0.5) / n
        geo.add(osp.cylinder(rb, z0, z1, 6, cap0=False), mat,
                matrix=Matrix.Translation(Vector(center) + Vector((r * math.cos(a), r * math.sin(a), 0))))


# ------------------------------------------------------------------ nozzle
nozzle = osp.make_object("H25_TubeWallNozzle", osp.Geo().add(osp.lathe(bell, 2 * TUBES, ripple=tube_ripple)),
                         [mats["tubes"], mats["tubes_in"]], coll, 45,
                         solidify=dict(thickness=0.009, material_offset=1), concave_angle=12)
objs.append(nozzle)


def hat_band(z_top, h, proud, segs=48):
    """Open band profile (no hidden inner face) clearing the tube crests."""
    zb = z_top - h
    rt, rb = r_bell(z_top), r_bell(zb)
    return osp.lathe([(rb * (1 - AMP) - 0.002, zb), (rb + proud, zb), (rt + proud, z_top),
                      (rt * (1 - AMP) - 0.002, z_top)], segs)


BANDS = (-2.12, -2.55, -3.0, -3.45, -3.88)
g = osp.Geo()
for z in BANDS:
    g.add(hat_band(z, 0.05, 0.018))
objs.append(osp.make_object("H25_HatBands", g, [mats["band"]], coll, 40))

# Coolant plumbing: forward manifold at the joint, inlet manifold fed by the steerhorn, three
# drop lines down the bell, aft manifold at the exit.
g = osp.Geo()
zf = Z_JOINT - 0.075
g.add(osp.lathe([(r_bell(zf) * (1 - AMP) - 0.002, zf), (r_bell(zf) + 0.03, zf),
                 (r_joint + 0.055, Z_JOINT - 0.045), (r_joint + 0.055, Z_JOINT),
                 (r_joint * (1 - AMP) - 0.002, Z_JOINT)], 48))
Z_IM = -1.86
R_IM = r_bell(Z_IM) * (1 + AMP) + 0.042
g.add(osp.torus(R_IM, 0.04, 40, 5, Z_IM))
Z_AM = -LENGTH + 0.04
R_AM = RE * (1 + AMP) + 0.034
g.add(osp.torus(R_AM, 0.032, 60, 5, Z_AM))
for az in (100, 220, 340):
    zs = [-1.97 + (Z_AM + 0.04 + 1.97) * k / 8 for k in range(9)]
    line = [cyl_pt(az, R_IM, Z_IM - 0.02)] + [cyl_pt(az, r_bell(z) + 0.052, z) for z in zs]
    line.append(cyl_pt(az, R_AM, Z_AM + 0.012))
    g.add(osp.sweep(line, 0.021, 6, False, False))
    for z in (BANDS[0], BANDS[2], BANDS[4]):   # clamps where the lines cross the hat bands
        c = cyl_pt(az, r_bell(z - 0.025) + 0.036, z - 0.025)
        g.add(osp.box((0.05, 0.075, 0.06)),
              matrix=Matrix.Translation(c) @ Matrix.Rotation(math.radians(az), 4, "Z"))
objs.append(osp.make_object("H25_CoolantManifolds", g, [mats["manifold"]], coll, 40))

# ------------------------------------------------- main combustion chamber
outer = [(r_joint, Z_JOINT), (r_joint + 0.07, Z_JOINT), (r_joint + 0.07, Z_JOINT + 0.04),
         (r_joint + 0.028, Z_JOINT + 0.04), (0.205, Z_T - 0.065), (0.19, Z_T), (0.2, Z_T + 0.06),
         (0.285, -0.97), (0.285, Z_INJ)]
inner = list(reversed(chamber))[:-1]   # injector face edge down to the joint
for r_o, z in outer[3:]:
    assert r_o > osp.interp_r(chamber, z) + 0.025, z
segs = 32
verts, faces = osp.lathe(outer + inner, segs, closed=True)
seg_mat = [0] * (len(outer) - 1) + [1] * (len(inner) + 1)
g = osp.Geo().add((verts, faces), [seg_mat[k // segs] for k in range(len(faces))])
g.add(osp.torus(0.232, 0.03, 28, 6, Z_T - 0.075), 0)    # coolant inlet manifold at the throat
mcc = osp.make_object("H25_CombustionChamber", g, [mats["mcc"], mats["liner"]], coll, 40)
objs.append(mcc)

# ------------------------------------------------------- hot-gas manifold
g = osp.Geo()
drum = osp.fillet([Vector((0.34, Z_INJ)), Vector((0.39, Z_INJ + 0.025)), Vector((0.39, -0.485)),
                   Vector((0.0, -0.485))], 0.08, 2)
drum = [(0.0, Z_INJ), (RC, Z_INJ)] + [(p.x, p.y) for p in drum]
verts, faces = osp.lathe(drum, 36)
g.add((verts, faces), [1] * 36 + [0] * (len(faces) - 36))   # first ring: the sooted injector face
for tp, r_h in ((FTP, 0.25), (OTP, 0.22)):
    hs = [(0.0, -0.865), (r_h - 0.035, -0.865), (r_h, -0.83), (r_h, -0.62), (r_h + 0.016, -0.615),
          (r_h + 0.016, -0.585), (r_h, -0.58), (r_h, -0.53), (r_h - 0.035, -0.495), (0.0, -0.495)]
    g.add(osp.lathe(hs, 32), 0, matrix=Matrix.Translation(tp))
for r_d, y, sx in ((0.11, 0.115, 1), (0.11, -0.115, 1), (0.15, 0.0, -1)):
    a, b = Vector((sx * 0.3, y, -0.665)), Vector((sx * 0.56, y, -0.665))
    g.add(osp.cyl_between(a, b, r_d, 18, caps=False), 0)
    g.add(osp.cyl_between(a + (b - a) * 0.4, a + (b - a) * 0.52, r_d + 0.02, 18, caps=False), 0)
objs.append(osp.make_object("H25_HotGasManifold", g, [mats["hgm"], mats["face"]], coll, 40))

# Main injector LOX dome on top of the manifold.
g = osp.Geo()
dome = [(0.0, -0.495), (0.325, -0.495), (0.325, -0.465), (0.305, -0.465)]
for k in range(1, 5):
    a = math.radians(90 * k / 5)
    dome.append((0.15 + 0.155 * math.cos(a), -0.465 + 0.13 * math.sin(a)))
dome += [(0.15, -0.30), (0.0, -0.30)]
g.add(osp.lathe(dome, 32))
bolts(g, (0, 0, 0), 0.313, 10, -0.466, -0.45, rb=0.011)
objs.append(osp.make_object("H25_MainInjector", g, [mats["mcc"]], coll, 40))

# ------------------------------------------------------------ preburners
g = osp.Geo()
for tp, r_p, z_top, z_dome in ((FTP, 0.165, -0.25, -0.11), (OTP, 0.135, -0.31, -0.2)):
    p = [(0.0, -0.495), (r_p + 0.045, -0.495), (r_p + 0.045, -0.46), (r_p, -0.46),
         (r_p, -0.4), (r_p + 0.012, -0.39), (r_p + 0.012, -0.37), (r_p, -0.36),
         (r_p, z_top - 0.035), (r_p + 0.028, z_top - 0.035), (r_p + 0.028, z_top), (r_p, z_top)]
    for k in range(1, 5):
        a = math.radians(90 * k / 4)
        p.append((r_p * math.cos(a), z_top + (z_dome - z_top) * math.sin(a)))
    g.add(osp.lathe(p, 24), matrix=Matrix.Translation(tp))
    # propellant feed stub on the dome and a valve on its side
    g.add(osp.lathe([(0.045, z_dome - 0.01), (0.045, z_dome + 0.03), (0.06, z_dome + 0.03),
                     (0.06, z_dome + 0.05), (0.0, z_dome + 0.05)], 12), matrix=Matrix.Translation(tp))
    g.add(osp.box((0.1, 0.09, 0.13), tp + Vector((0, -(r_p + 0.04), z_top - 0.12))), 1)
    bolts(g, tp, r_p + 0.016, 8, z_top - 0.002, z_top + 0.012)
objs.append(osp.make_object("H25_Preburners", g, [mats["preburner"], mats["valve"]], coll, 40))

# -------------------------------------------------- high-pressure turbopumps
g = osp.Geo()
fp = [(0.0, -1.5), (0.1, -1.5), (0.13, -1.47), (0.18, -1.43), (0.2, -1.39), (0.2, -1.26),
      (0.218, -1.245), (0.218, -1.215), (0.2, -1.2), (0.2, -1.12), (0.218, -1.105), (0.218, -1.075),
      (0.2, -1.06), (0.2, -0.915), (0.255, -0.915), (0.255, -0.868), (0.0, -0.868)]
g.add(osp.lathe(fp, 28), matrix=Matrix.Translation(FTP))
g.add(osp.torus(0.205, 0.05, 28, 6, -1.33), matrix=Matrix.Translation(FTP))           # discharge volute
op = [(0.0, -1.38), (0.09, -1.38), (0.115, -1.35), (0.15, -1.31), (0.15, -1.22), (0.172, -1.2),
      (0.18, -1.15), (0.18, -0.915), (0.225, -0.915), (0.225, -0.868), (0.0, -0.868)]
g.add(osp.lathe(op, 28), matrix=Matrix.Translation(OTP))
g.add(osp.torus(0.182, 0.048, 28, 6, -1.03), matrix=Matrix.Translation(OTP))          # main pump volute
for tp, r_f in ((FTP, 0.235), (OTP, 0.205)):
    bolts(g, tp, r_f, 8, -0.93, -0.913, rb=0.01)
objs.append(osp.make_object("H25_Turbopumps", g, [mats["pump"]], coll, 40))

# Low-pressure pumps near the top (fuel at -Y, oxidiser at +Y) with stage inlet flanges.
LPF, LPO = cyl_pt(-90, 0.68, 0.0), cyl_pt(90, 0.68, 0.0)
g = osp.Geo()
for c, r_lp in ((LPF, 0.13), (LPO, 0.12)):
    p = [(0.0, -0.47), (0.07, -0.47), (r_lp - 0.03, -0.43), (r_lp - 0.03, -0.35), (r_lp, -0.33),
         (r_lp, -0.18), (r_lp - 0.035, -0.15), (r_lp - 0.04, -0.075), (r_lp - 0.01, -0.07),
         (r_lp - 0.01, -0.028), (0.0, -0.028)]
    g.add(osp.lathe(p, 24), matrix=Matrix.Translation(c))
    g.add(osp.torus(r_lp, 0.04, 20, 5, -0.255), matrix=Matrix.Translation(c))
objs.append(osp.make_object("H25_LowPressurePumps", g, [mats["pump"]], coll, 40))

# --------------------------------------------------------------- ducts
g = osp.Geo()
# Low-pressure pump discharge ducts: down and around the outside to the HP pump inlets.
fuel = osp.fillet([LPF + Vector((0, 0, -0.44)), LPF + Vector((0.02, -0.1, -0.7)),
                   Vector((0.48, -0.84, -1.02)), Vector((0.94, -0.5, -1.36)),
                   Vector((1.02, -0.12, -1.6)), FTP + Vector((0, 0, -1.76)),
                   FTP + Vector((0, 0, -1.46))], 0.12, 4)
g.add(osp.sweep(fuel, 0.085, 14, False, False))
lox = osp.fillet([LPO + Vector((0, 0, -0.44)), LPO + Vector((-0.02, 0.1, -0.7)),
                  Vector((-0.48, 0.82, -0.98)), Vector((-0.92, 0.47, -1.25)),
                  Vector((-1.0, 0.1, -1.46)), OTP + Vector((0, 0, -1.6)),
                  OTP + Vector((0, 0, -1.35))], 0.11, 4)
g.add(osp.sweep(lox, 0.078, 14, False, False))
for duct, r in ((fuel, 0.085), (lox, 0.078)):     # flex-joint flanges along the ducts
    for f in (0.12, 0.45, 0.8):
        i = int(f * (len(duct) - 1))
        g.add(disc(duct[i], duct[i + 1] - duct[i], r + 0.022, 0.03, 14))
# HP oxidiser pump discharge up to the main oxidiser valve and the LOX dome.
hpo = osp.fillet([OTP + Vector((0.0, -0.2, -1.03)), OTP + Vector((0.0, -0.4, -1.03)),
                  Vector((-0.7, -0.44, -0.6)), Vector((-0.45, -0.28, -0.4)),
                  Vector((-0.2, -0.14, -0.4))], 0.1, 4)
g.add(osp.sweep(hpo, 0.062, 12, False, False))
# HP fuel pump discharge to the main fuel valve.
mfv = FTP + Vector((0.08, 0.4, -1.33))
g.add(osp.sweep(osp.fillet([FTP + Vector((0.0, 0.2, -1.33)), FTP + Vector((0.02, 0.33, -1.33)),
                            mfv], 0.05, 3), 0.065, 12, False, False))
objs.append(osp.make_object("H25_PropellantDucts", g, [mats["ducts"]], coll, 40))

# Valves: main oxidiser valve on the discharge duct, main fuel valve at the steerhorn root.
g = osp.Geo()
mov_i = len(hpo) * 2 // 3
g.add(osp.box((0.15, 0.15, 0.15), bevel=0.02, bevel_segs=1),
      matrix=osp.align_z(hpo[mov_i], hpo[mov_i + 1] - hpo[mov_i - 1]))
g.add(osp.box((0.17, 0.18, 0.16), mfv, bevel=0.02, bevel_segs=1))
g.add(osp.cylinder(0.05, 0.0, 0.07, 12, cap0=False), matrix=Matrix.Translation(mfv + Vector((0, 0, 0.08))))
objs.append(osp.make_object("H25_Valves", g, [mats["valve"]], coll, 40))

# Steerhorn: from the main fuel valve out, down and around the bell to the coolant inlet manifold.
g = osp.Geo()
horn = [mfv + Vector((0, 0.06, -0.02))]
for az, r, z in ((38, 0.95, -1.4), (52, 0.98, -1.52), (64, 0.9, -1.66), (74, 0.76, -1.77),
                 (80, R_IM, Z_IM + 0.02)):
    horn.append(cyl_pt(az, r, z))
horn = osp.fillet(horn, 0.18, 5)
g.add(osp.sweep(horn, 0.068, 14, False, False))
for f in (0.25, 0.7):
    i = int(f * (len(horn) - 1))
    g.add(disc(horn[i], horn[i + 1] - horn[i], 0.09, 0.03, 14))
objs.append(osp.make_object("H25_Steerhorn", g, [mats["ducts"]], coll, 40))

# --------------------------------------------------------- gimbal bearing
g = osp.Geo()
g.add(osp.lathe([(0.0, -0.045), (0.19, -0.045), (0.2, -0.035), (0.2, -0.008), (0.19, 0.0),
                 (0.0, 0.0)], 32))
ball = [(0.0, -0.3), (0.17, -0.3), (0.17, -0.25), (0.13, -0.25)]
for a in (-20, 0, 20):
    ball.append((0.15 * math.cos(math.radians(a)), -0.175 + 0.15 * math.sin(math.radians(a))))
ball += [(0.13, -0.1), (0.165, -0.1), (0.165, -0.045), (0.0, -0.045)]
g.add(osp.lathe(ball, 32))
for k in range(8):
    a = math.tau * (k + 0.5) / 8
    g.add(osp.box((0.12, 0.022, 0.09)),
          matrix=Matrix.Translation((0.2 * math.cos(a), 0.2 * math.sin(a), -0.08)) @ Matrix.Rotation(a, 4, "Z"))
objs.append(osp.make_object("H25_GimbalBearing", g, [mats["dark"]], coll, 40))


# --------------------------------------------------------------- actuators
def actuator(geo, top, bot, rb, rr):
    top, bot = Vector(top), Vector(bot)
    d = bot - top
    length, u = d.length, d.normalized()
    side = u.cross(Vector((0, 0, 1))).normalized()
    geo.add(osp.cyl_between(top + u * 0.05, top + u * 0.58 * length, rb, 16), 0)
    geo.add(osp.cyl_between(top + u * 0.58 * length, top + u * 0.62 * length, rb * 0.78, 16), 0)
    geo.add(osp.cyl_between(top + u * 0.6 * length, bot - u * 0.04, rr, 12), 1)
    geo.add(osp.box((0.07, 0.07, 0.12), top + u * 0.28 * length - side.cross(u) * (rb + 0.03),
                    bevel=0.008, bevel_segs=1), 1)
    for p in (top + u * 0.02, bot - u * 0.02):
        geo.add(osp.cyl_between(p - side * 0.04, p + side * 0.04, 0.03, 12), 0)


g = osp.Geo()
for az in (45, -45):
    top, bot = cyl_pt(az, 0.66, -0.035), cyl_pt(az, 0.43, -0.99)
    actuator(g, top, bot, 0.055, 0.026)
    g.add(osp.box((0.16, 0.08, 0.07), bevel=0.01, bevel_segs=1), 1,
          matrix=Matrix.Translation(cyl_pt(az, 0.35, -0.99)) @ Matrix.Rotation(math.radians(az), 4, "Z"))
objs.append(osp.make_object("H25_Actuators", g, [mats["act"], mats["black"]], coll, 40))

# ------------------------------------------------------------------ export
path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
osp.report(sc, objs, path, [
    ("length (-zmin)", -lo[2], LENGTH),
    ("top z (pivot at 0)", hi[2], 0.0),
    ("exit diameter", 2 * osp.max_radius_near(sc, nozzle, -LENGTH), 2 * RE),
    ("area ratio", (RE / RT) ** 2, 69.0),
])
print(f"  envelope radius {osp.max_radius(sc, objs):.4f} (limit {ENVELOPE})")

# Clearance of the powerhead parts from the bell's tube crests (negative = intersecting).
dg = osp.depsgraph(sc)
worst = {}
for ob in objs:
    if ob.name in ("H25_TubeWallNozzle", "H25_HatBands", "H25_CoolantManifolds", "H25_CombustionChamber"):
        continue
    ev = ob.evaluated_get(dg)
    me = ev.to_mesh()
    for v in me.vertices:
        if bell[0][1] <= v.co.z <= bell[-1][1]:
            c = v.co.xy.length - r_bell(v.co.z) * (1 + AMP)
            worst[ob.name] = min(worst.get(ob.name, 9.0), c)
    ev.to_mesh_clear()
print("  bell clearance: " + ", ".join(f"{k} {v:+.3f}" for k, v in worst.items()))
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=-12)

DEBUG_VIEWS = [("below", -65, -40), ("above", 30, 140), ("side", 0, 90),
               ("top", 15, -40, ((-1.0, -1.0, -1.9), (1.0, 1.0, 0.0)))]
