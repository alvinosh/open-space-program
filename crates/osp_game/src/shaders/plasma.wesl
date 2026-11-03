// The plasma around a vessel coming in too fast for the air, after Lindner's atmospheric entry
// shader. She extrudes strips from the hull's triangles along the flow in a geometry shader; there
// are none here, so the strips are all in the mesh already, each sitting on a point of the hull
// (`osp_sim::plasma`), and this shapes them every frame. Points scattered by area instead of the
// mesh's own vertices also keep the stream even, whatever the mesh, which was her concern.
//
// One mesh and one blend, in this order, each piece a layer:
// - GLOW, the hull's own triangles: what the air reaches is hot, from dull red to white.
// - WAKE: the long stream behind it all.
// - WRAP: blue streaks that close in behind the hull.
// - TRAIL: the heart of it. Streaks off the edge of what the air reaches, long where the surface
//   runs along the flow and short where it faces it, so the middle of a heat shield doesn't poke
//   through the vessel. Yellow or blue at the hull, orange, then pale red, and rippling.
// - CONE: a faint blue shock off the edge, flaring out and back.
// - SPARK: bits of the hull torn off.
// - BOW: the hull's triangles again, pushed forward along their normals by how squarely they face
//   the air, in blue: the gas heaped up in front of it.
//
// What the air reaches comes from a depth map looking along the flow, as a shadow map would.
//
// Output is premultiplied: light, and how much of what is behind it hides. Gas this hot lets most
// through, but keeping a little from piling up is what stays orange instead of burning out to white.

import bevy_pbr::render::{
    mesh_functions::{get_local_from_world, get_world_from_local},
    mesh_view_bindings::{view, globals},
};

struct Params {
    /// x: how many of the hull's points streaks can come off.
    edge: vec4<f32>,
    /// xyz: the way the air goes, in the mesh's axes; w: half the airstream map's width, m.
    flow: vec4<f32>,
    /// The map's axes across the flow; x's w: how far the hull reaches across it, m; y's w: how far
    /// back the hull goes along the flow from the map's middle, m.
    across_x: vec4<f32>,
    across_y: vec4<f32>,
    /// xyz: the map's middle; w: how far forward the hull goes along the flow from there, m.
    centre: vec4<f32>,
    /// x: strength, 0 to 1; y: how long the streaks are, m; z: how thin the air is, 0 to 1; w: how
    /// much is a Mach cone's white.
    look: vec4<f32>,
    /// x: how far the streaks flare out, m; y: distance between the hull's points, m; z: brightness
    /// of the gas at full strength, cd/m²; w: how fast bits of hull are shown to go, m/s.
    extent: vec4<f32>,
    /// The gas's colours: at the hull, along the streak, at its end, the closing streaks, the shock.
    hot: vec4<f32>,
    mid: vec4<f32>,
    far: vec4<f32>,
    wrap: vec4<f32>,
    shock: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: Params;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var airstream: texture_2d<f32>;

const PI: f32 = 3.14159265;
const TAU: f32 = 6.2831853;

/// The layers (`Layer` in `plasma.rs`).
const GLOW: i32 = 0;
const WAKE: i32 = 1;
const WRAP: i32 = 2;
const TRAIL: i32 = 3;
const CONE: i32 = 4;
const SPARK: i32 = 5;
const BOW: i32 = 6;

/// As `osp_sim::plasma` has them.
const EDGE_ON: f32 = 0.15;
const UPSTREAM_TEXELS: f32 = 2.0;
/// Below this strength, none of the wrap, the shock or the sparks: only strong entries have them.
const WRAP_FROM: f32 = 0.4;
const SHOCK_FROM: f32 = 0.22;
const SPARKS_FROM: f32 = 0.3;
/// How wide the band around the hull's outline that streaks come off is, as a share of its reach.
const RIM_BAND: f32 = 0.14;
/// A surface facing the air more squarely than this gives no streak.
const LEAST_LEAN: f32 = 0.02;
/// How many streaks there are around a hull, about: enough to read as a sheet of fire, and few
/// enough to read as streaks. It doesn't go up with the number of points on the hull.
const STREAKS: f32 = 170.0;
/// Of those, this share are thin bright ones. The rest are wide and faint: the fire's body.
const BRIGHT_STREAKS: f32 = 0.3;
/// How bright a hot surface is at full strength, cd/m².
const SHIELD_BRIGHTNESS: f32 = 60000.0;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    /// The point of the hull this is on.
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    /// x: how far along the strip, 0 to 1; y: which side of it, -1 or 1.
    @location(2) place: vec2<f32>,
    /// x: the layer; y, z: this strip's random numbers; w: its width at the hull, m.
    @location(3) strip: vec4<f32>,
};

