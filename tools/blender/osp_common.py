"""Shared helpers for the Open Space Program part generators (run inside Blender 5.2).

Geometry is generated in world space and every object keeps an identity transform, so a
model's origin is simply the Blender world origin. Units are metres, Blender is Z-up.
"""
import contextlib
import io
import json
import math
import os
import struct

import bmesh
import bpy
from mathutils import Matrix, Vector

TAU = 2.0 * math.pi
TOOLS_DIR = os.path.dirname(os.path.abspath(__file__))
REPO_DIR = os.path.dirname(os.path.dirname(TOOLS_DIR))
ASSET_DIR = os.path.join(REPO_DIR, "assets", "core")
SMOOTH_GROUP = "OSP Smooth by Angle"


# --------------------------------------------------------------------------- scene

def _window():
    wm = bpy.context.window_manager
    return wm.windows[0] if wm is not None and len(wm.windows) else None


def context(sc):
    """Context override making `sc` the active scene. Only the window and scene are overridden:
    overriding view_layer as well crashes Blender 5.2 inside the glTF exporter."""
    win = _window()
    if win is None:
        return bpy.context.temp_override(scene=sc)
    if win.scene != sc:
        win.scene = sc
    return bpy.context.temp_override(window=win, scene=sc)


def depsgraph(sc):
    with context(sc):
        return bpy.context.evaluated_depsgraph_get()


def new_scene(name):
    """(Re)create scene `name` (must start with osp_), shown in the window if there is one.
    Returns (scene, collection that holds the exported model)."""
    assert name.startswith("osp_"), name
    win = _window()
    old = bpy.data.scenes.get(name)
    if old is not None:
        if win is not None and win.scene == old:
            win.scene = next(s for s in bpy.data.scenes if s != old)
        obs = [o for o in old.collection.all_objects if all(s == old for s in o.users_scene)]
        datas = [o.data for o in obs if o.data is not None]
        colls = [c for c in old.collection.children_recursive if c.users <= 1]
        bpy.data.batch_remove(obs + colls + [old])
        bpy.data.batch_remove([d for d in datas if d.users == 0])
    sc = bpy.data.scenes.new(name)
    sc.unit_settings.system = "METRIC"
    sc.unit_settings.scale_length = 1.0
    if win is not None:
        win.scene = sc
    model = bpy.data.collections.new(name[4:] + "_model")
    sc.collection.children.link(model)
    return sc, model


# ----------------------------------------------------------------------- materials

def _lin(c):
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def material(name, hex_color, metallic=0.0, roughness=0.5):
    """Principled-only material from an sRGB hex colour (e.g. "#3a2c22")."""
    h = hex_color.lstrip("#")
    rgb = [_lin(int(h[i:i + 2], 16) / 255.0) for i in (0, 2, 4)]
    m = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    m.use_nodes = True
    nt = m.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    bsdf = nt.nodes.new("ShaderNodeBsdfPrincipled")
    bsdf.location = (-300, 0)
    nt.links.new(bsdf.outputs["BSDF"], out.inputs["Surface"])
    bsdf.inputs["Base Color"].default_value = (*rgb, 1.0)
    bsdf.inputs["Metallic"].default_value = metallic
    bsdf.inputs["Roughness"].default_value = roughness
    m.diffuse_color = (*rgb, 1.0)
    m.metallic = metallic
    m.roughness = roughness
    return m


# ------------------------------------------------------------------ mesh building

class Geo:
    """Accumulates (verts, faces) parts with a per-face material slot index."""

    def __init__(self):
        self.verts, self.faces, self.mats = [], [], []

    def add(self, part, mat=0, matrix=None):
        """mat: material slot for every face, or a list with one slot per face."""
        verts, faces = part
        off = len(self.verts)
        if matrix is not None:
            verts = [matrix @ Vector(v) for v in verts]
        self.verts.extend(tuple(v) for v in verts)
        self.faces.extend(tuple(i + off for i in f) for f in faces)
        self.mats.extend(mat if isinstance(mat, (list, tuple)) else [mat] * len(faces))
        return self


