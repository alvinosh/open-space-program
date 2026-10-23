"""M-2: sea-level full-flow staged-combustion methalox engine (Raptor 2 class).

Length 3.1 m, exit diameter 1.30 m (area ratio ~34), envelope radius 0.75 m. The M-2 powerhead
(m2_common) over a compact regeneratively cooled chamber and a short, smooth, dark burnished
sea-level bell with a regen inlet manifold below the throat and a stiffened exit lip.
Origin = gimbal pivot at the top centre; nozzle exit plane at z = -3.1.
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

NAME, OUT = "m2", "models/engines/m2.glb"
LENGTH, RE, ENVELOPE = 3.1, 0.65, 0.75
RT = RE / math.sqrt(34.0)
Z_FM = -1.45                      # regen inlet manifold

sc, coll = osp.new_scene("osp_" + NAME)
mats = m2.materials()
prof = m2.chamber_and_bell(RT, RE, -LENGTH, theta_n=34.0, theta_e=7.0, n_bell=14)
r_fm = osp.interp_r(prof, Z_FM) + 0.03
objs = m2.build_powerhead(coll, mats, "M2", prof, Z_FM, r_fm)
objs.append(m2.chamber_details(coll, mats, "M2", prof))

# Nozzle and chamber: one regen shell; chamber above the throat jacket, burnished bell below.
segs = m2.SEGS
z_split = m2.Z_T - 0.03
slots = m2.segment_slots(prof, segs, lambda z: 2 if z > z_split else 0)
nozzle = osp.make_object("M2_Nozzle", osp.Geo().add(osp.lathe(prof, segs), slots),
                         [mats["bell"], mats["bell_in"], mats["chamber"], mats["chamber_in"]], coll, 40,
                         solidify=dict(thickness=m2.WALL, material_offset=1))
objs.append(nozzle)

# Bell hardware: the collar under the regen inlet manifold and the rolled exit stiffener.
g = osp.Geo()
g.add(osp.lathe(osp.band(prof, Z_FM + 0.04, 0.08, 0.012, inset=0.003), 64))   # open: inner face
g.add(osp.lathe(osp.band(prof, -LENGTH + 0.05, 0.045, 0.012, inset=0.003), segs))  # is buried
objs.append(osp.make_object("M2_NozzleBands", g, [mats["bell"]], coll, 40))

path = osp.export_glb(sc, objs, OUT)
lo, hi, _ = osp.evaluated_stats(sc, objs)
osp.report(sc, objs, path, [
    ("length (-zmin)", -lo[2], LENGTH),
    ("top z (pivot at 0)", hi[2], 0.0),
    ("exit diameter", 2 * osp.max_radius_near(sc, nozzle, -LENGTH), 2 * RE),
    ("area ratio", (RE / RT) ** 2, 34.0),
])
print(f"  envelope radius {osp.max_radius(sc, objs):.4f} (limit {ENVELOPE})")
if "OSP_PREVIEW_DIR" in globals():
    osp.render_preview(sc, objs, os.path.join(OSP_PREVIEW_DIR, NAME + ".png"), elevation=-14)

DEBUG_VIEWS = [("below", -65, -40), ("above", 30, 140),
               ("top", 15, -40, ((-0.75, -0.75, -1.4), (0.75, 0.75, 0.0)))]
