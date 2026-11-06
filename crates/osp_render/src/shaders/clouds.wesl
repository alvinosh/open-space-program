// The clouds as a sheet over the planet, the way EnvironmentalVisualEnhancements draws them (MIT, see
// assets/core/clouds/LICENSE.txt): its Cloud shader, on a sphere found by the ray instead of a mesh.
//
// How much cloud is at a point is a cube map of cloud imagery, shaken by an animated noise so that its
// edges slowly work, times a tiling detail texture that fades out with distance. The sheet is
// transparent right around the camera, where the puffs of `cloud_puffs.wesl` take over, and fades at
// the limb, where it would otherwise show as a hard-edged shell. A second, thin sheet up high is the
// same map turned another way.
//
// The result is what must be added over the scene, air in front of the cloud included, and how much
// of what is behind still shows, so the composite is a plain blend.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import bevy_render::view::View;
import osp_render::shaders::atmosphere::{
    air, eclipse_at, weather, lut_sampler, scatter, sky_irradiance, sphere_hits,
    sun_transmittance, PI,
};
import osp_render::shaders::clouds_types::{CloudDraw, view_ray};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> draw: CloudDraw;
@if(MULTISAMPLED)
@group(0) @binding(2) var depth_texture: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(0) @binding(2) var depth_texture: texture_depth_2d;
@group(0) @binding(3) var cloud_map: texture_cube<f32>;
@group(0) @binding(4) var detail_texture: texture_2d<f32>;
@group(0) @binding(5) var tile_sampler: sampler;
@group(0) @binding(6) var noise_texture: texture_2d<f32>;

/// How often the detail tiles, m, big and fine (`DETAIL_TILES` in `crate::clouds`), and the size of a
/// texel of it. Its big copy is the mod's own; the fine one is for a planet as big as Earth, where a
/// texel of the map is kilometres across.
const DETAIL_TILE: f32 = 40000.0;
const FINE_TILE: f32 = 5300.0;
const DETAIL_SIZE: f32 = 1024.0;
/// Past about this distance, m, a sheet has no detail. Its `_DetailDist` of 2e-6.
const DETAIL_GONE: f32 = 250000.0;
/// The UV noise's period as a share of the planet's radius, and how far it shakes the map, in units of
/// a face's half-width: its `_UVNoiseScale` of 0.01 and `_UVNoiseStrength` of 0.002 of a face.
const NOISE_PERIOD: f32 = 0.02;
const SHAKE: f32 = 0.004;
/// Its `_DistFadeVert`: the sheet comes in from the camera out to this many metres.
const NEAR_FADE: f32 = 0.000085;
/// Its `_FalloffScale` and `_FalloffPow`: how the sheet fades seen edge-on.
const RIM_SCALE: f32 = 3.0;
const RIM_POWER: f32 = 2.0;
/// Its `_RimDist` and how far above the sheet it starts: from near the sheet the fade is by distance,
/// from high above by the rim.
const RIM_DISTANCE: f32 = 0.0001;
const RIM_ABOVE: f32 = 6000.0;
/// As `osp_sim::clouds` has it.
const MAP_LIFT: f32 = 0.75;
/// What a thick cloud reflects of the light on it.
const THICK_ALBEDO: f32 = 0.85;
/// How much of a cloud faces a sun on the horizon, as a share of what faces one overhead: a real cloud
/// has sides, even if this one doesn't.
const SIDES_LIT: f32 = 0.45;
/// How much darker the underside of a thick cloud is.
const UNDERSIDE: f32 = 0.5;
/// How far to look towards the sun on the map, in texels, and how much a difference there shades: the
/// sun's side of a cloud is brighter, as its normal map would make it.
const RELIEF_TEXELS: f32 = 2.5;
const RELIEF: f32 = 0.9;
/// How much of the thin high layer's map shows.
const HIGH_LAYER: f32 = 0.45;
/// The high layer's map is this one turned this way.
const HIGH_TURN: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.3536, 0.6124, 0.7071),
    vec3<f32>(-0.8660, 0.5, 0.0),
    vec3<f32>(-0.3536, -0.6124, 0.7071),
);
/// Steps for the air in front of the cloud.
const AIR_STEPS: i32 = 10;

fn saturate(x: f32) -> f32 {
    return clamp(x, 0.0, 1.0);
}

/// Where a point falls on the tiling textures, on charts wrapped around the planet: distance along
/// the surface, on the faces of a cube, overlapping where they meet.
struct Charts {
    /// How much of each chart (with x, y or z as its axis) there is here; they add up to 1.
    weights: vec3<f32>,
    x: vec2<f32>,
    y: vec2<f32>,
    z: vec2<f32>,
};

/// How wide the overlap between charts is, as a difference of direction components.
const CHART_OVERLAP: f32 = 0.03;