def lathe(profile, segs, closed=False, phase=0.0, ripple=None):
    """Revolve (r, z) points about Z.

    Normals point outward when the profile runs counter-clockwise in the (r, z) half-plane
    (upward along an outer wall, inward along a bottom face). Points with r == 0 become poles.
    ripple(r, z, theta) returns an extra radial offset (used for tube-wall nozzles).
    """
    verts, faces, rings = [], [], []
    for r, z in profile:
        if r < 1e-7:
            rings.append([len(verts)] * segs)
            verts.append((0.0, 0.0, z))
            continue
        ring = []
        for j in range(segs):
            t = phase + TAU * j / segs
            rr = r + (ripple(r, z, t) if ripple else 0.0)
            ring.append(len(verts))
            verts.append((rr * math.cos(t), rr * math.sin(t), z))
        rings.append(ring)
    n = len(rings)
    for i in range(n if closed else n - 1):
        a, b = rings[i], rings[(i + 1) % n]
        for j in range(segs):
            k = (j + 1) % segs
            f = []
            for v in (a[j], a[k], b[k], b[j]):
                if v not in f:
                    f.append(v)
            if len(f) >= 3:
                faces.append(tuple(f))
    return verts, faces


def cylinder(r, z0, z1, segs, cap0=True, cap1=True):
    prof = ([(0.0, z0)] if cap0 else []) + [(r, z0), (r, z1)] + ([(0.0, z1)] if cap1 else [])
    return lathe(prof, segs)


def torus(R, r, segs, minor_segs, z=0.0):
    prof = [(R + r * math.cos(TAU * i / minor_segs), z + r * math.sin(TAU * i / minor_segs))
            for i in range(minor_segs)]
    return lathe(prof, segs, closed=True)


def align_z(p0, direction, roll=0.0):
    """Matrix taking local +Z to `direction`, local origin to p0."""
    d = Vector(direction).normalized()
    rot = Vector((0, 0, 1)).rotation_difference(d).to_matrix().to_4x4()
    return Matrix.Translation(Vector(p0)) @ rot @ Matrix.Rotation(roll, 4, "Z")


def cyl_between(p0, p1, r, segs, caps=True):
    p0, p1 = Vector(p0), Vector(p1)
    verts, faces = cylinder(r, 0.0, (p1 - p0).length, segs, caps, caps)
    m = align_z(p0, p1 - p0)
    return [tuple(m @ Vector(v)) for v in verts], faces


def box(size, center=(0, 0, 0), bevel=0.0, bevel_segs=2):
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=Vector(size), verts=bm.verts)
    if bevel > 0:
        bmesh.ops.bevel(bm, geom=bm.verts[:] + bm.edges[:], offset=bevel, segments=bevel_segs,
                        profile=0.5, affect="EDGES", clamp_overlap=True)
    bmesh.ops.translate(bm, vec=Vector(center), verts=bm.verts)
    idx = {v: i for i, v in enumerate(bm.verts)}
    verts = [tuple(v.co) for v in bm.verts]
    faces = [tuple(idx[v] for v in f.verts) for f in bm.faces]
    bm.free()
    return verts, faces


def prism(outline, w0, w1):
    """Closed prism: counter-clockwise 2D outline (x, y) extruded along local Z from w0 to w1."""
    n = len(outline)
    verts = [(x, y, w0) for x, y in outline] + [(x, y, w1) for x, y in outline]
    faces = [tuple(reversed(range(n))), tuple(range(n, 2 * n))]
    faces += [(i, (i + 1) % n, n + (i + 1) % n, n + i) for i in range(n)]
    return verts, faces


