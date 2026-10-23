// Vessel surfaces: standard PBR plus procedural detail for the finish a surface stands for
// (see `crate::vessel_material`): paint, carbon composite, bare metal, heat-tinted metal
// or ablative char. The detail lives in the part's own axes, in metres, so it stays put on
// the vessel; anything finer than a pixel fades out rather than shimmer.

import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    mesh_view_bindings::view,
    mesh_functions::get_world_from_local,
};

struct VesselParams {
    /// x: finish (below); y, z: oxide film thickness range for heat tint, nm.
    finish: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> params: VesselParams;

const PAINT: u32 = 0u;
const COMPOSITE: u32 = 1u;
const METAL: u32 = 2u;
const HEAT_TINT: u32 = 3u;
const CHAR: u32 = 4u;

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
fn composite(p: vec3<f32>, footprint: f32) -> Detail {
    var d: Detail;
    // Around and along the stage axis, metres.
    let around = normalize(vec3<f32>(p.z, 0.0, -p.x) + vec3<f32>(1e-6, 0.0, 0.0));
    let uv = vec2<f32>(atan2(-p.z, p.x) * length(p.xz), p.y);
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
            slope = around * 0.38 * cos(PI * t.x);
            sheen = -0.05;
        } else {
            slope = vec3<f32>(0.0, 0.38 * cos(PI * t.y), 0.0);
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

    var d: Detail;
    switch u32(params.finish.x) {
        case COMPOSITE: { d = composite(p, footprint); }
        case METAL: { d = metal(p, footprint); }
        case HEAT_TINT: { d = heat_tint(p, footprint, cos_view); }
        case CHAR: { d = charred(p, footprint); }
        default: { d = paint(p, footprint); }
    }

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
