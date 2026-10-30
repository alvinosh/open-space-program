"""Convert KSP .mu part models to .glb files for the game.

Runs inside Blender 5.2 (headless):

    Blender -b --factory-startup --python tools/blender/convert_mu.py -- <jobs.json> <results.json>

Every job assembles one KSP part from its MODEL nodes, applies rescaleFactor and the origin, drops
colliders and effect meshes, rebuilds the materials as Principled BSDF with PNG textures and writes
one baked, single-object GLB (glTF Y up, metres). The results file is rewritten after every job,
so a crash keeps the results of the jobs before it. The job and result formats are in README.md.

The vendored io_object_mu addon reads the .mu files. Its shader builder does not run on Blender 5.2,
so make_shader/create_textures/create_action are replaced with the functions below.
"""

from __future__ import annotations

import json
import math
import os
import re
import shutil
import struct
import sys
import tempfile
import time
import traceback
import zlib

import bpy
import numpy as np
from mathutils import Matrix, Quaternion, Vector

HERE = os.path.dirname(os.path.abspath(__file__))
VENDOR = os.path.join(HERE, "vendor")

TEXTURE_EXTS = (".dds", ".png", ".tga", ".mbm", ".jpg", ".jpeg")
PNG_LEVEL = 6
DEFAULT_MAX_TEXTURE = 2048

# Effect transforms by name: the last word (camelCase/underscore words, numbers ignored) is one of
# these, or the first word is "fx". innerGlow, exhaustFX and FX_plume match; PlumeDeflectors,
# VikingLower and NozzleFlared don't.
EFFECT_WORDS = {"fx", "glow", "glows", "flare", "flares", "plume", "plumes"}
# Transforms reported in the results besides the job's report_transforms.
TRANSFORM_NAME = re.compile(r"(thrusttransform|rcstransform|transform$|^node_)", re.IGNORECASE)

STATE = None  # JobState of the job being converted


class JobState:
    def __init__(self, job: dict, tmpdir: str):
        self.job = job
        self.tmpdir = tmpdir
        self.max_texture = int(job.get("max_texture") or DEFAULT_MAX_TEXTURE)
        self.warnings: list[str] = []
        self.image_cache: dict = {}
        self.textures: list[dict] = []
        self.sources: set[str] = set()
        self.replacements: dict[str, str] = {}
        self.dir_cache: dict[str, dict[str, str]] = {}
        self.pending: dict[int, tuple] = {}  # materials not built yet, by pointer

    def warn(self, msg: str) -> None:
        if msg not in self.warnings:
            self.warnings.append(msg)


# --------------------------------------------------------------------------------------------
# Importer hooks


def pose_clip(mu, path, clip):
    """Leave the transforms a clip moves where its last keys put them, for the clips the job
    names in `pose_animations` (a nose shroud closed for flight). The game plays no
    animations, so every other clip is dropped and the model keeps the pose it was saved in."""
    if clip.name not in (STATE.job.get("pose_animations") or []):
        return
    anim = sys.modules["io_object_mu.import_mu.animation"]
    for curve in clip.curves:
        if not curve.keys or curve.property not in anim.property_map:
            continue
        mu_path = "/".join([path, curve.path]) if curve.path else path
        muobj = mu.object_paths.get(mu_path)
        obj = getattr(muobj, "bobj", None)
        subpath, data_path, index, mult = anim.property_map[curve.property][:4]
        if obj is None or hasattr(muobj, "bone") or subpath != "obj":
            STATE.warnings.append(f"pose {clip.name}: cannot pose {curve.property} of {mu_path}")
            continue
        if data_path == "rotation_quaternion":
            obj.rotation_mode = "QUATERNION"
        elif data_path == "rotation_euler":
            obj.rotation_mode = "YXZ"
        getattr(obj, data_path)[index] = curve.keys[-1].value * mult


def setup_importer():
    sys.path.insert(0, VENDOR)
    import io_object_mu

    io_object_mu.register()
    import io_object_mu.import_mu  # noqa: F401  (the package shadows the module name)
    from io_object_mu.mu import MuParticles, MuSkinnedMeshRenderer

    im = sys.modules["io_object_mu.import_mu.import_mu"]
    mumesh = sys.modules["io_object_mu.import_mu.mesh"]
    im.make_shader = make_material
    im.create_textures = resolve_textures
    im.create_action = pose_clip
    im.type_handlers[MuParticles] = im.skip_component

    # The addon's armature code fails on some rigs. Fall back to the unskinned mesh: its bind-pose
    # vertices at the renderer's transform, which is the default pose when the bones are at rest.
    create_armatures = im.create_armatures
    skinned = im.type_handlers[MuSkinnedMeshRenderer]

    def object_mode():
        if bpy.context.mode != "OBJECT":
            bpy.ops.object.mode_set(mode="OBJECT")

    def safe_create_armatures(mu):
        try:
            create_armatures(mu)
        except Exception as exc:
            object_mode()
            STATE.warn(f"armature import failed ({type(exc).__name__}: {exc})")

    def skinned_or_static(mu, muobj, skin, name):
        try:
            return skinned(mu, muobj, skin, name)
        except Exception as exc:
            object_mode()
            STATE.warn(f"skinned mesh {name}: {type(exc).__name__}: {exc}; using the unskinned mesh")
            mesh = mumesh.create_mesh(mu, skin.mesh, name)
            mumesh.attach_material(mesh, skin, mu)
            return "mesh", mesh, None

    im.create_armatures = safe_create_armatures
    im.type_handlers[MuSkinnedMeshRenderer] = skinned_or_static

    # Faster mesh setup. Faces are made smooth before the custom normals go in: with flat faces
    # every corner gets its own normal space, the decoded normals differ by float noise and the
    # glTF exporter splits nearly every vertex.
    mumesh.create_uvs = create_uvs
    mumesh.create_normals = create_normals
    mumesh.create_colors = lambda mu, colors, mesh: None
    return im