def frame(outer, inner, w0, w1):
    """Picture frame between two counter-clockwise outlines with equal point counts."""
    n = len(outer)
    verts = [(x, y, w) for w in (w0, w1) for x, y in outer] + \
            [(x, y, w) for w in (w0, w1) for x, y in inner]
    ob, ot, ib, it = 0, n, 2 * n, 3 * n
    faces = []
    for i in range(n):
        j = (i + 1) % n
        faces += [(ob + i, ob + j, ot + j, ot + i),      # outer wall
                  (ib + j, ib + i, it + i, it + j),      # inner wall
                  (ot + i, ot + j, it + j, it + i),      # front
                  (ob + j, ob + i, ib + i, ib + j)]      # back
    return verts, faces


def surface_frame(origin, normal, up):
    """Matrix with local Z = normal, local Y = `up` projected onto the tangent plane."""
    n = Vector(normal).normalized()
    y = (Vector(up) - n * Vector(up).dot(n)).normalized()
    x = y.cross(n)
    m = Matrix((x, y, n)).transposed().to_4x4()
    m.translation = Vector(origin)
    return m


def recess(part, mats, pockets):
    """Push face regions of a surface inward to form pockets (windows, thruster pods).

    part: (verts, faces) with outward normals; mats: material slot per face;
    pockets: list of (face indices, depth, floor slot, wall slot[, (frame width, frame slot)]);
    the optional frame is an inset border left at the surface. Returns (verts, faces, mats).
    """
    verts, faces = part
    bm = bmesh.new()
    bv = [bm.verts.new(v) for v in verts]
    bf = [bm.faces.new([bv[i] for i in f]) for f in faces]
    for f, mi in zip(bf, mats):
        f.material_index = mi
    bm.normal_update()
    for ids, depth, floor_mat, wall_mat, *frame_opt in pockets:
        region = [bf[i] for i in ids]
        n = sum((f.normal * f.calc_area() for f in region), Vector()).normalized()
        if frame_opt:
            width, frame_mat = frame_opt[0]
            border = bmesh.ops.inset_region(bm, faces=region, thickness=width, depth=0.0,
                                            use_even_offset=True)["faces"]
            for f in border:
                f.material_index = frame_mat
        ret = bmesh.ops.extrude_face_region(bm, geom=region)
        new_verts = {e for e in ret["geom"] if isinstance(e, bmesh.types.BMVert)}
        new_faces = [e for e in ret["geom"] if isinstance(e, bmesh.types.BMFace)]
        bmesh.ops.translate(bm, vec=-n * depth, verts=list(new_verts))
        bmesh.ops.delete(bm, geom=[f for f in region if f.is_valid], context="FACES")
        center = sum((v.co for v in new_verts), Vector()) / len(new_verts)
        floor = set()
        for f in new_faces:
            if all(v in new_verts for v in f.verts):
                floor.add(f)
                f.material_index = floor_mat
                if f.normal.dot(n) < 0:
                    f.normal_flip()
        for f in bm.faces:
            if f in floor or not any(v in new_verts for v in f.verts) or \
                    not any(v not in new_verts for v in f.verts):
                continue
            f.material_index = wall_mat
            out = f.calc_center_median() - center
            out -= n * out.dot(n)
            if f.normal.dot(out) > 0:  # walls face into the pocket
                f.normal_flip()
        bm.normal_update()
    idx = {v: i for i, v in enumerate(bm.verts)}
    out_v = [tuple(v.co) for v in bm.verts]
    out_f = [tuple(idx[v] for v in f.verts) for f in bm.faces]
    out_m = [f.material_index for f in bm.faces]
    bm.free()
    return out_v, out_f, out_m


def fillet(points, radius, n=6):
    """Round the interior corners of a polyline with quadratic-Bezier fillets."""
    pts = [Vector(p) for p in points]
    out = [pts[0]]
    for i in range(1, len(pts) - 1):
        p = pts[i]
        a, c = pts[i - 1] - p, pts[i + 1] - p
        la, lc = a.length, c.length
        ang = a.angle(c)
        if ang > math.pi - 1e-3:
            out.append(p)
            continue
        t = min(radius / math.tan(ang / 2), 0.5 * la, 0.5 * lc)
        s, e = p + a / la * t, p + c / lc * t
        for k in range(n + 1):
            u = k / n
            out.append((1 - u) ** 2 * s + 2 * (1 - u) * u * p + u * u * e)
    out.append(pts[-1])
    return out