fn charts_at(d: vec3<f32>, scale: f32) -> Charts {
    var out: Charts;
    let a = abs(d);
    let w = vec3<f32>(
        clamp((a.x - max(a.y, a.z)) / CHART_OVERLAP + 0.5, 0.0, 1.0),
        clamp((a.y - max(a.x, a.z)) / CHART_OVERLAP + 0.5, 0.0, 1.0),
        clamp((a.z - max(a.x, a.y)) / CHART_OVERLAP + 0.5, 0.0, 1.0),
    );
    out.weights = w / max(w.x + w.y + w.z, 1e-5);
    if w.x > 0.0 {
        out.x = vec2<f32>(atan2(d.y, a.x), atan2(d.z, a.x)) * scale;
    }
    if w.y > 0.0 {
        out.y = vec2<f32>(atan2(d.x, a.y), atan2(d.z, a.y)) * scale;
    }
    if w.z > 0.0 {
        out.z = vec2<f32>(atan2(d.x, a.z), atan2(d.y, a.z)) * scale;
    }
    return out;
}

fn read_detail(charts: Charts, tile: f32, wind: vec2<f32>, lod: f32) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    if charts.weights.x > 0.0 {
        sum += textureSampleLevel(detail_texture, tile_sampler, charts.x / tile + wind, lod) * charts.weights.x;
    }
    if charts.weights.y > 0.0 {
        sum += textureSampleLevel(detail_texture, tile_sampler, charts.y / tile + wind, lod) * charts.weights.y;
    }
    if charts.weights.z > 0.0 {
        sum += textureSampleLevel(detail_texture, tile_sampler, charts.z / tile + wind, lod) * charts.weights.z;
    }
    return sum;
}

fn read_noise(charts: Charts, tile: f32, wind: vec2<f32>) -> vec2<f32> {
    var sum = vec2<f32>(0.0);
    if charts.weights.x > 0.0 {
        sum += textureSampleLevel(noise_texture, tile_sampler, charts.x / tile + wind, 0.0).rg * charts.weights.x;
    }
    if charts.weights.y > 0.0 {
        sum += textureSampleLevel(noise_texture, tile_sampler, charts.y / tile + wind, 0.0).rg * charts.weights.y;
    }
    if charts.weights.z > 0.0 {
        sum += textureSampleLevel(noise_texture, tile_sampler, charts.z / tile + wind, 0.0).rg * charts.weights.z;
    }
    return sum;
}

fn tile_level(across: f32, tile: f32) -> f32 {
    return max(log2(max(across, 1.0) / (tile / DETAIL_SIZE)), 0.0);
}

/// How much cloud the map and its copy come to in a direction (`osp_sim::clouds::stacked`).
fn read_map(dir: vec3<f32>, lod: f32) -> f32 {
    let first = textureSampleLevel(cloud_map, tile_sampler, dir, lod).r;
    let second = textureSampleLevel(cloud_map, tile_sampler, draw.copy_from_map * dir, lod).r;
    return 1.0 - (1.0 - pow(first, MAP_LIFT)) * (1.0 - pow(second, MAP_LIFT));
}

/// A sheet at a point `t` along the ray: rgb its light, a how much of what is behind it hides.
fn sheet(p: vec3<f32>, t: f32, ray: vec3<f32>, across: f32, turn: mat3x3<f32>, wobble: vec2<f32>, strength: f32) -> vec4<f32> {
    let eye = draw.eye.xyz;
    let sun = draw.sun.xyz;
    let radius = length(p);
    let normal = p / radius;
    let up = normalize(draw.local_from_render * normal);
    let charts = charts_at(up, radius);

    // The map, shaken.
    let shake = read_noise(charts, NOISE_PERIOD * radius, wobble) - 0.5;
    let side = normalize(cross(up, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(up.y) > 0.9)));
    let shaken = up + (side * shake.x + cross(up, side) * shake.y) * SHAKE;
    let dir = turn * shaken;
    let lod = max(log2(max(across, 1.0) / draw.shell.z), 0.0);
    var main = read_map(dir, lod);

    // The sun's side of a cloud: towards the sun the map is thinner.
    let towards = sun - normal * dot(sun, normal);
    let along = turn * (draw.local_from_render * (towards / max(length(towards), 1e-4)));
    let reach = max(across, draw.shell.z) * RELIEF_TEXELS / radius;
    let next = read_map(dir + along * reach, lod);
    let relief = clamp(1.0 + (main - next) * RELIEF, 0.6, 1.3);

    // A body with no map of its own shows the shared one where the weather says, and all of it where
    // there is more cloud than not.
    if draw.shell.w < 0.5 {
        let cover = textureSampleLevel(weather, lut_sampler, up, 1.0).r;
        main = saturate(cover * main + max(cover - 0.5, 0.0) * 2.0);
    }
    if main <= 0.002 {
        return vec4<f32>(0.0);
    }

    // The detail eats into the map, and mottles it, less with distance.
    let big = read_detail(charts, DETAIL_TILE, draw.detail_wind.xy, tile_level(across, DETAIL_TILE));
    let fine = read_detail(charts, FINE_TILE, draw.detail_wind.zw, tile_level(across, FINE_TILE));
    let far = saturate(t / DETAIL_GONE);
    let detail = mix(big * mix(fine, vec4<f32>(1.0), 0.35), vec4<f32>(1.0), far);

    // Its fades: from the camera out, and at the limb, with a change from one to the other in between.
    let near = 1.0 - saturate(t) * saturate(1.0 - NEAR_FADE * t);
    let edge_on = saturate(abs(dot(ray, normal)));
    let rim = saturate(pow(RIM_SCALE * edge_on, RIM_POWER));
    let high = saturate(RIM_DISTANCE * (length(eye) - radius - RIM_ABOVE));
    let alpha = saturate(main * detail.a * mix(near, rim, high) * strength);
    if alpha <= 0.001 {
        return vec4<f32>(0.0);
    }

    // Lit from above, whichever side it's seen from.
    let height_of_sun = max(dot(normal, sun), 0.0);
    let lit_area = mix(SIDES_LIT, 1.0, height_of_sun);
    let sun_color = vec3<f32>(draw.sun.w);
    // The eye takes daylight as white: the air's yellowing of the sun only shows when it is low.
    let reached = sun_transmittance(p, sun);
    let gray = vec3<f32>(dot(reached, vec3<f32>(0.2126, 0.7152, 0.0722)));
    let white = mix(gray, reached, 1.0 - smoothstep(0.08, 0.5, height_of_sun));
    var light = draw.albedo.rgb * detail.rgb
        * (white * eclipse_at(p) * sun_color * THICK_ALBEDO * lit_area / PI * relief
            + sky_irradiance(p, sun) * sun_color / PI);
    if length(eye) < radius {
        light *= mix(1.0, UNDERSIDE, main);
    }
    return vec4<f32>(light, alpha);
}

