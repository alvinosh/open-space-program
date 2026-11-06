// Vessel surfaces: standard PBR plus procedural detail for the finish a surface stands for
// (see `crate::vessel_material`): paint, carbon composite, bare metal, heat-tinted metal,
// ablative char, spray-on foam, cork or welded stainless steel. Paint layers go on top
// (`crate::paint_layers`): bands, stripes, roll patterns and stringers worked out here, so
// their edges are sharp however close, and lettering and pictures from a texture. The detail lives in the part's own axes, in metres, so it stays put on
// the vessel; anything finer than a pixel fades out rather than shimmer. Woven detail runs
// along the surface: by the mesh's surface coordinates (metres round and along it) where
// it has them, as parts drawn from their geometry do, and round the part's axis otherwise.

import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    mesh_view_bindings::view,
    mesh_functions::get_world_from_local,
};

struct VesselParams {
    /// x: finish (below); y, z: oxide film thickness range for heat tint, nm; w: 1 where
    /// the mesh's surface coordinates are metres along it, with tangents.
    finish: vec4<f32>,
};

/// Where a point is on its surface, metres, and which ways its coordinates run there, in
/// the part's axes.
struct Surface {
    uv: vec2<f32>,
    around: vec3<f32>,
    along: vec3<f32>,
};

/// Paint layers, four vectors each: (kind, finish or −1 to keep what is under it, metallic,
/// roughness), (colour, clearcoat), and two of the kind's own.
struct VesselLayers {
    count: vec4<f32>,
    data: array<vec4<f32>, 64>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> params: VesselParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var<uniform> layers: VesselLayers;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var decals: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var decal_sampler: sampler;

const PAINT: u32 = 0u;
const COMPOSITE: u32 = 1u;
const METAL: u32 = 2u;
const HEAT_TINT: u32 = 3u;
const CHAR: u32 = 4u;
const FOAM: u32 = 5u;
const CORK: u32 = 6u;
const STAINLESS: u32 = 7u;

const BAND: u32 = 1u;
const STRIPE: u32 = 2u;
const CHECKER: u32 = 3u;
const DECAL: u32 = 4u;
const DECAL_VERTICAL: u32 = 5u;
const STRINGERS: u32 = 6u;

const PI: f32 = 3.141592653589793;

/// What a finish changes at a point.
struct Detail {
    /// Gradient of the surface relief in the part's axes (a slope, metres per metre).
    slope: vec3<f32>,
    /// Multiplies the base colour.
    tint: vec3<f32>,
    /// Added to the perceptual roughness.
    roughness: f32,
};

fn hash(p: vec3<i32>) -> f32 {
    var h = (bitcast<u32>(p.x) * 1664525u) ^ (bitcast<u32>(p.y) * 1013904223u)
        ^ (bitcast<u32>(p.z) * 2654435761u);
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h) / 4294967295.0;
}

/// Value noise on a unit lattice: xyz its gradient, w its value in [-1, 1].
fn noise(p: vec3<f32>) -> vec4<f32> {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let du = 6.0 * f * (1.0 - f);
    let a = hash(i);
    let b = hash(i + vec3<i32>(1, 0, 0));
    let c = hash(i + vec3<i32>(0, 1, 0));
    let d = hash(i + vec3<i32>(1, 1, 0));
    let e = hash(i + vec3<i32>(0, 0, 1));
    let g = hash(i + vec3<i32>(1, 0, 1));
    let h = hash(i + vec3<i32>(0, 1, 1));
    let k = hash(i + vec3<i32>(1, 1, 1));
    let k1 = b - a;
    let k2 = c - a;
    let k3 = e - a;
    let k4 = a - b - c + d;
    let k5 = a - c - e + h;
    let k6 = a - b - e + g;
    let k7 = -a + b + c - d + e - g - h + k;
    let value = a + k1 * u.x + k2 * u.y + k3 * u.z + k4 * u.x * u.y + k5 * u.y * u.z
        + k6 * u.z * u.x + k7 * u.x * u.y * u.z;
    let gradient = du * vec3<f32>(
        k1 + k4 * u.y + k6 * u.z + k7 * u.y * u.z,
        k2 + k5 * u.z + k4 * u.x + k7 * u.z * u.x,
        k3 + k6 * u.x + k5 * u.y + k7 * u.x * u.y,
    );
    return vec4<f32>(2.0 * gradient, 2.0 * value - 1.0);
}

