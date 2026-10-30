"""H-2: gas-generator hydrolox upper-stage engine (J-2 class, Saturn V second and third stages).

Length 3.38 m, exit diameter 1.98 m (area ratio ~27.5), envelope radius 1.05 m.
Gimbal block on the LOX dome; tube-wall regeneratively cooled chamber and bell with hat bands, a
fuel inlet manifold below the throat and a return manifold at the exit. The fuel turbopump (+X)
and LOX turbopump (-X) sit on opposite sides of the chamber, driven in series: the gas generator
feeds the fuel turbine, a crossover duct wraps round the chamber to the LOX turbine, and the LOX
turbine exhaust runs through the heat exchanger down into the big turbine exhaust manifold torus
around the bell. Bellows inlet ducts come down from the stage; the hydrogen start tank sphere sits
on the side. Origin = gimbal pivot at the top centre; exit plane at z = -3.38.
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

NAME, OUT = "h2", "models/engines/h2.glb"
LENGTH, RE, ENVELOPE = 3.38, 0.99, 1.05
RT = RE / math.sqrt(27.5)
RC, Z_INJ, Z_T = 0.25, -0.42, -0.88
TUBES = 64
AMP = 0.5 * 0.15 * math.tau / TUBES   # tube crest height, as a fraction of the local radius
FTP = Vector((0.54, 0.0, 0.0))        # fuel turbopump axis
OTP = Vector((-0.55, 0.0, 0.0))       # LOX turbopump axis
Z_EM = Z_T - 0.4 * (Z_T + LENGTH)     # turbine exhaust manifold, 40% of the way down the bell
Z_FM = -1.28                          # fuel inlet manifold

sc, coll = osp.new_scene("osp_" + NAME)
m = osp.material
mats = {
    "tubes": m("H2_TubeWallNozzle_Steel", "#bdbcb7", 1.0, 0.3),
    "tubes_in": m("H2_TubeWallNozzle_HeatTintedInner", "#8c857b", 0.85, 0.42),
    "band": m("H2_HatBands_DarkSteel", "#7f7e7a", 0.95, 0.38),
    "manifold": m("H2_Manifolds_Steel", "#b0b0ad", 1.0, 0.3),
    "exhaust": m("H2_TurbineExhaust_HeatTinted", "#867563", 0.85, 0.45),
    "dome": m("H2_LoxDome_Steel", "#b4b4b1", 1.0, 0.3),
    "face": m("H2_InjectorFace_Sooted", "#2a2826", 0.3, 0.75),
    "pump": m("H2_Turbopump_Aluminium", "#c6c3bc", 1.0, 0.36),
    "ducts": m("H2_PropellantDucts_Steel", "#a6a7a9", 1.0, 0.27),
    "valve": m("H2_Valves_Aluminium", "#cbc8c1", 1.0, 0.38),
    "tank": m("H2_StartTank_Titanium", "#9a9894", 0.9, 0.34),
    "dark": m("H2_GimbalBlock_DarkSteel", "#46494e", 0.85, 0.45),
    "act": m("H2_Actuator_Silver", "#d2d3d4", 1.0, 0.25),
    "black": m("H2_Actuator_Black", "#1c1c1e", 0.2, 0.55),
}
objs = []

prof = osp.nozzle_contour(RC, Z_INJ, RT, Z_T, RE, -LENGTH, 25.0, 30.0, 8.0, n_bell=10, n_throat=2,
                          r_fillet=0.1, bell_bias=1.25)


def r_wall(z):
    return osp.interp_r(prof, z)


def r_crest(z):
    return r_wall(z) * (1 + AMP)


def cyl_pt(az_deg, r, z):
    a = math.radians(az_deg)
    return Vector((r * math.cos(a), r * math.sin(a), z))


def tube_ripple(r, z, theta):
    """Brazed tube bundle: crest/valley alternation; the exit lip ring stays round."""
    if z < -LENGTH + 0.003:
        return 0.0
    return AMP * r * math.cos(TUBES * theta)


def wall_torus(fn, segs, minor, z0):
    """Manifold welded onto the bell: a torus whose centre radius and tube radius vary around Z
    (fn(theta) -> (R, rho)), with the part of the section inside the wall flattened onto the
    tube valleys, so the section is a D bulging out of the wall."""
    verts, faces = [], []
    for i in range(segs):
        t = math.tau * i / segs
        R, rho = fn(t)
        for j in range(minor):
            p = math.tau * j / minor
            z = z0 + rho * math.sin(p)
            rr = max(R + rho * math.cos(p), r_wall(z) * (1 - AMP))
            verts.append((rr * math.cos(t), rr * math.sin(t), z))
    for i in range(segs):
        i2 = (i + 1) % segs
        for j in range(minor):
            j2 = (j + 1) % minor
            faces.append((i * minor + j, i2 * minor + j, i2 * minor + j2, i * minor + j2))
    return verts, faces


def disc(p, d, r, h, segs=12):
    """Flange disc of radius r and thickness h centred at p, axis d."""
    verts, faces = osp.lathe([(0.0, -h / 2), (r, -h / 2), (r, h / 2), (0.0, h / 2)], segs)
    mtx = osp.align_z(p, d)
    return [tuple(mtx @ Vector(v)) for v in verts], faces


def ring_at(path, f, r, minor_r, segs=14):
    """Bellows convolution / flange ring around a swept path at fraction f of its length."""
    lens = [0.0]
    for a, b in zip(path, path[1:]):
        lens.append(lens[-1] + (b - a).length)
    s = f * lens[-1]
    i = max(k for k in range(len(path) - 1) if lens[k] <= s)
    u = (s - lens[i]) / max(lens[i + 1] - lens[i], 1e-9)
    p = path[i].lerp(path[i + 1], u)
    verts, faces = osp.torus(r, minor_r, segs, 4)
    mtx = osp.align_z(p, path[i + 1] - path[i])
    return [tuple(mtx @ Vector(v)) for v in verts], faces


def bolts(geo, center, r, n, z0, z1, mat=0, rb=0.008):
    for k in range(n):
        a = math.tau * (k + 0.5) / n
        geo.add(osp.cylinder(rb, z0, z1, 6, cap0=False), mat,
                matrix=Matrix.Translation(Vector(center) + Vector((r * math.cos(a), r * math.sin(a), 0))))


# ------------------------------------------------------------- thrust chamber
nozzle = osp.make_object("H2_TubeWallNozzle", osp.Geo().add(osp.lathe(prof, 2 * TUBES, ripple=tube_ripple)),
                         [mats["tubes"], mats["tubes_in"]], coll, 45,
                         solidify=dict(thickness=0.008, material_offset=1), concave_angle=12)
objs.append(nozzle)


def hat_band(z_top, h, proud, segs=48):
    zb = z_top - h
    rt, rb = r_wall(z_top), r_wall(zb)
    return osp.lathe([(rb * (1 - AMP) - 0.002, zb), (rb + proud, zb), (rt + proud, z_top),
                      (rt * (1 - AMP) - 0.002, z_top)], segs)


g = osp.Geo()
for z in (-1.58, -2.32, -2.72, -3.08):
    g.add(hat_band(z, 0.045, 0.016))
objs.append(osp.make_object("H2_HatBands", g, [mats["band"]], coll, 40))

# Fuel inlet manifold below the throat, return manifold at the exit, injector fuel manifold on top.
g = osp.Geo()
R_FM = r_wall(Z_FM) + 0.6 * 0.042
g.add(wall_torus(lambda t: (R_FM, 0.042), 40, 6, Z_FM))
zt, zb = -LENGTH + 0.07, -LENGTH + 0.008
rt_, rb_ = r_wall(zt), r_wall(zb)
g.add(osp.lathe([(rb_ * (1 - AMP) - 0.002, zb), (rb_ + 0.02, zb), (rb_ + 0.032, zb + 0.014),
                 (rt_ + 0.032, zt - 0.014), (rt_ + 0.02, zt), (rt_ * (1 - AMP) - 0.002, zt)], 56))
g.add(osp.torus(RC * (1 + AMP) + 0.028, 0.03, 32, 6, Z_INJ - 0.05))
objs.append(osp.make_object("H2_Manifolds", g, [mats["manifold"]], coll, 40))

# Turbine exhaust manifold welded round the bell, fattest where the exhaust duct comes in
# (azimuth 150) and tapering round the far side.
AZ_IN = math.radians(150)


def em_shape(t):
    rho = 0.062 + 0.034 * 0.5 * (1 + math.cos(t - AZ_IN))
    return r_wall(Z_EM) + 0.5 * rho, rho


g = osp.Geo().add(wall_torus(em_shape, 64, 8, Z_EM))
R_EM_IN, RHO_IN = em_shape(AZ_IN)
hx = cyl_pt(150, 0.73, 0.0)
duct = osp.fillet([hx + Vector((0, 0, -1.4)), cyl_pt(152, 0.78, -1.58),
                   cyl_pt(150, R_EM_IN + 0.5 * RHO_IN, -1.76),
                   cyl_pt(150, R_EM_IN + 0.3 * RHO_IN, Z_EM + 0.3 * RHO_IN)], 0.1, 5)
g.add(osp.sweep(duct, 0.075, 14, False, False))
g.add(ring_at(duct, 0.12, 0.075, 0.014))
g.add(ring_at(duct, 0.55, 0.075, 0.014))
# LOX turbine exhaust over to the heat exchanger
tx = osp.fillet([OTP + Vector((0.0, 0.1, -0.77)), OTP + Vector((0.0, 0.25, -0.79)),
                 cyl_pt(156, 0.7, -0.84), hx + Vector((0, 0, -0.96))], 0.06, 4)
g.add(osp.sweep(tx, 0.065, 12, False, False))
# heat exchanger
g.add(osp.lathe([(0.0, -1.42), (0.08, -1.42), (0.115, -1.39), (0.115, -1.34), (0.128, -1.33),
                 (0.128, -1.3), (0.115, -1.29), (0.115, -1.12), (0.128, -1.11), (0.128, -1.08),
                 (0.115, -1.07), (0.115, -1.03), (0.08, -1.0), (0.0, -1.0)], 20), matrix=Matrix.Translation(hx))
objs.append(osp.make_object("H2_TurbineExhaust", g, [mats["exhaust"]], coll, 40))

# ------------------------------------------------------------- injector / LOX dome
g = osp.Geo()
dome = [(0.0, Z_INJ - 0.005), (RC, Z_INJ - 0.005), (0.285, Z_INJ - 0.005), (0.285, Z_INJ + 0.03),
        (0.26, Z_INJ + 0.03)]
for k in range(1, 5):
    a = math.radians(90 * k / 5)
    dome.append((0.1 + 0.16 * math.cos(a), Z_INJ + 0.03 + 0.155 * math.sin(a)))
dome += [(0.1, -0.235), (0.0, -0.235)]
verts, faces = osp.lathe(dome, 36)
g.add((verts, faces), [1] * 36 + [0] * (len(faces) - 36))   # first ring: sooted injector face
bolts(g, (0, 0, 0), 0.272, 12, Z_INJ + 0.029, Z_INJ + 0.043)
objs.append(osp.make_object("H2_LoxDome", g, [mats["dome"], mats["face"]], coll, 40))

# ------------------------------------------------------------- gimbal block
g = osp.Geo()
g.add(osp.lathe([(0.0, -0.03), (0.14, -0.03), (0.15, -0.022), (0.15, -0.007), (0.142, 0.0),
                 (0.0, 0.0)], 32))
g.add(osp.box((0.2, 0.2, 0.11), (0, 0, -0.095), bevel=0.014, bevel_segs=2))
for axis in ((1, 0, 0), (0, 1, 0)):
    a = Vector(axis)
    g.add(osp.cyl_between(Vector((0, 0, -0.095)) - a * 0.15, Vector((0, 0, -0.095)) + a * 0.15, 0.032, 16))
g.add(osp.lathe([(0.0, -0.245), (0.2, -0.245), (0.2, -0.228), (0.12, -0.17), (0.09, -0.15),
                 (0.0, -0.15)], 32))
bolts(g, (0, 0, 0), 0.115, 8, -0.042, -0.03, rb=0.01)
objs.append(osp.make_object("H2_GimbalBlock", g, [mats["dark"]], coll, 40))

# ------------------------------------------------------------- turbopumps
g = osp.Geo()
# Fuel pump (+X): axial inlet on top, seven-stage axial pump, discharge volute, two-stage turbine.
fp = [(0.0, -1.03), (0.07, -1.03), (0.125, -1.01), (0.165, -0.98), (0.165, -0.88), (0.145, -0.86),
      (0.135, -0.84), (0.135, -0.74), (0.145, -0.73), (0.145, -0.71), (0.135, -0.7), (0.135, -0.6),
      (0.145, -0.59), (0.145, -0.57), (0.135, -0.56), (0.135, -0.45), (0.115, -0.41), (0.092, -0.4),
      (0.092, -0.33), (0.108, -0.33), (0.108, -0.3), (0.0, -0.3)]
g.add(osp.lathe(fp, 24), matrix=Matrix.Translation(FTP))
g.add(osp.torus(0.15, 0.045, 24, 6, -0.8), matrix=Matrix.Translation(FTP))
# LOX pump (-X): centrifugal pump with a big volute on top, turbine below.
op = [(0.0, -0.87), (0.07, -0.87), (0.135, -0.85), (0.16, -0.82), (0.16, -0.7), (0.13, -0.68),
      (0.12, -0.64), (0.165, -0.6), (0.165, -0.43), (0.12, -0.39), (0.09, -0.38), (0.09, -0.355),
      (0.105, -0.355), (0.105, -0.325), (0.0, -0.325)]
g.add(osp.lathe(op, 24), matrix=Matrix.Translation(OTP))
g.add(osp.torus(0.165, 0.055, 24, 6, -0.51), matrix=Matrix.Translation(OTP))
# Gas generator beside the fuel turbine.
gg = cyl_pt(-28, 0.6, 0.0)
gp = [(0.0, -1.0), (0.04, -1.0), (0.055, -0.98), (0.055, -0.84), (0.065, -0.84), (0.065, -0.82)]
for k in range(1, 4):
    a = math.radians(90 * k / 3)
    gp.append((0.06 * math.cos(a), -0.82 + 0.05 * math.sin(a)))
g.add(osp.lathe(gp, 16), matrix=Matrix.Translation(gg))
g.add(osp.sweep(osp.fillet([gg + Vector((0, 0, -0.97)), gg + Vector((0, 0, -1.03)),
                            FTP + Vector((0.0, -0.08, -0.96))], 0.04, 3), 0.032, 10, False, False))
objs.append(osp.make_object("H2_Turbopumps", g, [mats["pump"]], coll, 40))

# ------------------------------------------------------------- ducts
g = osp.Geo()
# Bellows inlet ducts from the stage flanges down to the pump inlets.
fin = osp.fillet([FTP + Vector((0.03, 0.08, -0.02)), FTP + Vector((0.03, 0.08, -0.1)),
                  FTP + Vector((0.0, 0.0, -0.24)), FTP + Vector((0, 0, -0.32))], 0.06, 4)
lin = osp.fillet([OTP + Vector((-0.03, -0.08, -0.02)), OTP + Vector((-0.03, -0.08, -0.1)),
                  OTP + Vector((0.0, 0.0, -0.26)), OTP + Vector((0, 0, -0.34))], 0.06, 4)
for path, r in ((fin, 0.078), (lin, 0.084)):
    g.add(osp.sweep(path, r, 14, False, False))
    g.add(disc(path[0] + Vector((0, 0, -0.008)), (0, 0, 1), r + 0.03, 0.022, 16))
    for f in (0.3, 0.45, 0.6, 0.75):
        g.add(ring_at(path, f, r, 0.011))
# Turbine crossover: fuel turbine round the -Y side of the chamber to the LOX turbine.
xo = osp.fillet([FTP + Vector((0, 0, -1.0)), FTP + Vector((0.0, -0.03, -1.12)), cyl_pt(-40, 0.58, -1.16),
                 cyl_pt(-90, 0.59, -1.16), cyl_pt(-140, 0.58, -1.08), OTP + Vector((0.0, -0.03, -1.0)),
                 OTP + Vector((0, 0, -0.85))], 0.12, 4)
g.add(osp.sweep(xo, 0.062, 12, False, False))
g.add(ring_at(xo, 0.3, 0.062, 0.012))
g.add(ring_at(xo, 0.7, 0.062, 0.012))
# Fuel discharge: pump volute -> main fuel valve -> fuel inlet manifold.
mfv = FTP + Vector((0.05, 0.35, -0.9))
fd = osp.fillet([FTP + Vector((0.0, 0.14, -0.8)), FTP + Vector((0.03, 0.29, -0.82)), mfv,
                 cyl_pt(40, 0.62, -1.1), cyl_pt(44, R_FM + 0.01, Z_FM + 0.02)], 0.08, 4)
g.add(osp.sweep(fd, 0.05, 12, False, False))
# LOX discharge: pump volute -> main oxidiser valve -> LOX dome.
mov = Vector((-0.4, 0.34, -0.42))
od = osp.fillet([OTP + Vector((0.0, 0.17, -0.51)), OTP + Vector((0.06, 0.32, -0.49)), mov,
                 Vector((-0.2, 0.2, -0.32)), Vector((-0.1, 0.1, -0.3))], 0.07, 4)
g.add(osp.sweep(od, 0.055, 12, False, False))
objs.append(osp.make_object("H2_PropellantDucts", g, [mats["ducts"]], coll, 40))

# Valves and the hydrogen start tank.
g = osp.Geo()
g.add(osp.box((0.12, 0.13, 0.12), mfv, bevel=0.015, bevel_segs=1), 0)
g.add(osp.box((0.12, 0.13, 0.12), mov, bevel=0.015, bevel_segs=1), 0)
g.add(osp.cylinder(0.04, -0.06, 0.0, 12, cap0=False), 0, matrix=Matrix.Translation(mov + Vector((0, 0, 0.12))))
tank_c = cyl_pt(100, 0.62, -0.76)
sphere = [(0.2 * math.cos(math.radians(a)), 0.2 * math.sin(math.radians(a))) for a in range(-90, 91, 15)]
g.add(osp.lathe(sphere, 24), 1, matrix=Matrix.Translation(tank_c))
g.add(osp.torus(0.2, 0.012, 24, 4, 0.0), 1, matrix=Matrix.Translation(tank_c))     # weld flange
for p0 in (tank_c + Vector((0, 0, 0.19)), tank_c + Vector((0, 0, -0.19))):
    g.add(osp.cylinder(0.035, -0.02, 0.03, 10, cap0=False), 1, matrix=Matrix.Translation(p0))
for sgn in (-1, 1):   # mounting struts to the chamber
    g.add(osp.cyl_between(tank_c + cyl_pt(280 + 40 * sgn, 0.17, -0.06), cyl_pt(100 + 28 * sgn, 0.27, -0.55),
                          0.014, 6), 1)
# Electrical control package on a bracket off the chamber.
rot = Matrix.Rotation(math.radians(-100), 4, "Z")
g.add(osp.box((0.12, 0.22, 0.26), bevel=0.012, bevel_segs=1), 0,
      matrix=Matrix.Translation(cyl_pt(-100, 0.385, -0.6)) @ rot)
g.add(osp.box((0.1, 0.06, 0.08)), 0, matrix=Matrix.Translation(cyl_pt(-100, 0.29, -0.6)) @ rot)
objs.append(osp.make_object("H2_ValvesAndStartTank", g, [mats["valve"], mats["tank"]], coll, 40))


# ------------------------------------------------------------- actuators
def actuator(geo, top, bot, rb, rr):
    top, bot = Vector(top), Vector(bot)
    d = bot - top
    length, u = d.length, d.normalized()
    side = u.cross(Vector((0, 0, 1))).normalized()
    geo.add(osp.cyl_between(top + u * 0.04, top + u * 0.58 * length, rb, 14), 0)
    geo.add(osp.cyl_between(top + u * 0.58 * length, top + u * 0.62 * length, rb * 0.78, 14), 0)
    geo.add(osp.cyl_between(top + u * 0.6 * length, bot - u * 0.03, rr, 10), 1)
    geo.add(osp.box((0.055, 0.055, 0.09), top + u * 0.28 * length - side.cross(u) * (rb + 0.025),
                    bevel=0.007, bevel_segs=1), 1)
    for p in (top + u * 0.015, bot - u * 0.015):
        geo.add(osp.cyl_between(p - side * 0.03, p + side * 0.03, 0.024, 10), 0)


g = osp.Geo()
for az in (60, -30):
    top, bot = cyl_pt(az, 0.48, -0.03), cyl_pt(az, 0.34, -0.72)
    actuator(g, top, bot, 0.042, 0.02)
    g.add(osp.box((0.12, 0.06, 0.06), bevel=0.008, bevel_segs=1), 1,
          matrix=Matrix.Translation(cyl_pt(az, 0.29, -0.72)) @ Matrix.Rotation(math.radians(az), 4, "Z"))
objs.append(osp.make_object("H2_Actuators", g, [mats["act"], mats["black"]], coll, 40))

# ------------------------------------------------------------- export
path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
osp.report(sc, objs, path, [
    ("length (-zmin)", -lo[2], LENGTH),
    ("top z (pivot at 0)", hi[2], 0.0),
    ("exit diameter", 2 * osp.max_radius_near(sc, nozzle, -LENGTH), 2 * RE),
    ("area ratio", (RE / RT) ** 2, 27.5),
])
print(f"  envelope radius {osp.max_radius(sc, objs):.4f} (limit {ENVELOPE})")

# Clearance of the parts beside the chamber from the tube crests (negative = intersecting).
dg = osp.depsgraph(sc)
worst = {}
for ob in objs:
    if ob.name in ("H2_TubeWallNozzle", "H2_HatBands", "H2_Manifolds", "H2_TurbineExhaust", "H2_LoxDome"):
        continue
    ev = ob.evaluated_get(dg)
    me = ev.to_mesh()
    for v in me.vertices:
        if -LENGTH <= v.co.z <= Z_INJ:
            c = v.co.xy.length - r_crest(v.co.z)
            worst[ob.name] = min(worst.get(ob.name, 9.0), c)
    ev.to_mesh_clear()
print("  chamber clearance: " + ", ".join(f"{k} {v:+.3f}" for k, v in worst.items()))
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=-12)

DEBUG_VIEWS = [("below", -65, -40), ("above", 30, 140), ("side", 0, -90),
               ("top", 12, -40, ((-0.85, -0.85, -1.6), (0.85, 0.85, 0.0)))]
