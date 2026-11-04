"""The Saturn V's parts that had no model of their own, under mods/realism/models/parts.

Each model's origin is its part's: the base centre, Blender axes being the part's, +Z the nose.
Sizes are the ones the craft gives the parts (mods/realism/craft/saturn_v.json5).
"""
import importlib
import math
import os
import random
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)
import osp_common as osp
importlib.reload(osp)
from mathutils import Matrix, Vector

OUT = os.path.join(osp.REPO_DIR, "mods", "realism", "models", "parts")
m = osp.material
PREVIEW = globals().get("OSP_PREVIEW_DIR")


def arc_panel(R, a0, a1, z0, z1, proud, segs=8):
    """A plate lying on a cylinder of radius R, from angle a0 to a1 (radians)."""
    verts, faces = [], []
    for j in range(segs + 1):
        a = a0 + (a1 - a0) * j / segs
        c, s = math.cos(a), math.sin(a)
        for r in (R - 0.01, R + proud):
            verts += [(r * c, r * s, z0), (r * c, r * s, z1)]
    for j in range(segs):
        i, n = 4 * j, 4 * (j + 1)
        faces += [(i + 2, n + 2, n + 3, i + 3), (i + 3, n + 3, n + 1, i + 1), (i, n, n + 2, i + 2)]
    faces += [(0, 2, 3, 1), (4 * segs + 2, 4 * segs, 4 * segs + 1, 4 * segs + 3)]
    return verts, faces


def around(n, offset=0.0):
    return [offset + osp.TAU * k / n for k in range(n)]


def turn(a):
    return Matrix.Rotation(a, 4, "Z")


def finish(name, objs, sc, checks=(), **view):
    path = osp.export_glb(sc, objs, os.path.join(OUT, name + ".glb"))
    osp.report(sc, objs, path, checks)
    if PREVIEW:
        osp.render_preview(sc, objs, os.path.join(PREVIEW, name + ".png"), samples=24, **view)


def service_module():
    R, H = 1.955, 4.8
    sc, coll = osp.new_scene("osp_service_module")
    shell = m("SM_Shell_Aluminium", "#c9cac6", 0.85, 0.38)
    dark = m("SM_Skirt_Dark", "#55565a", 0.7, 0.5)
    white = m("SM_Radiator_White", "#e8e8e2", 0.1, 0.35)
    steel = m("SM_Ring_Steel", "#8e8f92", 1.0, 0.3)
    seam = m("SM_Seam_Dark", "#3a3b3e", 0.6, 0.5)
    objs = [
        osp.make_object("SM_Shell", osp.Geo().add(osp.cylinder(R, 0.5, 4.74, 96, cap0=False, cap1=False)), [shell], coll, 30),
        osp.make_object("SM_Aft", osp.Geo().add(osp.lathe([(0, 0.15), (R - 0.06, 0.15), (R - 0.06, 0.0), (R, 0.0), (R, 0.5)], 96)), [dark], coll, 30),
        osp.make_object("SM_Ring", osp.Geo().add(osp.lathe([(R, 4.74), (R + 0.015, 4.75), (R + 0.015, H), (0, H)], 96)), [steel], coll, 30),
    ]
    g = osp.Geo()
    for a in around(6, math.radians(30)):  # the sector beams' seams
        g.add(arc_panel(R, a - 0.008, a + 0.008, 0.5, 4.74, 0.006, 1))
    for a in around(16):  # the aft and forward frames' fasteners
        for z in (0.56, 4.68):
            g.add(arc_panel(R, a - 0.012, a + 0.012, z - 0.012, z + 0.012, 0.006, 1))
    objs.append(osp.make_object("SM_Seams", g, [seam], coll, 30))
    g = osp.Geo()
    for a in around(4, math.radians(45)):  # what each RCS quad stands on
        g.add(arc_panel(R, a - 0.3, a + 0.3, 3.05, 4.15, 0.012, 10))
    g.add(arc_panel(R, -0.07, 0.07, 3.25, H, 0.09, 4))  # the umbilical's tunnel, to the capsule
    objs.append(osp.make_object("SM_Quad_Panels", g, [shell], coll, 30))
    g = osp.Geo()
    for c in (math.radians(90), math.radians(270)):  # the cooling loops' radiators
        for j in range(9):
            a = c - 0.5 + j * 0.125
            g.add(arc_panel(R, a, a + 0.105, 1.0, 2.8, 0.01, 2))
    for a in around(8, math.radians(22.5)):  # the fuel cells' radiators, between the quads
        if abs(math.sin(2 * a)) < 0.75:
            g.add(arc_panel(R, a - 0.13, a + 0.13, 4.2, 4.65, 0.012, 4))
    objs.append(osp.make_object("SM_Radiators", g, [white], coll, 30))
    # What the high gain antenna's hinge hangs from, on the aft bulkhead.
    objs.append(osp.make_object("SM_Antenna_Bracket", osp.Geo().add(
        osp.box((0.3, 0.3, 0.2), (0, -1.6, 0.05), bevel=0.01, bevel_segs=1)), [dark], coll, 35))
    finish("service_module", objs, sc, [("height", osp.evaluated_stats(sc, objs)[1][2], H)], elevation=12, azimuth=-60)


