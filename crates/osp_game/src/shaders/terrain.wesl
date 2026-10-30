// Planet surface. Extends the standard PBR material with per-chunk data textures:
//
// - colour: satellite imagery (rgb, sRGB) and night lights (a)
// - surface: normal (east, north components), raw elevation in metres (bathymetry
//   included) and water fraction
// - cover: land-cover class per sample (see `osp_terrain::Cover`)
//
// Far away the imagery dominates. Where land cover is much more detailed than the imagery
// (around launch sites) the classes drive the colour, and near the camera each class gets
// procedural detail. Lighting is corrected for the atmosphere per fragment.

import bevy_pbr::render::{
    pbr_fragment::pbr_input_from_standard_material,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
    mesh_view_bindings::{view, lights, globals},
    mesh_functions::{get_world_from_local, get_local_from_world},
};
import osp::shaders::atmosphere::{
    scatter, sun_transmittance, light_transmittance, luminance, sphere_hits, THICKNESS,
    SKY_MESH_SCALE, PI,
};

struct TerrainParams {
    /// xyz: planet centre in chunk-local coordinates; w: mean radius.
    planet: vec4<f32>,
    /// xyz: chunk centre in body-fixed axes modulo DETAIL_PERIOD.
    detail_origin: vec4<f32>,
    /// x: chunk level, y: colour data level, z: cover data level, w: 1 for an Earth-like
    /// atmosphere.
    levels: vec4<f32>,
    /// x: ray-marching steps for the haze.
    quality: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> params: TerrainParams;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var color_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(102) var color_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(103) var surface_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(104) var surface_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(105) var cover_texture: texture_2d<u32>;

const CELLS: f32 = 128.0;
/// Detail noise repeats every this many metres along each body-fixed axis, so it can be
/// evaluated in f32 anywhere on a planet (see `planet::DETAIL_PERIOD`).
const DETAIL_PERIOD: f32 = 4096.0;

/// Skylight on level ground with the sun overhead, as a share of the sunlight above the
/// air, and its colour (unit luminance) with the sun at the horizon and high up. Fitted to
/// `atmosphere::scatter` integrated over the sky.
const SKYLIGHT: f32 = 0.103;
const SKY_LOW: vec3<f32> = vec3<f32>(1.15, 0.95, 1.05);
const SKY_HIGH: vec3<f32> = vec3<f32>(0.51, 1.04, 2.02);
/// Colour of sunlit shallows over sand.
const SHALLOW_WATER: vec3<f32> = vec3<f32>(0.05, 0.16, 0.15);

fn hash(p: vec3<u32>) -> f32 {
    var h = (p.x * 1664525u) ^ (p.y * 1013904223u) ^ (p.z * 2654435761u);
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h) / 4294967295.0;
}

/// Value noise in [-1, 1] on a lattice of `cell` metres, periodic over DETAIL_PERIOD.
fn noise(p: vec3<f32>, cell: f32) -> f32 {
    let period = DETAIL_PERIOD / cell;
    let q = p / cell;
    let i = floor(q);
    let f = q - i;
    let u = f * f * (3.0 - 2.0 * f);
    let wrap = i - floor(i / period) * period;
    let a = vec3<u32>(wrap);
    let b = vec3<u32>((wrap + 1.0) - floor((wrap + 1.0) / period) * period);
    let v000 = hash(a);
    let v100 = hash(vec3<u32>(b.x, a.y, a.z));
    let v010 = hash(vec3<u32>(a.x, b.y, a.z));
    let v110 = hash(vec3<u32>(b.x, b.y, a.z));
    let v001 = hash(vec3<u32>(a.x, a.y, b.z));
    let v101 = hash(vec3<u32>(b.x, a.y, b.z));
    let v011 = hash(vec3<u32>(a.x, b.y, b.z));
    let v111 = hash(b);
    let x0 = mix(mix(v000, v100, u.x), mix(v010, v110, u.x), u.y);
    let x1 = mix(mix(v001, v101, u.x), mix(v011, v111, u.x), u.y);
    return mix(x0, x1, u.z) * 2.0 - 1.0;
}

/// Octave weight that fades a pattern of `cell` metres out before it would alias.
fn octave_fade(cell: f32, distance: f32) -> f32 {
    return 1.0 - smoothstep(cell * 60.0, cell * 180.0, distance);
}

/// `noise(p, cell)` times `weight`, skipped where the weight is zero: most of the ground in
/// view is past where the fine octaves fade out, and each costs eight hashes.
fn weighted_noise(p: vec3<f32>, cell: f32, weight: f32) -> f32 {
    if weight <= 0.0 {
        return 0.0;
    }
    return noise(p, cell) * weight;
}

