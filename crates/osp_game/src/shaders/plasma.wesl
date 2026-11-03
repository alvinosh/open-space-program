// The air around a vessel coming in too fast for it: a bow shock standing off the hull, gas glowing
// between the two, and a wake of streaks and sparks torn off the sides. A ray march through a box
// around the vessel with its z axis along the flow, reading the hull as seen along the flow from a
// small map (`osp_sim::plasma`): how far it reaches upstream and downstream at each point across
// the flow, blurred more the further behind it the air is. After Lindner's breakdown of entry
// effects, but on the vessel's shape and in volume, so it holds up from every side.
//
// Only light is added: gas this hot is nearly transparent to its own glow.

import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_functions::get_local_from_world,
    mesh_view_bindings::{view, globals},
};
import bevy_pbr::prepass::utils::prepass_depth;

struct Params {
    /// x: half the box's width, m; y: where it starts along the flow; z: where it ends; w: steps.
    box_size: vec4<f32>,
    /// rgb: the colour of the hottest gas; w: how strong the effect is, 0 to 1.
    color: vec4<f32>,
    /// x: the hull's radius; y: the hull's front along the flow; z: its back; w: scroll, m/s.
    hull: vec4<f32>,
    /// x: brightness of the gas, cd/m²; y: the box's half width in the hull map, m.
    look: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: Params;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var hull_texture: texture_3d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var hull_sampler: sampler;

/// How many blurred copies of the map there are; each is blurred twice as wide as the last.
const HULL_LEVELS: f32 = 6.0;

/// The hull at a place on the map, at a level of blur.
fn hull_at(uv: vec2<f32>, level: f32) -> vec4<f32> {
    let w = (clamp(level, 0.0, HULL_LEVELS - 1.0) + 0.5) / HULL_LEVELS;
    return textureSampleLevel(hull_texture, hull_sampler, vec3<f32>(uv, w), 0.0);
}

fn hash(p: vec3<f32>) -> f32 {
    let q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    let r = q + dot(q, q.yzx + 33.33);
    return fract((r.x + r.y) * r.z);
}

/// Value noise.
fn noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let x00 = mix(hash(i), hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let x10 = mix(hash(i + vec3<f32>(0.0, 1.0, 0.0)), hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let x01 = mix(hash(i + vec3<f32>(0.0, 0.0, 1.0)), hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let x11 = mix(hash(i + vec3<f32>(0.0, 1.0, 1.0)), hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}

/// Long streaks along the flow, moving down it.
fn streaks(p: vec3<f32>, radius: f32, time: f32) -> f32 {
    let across = 3.2 / radius;
    let along = 0.22 / radius;
    let q = vec3<f32>(p.xy * across, p.z * along - time * params.hull.w * along);
    let a = noise(q);
    let b = noise(q * vec3<f32>(2.7, 2.7, 1.9) + 17.0);
    return a * 0.65 + b * 0.35;
}

/// Sparks: a thin bright streak in some cells of a grid moving down the flow.
fn sparks(p: vec3<f32>, radius: f32, time: f32) -> f32 {
    let cell = vec3<f32>(radius * 0.3, radius * 0.3, radius * 2.5);
    let moved = p - vec3<f32>(0.0, 0.0, time * params.hull.w * 1.4);
    let q = moved / cell;
    let i = floor(q);
    let chance = hash(i + 3.1);
    if chance < 0.95 {
        return 0.0;
    }
    // A line along the flow through a random point of the cell, faded at its ends.
    let f = q - i;
    let centre = vec2<f32>(hash(i + 1.7), hash(i + 5.3)) * 0.6 + 0.2;
    let off = (f.xy - centre) * cell.xy;
    let width = radius * 0.015;
    let line = exp(-dot(off, off) / (width * width));
    let ends = sin(f.z * 3.14159265);
    return line * ends * ends * (chance - 0.95) / 0.05;
}

/// The colour of the gas by how hot it is, 0 to 1: dim red to the hot colour to white.
fn gas_color(heat: f32) -> vec3<f32> {
    let hot = params.color.rgb;
    let cool = hot * vec3<f32>(0.8, 0.28, 0.2);
    let white = vec3<f32>(1.0, 0.92, 0.82);
    if heat < 0.5 {
        return mix(cool, hot, heat * 2.0);
    }
    return mix(hot, white, (heat - 0.5) * 2.0);
}

fn interleaved_noise(px: vec2<f32>, frame: u32) -> f32 {
    let moved = px + vec2<f32>(5.588238) * f32(frame % 64u);
    return fract(52.9829189 * fract(dot(moved, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(sample_index) sample_index: u32,
) -> @location(0) vec4<f32> {
    let local_from_world = get_local_from_world(in.instance_index);
    let half_width = params.box_size.x;
    let start = params.box_size.y;
    let end = params.box_size.z;
    let length = end - start;
    // The unit cube's coordinates to metres in the flow's frame.
    let to_metres = vec3<f32>(2.0 * half_width, 2.0 * half_width, length);
    let offset = vec3<f32>(0.0, 0.0, start + length * 0.5);
    let eye_unit = (local_from_world * vec4<f32>(view.world_position, 1.0)).xyz;
    let hit_unit = (local_from_world * in.world_position).xyz;
    let eye = eye_unit * to_metres + offset;
    let hit = hit_unit * to_metres + offset;
    let ray = normalize(hit - eye);

    // Where the ray is inside the box.
    let low = vec3<f32>(-half_width, -half_width, start);
    let high = vec3<f32>(half_width, half_width, end);
    let t_a = (low - eye) / ray;
    let t_b = (high - eye) / ray;
    let t_near = min(t_a, t_b);
    let t_far = max(t_a, t_b);
    var t0 = max(max(max(t_near.x, t_near.y), t_near.z), 0.0);
    var t1 = min(min(t_far.x, t_far.y), t_far.z);

    // What's in the way, from the depth prepass.
    let depth = prepass_depth(in.position, sample_index);
    if depth > 0.0 {
        let clip = view.clip_from_view;
        let ndc = (in.position.xy / view.viewport.zw) * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
        let through = vec3<f32>(ndc.x / clip[0][0], ndc.y / clip[1][1], -1.0);
        t1 = min(t1, clip[3][2] / depth * sqrt(dot(through, through)));
    }
    if t1 <= t0 {
        discard;
    }

    let radius = params.hull.x;
    let front = params.hull.y;
    let back = params.hull.z;
    let map_half = params.look.y;
    let strength = params.color.w;
    let steps = i32(params.box_size.w);
    let dt = (t1 - t0) / f32(steps);
    let time = globals.time;
    let wake_length = length * 0.45;
    // The shock sits closer the faster the flow: a thin bright skin at full strength.
    let standoff = radius * mix(0.42, 0.26, strength);
    let skin = radius * 0.1;

    var light = vec3<f32>(0.0);
    var t = t0 + dt * interleaved_noise(in.position.xy, globals.frame_count);
    for (var i = 0; i < steps; i++) {
        let p = eye + ray * t;
        let uv = p.xy / (2.0 * map_half) + 0.5;
        // The hull, blurred so the shock doesn't follow every step in it.
        let wide = hull_at(uv, 2.0);
        let wider = hull_at(uv, 3.5);
        let there = wide.b;
        var glow = 0.0;
        var heat = 0.0;

        let hull_front = select(front, wide.r / max(there, 1e-3), there > 0.02);
        // Sweeping back past the hull's edge, into the cone behind a blunt body.
        let flare = 1.0 - wider.b;
        let shock_at = hull_front - standoff + flare * flare * radius * 3.6;
        let behind_shock = p.z - shock_at;
        let reaches = smoothstep(0.015, 0.3, wider.b);
        if behind_shock > -skin * 4.0 {
            let shell = exp(-(behind_shock * behind_shock) / (skin * skin)) * reaches;
            glow += shell * 1.5;
            heat = max(heat, shell);
            // The gas squeezed between the shock and the hull, brightest against the hull.
            if behind_shock > 0.0 && p.z < hull_front + radius * 0.25 {
                let depth_in = clamp(behind_shock / max(standoff, 0.01), 0.0, 1.0);
                let gas = (0.2 + 0.8 * depth_in) * there;
                glow += gas * 1.2;
                heat = max(heat, gas);
            }
        }

        // Behind the front of the hull: the sheath streaming off its sides, into a widening wake.
        // Read ever more blurred with distance, so the envelope grows the way a wake does, with a
        // bright skin where the blur is half way, around a dimmer core.
        let along = p.z - hull_front;
        if along > 0.0 {
            let behind = max(p.z - back, 0.0);
            let spread = 1.2 + 0.7 * log2(1.0 + (along * 0.35 + behind) / radius);
            let w = hull_at(uv, spread).b;
            let edge = w * (1.0 - w) * 4.0;
            let fade = exp(-(along * 0.25 + behind) / wake_length);
            let body = streaks(p, radius, time);
            let streaky = smoothstep(0.18, 0.72, body);
            let sheath = (edge + w * 0.25) * (0.3 + 0.7 * streaky) * fade;
            glow += sheath * 1.3;
            heat = max(heat, sheath * fade);
            // Sparks only come off the hull, so they stay in the wake.
            glow += sparks(p, radius, time) * smoothstep(0.02, 0.2, w) * fade * fade * 0.5
                * smoothstep(0.35, 0.8, strength);
        }

        // Nothing reaches the box's walls: the gas fades out well before them.
        let walls = (1.0 - smoothstep(end - length * 0.45, end, p.z))
            * (1.0 - smoothstep(half_width * 0.55, half_width * 0.95, sqrt(dot(p.xy, p.xy))));
        glow *= walls;
        if glow > 0.0 {
            light += gas_color(clamp(heat * (0.35 + 0.65 * strength), 0.0, 1.0)) * glow * dt;
        }
        t += dt;
    }
    // Per metre of gas seen through: thin gas adds up along the line of sight.
    let shown = light / radius * params.look.x * strength * strength * view.exposure;
    return vec4<f32>(shown, 0.0);
}
