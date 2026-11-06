// Engine exhaust plumes, ray-marched through a cone that bounds a cluster's jets (see
// `plume.rs`).
//
// The mesh is a unit cylinder from the exit plane (local y = 0) down to y = -1, scaled to
// the bounding volume. Each pixel is shaded once: by the front face when the camera is
// outside the cylinder, by the back face when it is inside. The march runs in metres along
// the plume: `a` (y) from the exit plane along the exhaust, x and z across it. Every jet
// leaves its own nozzle, which may be gimballed off the plume axis, turns onto the axis
// within its bend length and spreads until it merges with its neighbours.
//
// The exhaust is a medium that glows, absorbs and scatters:
// - The gas glows in its own colours: a soft column brightest in the potential core and
//   the shock diamonds (the band emission of a clean flame). A jet well off the ambient
//   pressure ends each shock cell in a Mach disk, a flat white-hot disc of exhaust the
//   shock has heated again.
// - Particles (soot) hide what is behind them and glow like a black body at their
//   temperature: white-hot where the exhaust afterburns with the air, through yellow and
//   orange as it mixes and cools, to smoke that the sun, the sky and the flame light.
// - Cooled exhaust condenses into a white haze: the trail in the air, the sunlit cloud
//   around a plume in thin air.
// An open-cycle engine's turbine exhaust is a jet of its own from its pipe (cool, sooty, a
// dark streak beside the plume until it catches fire with the air) or a dark film lining
// the nozzle wall. Eddies push the jets' edges about and stir the temperature, so the steep
// black-body glow breaks into hot and cool pockets. Output is premultiplied: the light that reaches
// the camera, and how much of what is behind the plume it hides.

import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_functions::get_local_from_world,
    utils::interleaved_gradient_noise,
};
import bevy_pbr::render::mesh_view_bindings as bindings;

struct Jet {
    /// xy: the nozzle exit across the plume axis (x, z), z: how far down the axis it is,
    /// w: exit radius; metres.
    exit: vec4<f32>,
    /// xy: tangent of the nozzle axis against the plume axis (x, z), z: bend length, w:
    /// thrust level.
    bend: vec4<f32>,
    /// x: expansion, y: shock cell length, z: shock strength, w: glow fade length.
    flow: vec4<f32>,
    /// x: spread, y: the gas's brightness, z: exit temperature share, w: hot length.
    heat: vec4<f32>,
    /// x: Mach disk brightness, y: its radius as a share of the exit radius.
    disk: vec4<f32>,
    /// x: particles as a multiple of the look's, y: how far it takes to catch fire with the
    /// air, m, z: turbine exhaust film on the nozzle wall.
    mix: vec4<f32>,
};

