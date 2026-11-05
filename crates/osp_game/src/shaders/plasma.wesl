// The plasma around a vessel coming in too fast for the air, after Lindner's atmospheric entry
// shader. She extrudes strips from the hull's triangles along the flow in a geometry shader; there
// are none here, so the strips are all in the mesh already, each sitting on a point of the hull
// (`osp_sim::plasma`), and this shapes them every frame. Points scattered by area instead of the
// mesh's own vertices also keep the stream even, whatever the mesh, which was her concern.
//
// One mesh and one blend, in this order, each piece a layer:
// - GLOW, the hull's own triangles: what the air reaches is hot, from dull red to white, smoothly.
//   The rest of the hull only catches the light of the gas around it.
// - WAKE: the long stream behind it all, seen from further off.
// - WRAP: streaks that close in behind the hull, going blue.
// - TRAIL: the heart of it. Very many wide, faint strips off the outline the air sees, which add up
//   to a sheet of fire, and among them thin bright ones. What makes streaks of them is the noise in
//   the fragment shader, which goes by the angle around the flow: strips over the same place agree
//   with each other, so the streaks are as long as the fire and don't depend on the strips.
// - SPARK: bits of the hull torn off: few, small, and gone quickly.
// - GAS and SHOCK, a surface fitted to the outline (`osp_sim::plasma::Shock`): the hot gas heaped up
//   in front of the hull, and the shock standing ahead of that, sweeping back past the outline.
//   Neither has an edge: both fade out past the outline and where they are seen edge-on.
//
// What the air reaches comes from a depth map looking along the flow, as a shadow map would.
//
// All of it but the hot hull is pure light, added to what is behind it: gas this hot hides nothing.
// The colours go from warm to pink to blue rather than from orange to dim orange, which is brown.

import bevy_pbr::render::{
    mesh_functions::{get_local_from_world, get_world_from_local},
    mesh_view_bindings::{view, globals},
};

struct Params {
    /// x: how many of the hull's points streaks can come off; y: how bright each strip is, so that
    /// together they come to the same however many there are; z: how wide the hull is, m (`Shock`'s
    /// breadth); w: half the shock map's width, m.
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
    /// x: how far the streaks flare out, m; z: brightness of the gas at full strength, cd/m²; w: how
    /// much of a Mach cone's white the light here shows.
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
@group(constants::MATERIAL_BIND_GROUP) @binding(2) var noise_map: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(3) var noise_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(4) var shock_map: texture_2d<f32>;

const PI: f32 = 3.14159265;
const TAU: f32 = 6.2831853;

/// The layers (`Layer` in `plasma.rs`).
const GLOW: i32 = 0;
const WAKE: i32 = 1;
const WRAP: i32 = 2;
const TRAIL: i32 = 3;
const SPARK: i32 = 4;
const GAS: i32 = 5;
const SHOCK: i32 = 6;

/// As `osp_sim::plasma` has them.
const EDGE_ON: f32 = 0.15;
const UPSTREAM_TEXELS: f32 = 2.0;
/// In the map, at or above this nothing of the hull is there (`NOTHING_THERE` in `osp_sim::plasma`).
const NOTHING_THERE: f32 = 1.0e8;
/// Below this strength, none of the wrap, the shock or the sparks: only strong entries have them.
const WRAP_FROM: f32 = 0.4;
const SHOCK_FROM: f32 = 0.22;
const SPARKS_FROM: f32 = 0.3;
/// How wide the band around the hull's outline that streaks come off is, as a share of its reach.
const RIM_BAND: f32 = 0.14;
/// A surface facing the air more squarely than this gives no streak.
const LEAST_LEAN: f32 = 0.02;
/// How many strips there are around a hull, at most (`STRIPS` in `plasma.rs`). It doesn't go up with
/// the number of points on the hull.
const STRIPS: f32 = 1400.0;
/// Of those, this share are thin bright ones. The rest are wide and faint: the fire's body.
const BRIGHT_STREAKS: f32 = 0.18;
/// How many sparks there are at once, about.
const SPARKS: f32 = 60.0;
/// How bright a hot surface is at full strength, cd/m².
const SHIELD_BRIGHTNESS: f32 = 110000.0;
/// The most of the camera's exposure the fire is seen with (`MOST_EXPOSURE` in `plasma.rs`): opened
/// up for a dark night side, it would burn out to white, where an eye or a camera would stop back
/// down for something this bright. About a stop over daylight's.
const MOST_EXPOSURE: f32 = 5.0e-5;

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    /// The point of the hull this is on.
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    /// x: how far along the strip, 0 to 1; y: which side of it, -1 or 1. On a surface over the shock
    /// map, its texel.
    @location(2) place: vec2<f32>,
    /// x: the layer; y, z: this strip's random numbers; w: a spark's width, m.
    @location(3) strip: vec4<f32>,
};