def high_gain_antenna():
    """As stowed for launch: origin on the hinge under the service module, the boom hanging down -Z
    and the dishes facing +Y, in towards the engine. The part's `swing` turns it 90 degrees about X,
    which puts the boom straight out from the module and the dishes facing aft."""
    sc, coll = osp.new_scene("osp_high_gain_antenna")
    white = m("HGA_Dish_White", "#e6e6e0", 0.1, 0.45)
    metal = m("HGA_Boom_Metal", "#a9aaac", 0.95, 0.35)
    hub = -1.9
    g = osp.Geo()
    g.add(osp.box((0.2, 0.2, 0.14), (0, 0, -0.02), bevel=0.012, bevel_segs=1))
    g.add(osp.cylinder(0.035, hub, -0.05, 16))
    g.add(osp.box((0.22, 0.09, 0.22), (0, 0, hub), bevel=0.01, bevel_segs=1))
    out = (0, 1, 0)
    centres = [(sx * 0.43, 0.03, hub + sz * 0.43) for sx in (-1, 1) for sz in (-1, 1)]
    for c in centres:
        g.add(osp.cyl_between((0, 0, hub), c, 0.018, 10))
        dish = osp.align_z(c, out)
        for a in around(3, 0.5):  # the feed's tripod
            foot = dish @ Vector((0.3 * math.cos(a), 0.3 * math.sin(a), 0.073))
            g.add(osp.cyl_between(foot, dish @ Vector((0, 0, 0.27)), 0.005, 6))
    g.add(osp.cylinder(0.05, 0.04, 0.2, 14), matrix=osp.align_z((0, 0, hub), out))  # the wide-beam horn
    objs = [osp.make_object("HGA_Boom", g, [metal], coll, 35)]
    g, feeds = osp.Geo(), osp.Geo()
    profile = [(0.4 * j / 12, 0.13 * (j / 12) ** 2) for j in range(13)]
    for c in centres:
        g.add(osp.lathe(profile, 36), matrix=osp.align_z(c, out))
        feeds.add(osp.lathe([(0, 0.25), (0.035, 0.25), (0.05, 0.31), (0, 0.31)], 12), matrix=osp.align_z(c, out))
    objs.append(osp.make_object("HGA_Dishes", g, [white], coll, 60, solidify=dict(thickness=0.008)))
    objs.append(osp.make_object("HGA_Feeds", feeds, [metal], coll, 35))
    finish("high_gain_antenna", objs, sc, elevation=25, azimuth=-50)


