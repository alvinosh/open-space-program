"""K-5: gas-generator kerolox first-stage engine (F-1 class).

Length 5.79 m, exit diameter 3.76 m, envelope radius 1.95 m. Regeneratively cooled tube-wall
thrust chamber with hat bands down to area ratio 10, then a bolted-on shingled nozzle extension
to area ratio 16, film-cooled by turbine exhaust from the fat tapering manifold torus at the
joint. The turbopump hangs on the side of the chamber at +X (LOX pump on top, fuel pump, turbine
at the bottom) with two LOX and two fuel inlets, its exhaust duct and heat exchanger running down
to the manifold. Gimbal block and thrust structure on top of the LOX dome.
Origin = gimbal pivot at the top centre; nozzle exit plane at z = -5.79.
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

NAME, OUT = "k5", "models/engines/k5.glb"
LENGTH, RE, ENVELOPE = 5.79, 1.88, 1.95
RT = RE / 4.0                         # exit area ratio 16
RC, Z_INJ, Z_T = 0.59, -0.80, -1.42   # chamber radius, injector face, throat
TUBES = 56
TPX = 1.22                            # turbopump axis, at azimuth 0 (+X)

sc, coll = osp.new_scene("osp_" + NAME)
m = osp.material
mats = {
    "tubes": m("K5_TubeWallChamber_Inconel", "#4a4541", 0.85, 0.42),
    "tubes_in": m("K5_TubeWallChamber_SootedInner", "#2c2927", 0.3, 0.72),
    "hat": m("K5_HatBands_Inconel", "#5f5a54", 0.9, 0.36),
    "ext": m("K5_NozzleExtension_HeatTinted", "#8b857d", 0.9, 0.42),
    "ext_in": m("K5_NozzleExtension_SootedInner", "#292624", 0.3, 0.75),
    "flange": m("K5_JointFlange_Inconel", "#6b665f", 0.9, 0.38),
    "insul": m("K5_ThermalInsulation_Black", "#151515", 0.0, 0.82),
    "pumpfl": m("K5_TurbopumpFlanges_Steel", "#8f9092", 0.95, 0.36),
    "dome": m("K5_LOXDome_Steel", "#a8a8a5", 1.0, 0.32),
    "face": m("K5_InjectorFace_Copper", "#9a5a3a", 1.0, 0.4),
    "ducts": m("K5_PropellantDucts_Steel", "#9c9d9f", 1.0, 0.3),
    "valves": m("K5_Valves_Aluminium", "#c4c1ba", 1.0, 0.36),
    "lines": m("K5_Lines_Steel", "#b3b4b6", 1.0, 0.26),
    "dark": m("K5_GimbalBlock_DarkSteel", "#45484d", 0.85, 0.45),
    "struct": m("K5_ThrustStructure_Gunmetal", "#3a3c40", 0.8, 0.5),
    "act": m("K5_Actuator_Silver", "#d2d3d4", 1.0, 0.25),
    "black": m("K5_Actuator_Black", "#1c1c1e", 0.2, 0.55),
}
objs = []


def cyl(rho, deg, z):
    a = math.radians(deg)
    return Vector((rho * math.cos(a), rho * math.sin(a), z))


prof = osp.nozzle_contour(RC, Z_INJ, RT, Z_T, RE, -LENGTH, 22.0, 27.0, 10.0, n_bell=40, n_throat=3,
                          r_fillet=0.25)          # fine contour: sampled by the lathes below
R_JOINT = RT * math.sqrt(10.0)        # area ratio 10: chamber / extension joint
Z_JOINT = next(z0 + (R_JOINT - r0) * (z1 - z0) / (r1 - r0)
               for (r0, z0), (r1, z1) in zip(prof, prof[1:]) if z1 < Z_T and r1 <= R_JOINT <= r0)


def wall(z):
    return osp.interp_r(prof, z)


# --------------------------------------------------------------- thrust chamber (tube wall)
def tube_ripple(r, z, theta):
    depth = 0.2 * math.tau * r / TUBES
    return 0.5 * depth * math.cos(TUBES * theta)


# Rings only where the contour needs them: sparse down the bell, denser round the throat.
ch_z = [Z_JOINT + (Z_T - 0.1 - Z_JOINT) * (k / 6) ** 0.8 for k in range(7)]
ch_z += [Z_T - 0.03, Z_T + 0.12, -1.21, -1.08, -0.95, Z_INJ]
ch_prof = [(wall(z), z) for z in ch_z]
chamber = osp.make_object("K5_TubeWallChamber", osp.Geo().add(osp.lathe(ch_prof, 2 * TUBES, ripple=tube_ripple)),
                          [mats["tubes"], mats["tubes_in"]], coll, 45,
                          solidify=dict(thickness=0.014, material_offset=1), concave_angle=12)
objs.append(chamber)

# Hat bands from the throat down, the fuel inlet manifold at the top, the return flange at the joint.
g = osp.Geo()
for z in (-1.6, -1.9, -2.22, -2.56, -2.9, -3.25):
    g.add(kit.open_band(prof, z, 0.045, 0.022, 0.016, 44))
g.add(kit.open_band(prof, Z_JOINT + 0.05, 0.05, 0.05, 0.02, 56), 1)
objs.append(osp.make_object("K5_HatBands", g, [mats["hat"], mats["flange"]], coll, 40))

# --------------------------------------------------------- shingled nozzle extension (10 -> 16)
N_SH, D_SH, EPS, LIP = 7, 0.024, 0.008, 0.006
z_lo, z_hi = -LENGTH + LIP, Z_JOINT - 0.05
h_sh = (z_hi - z_lo) / N_SH
ext_prof = [(RE, -LENGTH), (RE, z_lo)]
for k in range(N_SH):
    zt = z_lo + (k + 1) * h_sh
    ext_prof.append((wall(zt) - D_SH, zt))                 # top of shingle k tucks under ...
    if k < N_SH - 1:
        ext_prof.append((wall(zt + EPS), zt + EPS))        # ... the lip of shingle k + 1
ext_prof += [(wall(z_hi + EPS), z_hi + EPS), (wall(Z_JOINT), Z_JOINT)]
extension = osp.make_object("K5_NozzleExtension", osp.Geo().add(osp.lathe(ext_prof, 56)),
                            [mats["ext"], mats["ext_in"]], coll, 50,
                            solidify=dict(thickness=0.008, material_offset=1), concave_angle=20)
objs.append(extension)

g = osp.Geo()
g.add(kit.open_band(prof, Z_JOINT, 0.05, 0.05, 0.03, 56))                 # extension top flange
g.add(kit.open_band(prof, -LENGTH + 0.075, 0.062, 0.02, 0.012, 56))         # exit stiffener
r_b = wall(Z_JOINT) + 0.03
for k in range(28):                                                          # flange bolts
    a = math.tau * (k + 0.5) / 28
    g.add(osp.cylinder(0.011, Z_JOINT - 0.075, Z_JOINT + 0.075, 4),
          matrix=Matrix.Translation((r_b * math.cos(a), r_b * math.sin(a), 0)) @ Matrix.Rotation(a, 4, "Z"))
objs.append(osp.make_object("K5_ExtensionFlanges", g, [mats["flange"]], coll, 40))

# ------------------------------------------- turbine exhaust duct, heat exchanger, manifold
# The manifold is a volute: fed tangentially at THETA0, its section shrinks as it wraps round.
THETA0, WRAP = 30.0, 343.0
R_MAX, R_MIN, R_DUCT = 0.25, 0.14, 0.19
Z_MB = Z_JOINT + 0.08                  # manifold underside, just above the bolted joint


def manifold_centre(theta, r):
    zc = Z_MB + r
    need = max(wall(zc + r * math.sin(p)) + r * math.cos(p)
               for p in (math.radians(a) for a in range(-90, 91, 10)))
    return need + 0.012, zc


m_pts, m_rad = [], []
steps = 24
for i in range(steps + 1):
    t = i / steps
    th, r = THETA0 + WRAP * t, R_MAX - (R_MAX - R_MIN) * t
    R, zc = manifold_centre(th, r)
    m_pts.append(cyl(R, th, zc))
    m_rad.append(r)
tan_end = (m_pts[-1] - m_pts[-2]).normalized()
for s in (0.45, 0.75, 0.93):          # rounded end of the volute
    m_pts.append(m_pts[steps] + tan_end * s * R_MIN)
    m_rad.append(R_MIN * math.sqrt(1 - s * s))

R0, zc0 = manifold_centre(THETA0, R_MAX)
HX_A, HX_B = cyl(1.3, 2.0, -2.24), cyl(1.52, 7.0, -2.86)   # heat exchanger axis
duct = osp.fillet([cyl(TPX, 0.0, -1.98), cyl(TPX + 0.01, 0.0, -2.12), HX_A, HX_B,
                   cyl(1.63, 14.0, -3.28), cyl(R0, THETA0 - 13.0, zc0 + 0.4),
                   cyl(R0, THETA0 - 4.0, zc0 + 0.06)], 0.16, 3)
path = duct + m_pts
radii = [R_DUCT] * len(duct) + m_rad
for i in range(len(duct) - 2, len(duct) + 2):                 # blend duct into the volute
    radii[i] = R_DUCT + (R_MAX - R_DUCT) * (i - len(duct) + 2) / 4
g = osp.Geo().add(osp.sweep(path, radii, 16, cap0=False, cap1=True))
# Heat exchanger shell on the duct, with end collars.
hx_dir = HX_B - HX_A
hx_len = hx_dir.length
g.add(kit.place(osp.lathe([(R_DUCT - 0.01, -0.08), (0.24, -0.04), (0.3, 0.04), (0.3, hx_len - 0.04),
                           (0.24, hx_len + 0.04), (R_DUCT - 0.01, hx_len + 0.08)], 20),
                osp.align_z(HX_A, hx_dir)))
objs.append(osp.make_object("K5_TurbineExhaust", g, [mats["insul"]], coll, 40))

g = osp.Geo()
for c, r_in, r_out in ((HX_A + hx_dir.normalized() * 0.12, 0.3, 0.322),
                       (HX_B - hx_dir.normalized() * 0.12, 0.3, 0.322),
                       (cyl(TPX + 0.005, 0.0, -2.08), R_DUCT, R_DUCT + 0.035)):
    g.add(kit.collar(c, hx_dir if r_in > 0.25 else (0, 0, -1), r_in, r_out, 0.035, 20))
objs.append(osp.make_object("K5_ExhaustFlanges", g, [mats["pumpfl"]], coll, 40))

# ------------------------------------------------------------------------ turbopump (+X)
# Mark 10 class: LOX pump on top, fuel pump, turbine at the bottom, all on one vertical shaft.
g = osp.Geo()
tp = Matrix.Translation((TPX, 0.0, 0.0))
g.add(osp.lathe([(0.0, -2.06), (0.19, -2.06), (0.25, -2.02), (0.31, -1.97), (0.34, -1.91), (0.34, -1.69),
                 (0.3, -1.64), (0.0, -1.64)], 24), matrix=tp)                     # turbine
g.add(osp.torus(0.33, 0.08, 24, 6, -1.8), matrix=tp)                               # turbine inlet manifold
g.add(osp.lathe([(0.0, -1.64), (0.29, -1.64), (0.31, -1.6), (0.31, -1.28), (0.25, -1.2),
                 (0.0, -1.2)], 24), matrix=tp)                                      # fuel pump
g.add(osp.torus(0.31, 0.1, 24, 6, -1.44), matrix=tp)                               # fuel volute
g.add(osp.lathe([(0.0, -1.2), (0.28, -1.2), (0.35, -1.13), (0.35, -0.84), (0.28, -0.75), (0.2, -0.68),
                 (0.17, -0.6), (0.0, -0.6)], 24), matrix=tp)                        # LOX pump
g.add(osp.torus(0.35, 0.12, 28, 6, -0.96), matrix=tp)                              # LOX volute
# Gas generator on the outboard side, feeding the turbine manifold.
gg = Matrix.Translation((TPX + 0.5, 0.0, 0.0))
g.add(osp.lathe([(0.0, -1.98), (0.05, -1.98), (0.085, -1.94), (0.085, -1.66), (0.065, -1.6), (0.03, -1.57),
                 (0.0, -1.57)], 14), matrix=gg)
g.add(osp.cyl_between((TPX + 0.44, 0.0, -1.8), (TPX + 0.38, 0.0, -1.8), 0.05, 12, caps=False))
# Two LOX inlets rising from the top of the LOX pump, two fuel inlets beside the pump.
inlet_tops = []
for s in (1, -1):
    p = osp.fillet([Vector((TPX, s * 0.07, -0.63)), Vector((TPX, s * 0.22, -0.5)),
                    Vector((TPX - 0.06, s * 0.4, -0.3)), Vector((TPX - 0.08, s * 0.42, -0.1))], 0.12, 4)
    g.add(osp.sweep(p, 0.16, 16, False, True))
    inlet_tops.append((p[-1], 0.16))
    a = math.radians(s * 55)
    foot = Vector((TPX + 0.6 * math.cos(a), 0.6 * math.sin(a), 0.0))
    p = osp.fillet([Vector((TPX + 0.15 * math.cos(a), 0.15 * math.sin(a), -1.4)),
                    foot + Vector((0, 0, -1.4)), foot + Vector((0, 0, -0.34))], 0.12, 4)
    g.add(osp.sweep(p, 0.12, 14, False, True))
    inlet_tops.append((p[-1], 0.12))
objs.append(osp.make_object("K5_Turbopump", g, [mats["insul"]], coll, 40))

g = osp.Geo()
for z, r in ((-1.64, 0.31), (-1.2, 0.35), (-1.97, 0.31)):                          # bolted housing joints
    g.add(osp.lathe([(r - 0.01, z - 0.022), (r + 0.028, z - 0.022), (r + 0.028, z + 0.022), (r - 0.01, z + 0.022)],
                    24), matrix=tp)
for top, r in inlet_tops:                                                           # stage-line flanges
    c = top - Vector((0, 0, 0.03))
    g.add(kit.collar(c, (0, 0, 1), r, r + 0.055, 0.045, 16))
    kit.bolt_circle(g, 0, c, (0, 0, 1), r + 0.036, 6, 0.009, -0.035, 0.035, 4)
objs.append(osp.make_object("K5_TurbopumpFlanges", g, [mats["pumpfl"]], coll, 40))

# -------------------------------------------------- LOX dome, injector, gimbal, thrust structure
g = osp.Geo()
dome = [(0.0, -0.84), (0.58, -0.84), (0.7, -0.835), (0.7, -0.76), (0.66, -0.755)]
dome += [(0.64 * math.cos(math.radians(a)), -0.755 + 0.33 * math.sin(math.radians(a))) for a in range(0, 91, 15)]
segs = 40
v, f = osp.lathe(dome, segs)
g.add((v, f), [1] * segs + [0] * (len(f) - segs))                                  # copper injector face
kit.bolt_circle(g, 0, Vector((0, 0, -0.755)), (0, 0, 1), 0.675, 16, 0.013, 0.0, 0.028, 4)
objs.append(osp.make_object("K5_LOXDome", g, [mats["dome"], mats["face"]], coll, 40))

g = osp.Geo()
g.add(osp.lathe([(0.0, -0.07), (0.3, -0.07), (0.32, -0.055), (0.32, -0.012), (0.306, 0.0), (0.0, 0.0)], 32))
for az in (45, 135, 225, 315):                                   # stub of the stage thrust beams (I-section)
    rot = Matrix.Rotation(math.radians(az), 4, "Z")
    for size, cz in (((0.66, 0.15, 0.026), -0.013), ((0.66, 0.15, 0.026), -0.187), ((0.66, 0.045, 0.16), -0.1)):
        g.add(osp.box(size), matrix=rot @ Matrix.Translation((0.6, 0.0, cz)))
    g.add(osp.cyl_between(Vector((0.86, -0.09, -0.24)), Vector((0.86, 0.09, -0.24)), 0.035, 10), matrix=rot)
    g.add(osp.box((0.09, 0.14, 0.09)), matrix=rot @ Matrix.Translation((0.86, 0.0, -0.215)))
objs.append(osp.make_object("K5_ThrustStructure", g, [mats["struct"]], coll, 40))

g = osp.Geo()
g.add(osp.box((0.36, 0.36, 0.2), (0, 0, -0.27), bevel=0.03, bevel_segs=2))
for axis in ((1, 0, 0), (0, 1, 0)):
    a = Vector(axis)
    g.add(osp.cyl_between(Vector((0, 0, -0.27)) - a * 0.25, Vector((0, 0, -0.27)) + a * 0.25, 0.07, 18))
g.add(osp.lathe([(0.0, -0.47), (0.3, -0.47), (0.26, -0.42), (0.17, -0.37), (0.0, -0.36)], 28))
g.add(osp.lathe([(0.0, -0.2), (0.22, -0.2), (0.22, -0.17), (0.2, -0.07), (0.0, -0.07)], 28))
objs.append(osp.make_object("K5_GimbalBlock", g, [mats["dark"]], coll, 40))

# --------------------------------------------------------------------- propellant ducts
g = osp.Geo()
FM_Z, FM_R = -1.1, wall(-1.1) + 0.02 + 0.07                                       # fuel inlet manifold
g.add(osp.torus(FM_R, 0.07, 40, 6, FM_Z))
valves = []
for s in (1, -1):
    # LOX pump volute -> LOX dome (two high-pressure LOX ducts).
    p = osp.fillet([cyl(0.35, s * 140, -0.96) + Vector((TPX, 0, 0)), Vector((0.9, s * 0.55, -0.86)),
                    Vector((0.48, s * 0.82, -0.72)), Vector((0.0, s * 0.72, -0.64)),
                    Vector((0.0, s * 0.5, -0.62))], 0.16, 3)
    g.add(osp.sweep(p, 0.11, 14, False, False))
    valves.append((Vector((0.25, s * 0.77, -0.68)), Vector((-1, -s * 0.2, 0.15)), 0.11))
    g.add(kit.collar(Vector((0.0, s * 0.64, -0.635)), (0, s, 0), 0.11, 0.15, 0.04, 14))
    # Fuel pump volute -> fuel inlet manifold (two high-pressure fuel ducts).
    p = osp.fillet([cyl(0.31, s * 150, -1.44) + Vector((TPX, 0, 0)), Vector((0.9, s * 0.48, -1.4)),
                    Vector((0.62, s * 0.68, -1.25)), cyl(FM_R, s * 48, FM_Z)], 0.12, 3)
    g.add(osp.sweep(p, 0.085, 12, False, False))
    valves.append((Vector((0.53, s * 0.585, -1.185)), Vector((-0.2, -s * 0.21, 0.15)), 0.085))
objs.append(osp.make_object("K5_PropellantDucts", g, [mats["ducts"]], coll, 40))

g = osp.Geo()
for c, d, r in valves:                                                              # main LOX / fuel valves
    g.add(kit.place(osp.lathe([(r - 0.005, -0.13), (r + 0.04, -0.11), (r + 0.06, -0.04), (r + 0.06, 0.04),
                               (r + 0.04, 0.11), (r - 0.005, 0.13)], 14), osp.align_z(c, d)))
    g.add(osp.box((0.12, 0.12, 0.14), bevel=0.015, bevel_segs=1),
          matrix=Matrix.Translation(c + Vector((0, 0, r + 0.1))))
objs.append(osp.make_object("K5_Valves", g, [mats["valves"]], coll, 40))

# Gas generator propellant feeds from the pump volutes.
g = osp.Geo()
for s in (1, -1):
    p = osp.fillet([Vector((TPX + 0.5, s * 0.03, -1.58)), Vector((TPX + 0.5, s * 0.13, -1.5)),
                    Vector((TPX + 0.36, s * 0.17, -1.44 if s > 0 else -1.0))], 0.04, 3)
    g.add(osp.sweep(p, 0.022, 8, False, False))
objs.append(osp.make_object("K5_Lines", g, [mats["lines"]], coll, 40))

# --------------------------------------------- gimbal actuators: thrust beams -> chamber outriggers
g = osp.Geo()
for az in (135, 225):
    top, bot = cyl(0.86, az, -0.24), cyl(0.8, az, -1.3)
    kit.actuator(g, top, bot, 0.08, 0.04, 0, 1, 12)
    a = math.radians(az)
    g.add(osp.box((0.4, 0.1, 0.13), bevel=0.01, bevel_segs=1), 1,
          matrix=Matrix.Translation(cyl(0.64, az, -1.3)) @ Matrix.Rotation(a, 4, "Z"))
objs.append(osp.make_object("K5_Actuators", g, [mats["act"], mats["black"]], coll, 40))

path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
osp.report(sc, objs, path, [
    ("length (-zmin)", -lo[2], LENGTH),
    ("top z (pivot at 0)", hi[2], 0.0),
    ("exit diameter", 2 * osp.max_radius_near(sc, extension, -LENGTH), 2 * RE),
    ("joint diameter (eps 10)", 2 * osp.max_radius_near(sc, extension, Z_JOINT, 0.002), 2 * R_JOINT),
])
print(f"  joint z {Z_JOINT:.3f}, throat r {RT:.3f}")
print(f"  envelope radius {osp.max_radius(sc, objs):.4f} (limit {ENVELOPE})")
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=-14)

DEBUG_VIEWS = [("below", -65, -40), ("above", 30, 140), ("side", 0, 90),
               ("top", 15, -40, ((-1.2, -1.2, -2.2), (1.9, 1.2, 0.0)))]