struct Output {
    @builtin(position) clip: vec4<f32>,
    @location(0) place: vec2<f32>,
    /// x: the layer; y: a random number; z: a spark's age; w: the strip's length, m.
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

/// A place across the flow, from the map's middle.
fn across_flow(p: vec3<f32>) -> vec2<f32> {
    let q = p - params.centre.xyz;
    return vec2<f32>(dot(q, params.across_x.xyz), dot(q, params.across_y.xyz));
}

/// The airstream map's texel at a place across the flow, which needn't be on the map.
fn map_cell(across: vec2<f32>) -> vec2<i32> {
    let half_width = params.flow.w;
    let texel = 2.0 * half_width / f32(textureDimensions(airstream).x);
    return vec2<i32>(floor((across + half_width) / texel));
}

/// How far inside the hull's outline a texel of the airstream map is, m.
fn inside_at(cell: vec2<i32>) -> f32 {
    let last = i32(textureDimensions(airstream).x) - 1;
    return textureLoad(airstream, clamp(cell, vec2<i32>(0), vec2<i32>(last)), 0).g;
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
    let cell = map_cell(across);
    var sum = 0.0;
    var hull = 0.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let c = cell + vec2<i32>(i, j);
            let map = textureLoad(airstream, clamp(c, vec2<i32>(0), vec2<i32>(last)), 0).r;
            // Open air beside the outline says nothing about this point. Counted, the wall of a long
            // hull is reached all the way back, and strips come off the length of it.
            if map >= NOTHING_THERE {
                continue;
            }
            hull += 1.0;
            let offset = (vec2<f32>(c) + 0.5) * texel - half_width - across;
            let there = z + clamp(dot(offset, slope), -most, most);
            sum += step(there, map + most);
        }
    }
    return select(1.0, sum / hull, hull > 0.0);
}

/// How near a point is to the hull's outline as the air sees it: 1 on it, 0 past the band around it.
fn rim(p: vec3<f32>) -> f32 {
    let half_width = params.flow.w;
    let texel = 2.0 * half_width / f32(textureDimensions(airstream).x);
    let inside = inside_at(map_cell(across_flow(p)));
    return 1.0 - smoothstep(0.0, params.across_x.w * RIM_BAND + texel, inside);
}

/// The way to the hull's outline from a place across the flow, as it is on the map rather than as it
/// would be from the middle: a long hull's streaks leave square to its side. `fallback` where the map
/// doesn't say, far from any edge.
fn to_outline(across: vec2<f32>, fallback: vec2<f32>) -> vec2<f32> {
    let cell = map_cell(across);
    let slope = vec2<f32>(
        inside_at(cell - vec2<i32>(3, 0)) - inside_at(cell + vec2<i32>(3, 0)),
        inside_at(cell - vec2<i32>(0, 3)) - inside_at(cell + vec2<i32>(0, 3)),
    );
    let size = length(slope);
    if size < 1e-4 {
        return fallback;
    }
    return slope / size;
}