def les_tower():
    sc, coll = osp.new_scene("osp_les_tower")
    white = m("LES_Case_White", "#e4e4de", 0.1, 0.45)
    metal = m("LES_Truss_Metal", "#b5b6b8", 0.8, 0.4)
    dark = m("LES_Nozzle_Dark", "#2d2d30", 0.6, 0.5)
    cover = m("LES_Cover_Cream", "#d8d6cb", 0.05, 0.55)
    objs = [osp.make_object("LES_Cover", osp.Geo().add(osp.lathe(
        [(1.98, -3.3), (1.98, -3.25), (0.47, 0.0), (0.45, 0.1), (0, 0.1)], 64)), [cover], coll, 25)]
    g = osp.Geo()
    corners = [(sx * 0.4, sy * 0.4) for sx, sy in ((1, 1), (-1, 1), (-1, -1), (1, -1))]
    for k, (x, y) in enumerate(corners):
        g.add(osp.cyl_between((x, y, 0.05), (x, y, 2.75), 0.045, 10))
        nx, ny = corners[(k + 1) % 4]
        for z in (0.05, 0.95, 1.85):  # three bays of crossed bracing on each side
            g.add(osp.cyl_between((x, y, z), (nx, ny, z + 0.9), 0.022, 6))
            g.add(osp.cyl_between((nx, ny, z), (x, y, z + 0.9), 0.022, 6))
            g.add(osp.cyl_between((x, y, z + 0.9), (nx, ny, z + 0.9), 0.022, 6))
    g.add(osp.box((1.0, 1.0, 0.08), (0, 0, 2.75), bevel=0.01, bevel_segs=1))
    g.add(osp.box((1.0, 1.0, 0.08), (0, 0, 0.1), bevel=0.01, bevel_segs=1))
    objs.append(osp.make_object("LES_Truss", g, [metal], coll, 35))
    objs.append(osp.make_object("LES_Motor", osp.Geo().add(osp.lathe(
        [(0, 2.8), (0.58, 2.8), (0.33, 3.25), (0.33, 3.3), (0.345, 3.31), (0.345, 3.37), (0.33, 3.38), (0.33, 5.2),
         (0.34, 5.21), (0.34, 5.27), (0.33, 5.28), (0.33, 7.45), (0.24, 7.95), (0.1, 8.3), (0.04, 8.36)], 40)),
        [white], coll, 25))
    objs.append(osp.make_object("LES_Q_Ball", osp.Geo().add(osp.lathe(
        [(0.04, 8.36), (0.06, 8.39), (0.05, 8.43), (0.015, 8.45), (0, 8.45)], 16)), [dark], coll, 40))
    g = osp.Geo()
    nozzle = [(0.075, 0.0), (0.095, 0.03), (0.16, 0.3), (0.16, 0.31), (0.145, 0.31), (0.085, 0.05), (0, 0.04)]
    for a in around(4, math.radians(45)):  # the escape motor's four, canted out
        g.add(osp.lathe(nozzle, 20), matrix=osp.align_z(
            (0.3 * math.cos(a), 0.3 * math.sin(a), 3.0), (0.57 * math.cos(a), 0.57 * math.sin(a), -0.82)))
    for a in (0.0, math.pi):  # the jettison motor's two, high on the case
        g.add(osp.lathe([(0.04, 0), (0.1, 0.16), (0.09, 0.16), (0, 0.02)], 16), matrix=osp.align_z(
            (0.3 * math.cos(a), 0.3 * math.sin(a), 7.5), (0.8 * math.cos(a), 0.8 * math.sin(a), -0.6)))
    objs.append(osp.make_object("LES_Nozzles", g, [dark], coll, 35))
    g = osp.Geo()
    for a in (math.pi / 2, -math.pi / 2):  # the canards, folded along the nose
        g.add(arc_panel(0.235, a - 0.28, a + 0.28, 7.5, 7.95, 0.05, 3))
    objs.append(osp.make_object("LES_Canards", g, [metal], coll, 30))
    finish("les_tower", objs, sc, [("nose tip", osp.evaluated_stats(sc, objs)[1][2], 8.45)], elevation=8, azimuth=-40)