def sweep(path, radius, segs=16, cap0=True, cap1=True):
    """Tube of circular cross-section along a polyline (parallel-transport frames).
    `radius` may be a number or a list with one radius per path point."""
    pts = [Vector(p) for p in path]
    radii = radius if isinstance(radius, (list, tuple)) else [radius] * len(pts)
    tans = []
    for i in range(len(pts)):
        if i == 0:
            t = pts[1] - pts[0]
        elif i == len(pts) - 1:
            t = pts[-1] - pts[-2]
        else:
            t = (pts[i + 1] - pts[i]).normalized() + (pts[i] - pts[i - 1]).normalized()
        tans.append(t.normalized())
    ref = Vector((0, 0, 1)) if abs(tans[0].z) < 0.9 else Vector((1, 0, 0))
    nrm = (ref - tans[0] * ref.dot(tans[0])).normalized()
    verts, faces, rings = [], [], []
    for i, (p, t) in enumerate(zip(pts, tans)):
        if i > 0:
            nrm = tans[i - 1].rotation_difference(t) @ nrm
            nrm = (nrm - t * nrm.dot(t)).normalized()
        bin_ = t.cross(nrm)
        ring = []
        for j in range(segs):
            a = TAU * j / segs
            ring.append(len(verts))
            verts.append(tuple(p + radii[i] * (math.cos(a) * nrm + math.sin(a) * bin_)))
        rings.append(ring)
    for i in range(len(rings) - 1):
        a, b = rings[i], rings[i + 1]
        for j in range(segs):
            k = (j + 1) % segs
            faces.append((a[j], a[k], b[k], b[j]))
    if cap0:
        faces.append(tuple(reversed(rings[0])))
    if cap1:
        faces.append(tuple(rings[-1]))
    return verts, faces


def bezier(p0, p1, p2, n=8, bias=1.0):
    """Points of a quadratic Bezier, endpoints included. bias > 1 packs samples towards p0."""
    p0, p1, p2 = Vector(p0), Vector(p1), Vector(p2)
    return [(1 - u) ** 2 * p0 + 2 * (1 - u) * u * p1 + u * u * p2
            for u in ((k / n) ** bias for k in range(n + 1))]


# ------------------------------------------------------------------ nozzle contours

def nozzle_contour(rc, z_inj, rt, z_t, re, z_e, conv_deg=30.0, theta_n=30.0, theta_e=8.0,
                   n_bell=16, n_throat=3, r_up=1.5, r_down=0.382, r_fillet=None, lip=0.006,
                   bell_bias=1.0):
    """Chamber + converging section + Rao (thrust-optimised parabolic) bell, as (r, z) pairs
    from the exit lip (z_e) up to the injector face (z_inj). The nozzle axis runs down -Z.
    A short vertical lip at the exit keeps a Solidify'd exit rim exactly on the z_e plane."""
    th_n, th_e, th_c = map(math.radians, (theta_n, theta_e, conv_deg))
    # Divergent: circular arc (r_down * rt) then a quadratic Bezier N -> Q -> E.
    ra = r_down * rt
    arc = [(rt + ra * (1 - math.cos(a)), ra * math.sin(a))
           for a in (th_n * k / n_throat for k in range(n_throat + 1))]
    nx, nr = arc[-1][1], arc[-1][0]
    ex, er = z_t - z_e - lip, re
    s = (er - nr - (ex - nx) * math.tan(th_e)) / (math.tan(th_n) - math.tan(th_e))
    qx, qr = nx + s, nr + s * math.tan(th_n)
    assert nx < qx < ex, "bell angles incompatible with length"
    bell = [(p.y, p.x) for p in bezier((nx, nr), (qx, qr), (ex, er), n=n_bell, bias=bell_bias)]
    # Convergent: arc (r_up * rt) up to conv angle, straight cone, fillet into chamber wall.
    ru = r_up * rt
    rf = r_fillet if r_fillet is not None else 0.6 * rc
    up = [(rt + ru * (1 - math.cos(a)), -ru * math.sin(a))
          for a in (th_c * k / n_throat for k in range(1, n_throat + 1))]
    r1, x1 = up[-1]
    r2 = rc - rf * (1 - math.cos(th_c))
    x2 = x1 - (r2 - r1) / math.tan(th_c)
    fil = [(rc - rf * (1 - math.cos(a)), x2 - rf * (math.sin(th_c) - math.sin(a)))
           for a in (th_c * (1 - k / n_throat) for k in range(n_throat + 1))]
    # (r, x) pairs, x = distance downstream of the throat: exit -> throat -> chamber wall
    contour = list(reversed(bell)) + arc[:-1][::-1] + up + fil
    prof = [(re, z_e)] + [(r, z_t - x) for r, x in contour]
    prof.append((rc, z_inj))
    return prof