struct PlumeParams {
    /// x: bounding length, y: bounding radius at the far end, z: seed, w: number of jets.
    shape: vec4<f32>,
    /// x: bounding cone radius at the exit plane, y: its growth per metre, z: distance to
    /// the ground along the axis, w: how much wider a jet gets where it splashes on it.
    bounds: vec4<f32>,
    /// The gas's colours; core.w: its luminance (cd/m² per m at full thrust).
    core: vec4<f32>,
    flame: vec4<f32>,
    shock: vec4<f32>,
    /// rgb: the particles' albedo, w: their load.
    smoke: vec4<f32>,
    /// x: the particles' hottest temperature (K), y: haze, z: eddy size at the exit, m.
    medium: vec4<f32>,
    /// rgb: luminance of the hot flame (cd/m²), w: its radius.
    glow: vec4<f32>,
    /// xyz: the local vertical (world axes), w: the hot flame's length.
    up: vec4<f32>,
    /// x: ray-marching steps, y: steps towards the sun, z: noise octaves.
    quality: vec4<f32>,
    jets: array<Jet, 32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: PlumeParams;

const PI: f32 = 3.14159265;
const TAU: f32 = 6.2831853;
/// March step as a share of the jet's width where the ray crosses it, and the fewest steps:
/// a ray across the plume needs a dozen or so, one along it all of the setting's.
const STEP_WIDTH: f32 = 0.25;
const MIN_STEPS: u32 = 12u;
/// Extinction by the particles per metre for a load of 1 at the exit density of a nozzle
/// 1 m across (as in `plume.rs`), and by the haze.
const PARTICLE_EXTINCTION: f32 = 6.0;
const HAZE_EXTINCTION: f32 = 3.0;
const HAZE_ALBEDO: f32 = 0.97;
/// Air temperature, K.
const AIR: f32 = 290.0;
/// Eddies grow by this much per metre along the plume, and so many pass a point a second.
/// The pattern advects in units of eddies, so it moves faster where they are bigger and
/// never stretches however long the engine burns; it repeats every `EDDY_PERIOD` eddies.
const EDDY_GROWTH: f32 = 0.15;
const EDDY_RATE: f32 = 9.0;
const EDDY_PERIOD: f32 = 64.0;

// --- Noise ---

fn hash(p: vec3<i32>) -> u32 {
    var h = (bitcast<u32>(p.x) * 73856093u) ^ (bitcast<u32>(p.y) * 19349663u)
        ^ (bitcast<u32>(p.z) * 83492791u);
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return h;
}

/// Dot of `f` with one of the edge directions of a cube (Perlin's improved noise).
fn grad(h: u32, f: vec3<f32>) -> f32 {
    let k = h & 15u;
    let u = select(f.y, f.x, k < 8u);
    let v = select(select(f.z, f.x, k == 12u || k == 14u), f.y, k < 4u);
    return select(-u, u, (k & 1u) == 0u) + select(-v, v, (k & 2u) == 0u);
}

/// Gradient noise in about [-1, 1] that repeats every `period` along y.
fn perlin(p: vec3<f32>, period: i32) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let y0 = ((i.y % period) + period) % period;
    let y1 = (y0 + 1) % period;
    let n000 = grad(hash(vec3<i32>(i.x, y0, i.z)), f);
    let n100 = grad(hash(vec3<i32>(i.x + 1, y0, i.z)), f - vec3<f32>(1.0, 0.0, 0.0));
    let n010 = grad(hash(vec3<i32>(i.x, y1, i.z)), f - vec3<f32>(0.0, 1.0, 0.0));
    let n110 = grad(hash(vec3<i32>(i.x + 1, y1, i.z)), f - vec3<f32>(1.0, 1.0, 0.0));
    let n001 = grad(hash(vec3<i32>(i.x, y0, i.z + 1)), f - vec3<f32>(0.0, 0.0, 1.0));
    let n101 = grad(hash(vec3<i32>(i.x + 1, y0, i.z + 1)), f - vec3<f32>(1.0, 0.0, 1.0));
    let n011 = grad(hash(vec3<i32>(i.x, y1, i.z + 1)), f - vec3<f32>(0.0, 1.0, 1.0));
    let n111 = grad(hash(vec3<i32>(i.x + 1, y1, i.z + 1)), f - vec3<f32>(1.0, 1.0, 1.0));
    return mix(
        mix(mix(n000, n100, u.x), mix(n010, n110, u.x), u.y),
        mix(mix(n001, n101, u.x), mix(n011, n111, u.x), u.y),
        u.z,
    );
}

/// The eddies at `p`, in about [-1, 1]: octaves of noise in coordinates where an eddy is
/// one unit across, the smaller ones turning over faster. Octaves finer than `footprint`
/// (metres per pixel there) fade out rather than shimmer.
fn eddies(p: vec3<f32>, footprint: f32) -> f32 {
    let size = params.medium.z;
    let scale = size + EDDY_GROWTH * max(p.y, 0.0);
    // Eddies passed since the exit plane.
    let along = log(scale / size) / EDDY_GROWTH;
    let across = p.xz / scale;
    let time = bindings::globals.time;
    let seed = params.shape.z;
    var sum = 0.0;
    var total = 0.0;
    var weight = 1.0;
    var frequency = 1.0;
    let octaves = u32(params.quality.z);
    for (var k = 0u; k < octaves; k++) {
        let rate = EDDY_RATE * (1.0 + 0.6 * f32(k));
        let shift = fract(rate * time / EDDY_PERIOD) * EDDY_PERIOD;
        let q = vec3<f32>(across.x, along - shift, across.y) * frequency
            + vec3<f32>(seed + 17.3 * f32(k), 0.0, 31.7 * f32(k));
        let shown = 1.0 - smoothstep(0.25, 0.5, footprint * frequency / scale);
        sum += weight * shown * perlin(q, i32(EDDY_PERIOD * frequency));
        total += weight;
        weight *= 0.5;
        frequency *= 2.0;
    }
    return clamp(2.6 * sum / max(total, 1e-6), -1.0, 1.0);
}