def _octagon(zs):
    """Points of an octagon's rim, in order, long faces across the axes."""
    a, b = 2.05, 1.04
    return [(a, -b), (a, b), (b, a), (-b, a), (-a, b), (-a, -b), (-b, -a), (b, -a)]


def _foil(corners, z0, z1, nu, nv, rng, depth):
    """A wall round a polygon, crumpled like foil. The corners stay put so that it closes."""
    verts, faces = [], []
    n = len(corners)
    for e in range(n):
        p, q = Vector((*corners[e], 0)), Vector((*corners[(e + 1) % n], 0))
        out = (q - p).cross(Vector((0, 0, 1))).normalized()
        base = len(verts)
        for j in range(nv + 1):
            for i in range(nu + 1):
                t = i / nu
                d = 0.0 if i in (0, nu) or j in (0, nv) else rng.uniform(-depth, depth)
                v = p.lerp(q, t) + out * d
                verts.append((v.x, v.y, z0 + (z1 - z0) * j / nv))
        for j in range(nv):
            for i in range(nu):
                c = base + j * (nu + 1) + i
                faces.append((c, c + 1, c + nu + 2, c + nu + 1))
    return verts, faces


def lm_descent_stage():
    sc, coll = osp.new_scene("osp_lm_descent_stage")
    rng = random.Random(11)
    gold = m("LM_Foil_Gold", "#c79a3a", 1.0, 0.33)
    black = m("LM_Foil_Black", "#17171a", 0.6, 0.42)
    silver = m("LM_Deck_Silver", "#b8b8ba", 0.95, 0.35)
    strut = m("LM_Strut_Metal", "#8a8476", 0.9, 0.4)
    rim = _octagon(None)
    objs = [osp.make_object("LM_Gold", osp.Geo().add(_foil(rim, 0.28, 1.56, 14, 10, rng, 0.016)), [gold], coll, 8)]
    objs.append(osp.make_object("LM_Skirt", osp.Geo().add(_foil(rim, 0.0, 0.28, 14, 3, rng, 0.012)), [black], coll, 8))
    g = osp.Geo().add(_foil(rim, 1.56, 1.7, 8, 1, rng, 0.0))
    n = len(g.verts)
    g.verts.extend((*c, 1.7) for c in rim)
    g.faces.append(tuple(range(n, n + 8)))
    g.mats.append(0)
    objs.append(osp.make_object("LM_Deck", g, [silver], coll, 30))
    # The underside, with the bay the descent engine hangs in.
    g = osp.Geo()
    n = 8
    g.verts.extend((*c, 0.0) for c in rim)
    bay = [(0.9 * math.cos(a), 0.9 * math.sin(a)) for a in around(8, math.radians(-26.9))]
    g.verts.extend((*c, 0.0) for c in bay)
    for k in range(8):
        g.faces.append((k + 8, (k + 1) % 8 + 8, (k + 1) % 8, k))
        g.mats.append(0)
    g.add(osp.lathe([(0.9, 0.0), (0.9, 1.35), (0, 1.35)], 8, phase=math.radians(-26.9)))
    objs.append(osp.make_object("LM_Underside", g, [black], coll, 25))
    g = osp.Geo()
    for k in range(4):  # where each leg folds from, with its bracing
        t = turn(k * math.pi / 2)
        g.add(osp.cyl_between((1.62, 0, 0.82), (2.08, 0, 0.47), 0.09, 14), matrix=t)
        g.add(osp.cyl_between((2.08, 0.42, 1.08), (2.03, 0, 0.5), 0.028, 8), matrix=t)
        g.add(osp.cyl_between((2.08, -0.42, 1.08), (2.03, 0, 0.5), 0.028, 8), matrix=t)
    for y in (-0.26, 0.26):  # the ladder's fixed end and the porch, on the forward face
        g.add(osp.cyl_between((2.2, y, 0.25), (2.2, y, 1.56), 0.018, 8))
        g.add(osp.cyl_between((2.2, y, 1.56), (2.06, y, 1.62), 0.018, 8))
    for z in [0.32 + 0.23 * j for j in range(6)]:
        g.add(osp.cyl_between((2.2, -0.26, z), (2.2, 0.26, z), 0.012, 6))
    g.add(osp.box((0.4, 0.86, 0.04), (2.25, 0, 1.68), bevel=0.006, bevel_segs=1))
    objs.append(osp.make_object("LM_Struts", g, [strut], coll, 35))
    g = osp.Geo()
    for k in (1, 3, 5, 7):  # the cargo bays' covers, on the short faces
        (x0, y0), (x1, y1) = rim[k], rim[(k + 1) % 8]
        mid = Vector(((x0 + x1) / 2, (y0 + y1) / 2, 0.86))
        out = Vector(((x0 + x1) / 2, (y0 + y1) / 2, 0)).normalized()
        g.add(osp.box((0.03, 0.82, 0.72), (0, 0, 0)), matrix=Matrix.Translation(mid + out * 0.014) @ turn(math.atan2(out.y, out.x)))
    objs.append(osp.make_object("LM_Bay_Covers", g, [silver], coll, 30))
    finish("lm_descent_stage", objs, sc, [("height", osp.evaluated_stats(sc, objs)[1][2], 1.7)], elevation=5, azimuth=-35)