def interp_r(profile, z):
    """Radius of a (r, z) profile at height z (first matching monotonic segment)."""
    for (r0, z0), (r1, z1) in zip(profile, profile[1:]):
        if min(z0, z1) <= z <= max(z0, z1) and z0 != z1:
            return r0 + (r1 - r0) * (z - z0) / (z1 - z0)
    raise ValueError(f"z={z} outside profile")


def split_profile(profile, z):
    """Split a bottom-to-top profile at height z -> (below, above), sharing the cut point."""
    for i, ((r0, z0), (r1, z1)) in enumerate(zip(profile, profile[1:])):
        if z0 <= z <= z1:
            p = (r0 + (r1 - r0) * (z - z0) / (z1 - z0), z)
            below = profile[:i + 1] + ([p] if p != profile[i] else [])
            above = ([p] if p != profile[i + 1] else []) + profile[i + 1:]
            return below, above
    raise ValueError(f"z={z} outside profile")


def band(profile, z_top, height, proud, inset=0.004):
    """Lathe profile of a hoop band hugging a bell contour (closed, counter-clockwise)."""
    zb = z_top - height
    rt, rb = interp_r(profile, z_top), interp_r(profile, zb)
    return [(rb - inset, zb), (rb + proud, zb), (rt + proud, z_top), (rt - inset, z_top)]


# ------------------------------------------------------------------------ objects

def _smooth_group(angle_deg, concave_deg=None):
    """Geometry-nodes group: all faces smooth; edges sharper than angle_deg (or, if given,
    concave edges sharper than concave_deg, e.g. the grooves between nozzle tubes) are sharp."""
    name = f"{SMOOTH_GROUP} {angle_deg:g}" + (f" concave {concave_deg:g}" if concave_deg else "")
    ng = bpy.data.node_groups.get(name)
    if ng is not None:
        return ng
    ng = bpy.data.node_groups.new(name, "GeometryNodeTree")
    ng.is_modifier = True
    ng.interface.new_socket("Geometry", in_out="INPUT", socket_type="NodeSocketGeometry")
    ng.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
    n, l = ng.nodes, ng.links
    gin, gout = n.new("NodeGroupInput"), n.new("NodeGroupOutput")
    edge_angle = n.new("GeometryNodeInputMeshEdgeAngle")

    def compare(socket, op, value):
        c = n.new("FunctionNodeCompare")
        c.data_type, c.operation = "FLOAT", op
        c.inputs[1].default_value = value
        l.new(socket, c.inputs[0])
        return c.outputs[0]

    smooth = compare(edge_angle.outputs["Unsigned Angle"], "LESS_EQUAL", math.radians(angle_deg))
    if concave_deg:
        both = n.new("FunctionNodeBooleanMath")
        both.operation = "AND"
        l.new(smooth, both.inputs[0])
        l.new(compare(edge_angle.outputs["Signed Angle"], "GREATER_EQUAL", -math.radians(concave_deg)),
              both.inputs[1])
        smooth = both.outputs[0]
    faces = n.new("GeometryNodeSetShadeSmooth")
    faces.domain = "FACE"
    faces.inputs["Shade Smooth"].default_value = True
    edges = n.new("GeometryNodeSetShadeSmooth")
    edges.domain = "EDGE"
    l.new(smooth, edges.inputs["Shade Smooth"])
    l.new(gin.outputs["Geometry"], faces.inputs["Geometry"])
    l.new(faces.outputs["Geometry"], edges.inputs["Geometry"])
    l.new(edges.outputs["Geometry"], gout.inputs["Geometry"])
    return ng