// --- The exhaust ---

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

/// Radiance of a black body at `kelvin`, linear Rec. 709 in cd/m² of luminance (as
/// `plume::blackbody`).
fn blackbody(kelvin: f32) -> vec3<f32> {
    let c2 = vec3<f32>(23021.0, 26768.0, 35747.0) / max(kelvin, 1.0);
    return vec3<f32>(1.144e11, 1.951e11, 1.199e12) / (exp(min(c2, vec3<f32>(80.0))) - 1.0);
}

struct Medium {
    /// Light given off per metre, cd/m² per m.
    emission: vec3<f32>,
    /// Extinction per metre.
    extinction: f32,
    /// Scattering per metre, coloured by what scatters.
    scattering: vec3<f32>,
    /// Width of the jets here, m.
    width: f32,
};

/// The exhaust at `p` (metres). `e` is the eddies there; `glow` false leaves out the light
/// the exhaust gives off (rays towards the sun need only how much it hides).
fn medium(p: vec3<f32>, e: f32, glow: bool) -> Medium {
    let n = u32(params.shape.w);
    let ground = params.bounds.z;
    let end = smoothstep(params.shape.x, 0.8 * params.shape.x, p.y);
    var gas = vec3<f32>(0.0);
    // Exhaust density (a share of the density at the exits, per metre of exit radius), and
    // the density-weighted temperature share and width.
    var density = 0.0;
    var soot = 0.0;
    var condensed = 0.0;
    var heat = 0.0;
    var width = 0.0;
    for (var i = 0u; i < n; i++) {
        let jet = params.jets[i];
        let a = p.y - jet.exit.z;
        if a < 0.0 {
            continue;
        }
        let re = jet.exit.w;
        let level = jet.bend.w;
        let expansion = jet.flow.x;
        let cell = jet.flow.y;
        let shock = jet.flow.z;
        let bend_length = max(jet.bend.z, 1e-3);
        let across = p.xz - jet.exit.xy
            - jet.bend.xy * bend_length * (1.0 - exp(-a / bend_length));
        let r = length(across);
        // The jet's edge: pinched and bulged by the shock cells in air, fanning out in
        // vacuum, spreading out along the ground where it hits it, and pushed about by the
        // eddies once they have grown out of the nozzle's lip.
        let wobble = 1.0 + 0.14 * shock * cos(TAU * a / cell) * exp(-a / (5.0 * cell));
        let splash = smoothstep(ground - min(6.0 * re, 0.7 * ground), ground, p.y);
        let w = (re * mix(0.9, 1.0, expansion) + a * jet.heat.x) * wobble
            * (1.0 + params.bounds.w * splash);
        let rough = smoothstep(0.0, 6.0 * re, a) * (1.0 - 0.6 * expansion);
        let q = r / w;
        // The eddies stir the mixing layer at the edge more than the core.
        let stir = rough * e * smoothstep(0.1, 0.8, q);
        let edge = q * (1.0 - 0.45 * stir);
        if edge > 2.6 {
            continue;
        }
        let body = exp(-2.2 * edge * edge);
        let thin = re / w;
        let rho = level * body * thin * thin / re * max(1.0 + 0.8 * stir, 0.1) * end;

        // Shock cells: in each, the oblique shocks from the nozzle's lip cross on the axis
        // in a lozenge of brighter gas, and a jet well off the ambient pressure ends it in
        // a Mach disk with a glowing cone of reheated exhaust behind. Both fade downstream.
        let k = a / cell - 0.6;
        let phase = k - round(k);
        let cells = shock * exp(-a / (4.0 * cell)) * smoothstep(0.2 * cell, 0.5 * cell, a);
        let half_width = 0.8 * re * max(1.0 - 2.2 * abs(phase), 0.0) + 1e-4;
        let knot = exp(-3.0 * pow(r / half_width, 2.0)) * exp(-phase * phase / 0.05) * cells;
        let disk_radius = jet.disk.y * re * max(1.0 - 0.15 * round(k), 0.3);
        let behind = max(phase, 0.0);
        let disk = jet.disk.x * cells * exp(-max(round(k), 0.0) / 1.5)
            * (exp(-phase * phase / 0.0012) * (1.0 - smoothstep(0.8 * disk_radius, disk_radius, r))
                + step(0.0, phase) * exp(-behind / 0.12)
                    * exp(-pow(r / (disk_radius * max(1.0 - 2.0 * behind, 0.05)), 2.0)));

        // In air the exhaust afterburns with it, mixes and cools; in vacuum it cools as
        // it expands. The rim, mixing with cold air, runs cooler, and turbine exhaust
        // lining the nozzle wall is a cool, sooty collar until it catches fire. The shocks
        // heat the exhaust again.
        let air = 1.0 - expansion;
        let afterburn = air * smoothstep(0.0, jet.mix.y, a);
        let hot = jet.heat.w;
        let cooling = hot / (hot + max(a - 0.5 * hot, 0.0)) * pow(thin, 0.4 * expansion);
        let rim = 1.0 - 0.35 * air * smoothstep(0.5, 1.5, edge);
        let film_length = 2.5 * re * (1.0 + 0.3 * e);
        let collar = jet.mix.z * step(1e-4, params.smoke.w) * smoothstep(0.3, 0.7, q)
            * (1.0 - smoothstep(0.6 * film_length, film_length, a));
        let theta = mix(jet.heat.z, 1.0, afterburn) * cooling * rim * (1.0 - 0.3 * splash)
            * (1.0 + 0.06 * rough * e + 0.12 * stir) * (1.0 - 0.65 * collar)
            * (1.0 + 0.12 * (knot + disk));
        let dense = rho * (1.0 + 6.0 * collar);
        density += dense;
        soot += dense * jet.mix.x;
        condensed = max(condensed, expansion * smoothstep(3.0, 10.0, w / re));
        heat += dense * theta;
        width += dense * w;

        if glow {
            // The gas: a soft column, dimming as it widens (less so in air, where it keeps
            // burning) and with distance, shifting towards red as it cools.
            let fade = jet.flow.w;
            let along = exp(-a / fade) * (1.0 - 0.5 * splash) * end;
            let reddening = smoothstep(0.0, 2.0 * fade, a);
            let flame = params.flame.rgb
                * mix(vec3<f32>(1.0), vec3<f32>(1.0, 0.45, 0.25), reddening);
            let turbulence = mix(1.0, 1.0 + 0.55 * e, rough);
            // Potential core: bright, narrowing to the axis.
            let core_length = re * (2.0 + 6.0 * level) * (1.0 - 0.6 * expansion);
            let core_radius = re * max(1.0 - a / core_length, 0.0) + 1e-3;
            let core = exp(-pow(r / core_radius, 2.0)) * (1.0 - smoothstep(0.5, 1.0, a / core_length));
            // The turbine exhaust film is too cool to glow and hides the core inside it.
            gas += jet.heat.y * (1.0 - 0.95 * collar) * (flame * body * pow(thin, mix(1.0, 2.0, expansion)) * along
                * turbulence + params.core.rgb * core * 5.0
                + params.shock.rgb * (knot * 5.0 + disk * 100.0));
        }
    }

    var out: Medium;
    let theta = heat / max(density, 1e-12);
    out.width = width / max(density, 1e-12);
    let particles = PARTICLE_EXTINCTION * params.smoke.w * soot;
    // Water condenses once the exhaust has mixed down to a few hundred kelvin, or, in thin
    // air, once it has expanded and cooled a few nozzle widths out.
    let haze = HAZE_EXTINCTION * params.medium.y * density
        * max(smoothstep(0.2, 0.06, theta), condensed);
    out.extinction = particles + haze;
    out.scattering = particles * params.smoke.rgb + haze * HAZE_ALBEDO;
    out.emission = gas * params.core.w;
    let hottest = params.medium.x;
    if glow && hottest > 0.0 {
        let kelvin = AIR + (hottest - AIR) * theta;
        out.emission += particles * (1.0 - params.smoke.rgb) * blackbody(kelvin);
    }
    return out;
}