def corner_verts(mesh) -> np.ndarray:
    idx = np.empty(len(mesh.loops), np.int32)
    mesh.loops.foreach_get("vertex_index", idx)
    return idx


def create_uvs(mu, uvs, mesh, name):
    layer = mesh.uv_layers.new(name=name)
    uv = np.asarray(uvs, np.float32).reshape(-1, 2)
    layer.data.foreach_set("uv", uv[corner_verts(mesh)].ravel())


def create_normals(mu, normals, mesh):
    mesh.polygons.foreach_set("use_smooth", np.ones(len(mesh.polygons), bool))
    mesh.normals_split_custom_set_from_vertices(np.asarray(normals, np.float32).reshape(-1, 3))


def resolve_textures(mu, mudir: str) -> None:
    """Replacement for import_mu.textures.create_textures: only resolves the file paths."""
    for tex in mu.textures:
        tex.phymi_path = find_texture(tex.name, mudir)


def list_dir(path: str) -> dict[str, str]:
    cache = STATE.dir_cache
    if path not in cache:
        try:
            cache[path] = {name.lower(): name for name in os.listdir(path)}
        except OSError:
            cache[path] = {}
    return cache[path]


def find_file(directory: str, base: str, pref_ext: str = "") -> str | None:
    entries = list_dir(directory)
    exts = list(TEXTURE_EXTS)
    if pref_ext in exts and pref_ext != ".mbm":
        exts.remove(pref_ext)
        exts.insert(0, pref_ext)
    candidates = [base + ext for ext in exts]
    if os.path.splitext(base)[1].lower() in TEXTURE_EXTS:
        candidates.insert(0, base)
    for cand in candidates:
        real = entries.get(cand.lower())
        if real:
            return os.path.join(directory, real)
    return None


def find_texture(name: str, mudir: str) -> str | None:
    base, ext = os.path.splitext(os.path.basename(name.replace("\\", "/")))
    ext = ext.lower()
    if ext not in TEXTURE_EXTS:
        base, ext = os.path.basename(name), ""
    target = STATE.replacements.get(base.lower())
    if target:
        found = find_file(os.path.dirname(target), os.path.basename(target), ext)
        if found:
            return found
        STATE.warn(f"texture replacement {base} -> {target}: file not found, using the original")
    return find_file(mudir, base, ext)


# --------------------------------------------------------------------------------------------
# Materials


def classify_shader(shader: str) -> str:
    s = shader.lower()
    if "particles" in s or "additive" in s or "depthmask" in s:
        return "effect"
    if "unlit" in s and ("alpha" in s or "transparent" in s):
        return "effect"
    if "cutoff" in s:
        return "mask"
    if "translucent" in s or "transparent" in s or s.startswith("ksp/alpha/"):
        return "blend"
    return "opaque"


def srgb_to_linear(c: float) -> float:
    c = max(0.0, min(1.0, float(c)))
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def mat_texture(mumat, mu, prop: str, warn: bool = True):
    """(path, scale, offset) of a material texture property, or None."""
    mt = mumat.textureProperties.get(prop)
    if mt is None or not (0 <= mt.index < len(mu.textures)):
        return None
    path = getattr(mu.textures[mt.index], "phymi_path", None)
    if not path:
        if warn:
            STATE.warn(f"texture {mu.textures[mt.index].name} ({mumat.name} {prop}): not found next to the model")
        return None
    return path, tuple(mt.scale), tuple(mt.offset)


def socket(sockets, identifier):
    for s in sockets:
        if s.identifier == identifier:
            return s
    raise KeyError(identifier)


def make_material(mumat, mu):
    """Replacement for shader.make_shader. Only classifies the material; build_materials fills in
    the nodes later for materials that kept meshes use, so hidden variants cost no texture work."""
    shader = mumat.shaderName or ""
    mat = bpy.data.materials.new(mumat.name)
    mat["ksp_shader"] = shader
    mat.use_backface_culling = True  # KSP shaders cull back faces; exported as doubleSided false
    kind = classify_shader(shader)
    if kind in ("mask", "blend") and mat_texture(mumat, mu, "_MainTex", warn=False) is None:
        kind = "decal"  # flag decals etc. get their texture at runtime
    mat["phymi_kind"] = kind
    STATE.pending[mat.as_pointer()] = (mat, mumat, mu)
    return mat


def build_materials(meshes) -> None:
    for obj in meshes:
        for mat in obj.data.materials:
            entry = STATE.pending.pop(mat.as_pointer(), None) if mat is not None else None
            if entry is None:
                continue
            mat, mumat, mu = entry
            try:
                build_material(mat, mumat, mu, mat["phymi_kind"])
            except Exception as exc:  # keep the mesh with a plain material
                STATE.warn(f"material {mumat.name}: {type(exc).__name__}: {exc}")