struct Output {
    @builtin(position) clip: vec4<f32>,
    @location(0) place: vec2<f32>,
    /// x: the layer; y: a random number; z: the strip's angle around the flow; w: its length, m.
    @location(1) strip: vec4<f32>,
    /// How much of it shows.
    @location(2) opacity: f32,
    @location(3) local: vec3<f32>,
    @location(4) normal: vec3<f32>,
    /// The camera, in the mesh's axes.
    @location(5) eye: vec3<f32>,
};

/// A direction, whatever is interpolated across a triangle: an exact zero would be a NaN on screen.
fn unit(v: vec3<f32>) -> vec3<f32> {
    return v / max(length(v), 1e-5);
}

fn hash(p: vec3<f32>) -> f32 {
    let q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    let r = q + dot(q, q.yzx + 33.33);
    return fract((r.x + r.y) * r.z);
}

/// Value noise, 0 to 1.
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

/// How much of a point on the hull the air reaches, 0 to 1, softened over a few of the map's texels:
/// `Airstream::reaches` in `osp_sim::plasma`.
fn exposure(p: vec3<f32>, n: vec3<f32>) -> f32 {
    let last = i32(textureDimensions(airstream).x) - 1;
    let half_width = params.flow.w;
    let texel = 2.0 * half_width / f32(last + 1);
    let q = p - params.centre.xyz;
    let across = vec2<f32>(dot(q, params.across_x.xyz), dot(q, params.across_y.xyz));
    let z = dot(q, params.flow.xyz);
    let along = dot(n, params.flow.xyz);
    var slope = vec2<f32>(0.0);
    if abs(along) >= EDGE_ON {
        slope = -vec2<f32>(dot(n, params.across_x.xyz), dot(n, params.across_y.xyz)) / along;
    }
    let most = UPSTREAM_TEXELS * texel;
    let cell = vec2<i32>(floor((across + half_width) / texel));
    var sum = 0.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let c = cell + vec2<i32>(i, j);
            let offset = (vec2<f32>(c) + 0.5) * texel - half_width - across;
            let there = z + clamp(dot(offset, slope), -most, most);
            let map = textureLoad(airstream, clamp(c, vec2<i32>(0), vec2<i32>(last)), 0).r;
            sum += step(there, map + most);
        }
    }
    return sum / 9.0;
}

/// How near a point is to the hull's outline as the air sees it: 1 on it, 0 past the band around it.
fn rim(p: vec3<f32>) -> f32 {
    let last = i32(textureDimensions(airstream).x) - 1;
    let half_width = params.flow.w;
    let texel = 2.0 * half_width / f32(last + 1);
    let q = p - params.centre.xyz;
    let across = vec2<f32>(dot(q, params.across_x.xyz), dot(q, params.across_y.xyz));
    let cell = clamp(vec2<i32>(floor((across + half_width) / texel)), vec2<i32>(0), vec2<i32>(last));
    let inside = textureLoad(airstream, cell, 0).g;
    return 1.0 - smoothstep(0.0, params.across_x.w * RIM_BAND + texel, inside);
}

