// The clouds right around the camera, the way EnvironmentalVisualEnhancements draws them (MIT, see
// assets/core/clouds/LICENSE.txt): its GeometryCloudVolumeParticle shader, on a mesh of quads instead
// of a geometry shader. A grid of big soft sprites at the height of the sheet, each as much as the
// map says is there, facing the camera, and fading in with distance from it as the sheet fades out.
//
// What makes them read as volumes is the sprite: three silhouettes in its three channels, each laid
// across one of the three planes of a puff turned at random, and mixed by how the camera looks at it,
// so that a puff changes shape as you go by. Each is lit like a ball, with light through its edges
// when you look towards the sun.

import bevy_pbr::render::{
    mesh_view_bindings::view,
};
import bevy_pbr::prepass::utils::prepass_depth;

struct Params {
    /// xyz: the middle of the grid, render axes; w: distance between puffs, m.
    origin: vec4<f32>,
    /// The grid's axes, and straight up at its middle; up's w: radius of the sheet, m.
    grid_x: vec4<f32>,
    grid_y: vec4<f32>,
    up: vec4<f32>,
    /// xy: which puff of the planet's lattice is in the middle; z: which face of the cube it is on; w:
    /// how big a puff is, m.
    cell: vec4<f32>,
    /// x: how many times bigger the biggest puff is; y: how far a puff strays sideways, m; z: up and
    /// down; w: size of the detail's tile, in the grid's cells.
    spread: vec4<f32>,
    /// xyz: direction to the sun; w: 1 if the map is the body's own.
    sun: vec4<f32>,
    /// The sunlight and the sky's light at the sheet, lux.
    sunlight: vec4<f32>,
    skylight: vec4<f32>,
    /// x: how high the sun is at the grid, as the sine of its angle.
    sun_height: vec4<f32>,
    /// x: its `_DistFade`; y: its `_DistFadeVert`; z: its `_InvFade`; w: 1 if drawn at all.
    fade: vec4<f32>,
    /// The map's axes from render axes, and its second copy's from those.
    map_from_render: mat3x3<f32>,
    copy_from_map: mat3x3<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: Params;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var cloud_map: texture_cube<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var cloud_map_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var detail_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var detail_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(5) var puff_texture: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(6) var puff_sampler: sampler;

const TAU: f32 = 6.2831853;
/// Its `_MinScatter` and `_Opacity`.
const SCATTER_LEAST: f32 = 1.05;
const SCATTER_OPACITY: f32 = 1.05;
/// What a thick cloud reflects of the light on it, and how much is the light through its edges.
const THICK_ALBEDO: f32 = 0.85;
const PI: f32 = 3.14159265;
/// How bright the side of a puff away from the sun is, as a share of the side facing it.
const AROUND: f32 = 0.62;
/// As `clouds.wesl` has it.
const SIDES_LIT: f32 = 0.45;
/// Between these amounts of the sprite, a puff goes from nothing to solid.
const PUFF_EDGE: vec2<f32> = vec2<f32>(0.04, 0.8);
/// Between these amounts of cloud on the map, a place goes from having no puffs to full ones: they
/// overlap many times over, so the thin parts of the map would otherwise close up the sky.
const PUFF_COVER: vec2<f32> = vec2<f32>(0.14, 0.62);
/// As `osp_sim::clouds` has it.
const MAP_LIFT: f32 = 0.75;
/// Puffs fade out this close to the camera: its 0.004 per metre.
const CAMERA_FADE: f32 = 0.004;

struct Vertex {
    /// xy: which puff, from the middle of the grid.
    @location(0) cell: vec3<f32>,
    /// Where on the quad.
    @location(1) uv: vec2<f32>,
};

struct Output {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    /// How much of the sprite's three channels shows at each of three planes.
    @location(1) plane_zy: vec2<f32>,
    @location(2) plane_xz: vec2<f32>,
    @location(3) plane_xy: vec2<f32>,
    @location(4) along: vec3<f32>,
    /// x: how much cloud, y: how far from the camera along the view, z: its fade close to the camera;
    /// w: how mottled.
    @location(5) amount: vec4<f32>,
    @location(6) light_on_quad: vec3<f32>,
    @location(7) view_on_quad: vec3<f32>,
};

fn hash(p: vec3<f32>) -> vec3<f32> {
    var q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    q += dot(q, q.yxz + 33.33);
    return fract((q.xxy + q.yzz) * q.zyx);
}

fn sign_of(x: f32) -> f32 {
    return select(-1.0, 1.0, x >= 0.0);
}

/// Arvo's random rotation (1991), from three numbers in 0..1.
fn random_rotation(x: vec3<f32>) -> mat3x3<f32> {
    let theta = x.x * TAU;
    let phi = x.y * TAU;
    let z = x.z * 2.0;
    let r = sqrt(z);
    let v = vec3<f32>(sin(phi) * r, cos(phi) * r, sqrt(2.0 - z));
    let st = sin(theta);
    let ct = cos(theta);
    let s = vec2<f32>(v.x * ct - v.y * st, v.x * st + v.y * ct);
    // Columns; `V S - R`, as written by rows in the paper.
    return mat3x3<f32>(
        vec3<f32>(v.x * s.x - ct, v.x * s.y - st, v.x * v.z),
        vec3<f32>(v.y * s.x + st, v.y * s.y - ct, v.y * v.z),
        vec3<f32>(v.z * s.x, v.z * s.y, 1.0 - z),
    );
}

@vertex
fn vertex(in: Vertex) -> Output {
    var out: Output;
    out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    out.uv = in.uv;
    if params.fade.w <= 0.0 {
        return out;
    }
    let spacing = params.origin.w;
    let radius = params.up.w;
    let lattice = params.cell.xy + in.cell.xy;
    let random = hash(vec3<f32>(lattice, params.cell.z * 17.0 + 3.0));

    // Where it is, on the curve of the planet.
    let across = vec2<f32>(in.cell.x + in.cell.y * 0.5, in.cell.y * 0.8660254) * spacing;
    let drop = dot(across, across) / (2.0 * radius);
    var origin = params.origin.xyz + params.grid_x.xyz * across.x + params.grid_y.xyz * across.y
        - params.up.xyz * drop;
    origin += (2.0 * random.yzx - 1.0) * vec3<f32>(params.spread.y, params.spread.z, params.spread.y);
    let from_eye = origin - view.world_position;
    let distance = length(from_eye);

    // How much cloud it is: from the map, a little eaten by the detail.
    let dir = params.up.xyz + (params.grid_x.xyz * across.x + params.grid_y.xyz * across.y) / radius;
    let on_map = params.map_from_render * dir;
    let first = textureSampleLevel(cloud_map, cloud_map_sampler, on_map, 0.0).r;
    let second = textureSampleLevel(cloud_map, cloud_map_sampler, params.copy_from_map * on_map, 0.0).r;
    let main = 1.0 - (1.0 - pow(first, MAP_LIFT)) * (1.0 - pow(second, MAP_LIFT));
    let tiled = (params.cell.xy + in.cell.xy) / params.spread.w;
    let detail = textureSampleLevel(detail_texture, detail_sampler, tiled, 0.0);
    let far_fade = clamp(params.fade.x * distance, 0.0, 1.0) * clamp(1.0 - params.fade.y * distance, 0.0, 1.0);
    let amount = smoothstep(PUFF_COVER.x, PUFF_COVER.y, main) * detail.a * far_fade;
    if amount < 1.0 / 255.0 {
        return out;
    }

    // The quad, facing the camera.
    let size = params.cell.w * (1.0 + random.x * (params.spread.x - 1.0));
    let in_view = view.view_from_world * vec4<f32>(origin, 1.0);
    let corner = in_view.xyz + vec3<f32>(in.uv - 0.5, 0.0) * size;
    out.clip = view.clip_from_view * vec4<f32>(corner, 1.0);

    // Where each plane of the turned puff falls on the quad.
    let view_turn = mat3x3<f32>(view.view_from_world[0].xyz, view.view_from_world[1].xyz, view.view_from_world[2].xyz);
    let turned = view_turn * random_rotation(random);
    // Its third row: how the view lies in the puff's axes.
    let v = vec3<f32>(turned[0].z, turned[1].z, turned[2].z);
    let t = 2.0 * in.uv - 1.0;
    var zy = vec3<f32>(0.0, t.y, t.x);
    var xz = vec3<f32>(t.x, 0.0, t.y);
    var xy = vec3<f32>(t.x, t.y, 0.0);
    zy.z *= sign_of(-v.x);
    xz.x *= sign_of(-v.y);
    xy.x *= sign_of(v.z);
    zy.x += sign_of(-v.x) * sign_of(zy.z) * v.z;
    xz.y += sign_of(-v.y) * sign_of(xz.x) * v.x;
    xy.z += sign_of(-v.z) * sign_of(xy.x) * v.x;
    zy.x += sign_of(-v.x) * sign_of(zy.y) * v.y;
    xz.y += sign_of(-v.y) * sign_of(xz.z) * v.z;
    xy.z += sign_of(-v.z) * sign_of(xy.y) * v.y;
    out.plane_zy = vec2<f32>(0.5) + 0.6 * (turned * zy).xy;
    out.plane_xz = vec2<f32>(0.5) + 0.6 * (turned * xz).xy;
    out.plane_xy = vec2<f32>(0.5) + 0.6 * (turned * xy).xy;
    out.along = abs(v);

    // The light and the view in the quad's axes, for lighting it as a ball.
    let looking = from_eye / max(distance, 1e-3);
    let normal = -looking;
    let tangent = view.world_from_view[0].xyz;
    let binormal = -cross(normal, tangent);
    let on_quad = transpose(mat3x3<f32>(tangent, binormal, normal));
    out.light_on_quad = on_quad * params.sun.xyz;
    out.view_on_quad = on_quad * looking;
    out.amount = vec4<f32>(amount, -in_view.z, smoothstep(0.0, 1.0, -in_view.z * CAMERA_FADE), 0.6 + 0.4 * detail.r);
    return out;
}

@fragment
fn fragment(in: Output, @builtin(sample_index) sample_index: u32) -> @location(0) vec4<f32> {
    let sprite = vec3<f32>(
        textureSample(puff_texture, puff_sampler, in.plane_zy).r,
        textureSample(puff_texture, puff_sampler, in.plane_xz).g,
        textureSample(puff_texture, puff_sampler, in.plane_xy).b,
    );
    let shape = dot(sprite, in.along) / max(in.along.x + in.along.y + in.along.z, 1e-4);

    // As a ball, facing the camera.
    let flat_part = 2.0 * in.uv - 1.0;
    let normal = vec3<f32>(flat_part, sqrt(1.0 - clamp(dot(flat_part, flat_part), 0.0, 1.0)));
    let to_sun = normalize(in.light_on_quad);
    let looking = normalize(in.view_on_quad);
    let lit = max(dot(normal, to_sun), 0.0);
    let towards_sun = 0.5 + 0.5 * dot(looking, to_sun);
    let scatter = clamp((SCATTER_LEAST - SCATTER_OPACITY * shape) * (1.0 - dot(normal, looking)) * towards_sun, 0.0, 1.0);
    // Light scattered many times inside comes out on every side: a ball's dark side isn't dark.
    let around = AROUND + (1.0 - AROUND) * lit;
    // The eye takes daylight as white: the air's yellowing of the sun only shows when it is low.
    let gray = vec3<f32>(dot(params.sunlight.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)));
    let sunlight = mix(gray, params.sunlight.rgb, 1.0 - smoothstep(0.08, 0.5, params.sun_height.x));
    let light = (sunlight * THICK_ALBEDO / PI * (in.amount.w * around * max(params.sun_height.x, SIDES_LIT) + scatter * 0.5)
        + params.skylight.rgb / PI * in.amount.w) * view.exposure;

    // Soft where it passes through the scene, and close to the camera.
    var soft = 1.0;
    let depth = prepass_depth(in.clip, sample_index);
    if depth > 0.0 {
        let scene = view.clip_from_view[3][2] / depth;
        soft = clamp(params.fade.z * (scene - in.amount.y), 0.0, 1.0);
    }
    // Harder than its sprite by itself, for puffs with edges, not a veil.
    let edged = smoothstep(PUFF_EDGE.x, PUFF_EDGE.y, shape);
    return vec4<f32>(light, in.amount.x * edged * soft * in.amount.z);
}