def lm_foot_pad():
    sc, coll = osp.new_scene("osp_lm_foot_pad")
    gold = m("LM_Pad_Gold", "#c79a3a", 1.0, 0.36)
    metal = m("LM_Pad_Metal", "#8a8476", 0.9, 0.4)
    objs = [
        osp.make_object("LM_Pad", osp.Geo().add(osp.lathe(
            [(0, -0.04), (0.24, -0.04), (0.38, 0.03), (0.4, 0.09), (0.395, 0.11), (0.36, 0.1), (0.22, 0.05), (0, 0.04)], 48)),
            [gold], coll, 40),
        osp.make_object("LM_Socket", osp.Geo().add(osp.lathe(
            [(0.1, 0.04), (0.1, 0.1), (0.07, 0.15), (0, 0.16)], 18)), [metal], coll, 40),
    ]
    finish("lm_foot_pad", objs, sc, [("lowest", osp.evaluated_stats(sc, objs)[0][2], -0.04)], elevation=25)


def separation_motor():
    """One metre across and three long, as `separation_motor` sizes them: `scale` is its diameter."""
    sc, coll = osp.new_scene("osp_separation_motor")
    case = m("Motor_Case_White", "#dedfd9", 0.1, 0.5)
    band = m("Motor_Band_Metal", "#9b9c9f", 0.9, 0.35)
    dark = m("Motor_Nozzle_Dark", "#2a2a2d", 0.5, 0.55)
    objs = [
        osp.make_object("Motor_Case", osp.Geo().add(osp.lathe(
            [(0.2, 0.55), (0.47, 0.72), (0.5, 0.86), (0.5, 2.6), (0.42, 2.82), (0.24, 2.96), (0, 3.0)], 32)), [case], coll, 30),
        osp.make_object("Motor_Nozzle", osp.Geo().add(osp.lathe(
            [(0.4, 0.0), (0.16, 0.38), (0.2, 0.55), (0.0, 0.55), (0.14, 0.38), (0.37, 0.0)], 32)), [dark], coll, 30),
    ]
    g = osp.Geo()
    for z in (0.95, 1.75, 2.5):
        g.add(osp.cylinder(0.515, z, z + 0.07, 32))
    objs.append(osp.make_object("Motor_Bands", g, [band], coll, 30))
    finish("separation_motor", objs, sc, [("length", osp.evaluated_stats(sc, objs)[1][2], 3.0)], elevation=10)