def make_object(name, geo, mats, coll, smooth_angle=35.0, solidify=None, concave_angle=None):
    """Mesh object from a Geo. solidify = dict(thickness, offset=-1, material_offset=0,
    material_offset_rim=0) adds a live Solidify modifier. A geometry-nodes "smooth by angle"
    modifier comes last so that solidify rims get sharp edges too."""
    me = bpy.data.meshes.new(name)
    me.from_pydata(geo.verts, [], geo.faces)
    me.validate()
    if len(me.polygons) != len(geo.faces):
        raise RuntimeError(f"{name}: {len(geo.faces) - len(me.polygons)} degenerate faces")
    for m in mats:
        me.materials.append(m)
    me.polygons.foreach_set("material_index", geo.mats)
    me.shade_smooth()
    me.update()
    ob = bpy.data.objects.new(name, me)
    coll.objects.link(ob)
    if solidify:
        mod = ob.modifiers.new("Solidify", "SOLIDIFY")
        mod.thickness = solidify["thickness"]
        mod.offset = solidify.get("offset", -1.0)
        mod.use_even_offset = True
        mod.use_quality_normals = True
        mod.use_rim = True
        mod.material_offset = solidify.get("material_offset", 0)
        mod.material_offset_rim = solidify.get("material_offset_rim", 0)
    mod = ob.modifiers.new("Smooth by Angle", "NODES")
    mod.node_group = _smooth_group(smooth_angle, concave_angle)
    return ob


# ------------------------------------------------------------- export and checks

def evaluated_stats(sc, objects, per_object=None):
    """World-space bbox (min, max) and triangle count of the evaluated objects."""
    dg = depsgraph(sc)
    lo, hi, tris = [1e9] * 3, [-1e9] * 3, 0
    for ob in objects:
        ev = ob.evaluated_get(dg)
        me = ev.to_mesh()
        me.calc_loop_triangles()
        tris += len(me.loop_triangles)
        if per_object is not None:
            per_object[ob.name] = len(me.loop_triangles)
        for v in me.vertices:
            w = ev.matrix_world @ v.co
            for i in range(3):
                lo[i], hi[i] = min(lo[i], w[i]), max(hi[i], w[i])
        ev.to_mesh_clear()
    return lo, hi, tris


def max_radius_near(sc, ob, z, tol=0.005):
    """Largest distance from the Z axis among evaluated vertices within tol of height z."""
    dg = depsgraph(sc)
    ev = ob.evaluated_get(dg)
    me = ev.to_mesh()
    r = max((math.hypot(v.co.x, v.co.y) for v in me.vertices if abs(v.co.z - z) <= tol), default=0.0)
    ev.to_mesh_clear()
    return r


def max_radius(sc, objects):
    """Largest distance from the Z axis over all evaluated vertices (envelope check)."""
    dg = depsgraph(sc)
    r = 0.0
    for ob in objects:
        ev = ob.evaluated_get(dg)
        me = ev.to_mesh()
        mw = ev.matrix_world
        r = max([r] + [(mw @ v.co).xy.length for v in me.vertices])
        ev.to_mesh_clear()
    return r


