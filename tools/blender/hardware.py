"""Small fixed-size hardware: the two reaction wheels, the fuel line fitting and the landing wheel.

Chunky shapes that read at a distance, in off-white, dark grey and warning yellow. Original designs.
Reaction wheels: origin = centre of the base, 0.5 m across and 0.2 m tall, +Z up. Fuel line fitting:
origin = centre of the base on the hull, +Z out of it, 0.18 m across. Wheel: radius 1, 0.7 wide, axle
along Z, scaled to the leg by the game.
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
SEGS = 48


def palette(prefix):
    return {
        "white": m(prefix + "_Body_OffWhite", "#dad9d2", 0.05, 0.55),
        "dark": m(prefix + "_Panel_DarkGrey", "#2f3237", 0.35, 0.55),
        "yellow": m(prefix + "_Band_WarningYellow", "#f0b410", 0.0, 0.45),
        "steel": m(prefix + "_Fitting_Steel", "#a9abad", 1.0, 0.32),
        "orange": m(prefix + "_Rotor_Orange", "#e0641e", 0.1, 0.4),
        "rubber": m(prefix + "_Tyre_Rubber", "#17181a", 0.0, 0.85),
    }


def at_angle(angle, x, z=0.0):
    return Matrix.Rotation(angle, 4, "Z") @ Matrix.Translation((x, 0, z))


def bolts(g, mat, count, radius, z, r=0.009, h=0.006, phase=0.0):
    for i in range(count):
        a = phase + osp.TAU * i / count
        g.add(osp.cylinder(r, z, z + h, 6), mat,
              matrix=Matrix.Translation((radius * math.cos(a), radius * math.sin(a), 0)))


def attitude_unit():
    name, out = "attitude_unit", "models/control/attitude_unit.glb"
    sc, coll = osp.new_scene("osp_" + name)
    mats = palette("AttitudeUnit")
    objs = []
    # The drum, its edges bevelled, a little waisted so the ribs stand out.
    drum = [(0.0, 0.0), (0.232, 0.0), (0.243, 0.008), (0.243, 0.026), (0.226, 0.034), (0.226, 0.141),
            (0.243, 0.149), (0.243, 0.167), (0.232, 0.175), (0.0, 0.175)]
    objs.append(osp.make_object("Drum", osp.Geo().add(osp.lathe(drum, SEGS)), [mats["white"]], coll))
    # A yellow band round the waist, and eight ribs across it.
    band = [(0.226, 0.074), (0.232, 0.078), (0.232, 0.098), (0.226, 0.102)]
    objs.append(osp.make_object("Band", osp.Geo().add(osp.lathe(band, SEGS)), [mats["yellow"]], coll))
    g = osp.Geo()
    for i in range(8):
        a = osp.TAU * i / 8
        g.add(osp.box((0.03, 0.05, 0.1), (0.0, 0.0, 0.0), bevel=0.006, bevel_segs=1),
              matrix=at_angle(a, 0.232, 0.0875))
    objs.append(osp.make_object("Ribs", g, [mats["dark"]], coll))
    # The lid: a dark inset plate, a bolt circle and a raised hub with a yellow ring.
    lid = [(0.205, 0.175), (0.205, 0.180), (0.196, 0.184), (0.0, 0.184)]
    objs.append(osp.make_object("Lid", osp.Geo().add(osp.lathe(lid, SEGS)), [mats["dark"]], coll))
    hub = [(0.075, 0.184), (0.075, 0.195), (0.066, 0.2), (0.0, 0.2)]
    ring = [(0.090, 0.184), (0.090, 0.190), (0.078, 0.190), (0.078, 0.184)]
    objs.append(osp.make_object("Hub", osp.Geo().add(osp.lathe(hub, SEGS)), [mats["steel"]], coll))
    objs.append(osp.make_object("HubRing", osp.Geo().add(osp.lathe(ring, SEGS)), [mats["yellow"]], coll))
    g = osp.Geo()
    bolts(g, 0, 12, 0.165, 0.184)
    objs.append(osp.make_object("Bolts", g, [mats["steel"]], coll))
    path = osp.export_glb(sc, objs, out)
    osp.report(sc, objs, path, [])


def small_wheel():
    name, out = "rw_1", "models/control/rw_1.glb"
    sc, coll = osp.new_scene("osp_" + name)
    mats = palette("RW1")
    objs = []
    # An open frame with the flywheel showing through it.
    base = [(0.0, 0.0), (0.24, 0.0), (0.25, 0.006), (0.25, 0.03), (0.0, 0.03)]
    rim = [(0.25, 0.03), (0.25, 0.17), (0.242, 0.18), (0.215, 0.18), (0.215, 0.03)]
    objs.append(osp.make_object("Base", osp.Geo().add(osp.lathe(base, SEGS)), [mats["dark"]], coll))
    objs.append(osp.make_object("Rim", osp.Geo().add(osp.lathe(rim, SEGS)), [mats["white"]], coll))
    # The flywheel, orange so that it reads, with a stepped face.
    fly = [(0.0, 0.05), (0.195, 0.05), (0.2, 0.06), (0.2, 0.10), (0.165, 0.115), (0.09, 0.115),
           (0.08, 0.145), (0.0, 0.145)]
    objs.append(osp.make_object("Flywheel", osp.Geo().add(osp.lathe(fly, SEGS)), [mats["orange"]], coll))
    # Three arms across the top carry the bearing.
    g = osp.Geo()
    for i in range(3):
        a = osp.TAU * i / 3 + 0.5
        g.add(osp.box((0.17, 0.04, 0.02), (0.0, 0.0, 0.0), bevel=0.005, bevel_segs=1),
              matrix=at_angle(a, 0.135, 0.175))
    objs.append(osp.make_object("Arms", g, [mats["dark"]], coll))
    bearing = [(0.05, 0.145), (0.05, 0.19), (0.04, 0.2), (0.0, 0.2)]
    objs.append(osp.make_object("Bearing", osp.Geo().add(osp.lathe(bearing, 24)), [mats["steel"]], coll))
    band = [(0.25, 0.09), (0.254, 0.094), (0.254, 0.116), (0.25, 0.12)]
    objs.append(osp.make_object("Band", osp.Geo().add(osp.lathe(band, SEGS)), [mats["yellow"]], coll))
    path = osp.export_glb(sc, objs, out)
    osp.report(sc, objs, path, [])


def fuel_line_fitting():
    name, out = "fuel_line", "models/control/fuel_line.glb"
    sc, coll = osp.new_scene("osp_" + name)
    mats = palette("FuelLine")
    objs = []
    # A six-sided flange on the hull, a valve body, a yellow collar and a pipe stub.
    flange = [(0.0, 0.0), (0.09, 0.0), (0.09, 0.02), (0.075, 0.03), (0.0, 0.03)]
    objs.append(osp.make_object("Flange", osp.Geo().add(osp.lathe(flange, 6)), [mats["dark"]], coll, 20))
    body = [(0.05, 0.03), (0.06, 0.04), (0.06, 0.11), (0.05, 0.12), (0.0, 0.12)]
    objs.append(osp.make_object("Body", osp.Geo().add(osp.lathe(body, 20)), [mats["white"]], coll))
    collar = [(0.064, 0.07), (0.064, 0.09), (0.058, 0.09), (0.058, 0.07)]
    objs.append(osp.make_object("Collar", osp.Geo().add(osp.lathe(collar, 20)), [mats["yellow"]], coll))
    stub = [(0.05, 0.12), (0.05, 0.135), (0.058, 0.14), (0.058, 0.165), (0.05, 0.17), (0.0, 0.17)]
    objs.append(osp.make_object("Stub", osp.Geo().add(osp.lathe(stub, 20)), [mats["steel"]], coll))
    g = osp.Geo()
    bolts(g, 0, 6, 0.072, 0.03, r=0.007, h=0.005, phase=math.pi / 6)
    objs.append(osp.make_object("Bolts", g, [mats["steel"]], coll))
    path = osp.export_glb(sc, objs, out)
    osp.report(sc, objs, path, [])


def wheel():
    name, out = "wheel", "models/gear/wheel.glb"
    sc, coll = osp.new_scene("osp_" + name)
    mats = palette("Wheel")
    objs = []
    # A tyre with grooves round it, about the axle (Z), radius 1, 0.7 wide.
    tyre = [(0.60, -0.35), (0.84, -0.35), (0.95, -0.30), (0.985, -0.22), (0.96, -0.20), (0.96, -0.15),
            (1.0, -0.13), (1.0, -0.05), (0.96, -0.03), (0.96, 0.03), (1.0, 0.05), (1.0, 0.13),
            (0.96, 0.15), (0.96, 0.20), (0.985, 0.22), (0.95, 0.30), (0.84, 0.35), (0.60, 0.35)]
    objs.append(osp.make_object("Tyre", osp.Geo().add(osp.lathe(tyre, 40)), [mats["rubber"]], coll, 50))
    # The hub, set in, with a yellow centre and a ring of bolts each side.
    hub = [(0.0, -0.30), (0.20, -0.30), (0.26, -0.27), (0.58, -0.27), (0.62, -0.33), (0.62, 0.33),
           (0.58, 0.27), (0.26, 0.27), (0.20, 0.30), (0.0, 0.30)]
    objs.append(osp.make_object("Hub", osp.Geo().add(osp.lathe(hub, 32)), [mats["white"]], coll))
    g = osp.Geo()
    for side in (-1, 1):
        cap = [(0.0, side * 0.30), (0.17, side * 0.30), (0.15, side * 0.345), (0.0, side * 0.345)]
        g.add(osp.lathe(cap, 24))
    objs.append(osp.make_object("Caps", g, [mats["yellow"]], coll))
    g = osp.Geo()
    for side in (-0.285, 0.265):
        bolts(g, 0, 8, 0.42, side, r=0.05, h=0.02)
    objs.append(osp.make_object("Bolts", g, [mats["steel"]], coll))
    path = osp.export_glb(sc, objs, out)
    osp.report(sc, objs, path, [])


attitude_unit()
small_wheel()
fuel_line_fitting()
wheel()