/// Albedo (linear) and perceptual roughness of a land-cover class, varied by noise `n`.
fn class_material(kind: u32, n: f32) -> vec4<f32> {
    switch kind {
        // Ocean and lake beds show only at the waterline: wet, dark sediment.
        case 0u, 1u: { return vec4<f32>(0.10, 0.09, 0.07, 0.5); }
        case 2u: { return vec4<f32>(vec3<f32>(0.78, 0.82, 0.88) * (1.0 + 0.05 * n), 0.45); }
        // Bare and sparsely vegetated ground: soil with tufts of dry grass.
        case 3u: { return vec4<f32>(mix(vec3<f32>(0.24, 0.20, 0.14), vec3<f32>(0.12, 0.13, 0.07), 0.5 + 0.5 * n), 0.9); }
        case 4u: { return vec4<f32>(vec3<f32>(0.56, 0.50, 0.38) * (1.0 + 0.08 * n), 0.95); }
        case 5u: { return vec4<f32>(vec3<f32>(0.17, 0.17, 0.11) * (1.0 + 0.25 * n), 0.9); }
        case 6u: { return vec4<f32>(mix(vec3<f32>(0.09, 0.15, 0.04), vec3<f32>(0.17, 0.17, 0.07), 0.5 + 0.5 * n), 0.9); }
        case 7u: { return vec4<f32>(mix(vec3<f32>(0.06, 0.09, 0.04), vec3<f32>(0.13, 0.12, 0.07), 0.5 + 0.5 * n), 0.9); }
        case 8u: { return vec4<f32>(mix(vec3<f32>(0.13, 0.14, 0.06), vec3<f32>(0.24, 0.20, 0.10), 0.5 + 0.5 * n), 0.9); }
        case 9u: { return vec4<f32>(mix(vec3<f32>(0.12, 0.16, 0.05), vec3<f32>(0.26, 0.22, 0.12), 0.5 + 0.5 * n), 0.9); }
        case 10u: { return vec4<f32>(mix(vec3<f32>(0.07, 0.10, 0.05), vec3<f32>(0.14, 0.13, 0.07), 0.5 + 0.5 * n), 0.6); }
        case 11u: { return vec4<f32>(vec3<f32>(0.035, 0.065, 0.03) * (1.0 + 0.3 * n), 0.85); }
        case 12u: { return vec4<f32>(vec3<f32>(0.03, 0.05, 0.03) * (1.0 + 0.3 * n), 0.9); }
        case 13u: { return vec4<f32>(vec3<f32>(0.035, 0.075, 0.025) * (1.0 + 0.3 * n), 0.9); }
        case 14u: { return vec4<f32>(vec3<f32>(0.05, 0.09, 0.035) * (1.0 + 0.3 * n), 0.9); }
        case 15u: { return vec4<f32>(vec3<f32>(0.04, 0.075, 0.03) * (1.0 + 0.3 * n), 0.9); }
        case 16u: { return vec4<f32>(vec3<f32>(0.20, 0.19, 0.18) * (1.0 + 0.35 * n), 0.85); }
        // Concrete and asphalt laid by launch sites.
        case 17u: { return vec4<f32>(vec3<f32>(0.42, 0.41, 0.38) * (1.0 + 0.06 * n), 0.85); }
        case 18u: { return vec4<f32>(vec3<f32>(0.055, 0.055, 0.06) * (1.0 + 0.10 * n), 0.9); }
        default: { return vec4<f32>(0.12, 0.15, 0.06, 0.9); }
    }
}

/// How bumpy a class is at small scales, metres.
fn class_relief(kind: u32) -> f32 {
    switch kind {
        case 0u, 1u: { return 0.02; }
        case 4u: { return 0.03; }
        case 17u, 18u: { return 0.004; }
        default: { return 0.12; }
    }
}

/// Bump mapping from screen-space derivatives of a height field (Mikkelsen 2010).
fn bump(normal: vec3<f32>, position: vec3<f32>, height: f32) -> vec3<f32> {
    let dpx = dpdx(position);
    let dpy = dpdy(position);
    let dhx = dpdx(height);
    let dhy = dpdy(height);
    let r1 = cross(dpy, normal);
    let r2 = cross(normal, dpx);
    let det = dot(dpx, r1);
    let gradient = sign(det) * (dhx * r1 + dhy * r2);
    let bumped = abs(det) * normal - gradient;
    if dot(bumped, bumped) < 1e-20 {
        return normal;
    }
    return normalize(bumped);
}