def build_material(mat, mumat, mu, kind: str) -> None:
    colors = mumat.colorProperties
    floats = mumat.floatProperties3
    shader = (mumat.shaderName or "").lower()
    if mat.node_tree is None:
        mat.use_nodes = True
    nodes = mat.node_tree.nodes
    links = mat.node_tree.links
    nodes.clear()
    out = nodes.new("ShaderNodeOutputMaterial")
    out.location = (400, 0)
    bsdf = nodes.new("ShaderNodeBsdfPrincipled")
    links.new(bsdf.outputs["BSDF"], out.inputs["Surface"])

    color = colors.get("_Color") or colors.get("_MainColor") or (1.0, 1.0, 1.0, 1.0)
    lin = [srgb_to_linear(c) for c in color[:3]]
    alpha_factor = float(color[3]) * float(floats.get("_Opacity", 1.0))
    cutoff = float(floats.get("_Cutoff", 0.5))

    main = mat_texture(mumat, mu, "_MainTex")
    tex_info = None
    if main:
        img, tex_info = get_texture(main[0], "color", want_alpha=(kind != "opaque"))
        mat["uv_scale"] = list(main[1])
        mat["uv_offset"] = list(main[2])
    else:
        img = None
    if kind == "blend" and (tex_info is None or not tex_info["alpha"]) and alpha_factor >= 0.99:
        kind = "opaque"
    if kind == "mask" and (tex_info is None or not tex_info["alpha"] or tex_info["alpha_min"] >= cutoff):
        kind = "opaque"
    if kind == "blend" and tex_info and tex_info["alpha_max"] < 0.02:
        STATE.warn(f"material {mumat.name}: {mumat.shaderName} with an empty alpha channel (invisible in KSP?)")
    mat["phymi_kind"] = kind

    if img is not None:
        tex = nodes.new("ShaderNodeTexImage")
        tex.image = img
        tex.location = (-700, 200)
        color_out = tex.outputs["Color"]
        if any(abs(c - 1.0) > 1e-4 for c in lin):
            mix = nodes.new("ShaderNodeMix")
            mix.data_type = "RGBA"
            mix.blend_type = "MULTIPLY"
            mix.location = (-350, 250)
            socket(mix.inputs, "Factor_Float").default_value = 1.0
            links.new(color_out, socket(mix.inputs, "A_Color"))
            socket(mix.inputs, "B_Color").default_value = (*lin, 1.0)
            color_out = socket(mix.outputs, "Result_Color")
        links.new(color_out, bsdf.inputs["Base Color"])
        if kind == "mask":
            lt = nodes.new("ShaderNodeMath")
            lt.operation = "LESS_THAN"
            lt.location = (-350, 0)
            links.new(tex.outputs["Alpha"], lt.inputs[0])
            lt.inputs[1].default_value = cutoff / max(float(color[3]), 1e-3)
            inv = nodes.new("ShaderNodeMath")
            inv.operation = "SUBTRACT"
            inv.location = (-180, 0)
            inv.inputs[0].default_value = 1.0
            links.new(lt.outputs[0], inv.inputs[1])
            links.new(inv.outputs[0], bsdf.inputs["Alpha"])
        elif kind == "blend" and not tex_info["alpha"]:
            bsdf.inputs["Alpha"].default_value = alpha_factor
        elif kind == "blend":
            alpha_out = tex.outputs["Alpha"]
            if alpha_factor < 0.999:
                mul = nodes.new("ShaderNodeMath")
                mul.operation = "MULTIPLY"
                mul.location = (-350, 0)
                links.new(alpha_out, mul.inputs[0])
                mul.inputs[1].default_value = alpha_factor
                alpha_out = mul.outputs[0]
            links.new(alpha_out, bsdf.inputs["Alpha"])
    else:
        bsdf.inputs["Base Color"].default_value = (*lin, 1.0) if colors.get("_Color") else (0.6, 0.6, 0.6, 1.0)
        if kind == "blend":
            bsdf.inputs["Alpha"].default_value = alpha_factor

    if kind == "blend":
        try:
            mat.surface_render_method = "BLENDED"
        except AttributeError:
            pass

    bump = mat_texture(mumat, mu, "_BumpMap")
    if bump:
        shift = (0.0, 0.0)
        if main and (bump[1] != main[1] or bump[2] != main[2]):
            if all(abs(a - b) < 1e-6 for a, b in zip(bump[1], main[1])):
                shift = (bump[2][0] - main[2][0], bump[2][1] - main[2][1])
            else:
                STATE.warn(f"material {mumat.name}: _BumpMap tiling differs from _MainTex, using _MainTex tiling")
        nimg, _ = get_texture(bump[0], "normal", shift=shift)
        if nimg is not None:
            ntex = nodes.new("ShaderNodeTexImage")
            ntex.image = nimg
            ntex.location = (-700, -300)
            nmap = nodes.new("ShaderNodeNormalMap")
            nmap.location = (-350, -300)
            links.new(ntex.outputs["Color"], nmap.inputs["Color"])
            links.new(nmap.outputs["Normal"], bsdf.inputs["Normal"])

    if shader == "standard" or shader.endswith("/standard"):
        mr = mat_texture(mumat, mu, "_MetallicGlossMap")
        mrimg = None
        if mr:
            mrimg, _ = get_texture(mr[0], "mr", gloss_scale=float(floats.get("_GlossMapScale", 1.0)))
        if mrimg is not None:
            mtex = nodes.new("ShaderNodeTexImage")
            mtex.image = mrimg
            mtex.location = (-700, -650)
            sep = nodes.new("ShaderNodeSeparateColor")
            sep.location = (-350, -650)
            links.new(mtex.outputs["Color"], sep.inputs["Color"])
            links.new(sep.outputs["Green"], bsdf.inputs["Roughness"])
            links.new(sep.outputs["Blue"], bsdf.inputs["Metallic"])
        else:
            bsdf.inputs["Metallic"].default_value = float(floats.get("_Metallic", 0.0))
            bsdf.inputs["Roughness"].default_value = 1.0 - float(floats.get("_Glossiness", 0.45))
    else:
        rough = 0.55
        if "specular" in shader:
            shin = float(floats.get("_Shininess", 0.078))
            rough = max(0.35, min(0.55, 0.55 - (shin - 0.3) * 0.5))
        bsdf.inputs["Metallic"].default_value = 0.0
        bsdf.inputs["Roughness"].default_value = rough
    bsdf.inputs["Emission Strength"].default_value = 0.0