def instrument_unit():
    R, H = 3.3, 0.91
    sc, coll = osp.new_scene("osp_instrument_unit")
    skin = m("IU_Skin_White", "#dcdcd6", 0.15, 0.5)
    box = m("IU_Box_Grey", "#6f7074", 0.7, 0.45)
    seam = m("IU_Seam_Dark", "#3a3b3e", 0.6, 0.5)
    objs = [osp.make_object("IU_Ring", osp.Geo().add(osp.lathe(
        [(R - 0.2, 0.0), (R, 0.0), (R, H), (R - 0.2, H), (R - 0.2, 0.0)], 128, closed=False)), [skin], coll, 30)]
    g = osp.Geo()
    for a in around(3):
        g.add(arc_panel(R, a - 0.006, a + 0.006, 0.0, H, 0.005, 1))
    for a in around(48):
        for z in (0.05, H - 0.05):
            g.add(arc_panel(R, a - 0.005, a + 0.005, z - 0.01, z + 0.01, 0.004, 1))
    objs.append(osp.make_object("IU_Seams", g, [seam], coll, 30))
    g = osp.Geo()
    for k, a in enumerate(around(20)):  # the equipment on its inner wall, with room over the ring
        d = 0.24 + 0.1 * (k % 3)
        g.add(osp.box((d, 0.62, 0.6), (R - 0.2 - d / 2, 0, 0.4), bevel=0.012, bevel_segs=1), matrix=turn(a))
    objs.append(osp.make_object("IU_Equipment", g, [box], coll, 30))
    finish("instrument_unit", objs, sc, [("diameter", osp.evaluated_stats(sc, objs)[1][0] * 2, 6.6)], elevation=35)


def s_ivb_aps():
    """An RCS block's frame: origin on the hull, +X out of it, +Z towards the nose."""
    sc, coll = osp.new_scene("osp_s_ivb_aps")
    white = m("APS_Fairing_White", "#e2e2dc", 0.1, 0.45)
    dark = m("APS_Nozzle_Dark", "#26262a", 0.7, 0.45)
    metal = m("APS_Base_Metal", "#8e8f92", 0.9, 0.4)
    profile = [(0, -0.75), (0.16, -0.68), (0.3, -0.5), (0.36, -0.25), (0.36, 0.25), (0.3, 0.5), (0.16, 0.68), (0, 0.75)]
    pod = Matrix.Diagonal((0.75, 1.0, 1.0, 1.0))
    objs = [osp.make_object("APS_Fairing", osp.Geo().add(osp.lathe(profile, 40), matrix=pod), [white], coll, 40)]
    objs.append(osp.make_object("APS_Base", osp.Geo().add(osp.box((0.03, 0.62, 1.2), (0.0, 0, 0))), [metal], coll, 40))
    g = osp.Geo()
    nozzle = [(0.018, 0.0), (0.05, 0.1), (0.044, 0.1), (0, 0.02)]
    for p, d in (((0.13, 0, 0.58), (0, 0, 1)), ((0.13, 0, -0.58), (0, 0, -1)), ((0.13, 0.32, 0), (0, 1, 0)), ((0.13, -0.32, 0), (0, -1, 0))):
        g.add(osp.lathe(nozzle, 16), matrix=osp.align_z(p, d))
    objs.append(osp.make_object("APS_Nozzles", g, [dark], coll, 40))
    finish("s_ivb_aps", objs, sc, elevation=15, azimuth=-20)


ONLY = globals().get("OSP_ONLY")
for build in (service_module, high_gain_antenna, les_tower, lm_descent_stage, lm_foot_pad,
              separation_motor, instrument_unit, s_ivb_aps):
    if not ONLY or build.__name__ in ONLY:
        build()