/// Where a ray meets a sphere from the camera: the first place ahead of it, or below 0 for nowhere.
fn hit_sphere(eye: vec3<f32>, ray: vec3<f32>, radius: f32) -> f32 {
    let hits = sphere_hits(eye, ray, radius);
    if hits.x > hits.y || hits.y <= 0.0 {
        return -1.0;
    }
    return select(hits.y, hits.x, hits.x > 0.0);
}

fn shade(in: FullscreenVertexOutput, depth: f32) -> vec4<f32> {
    let scaled = view_ray(in.uv, view.clip_from_view, view.world_from_view);
    let per_depth = length(scaled);
    let ray = scaled / per_depth;
    let eye = draw.eye.xyz;
    let sun = draw.sun.xyz;
    let radius = air.shape.x;

    // What the ray hits: drawn scene, or the planet if nothing has been drawn there.
    var scene = 1e12;
    if depth > 0.0 {
        scene = view.clip_from_view[3][2] / depth * per_depth;
    } else {
        let ground = sphere_hits(eye, ray, radius);
        if ground.x <= ground.y && ground.x > 0.0 {
            scene = ground.x;
        }
    }

    // A half-resolution pixel's size per metre of distance.
    let per_metre = 4.0 / (view.clip_from_view[1][1] * view.viewport.w);

    var near_t = hit_sphere(eye, ray, draw.shell.x);
    var far_t = -1.0;
    if draw.shell.y > 0.0 {
        far_t = hit_sphere(eye, ray, draw.shell.y);
    }
    var light = vec3<f32>(0.0);
    var through = 1.0;
    var weight = 0.0;
    var mean_t = 0.0;
    let main_first = far_t <= 0.0 || (near_t > 0.0 && near_t <= far_t);
    for (var k = 0; k < 2; k++) {
        let is_main = (k == 0) == main_first;
        let t = select(far_t, near_t, is_main);
        if t <= 0.0 || t >= scene {
            continue;
        }
        let p = eye + ray * t;
        let facing = max(abs(dot(ray, p / length(p))), 0.03);
        let across = t * per_metre / sqrt(facing);
        var layer: vec4<f32>;
        if is_main {
            layer = sheet(p, t, ray, across, draw.map_from_local, draw.wobble.xy, 1.0);
        } else {
            layer = sheet(p, t, ray, across, HIGH_TURN, draw.wobble.zw, HIGH_LAYER * draw.albedo.w);
        }
        let hidden = through * layer.a;
        light += layer.rgb * hidden;
        through *= 1.0 - layer.a;
        weight += hidden;
        mean_t += t * hidden;
    }
    if weight <= 0.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let distance = mean_t / weight;
    let entry = sphere_hits(eye, ray, air.shape.y);
    let front_air = scatter(eye, ray, max(entry.x, 0.0), distance, sun, vec3<f32>(draw.sun.w), AIR_STEPS);
    let total = front_air.transmittance * light + (1.0 - through) * front_air.inscatter;
    return vec4<f32>(total * view.exposure, through);
}

@if(MULTISAMPLED)
@fragment
fn main(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.position.xy) * 2;
    return shade(in, textureLoad(depth_texture, px, 0));
}

@if(!MULTISAMPLED)
@fragment
fn main(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // The full-resolution pixel the composite also compares against.
    let px = vec2<i32>(in.position.xy) * 2;
    return shade(in, textureLoad(depth_texture, px, 0));
}