# --------------------------------------------------------------------------------------------
# Textures


def read_mbm(path: str) -> np.ndarray:
    with open(path, "rb") as f:
        data = f.read()
    magic, width, height, _bump, bpp = struct.unpack("<5i", data[:20])
    if magic != 0x50534B03 or bpp not in (24, 32):
        raise RuntimeError(f"unsupported MBM (magic {magic:#x}, bpp {bpp})")
    ch = bpp // 8
    px = np.frombuffer(data, np.uint8, width * height * ch, 20).reshape(height, width, ch)
    out = np.ones((height, width, 4), np.float32)
    out[..., :ch] = px / 255.0
    return out


def read_pixels(path: str) -> np.ndarray:
    """Float32 (h, w, 4) in Blender/Unity row order (row 0 = v 0), as KSP samples the texture."""
    ext = os.path.splitext(path)[1].lower()
    if ext == ".mbm":
        return read_mbm(path)
    img = bpy.data.images.load(path, check_existing=False)
    try:
        img.colorspace_settings.name = "Non-Color"
        img.alpha_mode = "CHANNEL_PACKED"
        w, h = img.size
        ch = img.channels
        if w == 0 or h == 0 or ch == 0:
            raise RuntimeError("Blender could not decode the image")
        buf = np.empty(w * h * ch, np.float32)
        img.pixels.foreach_get(buf)
    finally:
        bpy.data.images.remove(img)
    buf = buf.reshape(h, w, ch)
    out = np.ones((h, w, 4), np.float32)
    if ch == 1:
        out[..., :3] = buf
    elif ch == 2:
        out[..., :3] = buf[..., :1]
        out[..., 3] = buf[..., 1]
    else:
        out[..., : min(ch, 4)] = buf[..., :4]
    if ext == ".dds":
        # KSP DDS textures are stored top row first and uploaded as is, so the first row is v = 0.
        out = out[::-1]
    return np.ascontiguousarray(out)


def resize_axis(a: np.ndarray, n: int, axis: int) -> np.ndarray:
    """Area-average resample of one axis to n samples."""
    length = a.shape[axis]
    if length == n:
        return a
    a = np.moveaxis(a, axis, 0)
    cs = np.concatenate([np.zeros((1,) + a.shape[1:], np.float32), np.cumsum(a, axis=0, dtype=np.float32)])
    edges = np.linspace(0.0, length, n + 1)
    i0 = np.minimum(np.floor(edges).astype(np.int64), length)
    i1 = np.minimum(i0 + 1, length)
    f = (edges - i0).astype(np.float32).reshape((-1,) + (1,) * (a.ndim - 1))
    ce = cs[i0] * (1.0 - f) + cs[i1] * f
    out = (ce[1:] - ce[:-1]) * (n / length)
    return np.moveaxis(out.astype(np.float32), 0, axis)