def glb_stats(path):
    """Bbox (in Blender axes) and triangle count read back from the exported .glb."""
    with open(path, "rb") as f:
        data = f.read()
    clen = struct.unpack_from("<I", data, 12)[0]
    gl = json.loads(data[20:20 + clen])
    lo, hi, tris = [1e9] * 3, [-1e9] * 3, 0
    for node in gl.get("nodes", []):
        assert not any(k in node for k in ("translation", "rotation", "scale", "matrix")), node
        if "mesh" not in node:
            continue
        for prim in gl["meshes"][node["mesh"]]["primitives"]:
            acc = gl["accessors"][prim["attributes"]["POSITION"]]
            (x0, y0, z0), (x1, y1, z1) = acc["min"], acc["max"]
            for i, (a, b) in enumerate(((x0, x1), (-z1, -z0), (y0, y1))):  # glTF Y-up -> Blender Z-up
                lo[i], hi[i] = min(lo[i], a), max(hi[i], b)
            tris += gl["accessors"][prim["indices"]]["count"] // 3
    mats = sorted(m["name"] for m in gl.get("materials", []))
    return lo, hi, tris, mats, len(gl.get("nodes", []))


def export_glb(sc, objects, relpath):
    path = os.path.join(ASSET_DIR, relpath)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    vl = sc.view_layers[0]
    for ob in sc.objects:
        ob.select_set(False, view_layer=vl)
    for ob in objects:
        ob.select_set(True, view_layer=vl)
    vl.objects.active = objects[0]
    with context(sc), contextlib.redirect_stdout(io.StringIO()):  # exporter logs to stdout
        bpy.ops.export_scene.gltf(
            filepath=path, export_format="GLB", use_selection=True, export_apply=True,
            export_yup=True, export_cameras=False, export_lights=False, export_extras=False,
            export_animations=False, export_texcoords=False, export_normals=True,
            export_tangents=False, export_materials="EXPORT", use_active_scene=True)
    return path


def report(sc, objects, path, checks=()):
    """Print evaluated and read-back stats. checks: (label, measured, expected) tuples."""
    per = {}
    lo, hi, tris = evaluated_stats(sc, objects, per)
    glo, ghi, gtris, mats, nodes = glb_stats(path)
    fmt = lambda v: "(" + ", ".join(f"{x:+.4f}" for x in v) + ")"
    print(f"{os.path.relpath(path, REPO_DIR)}")
    print(f"  blender bbox min {fmt(lo)} max {fmt(hi)}  tris {tris}")
    print(f"  glb     bbox min {fmt(glo)} max {fmt(ghi)}  tris {gtris}  nodes {nodes}")
    print(f"  materials: {', '.join(mats)}")
    print("  tris per object: " + ", ".join(f"{k} {v}" for k, v in per.items()))
    ok = True
    for label, got, want in checks:
        err = abs(got - want) / max(abs(want), 1e-9) if want else abs(got)
        flag = "ok" if err <= 0.02 else "OFF"
        ok &= flag == "ok"
        print(f"  {label:<28} {got:8.4f}  (spec {want:.4f})  {flag}")
    return ok


# ------------------------------------------------------------------------ preview

def _preview_world():
    """Neutral grey world, slightly lighter overhead than below so metals show some form."""
    world = bpy.data.worlds.get("OSP Preview World")
    if world is not None:
        return world
    world = bpy.data.worlds.new("OSP Preview World")
    world.use_nodes = True
    nt = world.node_tree
    bg = nt.nodes.get("Background")
    coord = nt.nodes.new("ShaderNodeTexCoord")
    xyz = nt.nodes.new("ShaderNodeSeparateXYZ")
    ramp = nt.nodes.new("ShaderNodeMapRange")
    ramp.inputs["From Min"].default_value, ramp.inputs["From Max"].default_value = -1.0, 1.0
    ramp.inputs["To Min"].default_value, ramp.inputs["To Max"].default_value = 0.1, 0.42
    nt.links.new(coord.outputs["Generated"], xyz.inputs[0])
    nt.links.new(xyz.outputs["Z"], ramp.inputs["Value"])
    nt.links.new(ramp.outputs["Result"], bg.inputs["Color"])
    bg.inputs["Strength"].default_value = 1.0
    return world