@vertex
fn vertex(in: Vertex) -> Output {
    var out: Output;
    let layer = i32(in.strip.x + 0.5);
    out.place = in.place;
    out.strip = vec4<f32>(in.strip.x, in.strip.y, 0.0, 1.0);
    out.local = in.position;
    out.normal = in.normal;
    out.opacity = 0.0;
    // Off the screen, which drops the whole strip.
    out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);

    let world_from_local = get_world_from_local(in.instance_index);
    let eye = (get_local_from_world(in.instance_index) * vec4<f32>(view.world_position, 1.0)).xyz;
    out.eye = eye;
    let flow = params.flow.xyz;
    let reach = max(params.across_x.w, 0.1);
    let strength = params.look.x;
    let streak = params.look.y;
    let n = unit(in.normal);
    let time = globals.time;

    if layer == GLOW || layer == BOW {
        var p = in.position + n * (0.012 + reach * 0.002);
        if layer == BOW {
            // Her dome: what squarely faces the air is pushed forward the most.
            let square = pow(max(-dot(n, flow), 0.0), 3.0);
            let lift = square * exposure(in.position, n);
            let show = smoothstep(SHOCK_FROM, SHOCK_FROM + 0.3, strength) * (1.0 - params.look.w);
            p += n * lift * reach * (0.22 + 0.2 * strength);
            out.opacity = lift * show;
        }
        out.local = p;
        out.clip = view.clip_from_world * (world_from_local * vec4<f32>(p, 1.0));
        return out;
    }

    let t = in.place.x;
    let r1 = in.strip.y;
    let r2 = in.strip.z;
    let q = in.position - params.centre.xyz;
    let rel = vec2<f32>(dot(q, params.across_x.xyz), dot(q, params.across_y.xyz));
    let off_axis = length(rel);
    let to_rim = (params.across_x.xyz * rel.x + params.across_y.xyz * rel.y) / max(off_axis, 1e-3);
    // Away from the hull, across the flow.
    let side_of_hull = n - flow * dot(n, flow);
    let outward = unit(side_of_hull + to_rim * 0.35 + vec3<f32>(1e-5));
    let windward = -dot(n, flow);
    // Her length multiplier: the nearer edge-on to the flow, the further it goes. A rim counts as
    // edge-on, whichever way it faces.
    let edge = 1.0 - clamp(windward, 0.0, 1.0);
    let on_rim = rim(in.position);
    let lean = max(edge * edge * edge, smoothstep(0.0, 0.7, on_rim));
    // Streaks in tongues around the hull, and each its own besides.
    let tongue = noise(vec3<f32>(rel / reach * 3.1, time * 0.9));
    let ripple = noise(vec3<f32>(rel / reach * 13.0, time * 3.5));
    // Only so many of the points get one, however many there are.
    let on_edge = max(params.edge.x, 1.0);
    let one_in = fract((r1 + r2) * 37.7) * on_edge;
    var start = in.position;
    var p = in.position;
    var width = in.strip.w;
    var opacity = 1.0;
    var length_of = 1.0;

    switch layer {
        case WAKE: {
            // From around the back of the hull, staying near the axis.
            let a = r1 * TAU;
            let r = sqrt(r2) * reach * 0.45;
            start = params.centre.xyz + params.across_x.xyz * r * cos(a)
                + params.across_y.xyz * r * sin(a) + flow * params.across_y.w;
            length_of = streak * (1.6 + 1.8 * r2) + reach * 2.0;
            let sway = vec2<f32>(
                noise(vec3<f32>(r1 * 40.0, t * 2.5, time * 0.6)) - 0.5,
                noise(vec3<f32>(r2 * 40.0, t * 2.5, time * 0.6 + 7.0)) - 0.5,
            ) * reach * t * 1.2;
            p = start + flow * length_of * t + params.across_x.xyz * sway.x
                + params.across_y.xyz * sway.y;
            width = reach * (0.2 + 0.35 * r1) * mix(1.0, 0.7, t);
            // It would fill the screen from the chase camera: it's for seeing from further off.
            opacity = smoothstep(0.08, 0.5, strength) * smoothstep(reach * 4.0, reach * 24.0, length(eye - p));
        }
        case WRAP: {
            if strength < WRAP_FROM || windward < -0.15 || lean < 0.05 || one_in > STREAKS {
                return out;
            }
            let reached = exposure(in.position, n);
            if reached < 0.34 {
                return out;
            }
            length_of = streak * (1.2 + 2.0 * tongue) * (0.7 + 0.6 * r1) + reach * 2.0;
            let closing = pow(t, 1.15) * off_axis * 0.95;
            let bulge = sin(min(t * 3.0, 1.0) * PI) * reach * 0.12;
            p = start + flow * length_of * t - to_rim * (closing - bulge);
            width = max(width, reach * 0.08) * mix(1.0, 3.0, t);
            opacity = reached * smoothstep(WRAP_FROM, WRAP_FROM + 0.3, strength) * (0.4 + 0.6 * lean);
        }
        case TRAIL: {
            if windward < -0.15 || lean < LEAST_LEAN || one_in > STREAKS {
                return out;
            }
            let reached = exposure(in.position, n);
            if reached < 0.34 {
                return out;
            }
            // Most are short, and a few run on well past the others.
            let own = r1 * r1 * r1;
            let bright = r2 < BRIGHT_STREAKS;
            let reaches_on = select(0.35 + 1.1 * tongue, 0.2 + 1.5 * tongue * tongue + 0.7 * ripple * ripple + 2.2 * own, bright);
            length_of = streak * lean * reaches_on;
            let flare = params.extent.x * lean * clamp(length_of / streak, 0.15, 1.6) * (0.5 + r1);
            p = start + flow * length_of * t + outward * flare * pow(t, 0.55);
            width = max(width, reach * select(0.11, 0.03, bright)) * (1.0 + 1.5 * r1 * r1) * mix(0.7, 3.0, t);
            opacity = reached * (0.3 + 0.7 * tongue) * select(0.3, 0.45 + 0.55 * r1, bright);
            // Near the camera they would be great flat bands.
            opacity *= smoothstep(reach * 0.5, reach * 6.0, length(eye - p));
        }
        case CONE: {
            if strength < SHOCK_FROM || windward < -0.15 || lean < 0.1 || one_in > STREAKS * 2.0 {
                return out;
            }
            let reached = exposure(in.position, n);
            if reached < 0.34 {
                return out;
            }
            length_of = reach * (3.0 + 1.0 * tongue) * (0.5 + 0.5 * strength);
            p = start + flow * length_of * t + outward * length_of * 0.6 * pow(t, 0.85);
            width = max(width, reach * 0.12) * mix(1.0, 5.0, t);
            opacity = reached * smoothstep(SHOCK_FROM, SHOCK_FROM + 0.3, strength)
                * (1.0 - params.look.w) * (0.5 + 0.5 * tongue);
        }
        default: {
            // SPARK, worked out from the time alone: a bit of hull born every life, and carried off.
            if strength < SPARKS_FROM || windward < -0.15 || lean < 0.1 || one_in > STREAKS {
                return out;
            }
            if exposure(in.position, n) < 0.5 {
                return out;
            }
            let life = mix(0.3, 1.1, r1);
            let age = fract(time / life + r2 * 61.0);
            let speed = params.extent.w * (0.7 + 0.7 * r2);
            length_of = speed * 0.02 + 0.4;
            let drift = reach * (r1 - 0.2) * age * 0.9;
            let wander = (noise(vec3<f32>(r1 * 50.0, age * 4.0, r2 * 50.0)) - 0.5) * reach * age;
            start = in.position + flow * speed * age * life + outward * drift
                + cross(flow, outward) * wander;
            p = start + flow * length_of * t;
            opacity = pow(1.0 - age, 1.3) * smoothstep(SPARKS_FROM, SPARKS_FROM + 0.25, strength)
                * smoothstep(0.0, 0.05, age);
        }
    }

    // The strip's breadth: facing the camera from the side, around the flow from along it.
    let around = unit(cross(flow, outward) + vec3<f32>(1e-5));
    let facing_there = cross(flow, eye - p);
    let side_on = length(facing_there) / max(length(eye - p), 1e-3);
    let flip = select(-1.0, 1.0, dot(cross(flow, eye - start), around) >= 0.0);
    let facing = unit(facing_there + vec3<f32>(1e-5)) * flip;
    let breadth = unit(mix(around, facing, smoothstep(0.15, 0.5, side_on)));

    // Closing streaks all meet on the axis: looking along it, they would add up to a solid beam.
    if layer == WRAP {
        opacity *= mix(0.15, 1.0, smoothstep(0.1, 0.6, side_on));
    }

    // Never thinner than a pixel, far away: dimmer instead, so it doesn't sparkle.
    let distance = length(eye - p);
    let pixel = distance * 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
    let thinnest = pixel * 1.3;
    opacity *= min(width / max(thinnest, 1e-5), 1.0);
    // What goes right past the camera would fill the screen.
    opacity *= smoothstep(0.5, 6.0, distance);
    p += breadth * max(width, thinnest) * in.place.y * 0.5;

    out.local = p;
    out.opacity = opacity;
    out.strip = vec4<f32>(in.strip.x, r1, atan2(rel.y, rel.x), length_of);
    out.clip = view.clip_from_world * (world_from_local * vec4<f32>(p, 1.0));
    return out;
}