def downscale(a: np.ndarray, max_side: int) -> np.ndarray:
    h, w = a.shape[:2]
    while max(h, w) > max_side and h % 2 == 0 and w % 2 == 0 and max(h, w) // 2 >= max_side:
        a = a.reshape(h // 2, 2, w // 2, 2, a.shape[2]).mean(axis=(1, 3), dtype=np.float32)
        h, w = a.shape[:2]
    if max(h, w) > max_side:
        s = max_side / max(h, w)
        a = resize_axis(a, max(1, round(h * s)), 0)
        a = resize_axis(a, max(1, round(w * s)), 1)
    return a


def encode_png(arr: np.ndarray, level: int = PNG_LEVEL) -> bytes:
    """8-bit PNG from a (h, w, c) uint8 array, top row first. Per-row adaptive filters."""
    h, w, c = arr.shape
    color_type = {1: 0, 2: 4, 3: 2, 4: 6}[c]
    stride = w * c
    raw = np.ascontiguousarray(arr).reshape(h, stride)
    comp = zlib.compressobj(level)
    parts = []
    prev = np.zeros(stride, np.int16)
    for y0 in range(0, h, 128):
        cur = raw[y0:y0 + 128].astype(np.int16)
        up = np.vstack([prev[None], cur[:-1]])
        left = np.zeros_like(cur)
        left[:, c:] = cur[:, :-c]
        upleft = np.zeros_like(cur)
        upleft[:, c:] = up[:, :-c]
        p = left + up - upleft
        pa, pb, pc = np.abs(p - left), np.abs(p - up), np.abs(p - upleft)
        paeth = np.where((pa <= pb) & (pa <= pc), left, np.where(pb <= pc, up, upleft))
        cands = np.stack([cur, cur - left, cur - up, cur - ((left + up) >> 1), cur - paeth])
        cands = (cands & 0xFF).astype(np.uint8)
        score = np.abs(cands.view(np.int8).astype(np.int32)).sum(axis=2)
        best = score.argmin(axis=0)
        rows = cands[best, np.arange(len(best))]
        filtered = np.concatenate([best.astype(np.uint8)[:, None], rows], axis=1)
        parts.append(comp.compress(filtered.tobytes()))
        prev = cur[-1]
    parts.append(comp.flush())

    def chunk(tag: bytes, payload: bytes) -> bytes:
        return struct.pack(">I", len(payload)) + tag + payload + struct.pack(">I", zlib.crc32(tag + payload))

    ihdr = struct.pack(">IIBBBBB", w, h, 8, color_type, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", b"".join(parts)) + chunk(b"IEND", b"")


def to_u8(a: np.ndarray) -> np.ndarray:
    return np.clip(np.rint(a * 255.0), 0, 255).astype(np.uint8)


def get_texture(path: str, role: str, want_alpha: bool = False, shift=(0.0, 0.0), gloss_scale: float = 1.0):
    """Processed Blender image (backed by a PNG in the job's tmp dir) plus info, cached per job."""
    key = (path, role, want_alpha, tuple(round(s, 6) for s in shift), gloss_scale)
    if key in STATE.image_cache:
        return STATE.image_cache[key]
    result = (None, None)
    try:
        result = process_texture(path, role, want_alpha, shift, gloss_scale)
    except Exception as exc:
        STATE.warn(f"texture {path}: {type(exc).__name__}: {exc}")
    STATE.image_cache[key] = result
    return result


def process_texture(path, role, want_alpha, shift, gloss_scale):
    STATE.sources.add(path)
    px = read_pixels(path)
    src_h, src_w = px.shape[:2]
    info = {"name": os.path.splitext(os.path.basename(path))[0], "source": path, "role": role,
            "source_size": [src_w, src_h], "alpha": False, "alpha_min": 1.0, "alpha_max": 1.0}
    if role == "color":
        alpha = px[..., 3]
        info["alpha_min"] = float(alpha.min())
        info["alpha_max"] = float(alpha.max())
        has_alpha = want_alpha and info["alpha_min"] < 0.996
        info["alpha"] = has_alpha
        out = downscale(px if has_alpha else px[..., :3], STATE.max_texture)
        colorspace = "sRGB"
    elif role == "normal":
        r, g, a = px[..., 0], px[..., 1], px[..., 3]
        dxt5nm = float(r.std()) < 0.02 and float(r.mean()) > 0.9 and (float(a.std()) > 0.005 or float(a.mean()) < 0.98)
        info["dxt5nm"] = dxt5nm
        nx = (a if dxt5nm else r) * 2.0 - 1.0
        ny = g * 2.0 - 1.0
        nz = np.sqrt(np.clip(1.0 - nx * nx - ny * ny, 0.0, 1.0))
        n = np.stack([nx, ny, nz], axis=-1).astype(np.float32)
        if shift != (0.0, 0.0):
            n = np.roll(n, (-round(shift[1] * src_h), -round(shift[0] * src_w)), axis=(0, 1))
        n = downscale(n, STATE.max_texture)
        n /= np.maximum(np.linalg.norm(n, axis=-1, keepdims=True), 1e-6)
        out = n * 0.5 + 0.5
        colorspace = "Non-Color"
    elif role == "mr":
        # Unity Standard: metallic in R, smoothness in A. glTF: roughness in G, metallic in B.
        out = np.ones(px.shape[:2] + (3,), np.float32)
        out[..., 1] = 1.0 - np.clip(px[..., 3] * gloss_scale, 0.0, 1.0)
        out[..., 2] = px[..., 0]
        out = downscale(out, STATE.max_texture)
        colorspace = "Non-Color"
    else:
        raise ValueError(role)
    h, w = out.shape[:2]
    data = encode_png(to_u8(out)[::-1])
    # The exporter names glTF images after the file, so keep the texture's own name.
    stem = re.sub(r"[^A-Za-z0-9_.-]+", "_", info["name"]) + ("_mr" if role == "mr" else "")
    folder = os.path.join(STATE.tmpdir, str(len(STATE.image_cache)))
    os.makedirs(folder)
    png = os.path.join(folder, stem + ".png")
    with open(png, "wb") as f:
        f.write(data)
    img = bpy.data.images.load(png, check_existing=False)
    img.name = f"{info['name']}_mr" if role == "mr" else info["name"]
    img.colorspace_settings.name = colorspace
    img.alpha_mode = "STRAIGHT" if info["alpha"] else "NONE"
    info.update({"size": [w, h], "png_bytes": len(data), "image": img.name})
    STATE.textures.append(info)
    return img, info


# --------------------------------------------------------------------------------------------
# Assembly


def unity_euler_to_quat(euler) -> Quaternion:
    """Unity Euler degrees (Z, then X, then Y) to a Blender-space quaternion (importer convention)."""

    def axis_quat(axis: int, deg: float) -> Quaternion:
        half = math.radians(deg) / 2.0
        q = [0.0, 0.0, 0.0]
        q[axis] = math.sin(half)
        x, y, z = q
        # Unity (x, y, z, w) -> Blender (w, -x, -z, -y), as mu.read_quaternion does.
        return Quaternion((math.cos(half), -x, -z, -y))

    ex, ey, ez = (float(v) for v in euler)
    return axis_quat(1, ey) @ axis_quat(0, ex) @ axis_quat(2, ez)


def unity_vec(v) -> Vector:
    return Vector((float(v[0]), float(v[2]), float(v[1])))


def model_matrix(model: dict) -> Matrix:
    pos = unity_vec(model.get("position") or (0, 0, 0))
    rot = unity_euler_to_quat(model.get("rotation") or (0, 0, 0))
    scale = unity_vec(model.get("scale") or (1, 1, 1))
    return Matrix.LocRotScale(pos, rot, scale)


def to_gltf(v) -> list[float]:
    return [round(float(v[0]), 6), round(float(v[2]), 6), round(float(-v[1]), 6)]


def clean_name(name: str) -> str:
    name = name.split("∧")[0]
    for suffix in (".skin", ".bindPose"):
        if name.endswith(suffix):
            name = name[: -len(suffix)]
    return name


def is_effect_name(name: str) -> bool:
    words = [w.lower() for w in re.findall(r"[A-Z]+(?![a-z])|[A-Z]?[a-z]+", name)]
    return bool(words) and (words[-1] in EFFECT_WORDS or words[0] == "fx")


def walk(obj):
    yield obj
    for child in obj.children:
        yield from walk(child)


def import_models(im, job: dict):
    roots = []
    part = Matrix.Translation(-float(job.get("rescale", 1.0)) * unity_vec(job.get("origin") or (0, 0, 0))) @ \
        Matrix.Scale(float(job.get("rescale", 1.0)), 4)
    collection = bpy.context.scene.collection
    by_name: dict[str, object] = {}
    for model in job["models"]:
        path = model["path"]
        if not os.path.isfile(path):
            raise FileNotFoundError(path)
        STATE.sources.add(path)
        STATE.replacements = {}
        for old, new in (model.get("textures") or {}).items():
            old_base = os.path.basename(old.replace("\\", "/"))
            if os.path.splitext(old_base)[1].lower() in TEXTURE_EXTS:
                old_base = os.path.splitext(old_base)[0]
            STATE.replacements[old_base.lower()] = new
        root, mu = im.import_mu(collection, path, False, False, False)
        if root is None:
            raise RuntimeError(f"{path}: importer returned no root object")
        # node_* empties get an extra -90 deg X rotation for display; undo it so every object's
        # Unity +Z is its Blender local +Y.
        node_rot = Quaternion((0.5 ** 0.5, -(0.5 ** 0.5), 0, 0)).inverted()
        for obj in walk(root):
            if obj.type == "EMPTY" and clean_name(obj.name).startswith("node_"):
                obj.rotation_mode = "QUATERNION"
                obj.rotation_quaternion = obj.rotation_quaternion @ node_rot
        parent = model.get("parent")
        if parent and parent in by_name:
            root.parent = by_name[parent]
            root.matrix_basis = model_matrix(model)
        else:
            if parent:
                STATE.warn(f"MODEL parent {parent} not found, using the part root")
            root.matrix_basis = part @ model_matrix(model)
        # KSP names the instantiated model "<GameData url>(Clone)"; part switches hide it by that name.
        norm = path.replace("\\", "/")
        if "/GameData/" in norm:
            root["phymi_alias"] = os.path.splitext(norm.rsplit("/GameData/", 1)[1])[0] + "(Clone)"
        for obj in walk(root):
            by_name.setdefault(clean_name(obj.name), obj)
        roots.append((root, mu))
    bpy.context.view_layer.update()
    return roots


def select_meshes(roots, job: dict):
    """Kept mesh objects, and the names of objects under hide_transforms (for the "hidden" flag)."""
    hide = set(job.get("hide_transforms") or [])
    keep = set(job.get("keep_transforms") or [])
    names = {clean_name(o.name) for root, _ in roots for o in walk(root)}
    names.update(root["phymi_alias"] for root, _ in roots if "phymi_alias" in root)
    lower = {n.lower(): n for n in names}
    for h in list(hide):
        if h not in names:
            if h.lower() in lower:
                hide.add(lower[h.lower()])
            else:
                STATE.warn(f"hide_transforms: {h} not found")
    kept, hidden = [], set()
    drops: dict[tuple, int] = {}

    def visit(obj, drop):
        name = clean_name(obj.name)
        low = name.lower()
        if drop is None and name not in keep:
            if name in hide or obj.get("phymi_alias") in hide:
                drop = (name, "hide_transforms")
            elif "collider" in low:
                drop = (name, "collider")
            elif is_effect_name(name):
                drop = (name, "effect name")
        if drop is not None and drop[1] == "hide_transforms":
            hidden.add(obj.name)
        if obj.type == "MESH" and len(obj.data.polygons):
            kinds = {m.get("phymi_kind") for m in obj.data.materials if m is not None}
            if drop is not None:
                drops[drop] = drops.get(drop, 0) + 1
            elif kinds & {"effect", "decal"}:
                mat = next(m for m in obj.data.materials if m is not None and m.get("phymi_kind") in ("effect", "decal"))
                what = "effect shader" if mat["phymi_kind"] == "effect" else "untextured alpha material"
                STATE.warn(f"dropped mesh {name} ({what} {mat.name}: {mat['ksp_shader']})")
            else:
                kept.append(obj)
        for child in obj.children:
            visit(child, drop)

    for root, _ in roots:
        visit(root, None)
    if not kept and hide and any(reason == "hide_transforms" for _, reason in drops):
        # Variant switches that hide everything: the part shows one of them by default.
        STATE.warn(f"hide_transforms would hide every mesh; ignoring {sorted(hide)}")
        return select_meshes(roots, {**job, "hide_transforms": []})
    if not kept:
        total = sum(1 for root, _ in roots for o in walk(root) if o.type == "MESH" and len(o.data.polygons))
        raise RuntimeError("the models have no meshes (transforms only)" if total == 0 else
                           f"all {total} meshes were dropped (colliders/effects/hide_transforms): "
                           + "; ".join(STATE.warnings + [f"{n} ({r})" for (n, r) in drops]))
    for (name, reason), count in drops.items():
        if reason != "collider":
            STATE.warn(f"dropped {name} ({reason}): {count} mesh{'es' if count > 1 else ''}")
    return kept, hidden


def prune_outliers(kept):
    """Drop meshes far bigger than the rest of the model: meshes the addon can't pose end up
    tens of metres out (an RD-180 pipe). A mesh goes if the model is over 12 m across with it
    and under a fifth of that without it; smaller models often are one big mesh plus details."""
    if len(kept) < 2:
        return kept
    dg = bpy.context.evaluated_depsgraph_get()
    boxes = []
    for obj in kept:
        ev = obj.evaluated_get(dg)
        corners = np.array([list(ev.matrix_world @ Vector(c)) for c in ev.bound_box])
        boxes.append((corners.min(axis=0), corners.max(axis=0)))

    def diagonal(indices) -> float:
        lo = np.min([boxes[i][0] for i in indices], axis=0)
        hi = np.max([boxes[i][1] for i in indices], axis=0)
        return float(np.linalg.norm(hi - lo))

    alive = list(range(len(kept)))
    while len(alive) > 1:
        whole = diagonal(alive)
        worst = min(alive, key=lambda i: diagonal([j for j in alive if j != i]))
        rest = diagonal([j for j in alive if j != worst])
        if whole < 12.0 or rest >= 0.2 * whole:
            break
        STATE.warn(f"dropped {clean_name(kept[worst].name)} (outlier: {whole:.1f} m model, {rest:.1f} m without it)")
        alive.remove(worst)
    return [kept[i] for i in alive]


def collect_transforms(roots, hidden, job: dict) -> dict:
    extra = set(job.get("report_transforms") or [])
    found: dict[str, dict] = {}

    def visit(muobj):
        obj = getattr(muobj, "bobj", None)
        name = muobj.transform.name
        if obj is not None and (TRANSFORM_NAME.search(name) or name in extra) and "collider" not in name.lower():
            m = obj.matrix_world
            # Unity +Z (forward) is Blender local +Y, Unity +Y (up) is Blender local +Z.
            d = (m.to_3x3() @ Vector((0.0, 1.0, 0.0))).normalized()
            up = (m.to_3x3() @ Vector((0.0, 0.0, 1.0))).normalized()
            inst = {"position": to_gltf(m.translation), "direction": to_gltf(d), "up": to_gltf(up),
                    "hidden": obj.name in hidden}
            entry = found.setdefault(name, {"position": inst["position"], "direction": inst["direction"],
                                            "up": inst["up"], "count": 0, "instances": []})
            entry["count"] += 1
            entry["instances"].append(inst)
        for child in muobj.children:
            visit(child)

    for _, mu in roots:
        visit(mu.obj)
    for name in extra:
        if name not in found:
            STATE.warn(f"report_transforms: {name} not found")
    return found


def bake(kept, job_id: str):
    dg = bpy.context.evaluated_depsgraph_get()
    coll = bpy.data.collections.new("phymi_export")
    bpy.context.scene.collection.children.link(coll)
    fallback = None
    baked = []
    for obj in kept:
        me = bpy.data.meshes.new_from_object(obj.evaluated_get(dg), preserve_all_data_layers=True, depsgraph=dg)
        while len(me.uv_layers) > 1:
            me.uv_layers.remove(me.uv_layers[-1])
        if len(me.uv_layers) == 1:
            me.uv_layers[0].name = "UVMap"
        for attr in list(me.color_attributes):
            me.color_attributes.remove(attr)
        if not me.materials or me.materials[0] is None:
            if fallback is None:
                fallback = bpy.data.materials.new("default")
                fallback.diffuse_color = (0.6, 0.6, 0.6, 1.0)
                fallback.use_backface_culling = True
            me.materials.clear()
            me.materials.append(fallback)
        mat = me.materials[0]
        scale = list(mat.get("uv_scale", [1.0, 1.0]))
        offset = list(mat.get("uv_offset", [0.0, 0.0]))
        if len(me.uv_layers) and (scale != [1.0, 1.0] or offset != [0.0, 0.0]):
            uv = np.empty(len(me.loops) * 2, np.float32)
            me.uv_layers[0].data.foreach_get("uv", uv)
            uv = uv.reshape(-1, 2) * np.array(scale, np.float32) + np.array(offset, np.float32)
            me.uv_layers[0].data.foreach_set("uv", uv.ravel())
        nob = bpy.data.objects.new(clean_name(obj.name), me)
        coll.objects.link(nob)
        nob.matrix_world = obj.matrix_world.copy()
        baked.append(nob)
    with bpy.context.temp_override(active_object=baked[0], object=baked[0],
                                   selected_objects=baked, selected_editable_objects=baked):
        bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
        if len(baked) > 1:
            bpy.ops.object.join()
    final = baked[0]
    final.name = job_id
    final.data.name = job_id
    return final


def mesh_stats(final, job: dict) -> dict:
    me = final.data
    co = np.empty(len(me.vertices) * 3, np.float32)
    me.vertices.foreach_get("co", co)
    co = co.reshape(-1, 3)
    g = np.stack([co[:, 0], co[:, 2], -co[:, 1]], axis=1)  # glTF axes
    me.calc_loop_triangles()
    lo, hi = g.min(axis=0), g.max(axis=0)
    r = np.sqrt(g[:, 0] ** 2 + g[:, 2] ** 2)
    y = g[:, 1]
    height = float(hi[1] - lo[1])
    stats = {
        "triangles": len(me.loop_triangles),
        "vertices": len(me.vertices),
        "bbox_min": [round(float(v), 5) for v in lo],
        "bbox_max": [round(float(v), 5) for v in hi],
        "height": round(height, 5),
        "max_diameter": round(2.0 * float(r.max()), 5),
        "top_diameter": round(2.0 * float(r[y >= hi[1] - 0.06 * height].max()), 5),
        "bottom_diameter": round(2.0 * float(r[y <= lo[1] + 0.06 * height].max()), 5),
    }
    if job.get("kind") == "engine":
        length = float(-lo[1])
        if length <= 0:
            STATE.warn("engine: no geometry below the origin")
        band = 0.04 * (length if length > 0 else height)
        stats["length"] = round(length, 5)
        stats["exit_diameter"] = round(2.0 * float(r[y <= lo[1] + band].max()), 5)
    return stats


def export_glb(final, output: str) -> None:
    for obj in bpy.context.scene.objects:
        obj.select_set(False)
    final.select_set(True)
    bpy.context.view_layer.objects.active = final
    os.makedirs(os.path.dirname(os.path.abspath(output)), exist_ok=True)
    tmp = f"{output[:-4] if output.lower().endswith('.glb') else output}.tmp{os.getpid()}.glb"
    bpy.ops.export_scene.gltf(
        filepath=tmp,
        export_format="GLB",
        use_selection=True,
        use_active_scene=True,
        export_yup=True,
        export_apply=True,
        export_animations=False,
        export_skins=False,
        export_morph=False,
        export_cameras=False,
        export_lights=False,
        export_extras=False,
        export_tangents=True,
        export_vertex_color="NONE",
        export_attributes=False,
        export_image_format="AUTO",
        export_keep_originals=False,
        export_draco_mesh_compression_enable=False,
        export_materials="EXPORT",
        will_save_settings=False,
    )
    if not os.path.isfile(tmp) or os.path.getsize(tmp) == 0:
        raise RuntimeError("glTF exporter wrote no file")
    os.replace(tmp, output)


def read_glb_json(path: str) -> dict:
    with open(path, "rb") as f:
        head = f.read(20)
        magic, _version, _length, chunk_len, chunk_type = struct.unpack("<4sIIII", head)
        if magic != b"glTF" or chunk_type != 0x4E4F534A:
            raise RuntimeError("output is not a GLB")
        return json.loads(f.read(chunk_len))


def convert(im, job: dict, res: dict) -> None:
    output = job["output"]
    roots = import_models(im, job)
    kept, hidden = select_meshes(roots, job)
    kept = prune_outliers(kept)
    res["transforms"] = collect_transforms(roots, hidden, job)
    build_materials(kept)
    res["meshes"] = sorted({clean_name(o.name) for o in kept})
    final = bake(kept, str(job["id"]))
    res.update(mesh_stats(final, job))
    export_glb(final, output)
    res["bytes"] = os.path.getsize(output)
    gl = read_glb_json(output)
    images = gl.get("images", [])
    bad = [i.get("mimeType") for i in images if i.get("mimeType") != "image/png"]
    if bad:
        raise RuntimeError(f"non-PNG images in output: {bad}")
    res["textures"] = [i.get("name") for i in images]
    res["materials"] = [{"name": m.get("name"), "alpha_mode": m.get("alphaMode", "OPAQUE")}
                        for m in gl.get("materials", [])]
    res["texture_details"] = [{k: t[k] for k in ("image", "source", "role", "source_size", "size", "png_bytes")}
                              for t in STATE.textures]


def run_job(im, job: dict) -> dict:
    global STATE
    t0 = time.time()
    res = {"id": job.get("id"), "ok": False, "error": None, "output": job.get("output"), "bytes": 0,
           "triangles": 0, "bbox_min": None, "bbox_max": None, "transforms": {}, "textures": [],
           "warnings": []}
    tmpdir = tempfile.mkdtemp(prefix="convert_mu_")
    STATE = JobState(job, tmpdir)
    try:
        bpy.ops.wm.read_factory_settings(use_empty=True)
        convert(im, job, res)
        res["ok"] = True
    except Exception as exc:
        res["error"] = f"{type(exc).__name__}: {exc}"
        res["traceback"] = traceback.format_exc()
    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)
    res["warnings"] = STATE.warnings
    res["sources"] = sorted(STATE.sources)
    res["seconds"] = round(time.time() - t0, 3)
    return res


def write_json(path: str, data) -> None:
    tmp = f"{path}.tmp{os.getpid()}"
    with open(tmp, "w") as f:
        json.dump(data, f, indent=1)
    os.replace(tmp, path)


def main() -> None:
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if len(argv) != 2:
        raise SystemExit("usage: Blender -b --factory-startup --python convert_mu.py -- <jobs.json> <results.json>")
    with open(argv[0]) as f:
        jobs = json.load(f)
    im = setup_importer()
    results = []
    for i, job in enumerate(jobs):
        print(f"[convert_mu] {i + 1}/{len(jobs)} {job.get('id')}", flush=True)
        res = run_job(im, job)
        results.append(res)
        write_json(argv[1], results)
        status = "ok" if res["ok"] else f"FAILED {res['error']}"
        print(f"[convert_mu] {job.get('id')}: {status} ({res['bytes']} bytes, {res['seconds']:.1f} s)", flush=True)


if __name__ == "__main__":
    main()
