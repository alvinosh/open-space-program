// Engine exhaust plume, ray-marched through a cone that bounds it (see `plume.rs`).
//
// The mesh is a unit cylinder from the nozzle exit (local y = 0) down to y = -1, scaled to
// the bounding volume. Each pixel is shaded once: by the front face when the camera is
// outside the cylinder, by the back face when it is inside. The march runs in metres along
// the plume: `a` from the nozzle exit along the exhaust, `r` out from the axis.
//
// What glows: a hot core that narrows to a point a few nozzle radii out, the flame (a soft
// column whose edge spreads with the jet's expansion, dimming as it widens and with
// distance), and shock diamonds on the axis. Cooling soot darkens what is behind the far
// part of a sooty plume. Output is premultiplied: emitted light plus how much of the
// background the soot hides.

import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::{view, globals},
    mesh_functions::get_local_from_world,
};

struct PlumeParams {
    /// x: exit radius, y: bounding length, z: bounding radius at the far end, w: seed.
    shape: vec4<f32>,
    /// x: bounding cone radius at the nozzle, y: its growth per metre, z: distance to the
    /// ground along the axis, w: how much wider the jet gets where it splashes on it.
    bounds: vec4<f32>,
    /// x: thrust level, y: expansion, z: shock cell length, w: shock strength.
    flow: vec4<f32>,
    /// x: glow fade length, y: spread, z: luminance (cd/m² per m), w: soot.
    glow: vec4<f32>,
    core: vec4<f32>,
    flame: vec4<f32>,
    shock: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: PlumeParams;

const STEPS: u32 = 40u;
const TAU: f32 = 6.2831853;

fn hash(p: vec3<f32>) -> f32 {
    let q = fract(p * 0.3183099 + vec3<f32>(0.1, 0.2, 0.3));
    let r = q * 17.0;
    return fract(r.x * r.y * r.z * (r.x + r.y + r.z));
}

/// Value noise in [0, 1].
fn noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(
            mix(hash(i), hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x),
            mix(hash(i + vec3<f32>(0.0, 1.0, 0.0)), hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x),
            u.y,
        ),
        mix(
            mix(hash(i + vec3<f32>(0.0, 0.0, 1.0)), hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x),
            mix(hash(i + vec3<f32>(0.0, 1.0, 1.0)), hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x),
            u.y,
        ),
        u.z,
    );
}

/// Span of the ray `o + t d` (metres: x, a, z) inside the bounding cone r <= c0 + c1 a,
/// 0 <= a <= length. The cone is convex, so the span is one interval (empty if x > y).
fn cone_span(o: vec3<f32>, d: vec3<f32>, c0: f32, c1: f32, len: f32) -> vec2<f32> {
    var lo = -1e30;
    var hi = 1e30;
    if abs(d.y) > 1e-9 {
        let t0 = -o.y / d.y;
        let t1 = (len - o.y) / d.y;
        lo = min(t0, t1);
        hi = max(t0, t1);
    } else if o.y < 0.0 || o.y > len {
        return vec2<f32>(1.0, 0.0);
    }
    let edge = c0 + c1 * o.y;
    let qa = d.x * d.x + d.z * d.z - c1 * c1 * d.y * d.y;
    let qb = o.x * d.x + o.z * d.z - c1 * edge * d.y;
    let qc = o.x * o.x + o.z * o.z - edge * edge;
    if abs(qa) < 1e-12 {
        if abs(qb) < 1e-12 {
            return select(vec2<f32>(lo, hi), vec2<f32>(1.0, 0.0), qc > 0.0);
        }
        let t = -qc / (2.0 * qb);
        if qb > 0.0 {
            hi = min(hi, t);
        } else {
            lo = max(lo, t);
        }
        return vec2<f32>(lo, hi);
    }
    let disc = qb * qb - qa * qc;
    if disc < 0.0 {
        return select(vec2<f32>(lo, hi), vec2<f32>(1.0, 0.0), qa > 0.0);
    }
    let s = sqrt(disc);
    let r1 = (-qb - s) / qa;
    let r2 = (-qb + s) / qa;
    let ta = min(r1, r2);
    let tb = max(r1, r2);
    if qa > 0.0 {
        return vec2<f32>(max(lo, ta), min(hi, tb));
    }
    // Inside is outside the roots; within the slab only one piece is on our nappe (the
    // apex is behind the nozzle).
    let first = vec2<f32>(lo, min(hi, ta));
    if first.x < first.y {
        return first;
    }
    return vec2<f32>(max(lo, tb), hi);
}

struct Sample {
    emission: vec3<f32>,
    /// Extinction by soot, per metre.
    extinction: f32,
};

