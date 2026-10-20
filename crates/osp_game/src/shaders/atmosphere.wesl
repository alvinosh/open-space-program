// Single scattering in an Earth-like atmosphere: exponential Rayleigh and Mie layers plus
// an ozone band, lit by one sun. Shared by the terrain (aerial perspective) and the sky.
//
// Positions are relative to the planet centre, in metres. `crate::atmosphere` mirrors
// `sun_transmittance` on the CPU; keep the two in sync.

const RAYLEIGH: vec3<f32> = vec3<f32>(5.802e-6, 13.558e-6, 33.1e-6);
const RAYLEIGH_HEIGHT: f32 = 8000.0;
const MIE_SCATTERING: f32 = 3.996e-6;
const MIE_EXTINCTION: f32 = 4.44e-6;
const MIE_HEIGHT: f32 = 1200.0;
const MIE_G: f32 = 0.76;
const OZONE: vec3<f32> = vec3<f32>(0.650e-6, 1.881e-6, 0.085e-6);
/// Height of the top of the scattering atmosphere above the surface.
const THICKNESS: f32 = 100000.0;
const PI: f32 = 3.141592653589793;

/// Light scattered more than once, as a multiple of single Rayleigh scattering. Without it
/// the daytime sky comes out too dark and too deep a blue.
const MULTIPLE_SCATTERING: f32 = 1.6;

const VIEW_STEPS: i32 = 12;
const SUN_STEPS: i32 = 6;

/// Relative densities of air, aerosols and ozone at a height above the surface.
fn densities(height: f32) -> vec3<f32> {
    let h = max(height, 0.0);
    let ozone = max(0.0, 1.0 - abs(h - 25000.0) / 15000.0);
    return vec3<f32>(exp(-h / RAYLEIGH_HEIGHT), exp(-h / MIE_HEIGHT), ozone);
}

fn extinction(d: vec3<f32>) -> vec3<f32> {
    return RAYLEIGH * d.x + vec3<f32>(MIE_EXTINCTION * d.y) + OZONE * d.z;
}

/// Distances along a ray to where it enters and leaves a sphere centred on the origin;
/// `x > y` when it misses. Written to stay accurate for points near the sphere.
fn sphere_hits(origin: vec3<f32>, dir: vec3<f32>, radius: f32) -> vec2<f32> {
    let b = dot(origin, dir);
    let r = length(origin);
    let c = (r - radius) * (r + radius);
    let disc = b * b - c;
    if disc < 0.0 {
        return vec2<f32>(1.0, -1.0);
    }
    let s = sqrt(disc);
    return vec2<f32>(-b - s, -b + s);
}

/// Fraction of sunlight reaching a point: atmospheric extinction towards the sun, and the
/// planet's shadow. The shadow edge is softened over sun rays grazing 1 to 6 km below the
/// mean surface, which stands in for the sun's disc and refraction (and keeps ground-level
/// points fully lit while the sun is above their horizon).
fn sun_transmittance(p: vec3<f32>, sun: vec3<f32>, radius: f32) -> vec3<f32> {
    let r = length(p);
    let b = dot(p, sun);
    // Lowest height of the sun ray above the centre, when the sun is below the horizon.
    let along = max(-b, 0.0);
    let closest = sqrt(max((r - along) * (r + along), 0.0));
    let shadow = smoothstep(radius - 6000.0, radius - 1000.0, closest);
    if shadow <= 0.0 {
        return vec3<f32>(0.0);
    }
    let top = radius + THICKNESS;
    let exit = max(sphere_hits(p, sun, top).y, 0.0);
    let ds = exit / f32(SUN_STEPS);
    var depth = vec3<f32>(0.0);
    for (var i = 0; i < SUN_STEPS; i++) {
        let q = p + sun * (ds * (f32(i) + 0.5));
        depth += extinction(densities(length(q) - radius)) * ds;
    }
    return exp(-depth) * shadow;
}

struct Scattering {
    /// Light scattered towards the viewer (same units as `sun_color`).
    inscatter: vec3<f32>,
    /// Fraction of light from beyond the segment that gets through.
    transmittance: vec3<f32>,
};

/// Scattering along the ray segment `origin + dir * [t0, t1]`.
fn scatter(
    origin: vec3<f32>,
    dir: vec3<f32>,
    t0: f32,
    t1: f32,
    sun: vec3<f32>,
    sun_color: vec3<f32>,
    radius: f32,
) -> Scattering {
    var result: Scattering;
    result.inscatter = vec3<f32>(0.0);
    result.transmittance = vec3<f32>(1.0);
    if t1 <= t0 {
        return result;
    }
    let ds = (t1 - t0) / f32(VIEW_STEPS);
    var transmittance = vec3<f32>(1.0);
    var rayleigh = vec3<f32>(0.0);
    var mie = vec3<f32>(0.0);
    for (var i = 0; i < VIEW_STEPS; i++) {
        let p = origin + dir * (t0 + ds * (f32(i) + 0.5));
        let d = densities(length(p) - radius);
        let step = exp(-extinction(d) * ds);
        // Light reaching the middle of the step, scattered there, attenuated back to the eye.
        let lit = sun_transmittance(p, sun, radius) * transmittance * sqrt(step) * ds;
        rayleigh += lit * d.x;
        mie += lit * d.y;
        transmittance *= step;
    }
    let mu = dot(dir, sun);
    let phase_r = 3.0 / (16.0 * PI) * (1.0 + mu * mu);
    let g2 = MIE_G * MIE_G;
    let phase_m = 3.0 / (8.0 * PI) * ((1.0 - g2) * (1.0 + mu * mu))
        / ((2.0 + g2) * pow(1.0 + g2 - 2.0 * MIE_G * mu, 1.5));
    let phase_r_ms = phase_r + MULTIPLE_SCATTERING / (4.0 * PI);
    result.inscatter = sun_color * (rayleigh * RAYLEIGH * phase_r_ms + mie * MIE_SCATTERING * phase_m);
    result.transmittance = transmittance;
    return result;
}

/// The sun's colour and direction, undoing the CPU's dimming of the light for the
/// atmosphere at the floating origin (see `crate::atmosphere`), so each fragment can apply
/// its own.
fn origin_sun_transmittance(planet_center: vec3<f32>, sun: vec3<f32>, radius: f32) -> vec3<f32> {
    return max(sun_transmittance(-planet_center, sun, radius), vec3<f32>(1e-4));
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