/// How much of a pattern `cell` metres across still shows when a pixel covers `footprint`.
fn resolved(cell: f32, footprint: f32) -> f32 {
    return 1.0 - smoothstep(0.3 * cell, 1.2 * cell, footprint);
}

/// Slope of random relief `height` metres high on cells `cell` metres across.
fn bumps(p: vec3<f32>, cell: vec3<f32>, height: f32, footprint: f32) -> vec3<f32> {
    let fade = resolved(min(cell.x, min(cell.y, cell.z)), footprint);
    if fade <= 0.0 {
        return vec3<f32>(0.0);
    }
    return noise(p / cell).xyz / cell * height * fade;
}

/// Glossy paint over a metal skin. The skin isn't perfectly true, so reflections of the
/// horizon waver along it; up close the paint shows orange peel; weathering leaves faint
/// streaks down the stage.
fn paint(p: vec3<f32>, footprint: f32) -> Detail {
    var d: Detail;
    d.slope = bumps(p, vec3<f32>(0.9, 1.4, 0.9), 0.011, footprint)
        + bumps(p, vec3<f32>(0.25, 0.4, 0.25), 0.0012, footprint)
        + bumps(p, vec3<f32>(0.003), 0.000018, footprint);
    let tone = noise(p / 2.3).w;
    let streak = noise(p / vec3<f32>(0.12, 2.5, 0.12)).w;
    let blotch = noise(p / 0.6).w;
    d.tint = vec3<f32>(1.0 + 0.035 * tone - 0.04 * max(streak, 0.0));
    d.roughness = 0.1 * max(streak, 0.0) - 0.03 + 0.04 * blotch;
    return d;
}

/// Carbon composite: a 2×2 twill of rounded tows under resin, glossy up close; from a
/// distance the weave blurs into a matte surface.
fn composite(p: vec3<f32>, s: Surface, footprint: f32) -> Detail {
    var d: Detail;
    let uv = s.uv;
    let tow = 0.004;
    let weave = resolved(tow, footprint);
    var slope = vec3<f32>(0.0);
    var sheen = 0.0;
    if weave > 0.0 {
        let cell = vec2<i32>(floor(uv / tow));
        let t = fract(uv / tow);
        // Tows along the axis show where (i + j) mod 4 < 2, the cross tows elsewhere; each
        // is rounded across its width.
        let along = ((cell.x + cell.y) % 4 + 4) % 4 < 2;
        if along {
            slope = s.around * 0.38 * cos(PI * t.x);
            sheen = -0.05;
        } else {
            slope = s.along * 0.38 * cos(PI * t.y);
            sheen = 0.05;
        }
    }
    // Plies laid up and cured unevenly: long patches of slightly different tone and gloss.
    let layup = noise(p / vec3<f32>(0.8, 2.5, 0.8)).w;
    d.slope = slope * weave + bumps(p, vec3<f32>(0.5), 0.002, footprint);
    d.tint = vec3<f32>(1.0 + 0.25 * layup + 0.15 * noise(p / 1.3).w + sheen * weave);
    // The resin is glossy; unresolved tows scatter the light like a rough surface.
    d.roughness = -0.3 * weave + sheen * weave + 0.08 * layup + 0.06 * noise(p / 0.8).w;
    return d;
}

/// Machined or brushed metal: fine grooves along the part's axis stretch the highlights;
/// the finish varies in streaks and patches.
fn metal(p: vec3<f32>, footprint: f32) -> Detail {
    var d: Detail;
    let brush = vec3<f32>(0.0008, 0.05, 0.0008);
    d.slope = bumps(p, brush, 0.000004, footprint)
        + bumps(p, vec3<f32>(0.6, 0.9, 0.6), 0.0008, footprint);
    let streak = noise(p / vec3<f32>(0.02, 0.6, 0.02)).w;
    // Grooves too fine to see still roughen the reflection.
    let lost = 1.0 - resolved(brush.x, footprint);
    d.roughness = 0.06 * streak + 0.04 * noise(p / 0.3).w + 0.05 * lost;
    d.tint = vec3<f32>(1.0 + 0.03 * noise(p / 0.9).w);
    return d;
}

