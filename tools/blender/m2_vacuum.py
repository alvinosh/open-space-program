"""M-2 Vacuum: the M-2 powerhead with a vacuum nozzle (Raptor Vacuum class).

Length 4.6 m, exit diameter 2.30 m (area ratio ~90), envelope radius 1.2 m. A short regeneratively
cooled nozzle section down to z = -1.8 ends in a bolted flange; below it a huge, thin, radiatively
cooled steel extension, stiffened by hat-section ribs and a hat-section exit stiffener.
Origin = gimbal pivot at the top centre; nozzle exit plane at z = -4.6.
"""
import importlib
import math
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)
import osp_common as osp
import m2_common as m2
importlib.reload(osp)
importlib.reload(m2)

NAME, OUT = "m2_vacuum", "models/engines/m2_vacuum.glb"
LENGTH, RE, ENVELOPE = 4.6, 1.15, 1.2
RT = RE / math.sqrt(90.0)
Z_JOINT = -1.8                    # regen section / extension flange
Z_FM = Z_JOINT + 0.08             # regen inlet manifold, just above the flange
EXT_SEGS = 64


def hat_rib(profile, zc, width, height, t=0.004, crown=0.4, flange=0.7, inset=0.003):
    """Closed lathe profile of a hat-section stiffening ring on the bell: thin flanges on the
    shell, sloped walls, a flat crown `height` proud of the shell."""
    r = lambda z: osp.interp_r(profile, z)
    z0, z5 = zc - width / 2, zc + width / 2
    z1, z4 = zc - flange * width / 2, zc + flange * width / 2
    z2, z3 = zc - crown * width / 2, zc + crown * width / 2
    return [(r(z0) - inset, z0), (r(z0) + t, z0), (r(z1) + t, z1), (r(z2) + height, z2),
            (r(z3) + height, z3), (r(z4) + t, z4), (r(z5) + t, z5), (r(z5) - inset, z5)]


sc, coll = osp.new_scene("osp_" + NAME)
mats = m2.materials()
m = osp.material
mats["ext"] = m("M2V_NozzleExtension_HeatTintedSteel", "#48423c", 0.85, 0.36)
mats["ext_in"] = m("M2V_NozzleExtension_HeatTintedInner", "#58514a", 0.75, 0.48)
mats["rib"] = m("M2V_HatRib_HeatTintedSteel", "#4c4640", 0.85, 0.34)
mats["flange"] = m("M2V_ExtensionFlange_Inconel", "#77746f", 1.0, 0.36)

prof = m2.chamber_and_bell(RT, RE, -LENGTH, theta_n=40.0, theta_e=7.0, n_bell=12)
ext, regen = osp.split_profile(prof, Z_JOINT)
r_joint = regen[0][0]
r_fm = osp.interp_r(prof, Z_FM) + 0.03
objs = m2.build_powerhead(coll, mats, "M2V", prof, Z_FM, r_fm)
objs.append(m2.chamber_details(coll, mats, "M2V", prof))

# Regen section: chamber above the throat jacket, burnished regen bell down to the flange.
segs = 56
z_split = m2.Z_T - 0.03
slots = m2.segment_slots(regen, segs, lambda z: 2 if z > z_split else 0)
objs.append(osp.make_object("M2V_RegenNozzle", osp.Geo().add(osp.lathe(regen, segs), slots),
                            [mats["bell"], mats["bell_in"], mats["chamber"], mats["chamber_in"]], coll, 40,
                            solidify=dict(thickness=m2.WALL, material_offset=1)))

# Radiatively cooled extension: one thin shell.
extension = osp.make_object("M2V_NozzleExtension", osp.Geo().add(osp.lathe(ext, EXT_SEGS)),
                            [mats["ext"], mats["ext_in"]], coll, 50,
                            solidify=dict(thickness=0.005, material_offset=1))
objs.append(extension)

# Bolted joint: regen flange over extension flange with a parting groove, bolt heads on top.
g = osp.Geo()
za, zb = Z_JOINT - 0.03, Z_JOINT + 0.035
ro = r_joint + 0.036
g.add(osp.lathe([(osp.interp_r(prof, za) - 0.003, za), (ro, za), (ro - 0.007, Z_JOINT), (ro, zb),
                 (osp.interp_r(prof, zb) - 0.003, zb)], EXT_SEGS))
m2.bolt_ring(g, (0, 0), r_joint + 0.022, 24, 0.008, zb - 0.002, zb + 0.012)
objs.append(osp.make_object("M2V_ExtensionFlange", g, [mats["flange"]], coll, 40))

# Hat-section stiffening ribs down the extension, and a hat-section exit stiffener whose lower
# edge sits just above the exit plane. Rings and flange are open lathes: their inner faces would
# be buried in the shell.
g = osp.Geo()
for zc in (-2.65, -3.55):
    g.add(osp.lathe(hat_rib(prof, zc, 0.11, 0.028), EXT_SEGS))
g.add(osp.lathe(hat_rib(prof, -LENGTH + 0.012 + 0.06, 0.12, 0.03), EXT_SEGS))
objs.append(osp.make_object("M2V_HatRibs", g, [mats["rib"]], coll, 40))

path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
osp.report(sc, objs, path, [
    ("length (-zmin)", -lo[2], LENGTH),
    ("top z (pivot at 0)", hi[2], 0.0),
    ("exit diameter", 2 * osp.max_radius_near(sc, extension, -LENGTH), 2 * RE),
    ("area ratio", (RE / RT) ** 2, 90.0),
])
print(f"  joint diameter {2 * r_joint:.4f}")
print(f"  envelope radius {osp.max_radius(sc, objs):.4f} (limit {ENVELOPE})")
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=-14)

DEBUG_VIEWS = [("below", -65, -40), ("above", 30, 140),
               ("top", 15, -40, ((-0.8, -0.8, -2.0), (0.8, 0.8, 0.0)))]