/// A surface over the shock map, in the mesh's axes: a dome ahead of the hull, `back` further along
/// the flow than the shock itself.
fn shock_surface(in: Vertex, back: f32, out: ptr<function, Output>) {
    let half_width = params.edge.w;
    let last = i32(textureDimensions(shock_map).x) - 1;
    let texel = 2.0 * half_width / f32(last + 1);
    let cell = vec2<i32>(in.place + 0.5);
    let here = textureLoad(shock_map, cell, 0).rg;
    let at = (vec2<f32>(cell) + 0.5) * texel - half_width;
    let x_axis = params.across_x.xyz;
    let y_axis = params.across_y.xyz;
    let flow = params.flow.xyz;
    let p = params.centre.xyz + x_axis * at.x + y_axis * at.y + flow * (here.r + back);
    let depth_at = vec4<f32>(
        textureLoad(shock_map, clamp(cell + vec2<i32>(1, 0), vec2<i32>(0), vec2<i32>(last)), 0).r,
        textureLoad(shock_map, clamp(cell - vec2<i32>(1, 0), vec2<i32>(0), vec2<i32>(last)), 0).r,
        textureLoad(shock_map, clamp(cell + vec2<i32>(0, 1), vec2<i32>(0), vec2<i32>(last)), 0).r,
        textureLoad(shock_map, clamp(cell - vec2<i32>(0, 1), vec2<i32>(0), vec2<i32>(last)), 0).r,
    );
    let slope = vec2<f32>(depth_at.x - depth_at.y, depth_at.z - depth_at.w) / (2.0 * texel);
    (*out).local = p;
    (*out).normal = unit(x_axis * slope.x + y_axis * slope.y - flow);
    // How far outside the outline, in widths of the hull.
    (*out).place = vec2<f32>(here.g / max(params.edge.z, 0.1), 0.0);
    (*out).opacity = smoothstep(SHOCK_FROM, SHOCK_FROM + 0.3, params.look.x) * (1.0 - params.look.w);
    (*out).clip = view.clip_from_world
        * (get_world_from_local(in.instance_index) * vec4<f32>(p, 1.0));
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

    if layer == SHOCK {
        shock_surface(in, 0.0, &out);
        return out;
    }
    if layer == GAS {
        shock_surface(in, params.edge.z * 0.14, &out);
        return out;
    }
    if layer == GLOW {
        let p = in.position + n * (0.012 + reach * 0.002);
        out.local = p;
        out.clip = view.clip_from_world * (world_from_local * vec4<f32>(p, 1.0));
        return out;
    }

    let t = in.place.x;
    let r1 = in.strip.y;
    let r2 = in.strip.z;
    let rel = across_flow(in.position);
    let off_axis = length(rel);
    let from_middle = rel / max(off_axis, 1e-3);
    let outline = to_outline(rel, from_middle);
    let to_rim = params.across_x.xyz * from_middle.x + params.across_y.xyz * from_middle.y;
    // Away from the hull, across the flow. What the surface itself does only adds to the way to the
    // outline: on a flat shield flying a little off its axis, it would be small and all one way.
    let side_of_hull = n - flow * dot(n, flow);
    let outward = unit(
        params.across_x.xyz * outline.x + params.across_y.xyz * outline.y + side_of_hull * 0.3
            + vec3<f32>(1e-5),
    );
    let windward = -dot(n, flow);
    // Her length multiplier: the nearer edge-on to the flow, the further it goes. A rim counts as
    // edge-on, whichever way it faces.
    let edge = 1.0 - clamp(windward, 0.0, 1.0);
    let on_rim = rim(in.position);
    let lean = max(edge * edge * edge, smoothstep(0.0, 0.7, on_rim));
    // Streaks in tongues around the hull, and each its own besides.
    let tongue = textureSampleLevel(
        noise_map,
        noise_sampler,
        vec2<f32>(atan2(rel.y, rel.x) / TAU * 2.0 + time * 0.03, time * 0.17),
        0.0,
    ).g;
    // Only so many of the points get a strip, however many there are.
    let on_edge = max(params.edge.x, 1.0);
    let one_in = fract((r1 + r2) * 37.7) * on_edge;
    // What comes off near the outline comes off the outline itself, not through the hull behind it.
    let inside_here = inside_at(map_cell(rel));
    var start = in.position
        + (params.across_x.xyz * outline.x + params.across_y.xyz * outline.y)
            * inside_here * smoothstep(0.0, 0.6, on_rim);
    var p = start;
    var width = in.strip.w;
    var opacity = 1.0;
    var length_of = 1.0;
    var age = 0.0;

    switch layer {
        case WAKE: {
            // From around the back of the hull, staying near the axis.
            let a = r1 * TAU;
            let r = sqrt(r2) * reach * 0.5;
            start = params.centre.xyz + params.across_x.xyz * r * cos(a)
                + params.across_y.xyz * r * sin(a) + flow * params.across_y.w;
            length_of = streak * (2.2 + 2.6 * r2) + reach * 3.0;
            p = start + flow * length_of * t;
            width = reach * (0.45 + 0.6 * r1) * mix(1.0, 1.5, t);
            // It would fill the screen from the chase camera: it's for seeing from further off.
            opacity = smoothstep(0.08, 0.5, strength) * smoothstep(reach * 3.0, reach * 16.0, length(eye - p));
        }
        case WRAP: {
            if strength < WRAP_FROM || windward < -0.15 || lean < 0.05 || one_in > STRIPS {
                return out;
            }
            let reached = exposure(in.position, n);
            if reached < 0.34 {
                return out;
            }
            length_of = streak * (0.8 + 0.7 * tongue) * (0.7 + 0.6 * r1) + reach * 2.0;
            // In to a neck behind the hull, not all the way to its axis.
            let closing = smoothstep(0.0, 0.8, t) * off_axis * 0.82;
            let bulge = sin(min(t * 3.0, 1.0) * PI) * reach * 0.1;
            p = start + flow * length_of * t - to_rim * (closing - bulge);
            width = reach * mix(0.22, 0.85, t);
            opacity = reached * smoothstep(WRAP_FROM, WRAP_FROM + 0.3, strength) * (0.4 + 0.6 * lean);
        }
        case TRAIL: {
            if windward < -0.15 || lean < LEAST_LEAN || one_in > STRIPS {
                return out;
            }
            let reached = exposure(in.position, n);
            if reached < 0.34 {
                return out;
            }
            // Most are short, and a few run on well past the others.
            let own = r1 * r1 * r1;
            let bright = r2 < BRIGHT_STREAKS;
            let reaches_on = select(0.45 + 0.75 * tongue, 0.5 + 1.1 * tongue * tongue + 2.0 * own, bright);
            length_of = streak * lean * reaches_on;
            let flare = params.extent.x * lean * clamp(length_of / streak, 0.15, 1.6) * (0.2 + 0.8 * r1);
            p = start + flow * length_of * t + outward * flare * pow(t, 0.8);
            width = reach * select(mix(0.14, 0.8, pow(t, 0.7)), mix(0.03, 0.16, t), bright);
            opacity = reached * (0.8 + 0.2 * tongue) * select(1.0, 1.6, bright);
            // Near the camera they would be great flat bands.
            opacity *= smoothstep(reach * 0.5, reach * 5.0, length(eye - p));
        }
        default: {
            // SPARK, worked out from the time alone: a bit of hull born every life, and carried off.
            if strength < SPARKS_FROM || windward < -0.15 || lean < 0.1 || one_in > SPARKS {
                return out;
            }
            if exposure(in.position, n) < 0.5 {
                return out;
            }
            let life = mix(0.12, 0.8, r1 * r1);
            age = fract(time / life + r2 * 61.0);
            let speed = reach * mix(16.0, 42.0, r2);
            length_of = reach * mix(0.08, 0.3, r2);
            let drift = reach * (r1 - 0.15) * age * 1.1;
            let wander = (fract(r1 * 91.7) - 0.5) * reach * age;
            start += flow * speed * age * life + outward * drift + cross(flow, outward) * wander;
            p = start + flow * length_of * t;
            width *= 0.6 + 0.9 * fract(r2 * 53.1);
            opacity = pow(1.0 - age, 1.2) * smoothstep(SPARKS_FROM, SPARKS_FROM + 0.25, strength)
                * smoothstep(0.0, 0.04, age);
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
    out.strip = vec4<f32>(in.strip.x, r1, age, length_of);
    out.clip = view.clip_from_world * (world_from_local * vec4<f32>(p, 1.0));
    return out;
}

/// The colour along a streak.
fn along_streak(t: f32) -> vec3<f32> {
    let early = smoothstep(0.0, 0.12, t);
    let late = smoothstep(0.22, 0.95, t);
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

/// Streaks running with the flow: noise that goes by the angle around it and how far along it, so
/// that everything over the same place agrees. `around` has to be a whole number, for the map to meet
/// itself. 0 to 1, about 0.5 on average, in the red channel or the green.
fn streaks(local: vec3<f32>, around: f32, along: f32, speed: f32) -> vec2<f32> {
    let q = local - params.centre.xyz;
    let angle = atan2(dot(q, params.across_y.xyz), dot(q, params.across_x.xyz));
    let down = dot(q, params.flow.xyz) / max(params.across_x.w, 0.1);
    return textureSampleLevel(
        noise_map,
        noise_sampler,
        vec2<f32>(angle / TAU * around, down * along - globals.time * speed),
        0.0,
    ).rg;
}

@fragment
fn fragment(in: Output) -> @location(0) vec4<f32> {
    let layer = i32(in.strip.x + 0.5);
    let strength = params.look.x;
    let exposed = min(view.exposure, MOST_EXPOSURE);
    // A Mach cone's white is vapour, lit by what falls on it (`extent.w`), where the fire is its own.
    let bright = params.extent.z * (0.35 + 0.65 * strength) * exposed
        * mix(1.0, params.extent.w, params.look.w);
    let time = globals.time;
    let flow = params.flow.xyz;
    let reach = max(params.across_x.w, 0.1);
    let t = in.place.x;
    // Across a strip, soft, and nothing at all at its edge.
    let across = max(exp(-in.place.y * in.place.y * 3.5) - 0.03, 0.0);
    let gain = params.edge.y;

    switch layer {
        case GLOW: {
            let n = unit(in.normal);
            let windward = -dot(n, flow);
            let reached = exposure(in.local, n);
            // Hottest at the shoulder, where the sheath is thinnest.
            let facing = smoothstep(-0.1, 0.45, windward);
            let middle = clamp(inside_at(map_cell(across_flow(in.local))) / max(params.edge.z, 0.1), 0.0, 1.0);
            let shoulder = (1.0 - middle) * (1.0 - middle);
            let crawl = textureSampleLevel(
                noise_map,
                noise_sampler,
                vec2<f32>(in.local.x + in.local.z, in.local.y) / reach * 0.03 + time * 0.004,
                0.0,
            ).r;
            let heat = strength * reached * facing * (0.72 + 0.5 * shoulder + 0.16 * crawl)
                * (1.0 - params.look.w);
            // Soon covers the paint: a shield barely glowing is a dull, dark red, not a pale one.
            let a = clamp(heat * 6.0, 0.0, 1.0);
            // What the air doesn't reach still catches the light of the gas around it, more towards
            // its edges, where there is more gas to see it through.
            let to_eye = unit(in.eye - in.local);
            let sheath = 1.0 - min(abs(dot(n, to_eye)), 1.0);
            let wash = params.mid.rgb * strength * strength
                * (1.0 - reached) * (0.015 + 0.13 * sheath * sheath) * (1.0 - params.look.w);
            return vec4<f32>((incandescent(heat) * a + wash) * SHIELD_BRIGHTNESS * exposed, a);
        }
        case GAS: {
            let to_eye = unit(in.eye - in.local);
            let facing = min(abs(dot(unit(in.normal), to_eye)), 1.0);
            // A long way through it edge-on, but with no line where it ends: thickest some way in
            // from where the surface turns away.
            let thick = smoothstep(0.0, 0.8, facing) * (0.5 + 1.5 * (1.0 - facing));
            let fade = 1.0 - smoothstep(-0.3, 0.75, t);
            // Not by the angle around the flow, as the streaks are: that all meets in its middle.
            let breathing = 0.85 + 0.3 * textureSampleLevel(
                noise_map,
                noise_sampler,
                across_flow(in.local) / reach * 0.05 + time * 0.03,
                0.0,
            ).g;
            let color = mix(params.hot.rgb, params.mid.rgb, smoothstep(-0.4, 0.45, t));
            return vec4<f32>(color * in.opacity * thick * fade * breathing * bright * 1.1, 0.0);
        }
        case SHOCK: {
            let to_eye = unit(in.eye - in.local);
            let facing = min(abs(dot(unit(in.normal), to_eye)), 1.0);
            let thick = smoothstep(0.0, 0.9, facing) * (0.08 + 0.8 * (1.0 - facing));
            let fade = 1.0 - smoothstep(-0.2, 1.5, t);
            let color = mix(params.mid.rgb, params.shock.rgb, smoothstep(-0.6, 0.15, t));
            return vec4<f32>(color * in.opacity * thick * fade * fade * bright * 0.3, 0.0);
        }
        case WAKE: {
            let lines = streaks(in.local, 4.0, 0.016, 0.8);
            let body = smoothstep(0.25, 0.8, lines.r * 0.6 + lines.g * 0.4);
            let fade = smoothstep(0.0, 0.12, t) * pow(1.0 - t, 1.4);
            let a = in.opacity * across * fade * (0.3 + 0.7 * body) * 0.02;
            let color = mix(params.mid.rgb, params.far.rgb, smoothstep(0.0, 0.6, t));
            return vec4<f32>(color * bright * a, 0.0);
        }
        case WRAP: {
            let lines = streaks(in.local, 2.0, 0.02, 0.35);
            let fade = smoothstep(0.0, 0.06, t) * pow(1.0 - t, 1.3);
            let a = in.opacity * across * fade * (0.35 + 0.9 * lines.r) * gain * 0.12;
            let color = mix(params.mid.rgb, params.wrap.rgb, smoothstep(0.05, 0.6, t));
            return vec4<f32>(color * bright * a, 0.0);
        }
        case TRAIL: {
            // Fine streaks over wide ones, running down the flow, wearing it away towards its end.
            let wide = streaks(in.local, 5.0, 0.03, 1.1);
            let fine = streaks(in.local, 21.0, 0.05, 1.7);
            let lines = wide.r * 0.5 + fine.g * 0.5;
            // By the metre, not the share of its length: a long strip and a short one over the same
            // place have to agree near the hull, or they make bars along it.
            let metres = t * in.strip.w / reach;
            let worn = mix(1.0, smoothstep(0.36, 0.68, lines), smoothstep(0.2, 2.2, metres));
            let fade = smoothstep(0.0, 0.03, metres) * pow(1.0 - t, 1.5);
            // Far brighter right at the hull, where it is hottest.
            let core = 0.72 + 3.0 * exp(-metres * 1.6);
            let a = in.opacity * across * fade * worn * core * gain * (0.3 + 0.7 * strength);
            return vec4<f32>(along_streak(t) * bright * a, 0.0);
        }
        default: {
            // SPARK: white-hot, cooling as it goes, and tapering to both ends of its streak.
            let a = in.opacity * across * sin(min(t, 1.0) * PI);
            let color = mix(vec3<f32>(1.0, 0.86, 0.6), params.mid.rgb, smoothstep(0.1, 0.8, in.strip.z));
            return vec4<f32>(color * params.extent.z * 2.5 * exposed * a, 0.0);
        }
    }
}