// --- Light on the smoke ---

/// Henyey-Greenstein phase function.
fn phase(cos_theta: f32, g: f32) -> f32 {
    let d = 1.0 + g * g - 2.0 * g * cos_theta;
    return (1.0 - g * g) / (4.0 * PI * d * sqrt(d));
}

/// Light from the sky and the ground around the vessel, averaged over all directions (the
/// sky light's cubemap; see `crate::sky_light`), cd/m².
fn surroundings() -> vec3<f32> {
    var light = vec3<f32>(0.0);
    @if(ENVIRONMENT_MAP) {
        let index = bindings::light_probes.view_cubemap_index;
        if index >= 0 {
            // Cubemaps are left-handed.
            let up = params.up.xyz * vec3<f32>(1.0, 1.0, -1.0);
            @if(MULTIPLE_LIGHT_PROBES_IN_ARRAY) {
                light = textureSampleLevel(bindings::diffuse_environment_maps[index],
                        bindings::environment_map_sampler, up, 0.0).rgb
                    + textureSampleLevel(bindings::diffuse_environment_maps[index],
                        bindings::environment_map_sampler, -up, 0.0).rgb;
            }
            @if(!MULTIPLE_LIGHT_PROBES_IN_ARRAY) {
                light = textureSampleLevel(bindings::diffuse_environment_map,
                        bindings::environment_map_sampler, up, 0.0).rgb
                    + textureSampleLevel(bindings::diffuse_environment_map,
                        bindings::environment_map_sampler, -up, 0.0).rgb;
            }
            light *= 0.5 * bindings::light_probes.intensity_for_view;
        }
    }
    return light;
}