/// Colour of the light a metal under an oxide film `thickness` nm thick reflects. The
/// film's two reflections interfere, so as it thickens the metal turns straw, bronze,
/// purple, then blue, and the colour shifts with the viewing angle.
fn thin_film(thickness: f32, cos_view: f32) -> vec3<f32> {
    let n = 2.3;
    let cos_film = sqrt(max(1.0 - (1.0 - cos_view * cos_view) / (n * n), 0.0));
    let waves = 2.0 * n * thickness * cos_film / vec3<f32>(610.0, 540.0, 465.0);
    let fringe = 0.5 + 0.5 * cos(2.0 * PI * waves);
    return mix(vec3<f32>(1.0), 0.15 + 0.95 * fringe, smoothstep(0.0, 30.0, thickness));
}

/// Metal tinted by heat: the oxide film is thickest towards the throat (the top of the
/// nozzle, near the model's origin), where the metal ran hottest, with faint rings where the
/// cooling changed and patches where it ran unevenly.
fn heat_tint(p: vec3<f32>, footprint: f32, cos_view: f32) -> Detail {
    var d = metal(p, footprint);
    let patches = noise(p / vec3<f32>(0.35, 0.6, 0.35)).w;
    let heat = exp(min(p.y, 0.0) / 1.2);
    let rings = 0.08 * sin(p.y * 2.0 * PI / 0.3 + 0.8 * patches);
    let share = clamp(heat + 0.15 * patches + rings, 0.0, 1.0);
    let thickness = mix(params.finish.y, params.finish.z, share);
    d.tint *= thin_film(thickness, cos_view);
    return d;
}

/// Charred ablator or soot: mottled, crazed and rough.
fn charred(p: vec3<f32>, footprint: f32) -> Detail {
    var d: Detail;
    let mottle = 0.6 * noise(p / 0.07).w + 0.4 * noise(p / 0.02).w;
    d.slope = bumps(p, vec3<f32>(0.02), 0.0015, footprint)
        + bumps(p, vec3<f32>(0.005), 0.0003, footprint);
    d.tint = vec3<f32>(0.75 + 0.35 * mottle);
    d.roughness = 0.05 * mottle;
    return d;
}

/// Spray-on foam insulation: lumpy where it was sprayed in passes, mottled and matte.
fn foam(p: vec3<f32>, footprint: f32) -> Detail {
    var d: Detail;
    d.slope = bumps(p, vec3<f32>(0.05, 0.035, 0.05), 0.004, footprint)
        + bumps(p, vec3<f32>(0.012), 0.0008, footprint)
        + bumps(p, vec3<f32>(0.6, 0.25, 0.6), 0.006, footprint);
    let passes = noise(p / vec3<f32>(1.5, 0.3, 1.5)).w;
    d.tint = vec3<f32>(1.0 + 0.12 * passes + 0.1 * noise(p / 0.08).w + 0.05 * noise(p / 0.02).w);
    d.roughness = 0.05 * noise(p / 0.4).w;
    return d;
}

/// Cork: pressed granules, speckled light and dark.
fn cork(p: vec3<f32>, footprint: f32) -> Detail {
    var d: Detail;
    d.slope = bumps(p, vec3<f32>(0.004), 0.0004, footprint);
    let grain = resolved(0.004, footprint);
    d.tint = vec3<f32>(1.0 + 0.3 * grain * noise(p / 0.003).w + 0.08 * noise(p / 0.5).w);
    d.roughness = 0.04 * noise(p / 0.2).w;
    return d;
}

/// How far `p` is from the nearest weld of steel rings `ring` metres tall, each rolled from
/// a sheet with two seams up it set round at random, m; and which way is across that weld.
fn weld(p: vec3<f32>, s: Surface, ring: f32) -> vec2<f32> {
    let level = p.y / ring;
    let to_ring = (abs(fract(level + 0.5) - 0.5)) * ring;
    let k = i32(floor(level));
    let radius = length(p.xz);
    let around = atan2(-p.z, p.x) / (2.0 * PI);
    let seam = hash(vec3<i32>(k, 7, 3));
    let turns = abs(fract(around - seam + 0.25) - 0.5) - 0.25;
    let to_seam = abs(turns) * 2.0 * PI * radius;
    return vec2<f32>(to_ring, to_seam);
}