def render_preview(sc, objects, path, elevation=-15.0, azimuth=-40.0, res=700, lens=55.0,
                   samples=48, frame=None):
    """3/4 view render with a neutral grey world and three sun lights.
    frame=(lo, hi) overrides the framed bounding box (for close-ups)."""
    lo, hi = frame if frame else evaluated_stats(sc, objects)[:2]
    center = Vector([(a + b) / 2 for a, b in zip(lo, hi)])
    corners = [Vector((x, y, z)) for x in (lo[0], hi[0]) for y in (lo[1], hi[1]) for z in (lo[2], hi[2])]
    el, az = math.radians(elevation), math.radians(azimuth)
    view_dir = Vector((math.cos(el) * math.cos(az), math.cos(el) * math.sin(az), math.sin(el)))
    rot = (-view_dir).to_track_quat("-Z", "Y")
    right, up, fwd = rot @ Vector((1, 0, 0)), rot @ Vector((0, 1, 0)), rot @ Vector((0, 0, -1))
    tan_half = 18.0 / lens
    dist = max(max(-(c - center).dot(fwd) + abs((c - center).dot(right)) / tan_half,
                   -(c - center).dot(fwd) + abs((c - center).dot(up)) / tan_half) for c in corners)
    dist *= 1.08

    rig = bpy.data.collections.get(sc.name[4:] + "_preview_rig")
    if rig is None:
        rig = bpy.data.collections.new(sc.name[4:] + "_preview_rig")
        sc.collection.children.link(rig)
    old = list(rig.objects)
    datas = [ob.data for ob in old]
    bpy.data.batch_remove(old)
    bpy.data.batch_remove([d for d in datas if d.users == 0])

    cam_data = bpy.data.cameras.new(sc.name + "_preview_cam")
    cam_data.lens = lens
    cam_data.clip_start, cam_data.clip_end = 0.01, 1000.0
    cam = bpy.data.objects.new(sc.name + "_preview_cam", cam_data)
    cam.location = center + view_dir * dist
    cam.rotation_mode = "QUATERNION"
    cam.rotation_quaternion = rot
    rig.objects.link(cam)
    sc.camera = cam

    def sun(name, energy, az_deg, el_deg, angle=8.0):
        d = bpy.data.lights.new(sc.name + "_" + name, "SUN")
        d.energy = energy
        d.angle = math.radians(angle)
        ob = bpy.data.objects.new(sc.name + "_" + name, d)
        a, e = math.radians(az_deg), math.radians(el_deg)
        to_light = Vector((math.cos(e) * math.cos(a), math.cos(e) * math.sin(a), math.sin(e)))
        ob.rotation_mode = "QUATERNION"
        ob.rotation_quaternion = (-to_light).to_track_quat("-Z", "Y")
        rig.objects.link(ob)

    sun("key", 3.2, azimuth + 45, 40)
    sun("fill", 0.9, azimuth - 80, 5)
    sun("rim", 2.0, azimuth + 160, 25)

    sc.world = _preview_world()

    r = sc.render
    r.engine = "BLENDER_EEVEE"
    r.resolution_x = r.resolution_y = res
    r.resolution_percentage = 100
    r.film_transparent = False
    r.image_settings.file_format = "PNG"
    r.filepath = path
    sc.eevee.taa_render_samples = samples
    try:
        sc.view_settings.view_transform = "AgX"
    except TypeError:
        pass
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with context(sc):
        bpy.ops.render.render(write_still=True, scene=sc.name)
    return path