/// The sea: 16 wind waves from 80 m down to 0.6 m long, each this steep (peak slope), which
/// together give about the slope spread Cox and Munk measured for a 7 m/s wind.
const WAVE_COUNT: i32 = 16;
const LONGEST_WAVE: f32 = 80.0;
const WAVE_SHRINK: f32 = 0.72;
const WAVE_STEEPNESS: f32 = 0.06;
/// Where the wind blows from, body-fixed; the waves run within a cone about it.
const WIND: vec3<f32> = vec3<f32>(0.70, 0.20, 0.69);
/// GGX roughness of the ripples too small to model.
const RIPPLE_ALPHA: f32 = 0.03;

struct Sea {
    /// Slope of the resolved waves (body-fixed axes).
    slope: vec3<f32>,
    /// GGX α² of everything finer than a pixel.
    alpha2: f32,
};

/// The sea surface at `p` (body-fixed, m) with `up` the local vertical (body-fixed axes) and
/// `footprint` the size of a pixel. Each wave is a plane wave in body-fixed space with a
/// whole number of cycles per DETAIL_PERIOD along each axis, so it is continuous across
/// chunks, travelling at the deep-water speed for its length; slow noise bends the crests
/// so they never line up into a grid. Waves too short for the pixel fade out and widen the
/// reflections instead: from orbit the sun's glint spreads over a wide patch of sea.
fn sea(p: vec3<f32>, up: vec3<f32>, t: f32, footprint: f32) -> Sea {
    var out: Sea;
    out.slope = vec3<f32>(0.0);
    out.alpha2 = RIPPLE_ALPHA * RIPPLE_ALPHA;
    let wind = normalize(WIND);
    let across = normalize(cross(wind, vec3<f32>(0.0, 1.0, 0.0)));
    let over = cross(wind, across);
    let bend = 2.5 * noise(p, 256.0) + 1.2 * noise(p, 64.0);
    let cycle = 2.0 * PI / DETAIL_PERIOD;
    var wavelength = LONGEST_WAVE;
    for (var i = 0; i < WAVE_COUNT; i++) {
        let fi = f32(i);
        let around = 2.399963 * fi;
        let spread = 0.25 + 0.55 * fract(fi * 0.618034);
        let direction = normalize(wind + spread * (cos(around) * across + sin(around) * over));
        let k = round(direction * DETAIL_PERIOD / wavelength) * cycle;
        let along = k - up * dot(k, up);
        let resolved = 1.0 - smoothstep(0.15 * wavelength, 0.6 * wavelength, footprint);
        let omega = sqrt(9.81 * length(k));
        let phase = dot(k, p) - omega * t + 1.7 * fi + bend * (1.0 + 0.15 * fi);
        out.slope += along / max(length(k), 1e-6) * WAVE_STEEPNESS * cos(phase) * resolved;
        // GGX α² is about twice the slope variance.
        out.alpha2 += WAVE_STEEPNESS * WAVE_STEEPNESS * (1.0 - resolved);
        wavelength *= WAVE_SHRINK;
    }
    return out;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    let world_from_local = get_world_from_local(in.instance_index);
    let local_from_world = get_local_from_world(in.instance_index);
    let planet_center = (world_from_local * vec4<f32>(params.planet.xyz, 1.0)).xyz;
    let radius = params.planet.w;
    let position = in.world_position.xyz;
    let camera = view.world_position;
    let distance_to_camera = distance(position, camera);

    // Local frame: up from the smooth sphere, east and north from the body's pole.
    let up = normalize(in.world_normal);
    let pole = normalize((world_from_local * vec4<f32>(0.0, 1.0, 0.0, 0.0)).xyz);
    var east = cross(pole, up);
    if dot(east, east) < 1e-10 {
        east = normalize(cross(vec3<f32>(1.0, 0.0, 0.0), up));
    }
    east = normalize(east);
    let north = cross(up, east);

    // Data textures. Samples are grid-registered: sample k sits at uv = k / CELLS.
    let uv = clamp(in.uv, vec2<f32>(0.0), vec2<f32>(1.0));
    let texcoord = (uv * CELLS + 0.5) / (CELLS + 1.0);
    let color = textureSample(color_texture, color_sampler, texcoord);
    let surface = textureSample(surface_texture, surface_sampler, texcoord);

    // Procedural detail in body-fixed coordinates.
    let local = (local_from_world * vec4<f32>(position, 1.0)).xyz;
    let p = params.detail_origin.xyz + local;
    let d = distance_to_camera;
    let near = 1.0 - smoothstep(300.0, 6000.0, d);
    // Size of a pixel on the ground, m.
    let footprint = max(length(dpdx(position)), length(dpdy(position)));
    let o1 = weighted_noise(p, 32.0, octave_fade(32.0, d));
    let o2 = weighted_noise(p, 8.0, octave_fade(8.0, d));
    let o3 = weighted_noise(p, 2.0, octave_fade(2.0, d));
    let o4 = weighted_noise(p, 0.5, octave_fade(0.5, d));
    // Large-scale variation for colour, small-scale for relief.
    let n_color = clamp(0.55 * o1 + 0.3 * o2 + 0.15 * o3, -1.0, 1.0);
    let n_relief = 0.15 * o1 + 0.3 * o2 + 0.35 * o3 + 0.2 * o4;

    // Land-cover classes, blended bilinearly between the four nearest samples.
    let st = uv * CELLS;
    let cell = min(floor(st), vec2<f32>(CELLS - 1.0));
    // Jitter the blend position so class edges are ragged rather than straight.
    let jitter = 0.45
        * vec2<f32>((o3 + 0.5 * o4) * near, weighted_noise(p + 31.0, 2.0, near) + 0.5 * o4 * near);
    let f = clamp(st - cell + jitter, vec2<f32>(0.0), vec2<f32>(1.0));
    let base = vec2<i32>(cell);
    let classes = vec4<u32>(
        textureLoad(cover_texture, base, 0).r,
        textureLoad(cover_texture, base + vec2<i32>(1, 0), 0).r,
        textureLoad(cover_texture, base + vec2<i32>(0, 1), 0).r,
        textureLoad(cover_texture, base + vec2<i32>(1, 1), 0).r,
    );
    let weights = vec4<f32>((1.0 - f.x) * (1.0 - f.y), f.x * (1.0 - f.y), (1.0 - f.x) * f.y, f.x * f.y);
    var palette = vec4<f32>(0.0);
    var relief = 0.0;
    for (var k = 0; k < 4; k++) {
        palette += class_material(classes[k], n_color) * weights[k];
        relief += class_relief(classes[k]) * weights[k];
    }
    // The palette is a touch darker than the imagery it replaces; match it.
    palette = vec4<f32>(palette.rgb * 1.25, palette.a);

    // Imagery (about 2 km per sample) from afar; class colours where the land cover is
    // detailed (around the Cape it goes to 10 m: it keeps up with the chunk, or is finer
    // than about 100 m anyway) and the pixels are small enough to show it. The blend follows
    // the size of a pixel on the ground, not the chunk, so it never steps at chunk edges,
    // and coarse cover stretched over fine chunks (the 2 km global layer) never shows as
    // blocks.
    let imagery = color.rgb;
    let cover_keeps_up = max(
        1.0 - smoothstep(0.5, 2.5, params.levels.x - params.levels.z),
        smoothstep(9.0, 10.5, params.levels.z),
    );
    let classes_show = (1.0 - smoothstep(8.0, 80.0, footprint)) * cover_keeps_up;
    var land = mix(imagery, palette.rgb, classes_show);
    land *= 1.0 + 0.25 * n_color * near * (1.0 - classes_show) + 0.15 * (o3 + 0.5 * o4) * near;
    var roughness = mix(0.9, palette.a, max(classes_show, near));

    // Water: crisp edge with a noisy shoreline; the imagery's colour, turning turquoise
    // over shallows.
    let water = smoothstep(0.42, 0.58, surface.w + 0.12 * o2 * near);
    let depth = max(-surface.z, 0.0);
    // Shallows are told by depth below sea level, so lakes up in the hills don't count.
    let shallows = exp(-depth / 8.0) * (1.0 - smoothstep(2.0, 10.0, surface.z));
    let water_albedo = mix(imagery * 0.85, SHALLOW_WATER, 0.6 * shallows);

    let albedo = mix(land, water_albedo, water);

    // Normal: baked terrain normal, then fine relief on land and waves on water.
    let tangent = surface.xy;
    let normal_up = sqrt(max(1.0 - dot(tangent, tangent), 0.0));
    var normal = normalize(east * tangent.x + north * tangent.y + up * normal_up);
    normal = bump(normal, position, n_relief * relief * (1.0 - water));
    let up_fixed = normalize((local_from_world * vec4<f32>(up, 0.0)).xyz);
    let waves = sea(p, up_fixed, globals.time, footprint);
    let slope = (world_from_local * vec4<f32>(waves.slope, 0.0)).xyz * water;
    normal = normalize(normal - (slope - dot(slope, normal) * normal));
    roughness = mix(roughness, sqrt(sqrt(waves.alpha2)), water);

    pbr_input.N = normal;
    pbr_input.material.base_color = vec4<f32>(albedo, 1.0);
    pbr_input.material.perceptual_roughness = roughness;
    pbr_input.material.reflectance = vec3<f32>(mix(0.5, 0.35, water));

    let sun = lights.directional_lights[0].direction_to_light;
    let earthlike = params.levels.w > 0.5;
    let from_center = position - planet_center;
    var sun_color = lights.directional_lights[0].color.rgb;
    var sun_here = vec3<f32>(1.0);
    if earthlike {
        // The light was dimmed for the air at the vessel; use this point's instead.
        let at_vessel = light_transmittance(sun_color);
        sun_color /= at_vessel;
        sun_here = sun_transmittance(from_center, sun, radius);
        let ratio = sun_here / at_vessel;
        let level = luminance(ratio);
        pbr_input.directional_shadow_factor = level;
        let tint = clamp(ratio / max(level, 1e-6), vec3<f32>(0.0), vec3<f32>(4.0));
        pbr_input.material.base_color = vec4<f32>(albedo * tint, 1.0);
    }

    // City lights on the night side, seen from a distance.
    let sun_up = dot(normalize(from_center), sun);
    let night = smoothstep(0.05, -0.12, sun_up);
    let lights_level = color.a * color.a * (1.0 - water);
    let lights_glow = vec3<f32>(1.0, 0.7, 0.4) * lights_level * night * smoothstep(3000.0, 30000.0, d);
    pbr_input.material.emissive = vec4<f32>(lights_glow * 250.0, 1.0);

    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);

    // Light from the sky dome over this point (the camera's environment map is the sky at
    // the vessel, see `crate::sky_light`): it fades through twilight and turns from grey
    // to blue as the sun climbs; slopes facing away from the sky get less.
    if earthlike {
        let level = 1.0 - exp(-max(sun_up + 0.06, 0.0) / 0.2);
        let tint = mix(SKY_LOW, SKY_HIGH, 1.0 - exp(-max(sun_up, 0.0) / 0.17));
        let facing = 0.5 + 0.5 * dot(normal, up);
        // Occluded like the rest of the indirect light (ambient occlusion, if on).
        let irradiance = sun_color * SKYLIGHT * level * tint * facing * pbr_input.diffuse_occlusion;
        out.color = vec4<f32>(out.color.rgb + albedo / PI * irradiance * view.exposure, out.color.a);
    }

    // The sky reflected in the water: the same scattering as the sky itself, along the
    // reflected ray (kept above the horizon), weighted by Fresnel. The sun's glint comes
    // from the lighting above, spread by the wave roughness.
    if earthlike && water > 0.0 {
        let view_dir = normalize(camera - position);
        var reflected = reflect(-view_dir, normal);
        reflected = normalize(reflected + up * max(0.02 - dot(reflected, up), 0.0));
        let origin = from_center + up * 2.0;
        let exit = sphere_hits(origin, reflected, radius + THICKNESS).y;
        let sky = scatter(
            origin, reflected, 0.0, max(exit, 0.0), sun, sun_color, radius, i32(params.quality.x),
        );
        let fresnel = 0.02 + 0.98 * pow(1.0 - max(dot(normal, view_dir), 0.0), 5.0);
        out.color = vec4<f32>(
            out.color.rgb + sky.inscatter * fresnel * water * pbr_input.specular_occlusion
                * view.exposure,
            out.color.a,
        );
    }

    // Aerial perspective while the camera is inside the sky mesh (from outside it, the
    // mesh's near side does it), over the part of the ray inside the atmosphere.
    if earthlike {
        let eye = camera - planet_center;
        let top = radius + THICKNESS;
        if length(eye) < top * SKY_MESH_SCALE {
            let ray = (position - camera) / max(distance_to_camera, 1e-3);
            let entry = max(sphere_hits(eye, ray, top).x, 0.0);
            let s = scatter(
                eye, ray, entry, distance_to_camera, sun, sun_color, radius, i32(params.quality.x),
            );
            out.color = vec4<f32>(out.color.rgb * s.transmittance + s.inscatter * view.exposure, out.color.a);
        }
    }

    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