/// Bare stainless steel: rolled rings welded together, each weld a slight bead with a
/// straw-to-grey heat tint either side of it.
fn stainless(p: vec3<f32>, s: Surface, footprint: f32, cos_view: f32) -> Detail {
    var d = metal(p, footprint);
    let w = weld(p, s, 1.83);
    let bead = 0.008;
    let show = resolved(bead, footprint);
    let ring = exp(-pow(w.x / bead, 2.0));
    let seam = exp(-pow(w.y / bead, 2.0));
    let ring_side = sign(fract(p.y / 1.83 + 0.5) - 0.5);
    d.slope += show * 0.15 * (s.along * ring * ring_side * (w.x / bead) + s.around * seam);
    let heat = max(exp(-w.x / 0.03), exp(-w.y / 0.03));
    d.tint *= mix(vec3<f32>(1.0), thin_film(25.0, cos_view) * 0.9, 0.5 * heat * show);
    d.roughness += 0.12 * heat;
    return d;
}

/// How much of the pixel a band from `low` to `high` covers, by `x` and its footprint `w`.
fn within(x: f32, low: f32, high: f32, w: f32) -> f32 {
    return clamp(min(x - low, high - x) / w + 0.5, 0.0, 1.0);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // The part's axes without its scale, so the detail is in metres on scaled models too.
    let world_from_local = get_world_from_local(in.instance_index);
    let axes = mat3x3<f32>(
        normalize(world_from_local[0].xyz),
        normalize(world_from_local[1].xyz),
        normalize(world_from_local[2].xyz),
    );
    let p = (in.world_position.xyz - world_from_local[3].xyz) * axes;
    let footprint = max(length(dpdx(p)), length(dpdy(p)));
    let n = pbr_input.N;
    let cos_view = abs(dot(n, normalize(view.world_position - in.world_position.xyz)));

    // Round and along the part's axis, unless the mesh says how its surface runs.
    var s: Surface;
    s.uv = vec2<f32>(atan2(-p.z, p.x) * length(p.xz), p.y);
    s.around = normalize(vec3<f32>(p.z, 0.0, -p.x) + vec3<f32>(1e-6, 0.0, 0.0));
    s.along = vec3<f32>(0.0, 1.0, 0.0);
    @if(VERTEX_UVS_A) {
        @if(VERTEX_TANGENTS) {
            if params.finish.w > 0.5 {
                let normal = n * axes;
                let tangent = normalize(in.world_tangent.xyz * axes);
                s.uv = in.uv;
                s.around = tangent;
                s.along = normalize(cross(normal, tangent)) * sign(in.world_tangent.w);
            }
        }
    }

    // The paint layers, laid out round the part's axis in turns and up it in metres.
    let around = fract(atan2(-p.z, p.x) / (2.0 * PI) + 1.0);
    let radius = length(p.xz);
    let turn = 2.0 * PI * radius;
    let height = p.y;
    var da = vec2<f32>(dpdx(around), dpdy(around));
    da -= round(da);
    let wa = max((abs(da.x) + abs(da.y)) * turn, 1e-5);
    let wh = max(fwidth(height), 1e-5);
    var color = pbr_input.material.base_color.rgb;
    var finish = u32(params.finish.x);
    var metallic = pbr_input.material.metallic;
    var roughness = pbr_input.material.perceptual_roughness;
    var clearcoat = pbr_input.material.clearcoat;
    var relief = vec3<f32>(0.0);
    let count = u32(layers.count.x);
    for (var i = 0u; i < count; i++) {
        let a = layers.data[4u * i];
        let b = layers.data[4u * i + 1u];
        let c = layers.data[4u * i + 2u];
        let e = layers.data[4u * i + 3u];
        let kind = u32(a.x);
        // A decal's place: metres round from its middle and up from it, as its picture's
        // coordinates (sampled whatever the kind, to keep the derivatives in step).
        let x = (fract(around - c.x + 0.5) - 0.5) * turn;
        let y = height - c.y;
        var st = vec2<f32>(0.5 + x / c.z, 0.5 - y / c.w);
        if kind == DECAL_VERTICAL {
            st = vec2<f32>(0.5 - y / c.z, 0.5 - x / c.w);
        }
        let uv = e.xy + clamp(st, vec2<f32>(0.0), vec2<f32>(1.0)) * e.zw;
        let texel = textureSampleGrad(decals, decal_sampler, uv, dpdx(e.xy + st * e.zw), dpdy(e.xy + st * e.zw));
        var cover = 0.0;
        switch kind {
            case BAND: { cover = within(height, c.x, c.y, wh); }
            case STRIPE: {
                let width = fract(c.y - c.x + 1.0);
                let into = fract(around - c.x + 1.0);
                let inside = select(-min(into - width, 1.0 - into), min(into, width - into), into < width);
                cover = min(clamp(inside * turn / wa + 0.5, 0.0, 1.0), within(height, c.z, c.w, wh));
            }
            case CHECKER: {
                let sectors = abs(c.x);
                let rows = u32(c.y);
                let at = array<f32, 6>(c.z, c.w, e.x, e.y, e.z, e.w);
                var row = -1;
                var edge = 1e9;
                for (var k = 0u; k + 1u < rows; k++) {
                    if height >= at[k] && height < at[k + 1u] {
                        row = i32(k);
                        edge = min(height - at[k], at[k + 1u] - height);
                    }
                }
                if row >= 0 {
                    let sector = floor(around * sectors);
                    let into = fract(around * sectors);
                    edge = min(edge, min(into, 1.0 - into) / sectors * turn);
                    let first = select(1, 0, c.x < 0.0);
                    let painted = (i32(sector) + row + first) % 2 == 1;
                    let sharp = clamp(edge / max(wa, wh), 0.0, 1.0);
                    cover = select(0.5 - 0.5 * sharp, 0.5 + 0.5 * sharp, painted);
                    cover *= within(height, at[0], at[rows - 1u], wh);
                }
            }
            case DECAL, DECAL_VERTICAL: {
                let inside = all(st >= vec2<f32>(0.0)) && all(st <= vec2<f32>(1.0)) && radius > 1e-3;
                let alpha = select(0.0, texel.a, inside);
                color = color * (1.0 - alpha) + select(vec3<f32>(0.0), texel.rgb, inside);
            }
            case STRINGERS: {
                let pitch = turn / max(c.z, 1.0);
                let phase = fract(around * c.z);
                let ridge = 0.5 + 0.5 * cos(2.0 * PI * phase);
                let rise = -c.w * 4.0 * pow(ridge, 3.0) * PI * sin(2.0 * PI * phase) * c.z / turn;
                relief += s.around * rise * resolved(pitch, footprint) * within(height, c.x, c.y, wh);
            }
            default: {}
        }
        color = mix(color, b.rgb, cover);
        if a.y >= 0.0 {
            metallic = mix(metallic, a.z, cover);
            roughness = mix(roughness, a.w, cover);
            clearcoat = mix(clearcoat, b.w, cover);
            if cover > 0.5 {
                finish = u32(a.y);
            }
        }
    }
    pbr_input.material.base_color = vec4<f32>(color, pbr_input.material.base_color.a);
    pbr_input.material.metallic = metallic;
    pbr_input.material.perceptual_roughness = roughness;
    pbr_input.material.clearcoat = clearcoat;

    var d: Detail;
    switch finish {
        case COMPOSITE: { d = composite(p, s, footprint); }
        case METAL: { d = metal(p, footprint); }
        case HEAT_TINT: { d = heat_tint(p, footprint, cos_view); }
        case CHAR: { d = charred(p, footprint); }
        case FOAM: { d = foam(p, footprint); }
        case CORK: { d = cork(p, footprint); }
        case STAINLESS: { d = stainless(p, s, footprint, cos_view); }
        default: { d = paint(p, footprint); }
    }
    d.slope += relief;

    let slope = axes * d.slope;
    let bumped = normalize(n - (slope - dot(slope, n) * n));
    pbr_input.N = bumped;
    pbr_input.clearcoat_N = bumped;
    let base = pbr_input.material.base_color;
    pbr_input.material.base_color = vec4<f32>(base.rgb * d.tint, base.a);
    pbr_input.material.perceptual_roughness =
        clamp(pbr_input.material.perceptual_roughness + d.roughness, 0.05, 1.0);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
