"""Small helpers shared by the K-5 and K-19 generators (run inside Blender 5.2).

Open hoop bands, pipe collars and bolt circles, bellows, gimbal actuators, and a debug-view
renderer. Everything returns (verts, faces) parts in world space, like osp_common.
"""
import math
import os

from mathutils import Vector

import osp_common as osp


def place(part, matrix):
    verts, faces = part
    return [tuple(matrix @ Vector(v)) for v in verts], faces


def open_band(profile, z_top, height, proud, inset, segs):
    """Hoop band hugging a contour without its hidden inner face (three quad rows)."""
    zb = z_top - height
    rt, rb = osp.interp_r(profile, z_top), osp.interp_r(profile, zb)
    return osp.lathe([(rb - inset, zb), (rb + proud, zb), (rt + proud, z_top), (rt - inset, z_top)], segs)


def collar(center, direction, r_in, r_out, thick, segs):
    """Flange ring around a pipe (no inner face), centred on `center`, axis along `direction`."""
    h = 0.5 * thick
    return place(osp.lathe([(r_in, -h), (r_out, -h), (r_out, h), (r_in, h)], segs),
                 osp.align_z(center, direction))


def bolt_circle(geo, mat, center, direction, radius, count, bolt_r, z0, z1, segs=6, phase=0.0):
    """`count` studs on a circle about an axis; z0..z1 measured along the axis from `center`."""
    m = osp.align_z(center, direction)
    for k in range(count):
        a = phase + math.tau * k / count
        p = (radius * math.cos(a), radius * math.sin(a), 0.0)
        v, f = osp.cylinder(bolt_r, z0, z1, segs)
        geo.add(([tuple(m @ (Vector(q) + Vector(p))) for q in v], f), mat)


def bellows(center, direction, r, length, convolutions, depth, segs):
    """Convoluted bellows section (open ends), axis along `direction`, starting at `center`."""
    prof = []
    n = 2 * convolutions
    for k in range(n + 1):
        prof.append((r + (depth if k % 2 else 0.0), length * k / n))
    return place(osp.lathe(prof, segs), osp.align_z(center, direction))


def actuator(geo, top, bot, body_r, rod_r, body_mat, rod_mat, segs=16):
    """Hydraulic gimbal actuator between two clevis points: body near the top, rod below,
    a servo-valve block on the body and clevis pins at both ends."""
    top, bot = Vector(top), Vector(bot)
    d = bot - top
    length, u = d.length, d.normalized()
    side = u.cross(Vector((0, 0, 1)))
    side = side.normalized() if side.length > 1e-6 else Vector((1, 0, 0))
    out = side.cross(u)
    geo.add(osp.cyl_between(top + u * 0.8 * body_r, top + u * 0.58 * length, body_r, segs), body_mat)
    geo.add(osp.cyl_between(top + u * 0.58 * length, top + u * 0.62 * length, 0.8 * body_r, segs), body_mat)
    geo.add(osp.cyl_between(top + u * 0.6 * length, bot - u * 1.4 * rod_r, rod_r, max(8, segs - 4)), rod_mat)
    geo.add(osp.cyl_between(bot - u * 2.4 * rod_r, bot, 1.35 * rod_r, max(8, segs - 4)), body_mat)
    blk = 1.4 * body_r
    geo.add(osp.box((blk, blk, 2.0 * blk), bevel=0.15 * blk, bevel_segs=1), rod_mat,
            matrix=osp.align_z(top + u * 0.3 * length - out * 1.2 * body_r, u))
    for p, r in ((top + u * 0.4 * body_r, 0.6 * body_r), (bot - u * 0.6 * rod_r, 1.2 * rod_r)):
        geo.add(osp.cyl_between(p - side * 1.4 * r, p + side * 1.4 * r, r, 10), body_mat)


def render_views(sc, objects, out_dir, name, views, res=700):
    """views: (tag, elevation, azimuth[, (lo, hi) frame]) -> <out_dir>/<name>_<tag>.png"""
    paths = []
    for tag, el, az, *fr in views:
        p = os.path.join(out_dir, f"{name}_{tag}.png")
        osp.render_preview(sc, objects, p, elevation=el, azimuth=az, res=res, frame=fr[0] if fr else None)
        paths.append(p)
    return paths