/// Optical depth of the exhaust from `p` towards the sun (`sun`, metres, unit length).
fn towards_sun(p: vec3<f32>, sun: vec3<f32>, width: f32, extinction: f32) -> f32 {
    let steps = u32(params.quality.y);
    if steps == 0u {
        // Through about half the jet's width.
        return 0.5 * extinction * width;
    }
    let span = cone_span(p, sun, params.bounds.x, params.bounds.y, params.shape.x);
    let reach = min(max(span.y, 0.0), 4.0 * width + 2.0 * params.medium.z);
    let dt = reach / f32(steps);
    var depth = 0.0;
    for (var k = 0u; k < steps; k++) {
        let q = p + sun * (f32(k) + 0.5) * dt;
        depth += medium(q, 0.0, false).extinction * dt;
    }
    return depth;
}

/// Illuminance from the hot flame at `p`, lux: a glowing column on the axis.
fn flame_light(p: vec3<f32>) -> vec3<f32> {
    let hot = params.up.w;
    let radius = max(params.glow.w, 0.1);
    let nearest = vec3<f32>(0.0, clamp(p.y, 0.0, hot), 0.0);
    let d = max(length(p - nearest), radius);
    // Like a long cylinder up close, like a point far away.
    return params.glow.rgb * min(PI * radius / d, 2.0 * radius * hot / (d * d));
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> @location(0) vec4<f32> {
    let local_from_world = get_local_from_world(in.instance_index);
    let camera_world = bindings::view.world_position;
    let camera = (local_from_world * vec4<f32>(camera_world, 1.0)).xyz;
    let surface = (local_from_world * vec4<f32>(in.world_position.xyz, 1.0)).xyz;
    let inside = dot(camera.xz, camera.xz) < 1.0 && camera.y < 0.0 && camera.y > -1.0;
    if is_front == inside {
        discard;
    }
    // Into metres along the plume.
    let to_metres = vec3<f32>(params.shape.y, -params.shape.x, params.shape.y);
    let o = camera * to_metres;
    let d = (surface - camera) * to_metres;
    let span = cone_span(o, d, params.bounds.x, params.bounds.y, params.shape.x);
    let t0 = max(span.x, 0.0);
    let t1 = span.y;
    if t1 <= t0 {
        discard;
    }
    let metres_per_t = length(d);
    let ray = d / metres_per_t;
    let most = max(u32(params.quality.x), 1u);
    let middle = clamp((o + 0.5 * (t0 + t1) * d).y, 0.0, params.shape.x);
    let width = params.jets[0].exit.w + middle * params.jets[0].heat.x;
    let path = (t1 - t0) * metres_per_t;
    let steps = clamp(u32(ceil(path / (STEP_WIDTH * width))), min(MIN_STEPS, most), most);
    let dt = (t1 - t0) / f32(steps);
    let ds = dt * metres_per_t;

    // The sun, in the plume's metres and in lux; the light around; how wide a pixel is per
    // metre of distance.
    var sun = vec3<f32>(0.0, 1.0, 0.0);
    var sunlight = vec3<f32>(0.0);
    if bindings::lights.n_directional_lights > 0u {
        let to_sun = bindings::lights.directional_lights[0].direction_to_light;
        sun = normalize((local_from_world * vec4<f32>(to_sun, 0.0)).xyz * to_metres);
        sunlight = bindings::lights.directional_lights[0].color.rgb;
    }
    let ambient = surroundings();
    let cos_sun = dot(ray, sun);
    let pixel = 2.0 / (bindings::view.clip_from_view[1][1] * bindings::view.viewport.w);

    @if(TEMPORAL_JITTER)
    let jitter = interleaved_gradient_noise(in.position.xy, bindings::globals.frame_count);
    @else
    let jitter = interleaved_gradient_noise(in.position.xy, 0u);
    var t = t0 + dt * jitter;
    var light = vec3<f32>(0.0);
    var transmittance = 1.0;
    for (var i = 0u; i < steps; i++) {
        let p = o + t * d;
        let e = eddies(p, t * metres_per_t * pixel);
        let m = medium(p, e, true);
        var source = m.emission;
        if any(m.scattering > vec3<f32>(1e-6)) {
            // Sunlight through the smoke towards the sun, with a few orders of multiple
            // scattering (each fainter, less forward and less shadowed), plus the sky and
            // ground and the flame, through about half the smoke's width.
            let depth = towards_sun(p, sun, m.width, m.extinction);
            var direct = 0.0;
            var weight = 1.0;
            var spread = 1.0;
            for (var k = 0; k < 3; k++) {
                direct += weight
                    * (0.8 * phase(cos_sun, 0.6 * spread) + 0.2 * phase(cos_sun, -0.25 * spread))
                    * exp(-depth * spread);
                weight *= 0.5;
                spread *= 0.5;
            }
            // The flame shines from inside the jet, so the smoke around it shadows it more.
            let around = exp(-0.3 * m.extinction * m.width);
            let inside = exp(-m.extinction * m.width);
            source += m.scattering * (sunlight * direct + ambient * around
                + flame_light(p) / (4.0 * PI) * inside);
        }
        // Energy-conserving integration over the step.
        let through = exp(-m.extinction * ds);
        let integral = select(source * ds, source * (1.0 - through) / m.extinction,
            m.extinction > 1e-5);
        light += transmittance * integral;
        transmittance *= through;
        if transmittance < 0.003 {
            break;
        }
        t += dt;
    }
    return vec4<f32>(light * bindings::view.exposure, 1.0 - transmittance);
}