fn sample(p: vec3<f32>, time: f32) -> Sample {
    let a = max(p.y, 0.0);
    let r = length(p.xz);
    let re = params.shape.x;
    let level = params.flow.x;
    let expansion = params.flow.y;
    let cell = params.flow.z;
    let shock = params.flow.w;
    let fade = params.glow.x;
    let spread = params.glow.y;

    // The jet's edge: pinched and bulged by the shock cells in air, fanning out in vacuum,
    // and spreading out along the ground where it hits it.
    let wobble = 1.0 + 0.14 * shock * cos(TAU * a / cell) * exp(-a / (5.0 * cell));
    let ground = params.bounds.z;
    let splash = smoothstep(ground - 6.0 * re, ground, a);
    let w = (re * mix(0.9, 1.0, expansion) + a * spread) * wobble * (1.0 + params.bounds.w * splash);
    let q = r / w;
    // Turbulence: eddies grow and slow down along the plume, and wash out its edge.
    let scale = 1.4 * re + 0.15 * a;
    let flow_speed = 40.0 * re * (0.4 + level);
    let moving = vec3<f32>(p.x, a - time * flow_speed, p.z) / scale + params.shape.w;
    let eddy = 0.7 * noise(moving) + 0.3 * noise(2.1 * moving + 3.1);
    let rough = smoothstep(0.0, 6.0 * re, a) * (1.0 - 0.6 * expansion);
    let edge = q * (1.0 + 0.35 * rough * (eddy - 0.5));
    let body = exp(-2.2 * edge * edge);
    // The same flow through a wider section glows less; in air less so, as the exhaust
    // keeps burning with the oxygen it mixes with.
    let thin = pow(re / w, mix(1.0, 2.0, expansion));
    // Dim with distance, reaching zero at the end of the bounds.
    let along = exp(-a / fade) * smoothstep(params.shape.y, 0.8 * params.shape.y, a)
        * (1.0 - 0.5 * splash);
    // Cooling: the far part shifts towards red.
    let cooling = smoothstep(0.0, 2.0 * fade, a);
    let flame = params.flame.rgb * mix(vec3<f32>(1.0), vec3<f32>(1.0, 0.45, 0.25), cooling);

    // Potential core: bright, narrowing to the axis.
    let core_length = re * (2.0 + 6.0 * level) * (1.0 - 0.6 * expansion);
    let core_radius = re * max(1.0 - a / core_length, 0.0);
    let core = exp(-pow(r / max(core_radius, 1e-3), 4.0)) * step(a, core_length);

    // Shock diamonds: a lozenge on the axis per cell, brightest at its Mach disk, fading
    // downstream.
    let k = a / cell - 0.6;
    let phase = k - round(k);
    let half_width = 0.8 * re * max(1.0 - 2.2 * abs(phase), 0.0) + 1e-4;
    let knot = exp(-3.0 * pow(r / half_width, 2.0)) * exp(-phase * phase / 0.05)
        * shock * exp(-a / (4.0 * cell)) * smoothstep(0.2 * cell, 0.5 * cell, a);

    let turbulence = mix(1.0, 0.45 + 1.1 * eddy, rough);
    var out: Sample;
    out.emission = params.glow.z
        * (flame * body * thin * along * turbulence + params.core.rgb * core * 5.0
            + params.shock.rgb * knot * 2.5);
    // Soot shows downstream of the bright part, and not in vacuum.
    out.extinction = params.glow.w * body * sqrt(thin) * smoothstep(0.4 * fade, 2.0 * fade, a)
        * (1.0 - expansion) * turbulence / re * 0.5;
    return out;
}

/// Interleaved gradient noise, to hide the steps of the march.
fn dither(pixel: vec2<f32>) -> f32 {
    return fract(52.982919 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> @location(0) vec4<f32> {
    let local_from_world = get_local_from_world(in.instance_index);
    let camera = (local_from_world * vec4<f32>(view.world_position, 1.0)).xyz;
    let surface = (local_from_world * vec4<f32>(in.world_position.xyz, 1.0)).xyz;
    let inside = dot(camera.xz, camera.xz) < 1.0 && camera.y < 0.0 && camera.y > -1.0;
    if is_front == inside {
        discard;
    }
    // Into metres along the plume.
    let to_metres = vec3<f32>(params.shape.z, -params.shape.y, params.shape.z);
    let o = camera * to_metres;
    let d = (surface - camera) * to_metres;
    let span = cone_span(o, d, params.bounds.x, params.bounds.y, params.shape.y);
    let t0 = max(span.x, 0.0);
    let t1 = span.y;
    if t1 <= t0 {
        discard;
    }
    let dt = (t1 - t0) / f32(STEPS);
    let ds = dt * length(d);
    let time = globals.time;
    var t = t0 + dt * dither(in.position.xy);
    var light = vec3<f32>(0.0);
    var transmittance = 1.0;
    for (var i = 0u; i < STEPS; i++) {
        let s = sample(o + t * d, time);
        let absorb = exp(-s.extinction * ds);
        light += transmittance * s.emission * ds;
        transmittance *= absorb;
        t += dt;
    }
    // A little flicker, the same over the whole plume.
    let flicker = 1.0 + 0.06 * (noise(vec3<f32>(time * 23.0, params.shape.w, 0.0)) - 0.5);
    return vec4<f32>(light * flicker * view.exposure, 1.0 - transmittance);
}
