"""The space centre's buildings: the Vehicle Assembly and the Hangar at each of the builder's four
levels, Research, Mission Control and the Tracking Station. Original designs, seen from a kilometre
or two: big plain volumes with ribs, doors, window bands and a teal band that say what they are.

Origin = the middle of the footprint on the ground. X across, +Y to the front (the doors), +Z up;
the export turns +Y into the game's -Z, the front a building's layout faces. Each model's
footprint and height are the `levels` sizes in `assets/core/space_centres/stock.json5`
([along +Y, across X, height]); the report checks them.

Run headless: `blender -b --factory-startup --python tools/blender/space_centre.py`.
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
from mathutils import Matrix

m = osp.material

# Along (Y), across (X) and height of each level, as the records have them.
ASSEMBLY = [(60, 70, 48), (85, 105, 72), (120, 150, 108), (158, 218, 160)]
HANGAR = [(36, 40, 14), (50, 60, 20), (70, 80, 26), (90, 130, 32)]
RESEARCH = (70, 95, 28)
MISSION = (60, 110, 24)
TRACKING = (45, 45, 34)


def palette(prefix):
    return {
        "wall": m(prefix + "_Wall_Concrete", "#d6d4cc", 0.0, 0.85),
        "panel": m(prefix + "_Panel_Concrete", "#bdbbb3", 0.0, 0.85),
        "roof": m(prefix + "_Roof_Grey", "#5a5d61", 0.0, 0.9),
        "door": m(prefix + "_Door_Steel", "#3c4148", 0.4, 0.5),
        "line": m(prefix + "_Seam_Dark", "#25292e", 0.3, 0.6),
        "glass": m(prefix + "_Window_Glass", "#1e2c39", 0.3, 0.15),
        "band": m(prefix + "_Band_Teal", "#1f8f86", 0.1, 0.5),
        "steel": m(prefix + "_Frame_Steel", "#8d9195", 0.8, 0.35),
        "white": m(prefix + "_Dish_White", "#e8e8e4", 0.05, 0.4),
    }


SLOTS = ["wall", "panel", "roof", "door", "line", "glass", "band", "steel", "white"]


def slot(name):
    return SLOTS.index(name)


def block(g, mat, x0, x1, y0, y1, z0, z1):
    """An axis-aligned box from its extents."""
    g.add(osp.box((x1 - x0, y1 - y0, z1 - z0), ((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2)),
          slot(mat))


def roof_kit(g, x0, x1, y0, y1, z, units, seed=1):
    """A roof slab with a parapet round it and a few plant units on it."""
    block(g, "roof", x0, x1, y0, y1, z, z + 0.4)
    p = 0.8
    for (a0, a1, b0, b1) in [(x0, x1, y0, y0 + p), (x0, x1, y1 - p, y1),
                             (x0, x0 + p, y0, y1), (x1 - p, x1, y0, y1)]:
        block(g, "panel", a0, a1, b0, b1, z, z + 1.4)
    w, d = x1 - x0, y1 - y0
    for k in range(units):
        u = ((k * 0.37 + seed * 0.19) % 1.0) * 0.6 + 0.2
        v = ((k * 0.61 + seed * 0.43) % 1.0) * 0.6 + 0.2
        s = min(w, d) * 0.06 + 1.5
        cx, cy = x0 + w * u, y0 + d * v
        block(g, "steel", cx - s, cx + s, cy - s * 0.7, cy + s * 0.7, z + 0.4, z + 0.4 + s * 0.6)


def window_bands(g, x0, x1, y0, y1, z0, z1, floor=4.0, sill=1.2, head=2.6):
    """A band of glass round each floor of a block, a little proud of its walls."""
    z = z0 + sill
    while z + head - sill <= z1 - 1.0:
        block(g, "glass", x0 - 0.15, x1 + 0.15, y0 - 0.15, y1 + 0.15, z, z + head - sill)
        z += floor


def vehicle_assembly(level):
    along, across, height = ASSEMBLY[level]
    name = f"vehicle_assembly_{level + 1}"
    sc, coll = osp.new_scene("osp_" + name)
    mats = palette("Assembly")
    g = osp.Geo()
    hx0, hx1 = -across / 2, across * 0.22
    y0, y1 = -along / 2, along / 2
    # A dark plinth, the high bay, and the low bay along its +X side.
    block(g, "line", -across / 2 - 0.5, across / 2 + 0.5, y0 - 0.5, y1 + 0.5, 0.0, 1.2)
    block(g, "wall", hx0, hx1, y0, y1, 0.0, height)
    low = height * 0.38
    block(g, "panel", hx1, across / 2, -along * 0.42, along * 0.42, 0.0, low)
    window_bands(g, hx1, across / 2, -along * 0.42, along * 0.42, 0.0, low, floor=6.0)
    roof_kit(g, hx1, across / 2, -along * 0.42, along * 0.42, low, 3, seed=level)
    # Ribs up every face of the high bay.
    step = max(6.0, along / 12)
    rib = 0.7
    n = int(along // step)
    for k in range(1, n):
        y = y0 + k * along / n
        block(g, "panel", hx0 - rib, hx0, y - 0.6, y + 0.6, 1.2, height - 2.0)
    n = int((hx1 - hx0) // step)
    for k in range(1, n):
        x = hx0 + k * (hx1 - hx0) / n
        for face in (y0, y1):
            sign = 1 if face == y1 else -1
            block(g, "panel", x - 0.6, x + 0.6, face, face + sign * rib, 1.2, height - 2.0)
    # Tall doors front and back, one for each bay: more bays the bigger it is.
    bays = 1 + level // 2
    span = (hx1 - hx0) / bays
    door_w = min(span * 0.62, 34.0)
    door_h = height * 0.86
    for b in range(bays):
        cx = hx0 + span * (b + 0.5)
        for face, sign in ((y1, 1), (y0, -1)):
            proud = face + sign * 0.5
            block(g, "door", cx - door_w / 2, cx + door_w / 2, min(face, proud), max(face, proud),
                  0.0, door_h)
            # The door's panels, which slide up.
            z = 6.0
            while z < door_h - 1.0:
                p0, p1 = sorted((proud, proud + sign * 0.25))
                block(g, "line", cx - door_w / 2, cx + door_w / 2, p0, p1, z - 0.3, z + 0.3)
                z += 7.0
            p0, p1 = sorted((proud, proud + sign * 0.3))
            block(g, "line", cx - 0.4, cx + 0.4, p0, p1, 0.0, door_h)
    # A teal band round the top of the high bay.
    top = height * 0.9
    block(g, "band", hx0 - 0.4, hx1 + 0.4, y0 - 0.4, y1 + 0.4, top - height * 0.035, top)
    roof_kit(g, hx0, hx1, y0, y1, height, 4 + level, seed=level + 3)
    ob = osp.make_object("Assembly", g, [mats[s] for s in SLOTS], coll, smooth_angle=20)
    return sc, [ob], f"models/space_centre/{name}.glb", (along, across, height)


def hangar(level):
    along, across, height = HANGAR[level]
    name = f"hangar_{level + 1}"
    sc, coll = osp.new_scene("osp_" + name)
    mats = palette("Hangar")
    g = osp.Geo()
    shed = across * 0.78
    x0 = -across / 2
    xc = x0 + shed / 2
    wall = height * 0.45
    y0, y1 = -along / 2, along / 2

    def section(width, low, high, out=0.0):
        """The arch, counter-clockwise in (x, z): walls to `low`, a vault up to `high`."""
        half = width / 2 + out
        pts = [(-half, 0.0), (half, 0.0), (half, low + out)]
        n = 16
        for i in range(1, n):
            t = math.pi * i / n
            pts.append((half * math.cos(t), low + out + (high - low) * math.sin(t)))
        pts.append((-half, low + out))
        return pts

    to_y = Matrix.Translation((xc, 0, 0)) @ Matrix.Rotation(math.pi / 2, 4, "X")
    g.add(osp.prism(section(shed, wall, height - 0.5), y0, y1), slot("wall"), matrix=to_y)
    # Steel ribs over the vault.
    ribs = 6 + level * 2
    for k in range(ribs + 1):
        y = y0 + 1.0 + k * (along - 2.0) / ribs
        g.add(osp.prism(section(shed, wall, height - 0.5, out=0.35), y - 0.4, y + 0.4),
              slot("steel"), matrix=to_y)
    # The doors across the front: wide, in leaves.
    door_h = wall + (height - wall) * 0.55
    door_w = shed * 0.86
    block(g, "door", xc - door_w / 2, xc + door_w / 2, y1, y1 + 0.6, 0.0, door_h)
    leaves = 4 + level
    for k in range(1, leaves):
        x = xc - door_w / 2 + k * door_w / leaves
        block(g, "line", x - 0.25, x + 0.25, y1 + 0.6, y1 + 0.85, 0.0, door_h)
    block(g, "band", xc - door_w / 2 - 0.5, xc + door_w / 2 + 0.5, y1, y1 + 0.9, door_h, door_h + 1.2)
    # Offices along its +X side: a low block with windows.
    ox0 = x0 + shed
    block(g, "panel", ox0, across / 2, -along * 0.4, along * 0.4, 0.0, min(9.0, height * 0.55))
    window_bands(g, ox0, across / 2, -along * 0.4, along * 0.4, 0.0, min(9.0, height * 0.55))
    roof_kit(g, ox0, across / 2, -along * 0.4, along * 0.4, min(9.0, height * 0.55), 2, seed=level)
    ob = osp.make_object("Hangar", g, [mats[s] for s in SLOTS], coll, smooth_angle=30)
    return sc, [ob], f"models/space_centre/{name}.glb", (along, across, height)


def research():
    along, across, height = RESEARCH
    sc, coll = osp.new_scene("osp_research")
    mats = palette("Research")
    g = osp.Geo()
    # Two wings in an L and a drum with a dome on the corner where they meet.
    block(g, "wall", -across / 2, across * 0.2, along * 0.05, along / 2, 0.0, 16.0)
    window_bands(g, -across / 2, across * 0.2, along * 0.05, along / 2, 0.0, 16.0)
    roof_kit(g, -across / 2, across * 0.2, along * 0.05, along / 2, 16.0, 3, seed=2)
    block(g, "panel", -across * 0.05, across / 2, -along / 2, along * 0.15, 0.0, 12.0)
    window_bands(g, -across * 0.05, across / 2, -along / 2, along * 0.15, 0.0, 12.0)
    roof_kit(g, -across * 0.05, across / 2, -along / 2, along * 0.15, 12.0, 2, seed=5)
    cx, cy, r = across * 0.3, along * 0.3, 9.0
    drum = [(0.0, 0.0), (r, 0.0), (r, 16.0), (r + 0.4, 16.0), (r + 0.4, 17.0), (0.0, 17.0)]
    g.add(osp.lathe(drum, 48), slot("panel"), matrix=Matrix.Translation((cx, cy, 0)))
    arc = [math.pi / 2 * i / 12 for i in range(1, 12)]
    dome = [(0.0, 17.0), (r - 0.3, 17.0)] + [(r * math.cos(t), 17.0 + r * math.sin(t)) for t in arc]
    dome.append((0.0, 17.0 + r))
    g.add(osp.lathe(dome, 48), slot("white"), matrix=Matrix.Translation((cx, cy, 0)))
    # The dome's slit, a dark strip over its top.
    block(g, "line", cx - 1.0, cx + 1.0, cy - 1.0, cy + r * 0.8, 17.0 + r * 0.55, 17.0 + r + 0.2)
    block(g, "band", -across / 2 - 0.3, across * 0.2 + 0.3, along / 2, along / 2 + 0.4, 14.2, 15.4)
    ob = osp.make_object("Research", g, [mats[s] for s in SLOTS], coll, smooth_angle=35)
    return sc, [ob], "models/space_centre/research.glb", (along, across, 17.0 + r)


def mission_control():
    along, across, height = MISSION
    sc, coll = osp.new_scene("osp_mission_control")
    mats = palette("Mission")
    g = osp.Geo()
    # A long block, all glass at the front between fins, and a tower with a mast in its middle.
    y0, y1 = -along / 2, along * 0.25
    block(g, "wall", -across / 2, across / 2, y0, y1, 0.0, 14.0)
    block(g, "glass", -across / 2 + 2.0, across / 2 - 2.0, y1, y1 + 0.3, 1.0, 13.0)
    fins = 22
    for k in range(fins + 1):
        x = -across / 2 + 2.0 + k * (across - 4.0) / fins
        block(g, "panel", x - 0.35, x + 0.35, y1, y1 + 1.2, 0.0, 14.0)
    roof_kit(g, -across / 2, across / 2, y0, y1, 14.0, 4, seed=7)
    block(g, "wall", -12.0, 12.0, y0 + 6.0, y0 + 30.0, 0.0, 20.0)
    window_bands(g, -12.0, 12.0, y0 + 6.0, y0 + 30.0, 14.0, 20.0, floor=3.0)
    block(g, "band", -12.4, 12.4, y0 + 5.6, y0 + 30.4, 18.6, 19.6)
    roof_kit(g, -12.0, 12.0, y0 + 6.0, y0 + 30.0, 20.0, 1, seed=9)
    g.add(osp.cylinder(0.35, 20.4, height, 8), slot("steel"),
          matrix=Matrix.Translation((4.0, y0 + 12.0, 0)))
    # A canopy over the way in.
    block(g, "panel", -8.0, 8.0, y1 + 1.2, along / 2, 4.2, 4.8)
    for x in (-7.5, 7.5):
        block(g, "steel", x - 0.3, x + 0.3, along / 2 - 0.9, along / 2 - 0.3, 0.0, 4.2)
    ob = osp.make_object("Mission", g, [mats[s] for s in SLOTS], coll, smooth_angle=30)
    return sc, [ob], "models/space_centre/mission_control.glb", (along, across, height)


def dish(g, radius, depth, centre, tilt, turn):
    """A parabolic dish of `radius`, its focus feed on three struts, facing up `tilt` from straight
    up and round `turn` about Z, its back on `centre`."""
    n = 14
    front = [(radius * i / n, depth * (i / n) ** 2) for i in range(n + 1)]
    # Counter-clockwise in (r, z): out along the back, up the rim, in along the dish's face.
    profile = [(0.0, -0.5)] + [(r, z - 0.5) for r, z in front[1:]] + \
        [(r, z) for r, z in reversed(front[1:])] + [(0.0, 0.0)]
    place = Matrix.Translation(centre) @ Matrix.Rotation(turn, 4, "Z") @ Matrix.Rotation(tilt, 4, "X")
    g.add(osp.lathe(profile, 40), slot("white"), matrix=place)
    focus = radius * radius / (4 * depth)
    for k in range(3):
        a = osp.TAU * k / 3
        rim = (radius * 0.95 * math.cos(a), radius * 0.95 * math.sin(a), depth * 0.9)
        g.add(osp.cyl_between(rim, (0, 0, focus), max(0.12, radius * 0.012), 6), slot("steel"),
              matrix=place)
    g.add(osp.box((radius * 0.12, radius * 0.12, radius * 0.16), (0, 0, focus)), slot("door"),
          matrix=place)


def tracking():
    along, across, height = TRACKING
    sc, coll = osp.new_scene("osp_tracking")
    mats = palette("Tracking")
    g = osp.Geo()
    # A low operations block at the back, a big dish on a pedestal and a small one beside it.
    block(g, "wall", -across / 2, across / 2, -along / 2, -along * 0.18, 0.0, 6.0)
    window_bands(g, -across / 2, across / 2, -along / 2, -along * 0.18, 0.0, 6.0)
    roof_kit(g, -across / 2, across / 2, -along / 2, -along * 0.18, 6.0, 2, seed=4)
    ped = [(0.0, 0.0), (4.0, 0.0), (3.2, 12.0), (0.0, 12.0)]
    g.add(osp.lathe(ped, 24), slot("panel"), matrix=Matrix.Translation((-4.0, along * 0.12, 0)))
    g.add(osp.box((8.0, 3.0, 4.0), (-4.0, along * 0.12, 14.0)), slot("steel"))
    dish(g, 11.0, 3.2, (-4.0, along * 0.12, 15.0), math.radians(40), math.radians(150))
    small = [(0.0, 0.0), (1.4, 0.0), (1.1, 5.0), (0.0, 5.0)]
    g.add(osp.lathe(small, 16), slot("panel"), matrix=Matrix.Translation((across * 0.32, along * 0.3, 0)))
    dish(g, 3.5, 1.0, (across * 0.32, along * 0.3, 5.5), math.radians(55), math.radians(200))
    ob = osp.make_object("Tracking", g, [mats[s] for s in SLOTS], coll, smooth_angle=40)
    return sc, [ob], "models/space_centre/tracking.glb", (along, across, height)


def build(made):
    sc, objs, out, (along, across, height) = made
    path = osp.export_glb(sc, objs, out)
    lo, hi, _ = osp.evaluated_stats(sc, objs)
    osp.report(sc, objs, path, [
        ("across (X), at most", min(hi[0] - lo[0], across), across),
        ("along (Y), at most", min(hi[1] - lo[1], along), along),
        ("height, at most", min(hi[2], height), height),
    ])


for lvl in range(4):
    build(vehicle_assembly(lvl))
    build(hangar(lvl))
build(research())
build(mission_control())
build(tracking())