/// The colour along a streak.
fn along_streak(t: f32) -> vec3<f32> {
    let early = smoothstep(0.0, 0.22, t);
    let late = smoothstep(0.22, 0.9, t);
    return mix(mix(params.hot.rgb, params.mid.rgb, early), params.far.rgb, late);
}

/// What a surface this hot gives off, 0 to 1: dull red to white, far brighter as it goes.
fn incandescent(heat: f32) -> vec3<f32> {
    let h = clamp(heat, 0.0, 1.0);
    var color = mix(vec3<f32>(0.5, 0.05, 0.0), vec3<f32>(1.0, 0.3, 0.05), smoothstep(0.0, 0.4, h));
    color = mix(color, vec3<f32>(1.0, 0.68, 0.28), smoothstep(0.35, 0.7, h));
    color = mix(color, vec3<f32>(1.0, 0.93, 0.78), smoothstep(0.65, 1.0, h));
    return color * (0.03 + 1.15 * h * h);
}

@fragment
fn fragment(in: Output) -> @location(0) vec4<f32> {
    let layer = i32(in.strip.x + 0.5);
    let strength = params.look.x;
    let bright = params.extent.z * (0.35 + 0.65 * strength) * view.exposure;
    let time = globals.time;
    let flow = params.flow.xyz;
    let reach = max(params.across_x.w, 0.1);
    let t = in.place.x;
    // Across a strip, soft.
    let across = exp(-in.place.y * in.place.y * 2.4);

    switch layer {
        case GLOW: {
            let n = unit(in.normal);
            let windward = -dot(n, flow);
            let reached = exposure(in.local, n);
            // Hottest where the air hits, and at the shoulder, where the sheath thins.
            let shoulder = smoothstep(0.1, 0.5, windward) * (1.0 - smoothstep(0.55, 0.9, windward));
            let facing = smoothstep(-0.1, 0.5, windward) * 0.82 + shoulder * 0.3;
            // Blotches of it, slowly crawling.
            let p = in.local / reach;
            let blotches = noise(vec3<f32>(p * 4.0 + time * 0.1)) * 0.65 + noise(vec3<f32>(p * 13.0 - time * 0.2)) * 0.35;
            let heat = strength * reached * facing * (0.72 + 0.56 * blotches) * (1.0 - params.look.w);
            let a = clamp(heat * 1.7, 0.0, 1.0);
            // The gas around lights the sides out of the air.
            let wash = params.wrap.rgb * strength * strength * 0.0025 * (1.0 - reached)
                * (1.0 - params.look.w);
            return vec4<f32>((incandescent(heat) * a + wash) * SHIELD_BRIGHTNESS * view.exposure, a);
        }
        case BOW: {
            // Limb-bright, as a thin shell of glowing gas is: mostly seen edge-on as the crescent.
            let to_eye = normalize(in.eye - in.local);
            let limb = pow(1.0 - min(abs(dot(unit(in.normal), to_eye)), 1.0), 1.6);
            let light = in.opacity * (0.22 + 0.9 * limb);
            return vec4<f32>(params.shock.rgb * light * bright * 0.55, 0.0);
        }
        case WAKE: {
            let ahead = t * in.strip.w / reach;
            let puffs = noise(vec3<f32>(ahead * 0.55 - time * 2.6, in.strip.y * 37.0, in.place.y * 1.1));
            let fine = noise(vec3<f32>(ahead * 2.2 - time * 4.0, in.strip.y * 11.0, in.place.y * 2.3));
            let body = smoothstep(0.4, 0.85, puffs * 0.7 + fine * 0.3);
            let fade = smoothstep(0.0, 0.1, t) * pow(1.0 - t, 1.4);
            let a = in.opacity * across * fade * body * 0.09;
            let color = mix(params.mid.rgb, params.far.rgb, smoothstep(0.1, 0.8, t));
            return vec4<f32>(color * bright * a, a * 0.3);
        }
        case WRAP: {
            let ahead = t * in.strip.w / reach;
            let flicker = noise(vec3<f32>(in.strip.z * 3.0 + in.strip.y * 9.0, ahead * 0.5 - time * 0.9, 5.0));
            let fade = smoothstep(0.0, 0.08, t) * pow(1.0 - t, 1.2);
            let a = in.opacity * across * fade * (0.3 + 0.7 * flicker) * 0.009;
            let color = mix(params.mid.rgb, params.wrap.rgb, smoothstep(0.15, 0.7, t));
            return vec4<f32>(color * bright * a, a * 0.2);
        }
        case TRAIL: {
            let ahead = t * in.strip.w / reach;
            // Ripples running down the streak, wearing it away towards its end.
            let run = noise(vec3<f32>(in.strip.z * 5.0 + in.strip.y * 6.0, ahead * 0.9 - time * 5.5, 1.0));
            let fine = noise(vec3<f32>(in.strip.z * 17.0 + in.strip.y * 13.0, ahead * 3.0 - time * 8.0, 3.0));
            let worn = mix(1.0, smoothstep(0.1, 0.72, run * 0.7 + fine * 0.3), smoothstep(0.04, 0.6, t));
            let fade = smoothstep(0.0, 0.015, t) * pow(1.0 - t, 1.7);
            let a = in.opacity * across * fade * worn * 0.3 * (0.3 + 0.7 * strength);
            return vec4<f32>(along_streak(t) * bright * a, a * 0.75);
        }
        case CONE: {
            let fade = smoothstep(0.0, 0.1, t) * pow(1.0 - t, 1.5);
            let a = in.opacity * across * fade * 0.0035;
            return vec4<f32>(params.shock.rgb * bright * a, 0.0);
        }
        default: {
            // SPARK: white-hot, cooling along its streak.
            let a = in.opacity * across * (1.0 - t);
            let color = mix(vec3<f32>(1.0, 0.85, 0.6), params.mid.rgb, in.place.x);
            return vec4<f32>(color * params.extent.z * 2.0 * view.exposure * a, 0.0);
        }
    }
}
