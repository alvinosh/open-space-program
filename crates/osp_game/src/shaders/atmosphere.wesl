// Single scattering in an Earth-like atmosphere: exponential Rayleigh and Mie layers plus
// an ozone band, lit by one sun. Shared by the terrain (aerial perspective) and the sky.
//
// Positions are relative to the planet centre, in metres. Sunlight transmittance comes
// from a table built by `crate::atmosphere::TransmittanceLut`, which every material that
// imports this module binds at 110 (texture) and 111 (sampler). The CPU mirrors
// `sun_transmittance`; keep the two in sync.

@group(constants::MATERIAL_BIND_GROUP) @binding(110) var transmittance_lut: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(111) var transmittance_sampler: sampler;

const RAYLEIGH: vec3<f32> = vec3<f32>(5.802e-6, 13.558e-6, 33.1e-6);
const RAYLEIGH_HEIGHT: f32 = 8000.0;
const MIE_SCATTERING: f32 = 3.996e-6;
const MIE_EXTINCTION: f32 = 4.44e-6;
const MIE_HEIGHT: f32 = 1200.0;
const MIE_G: f32 = 0.76;
const OZONE: vec3<f32> = vec3<f32>(0.650e-6, 1.881e-6, 0.085e-6);
/// Height of the top of the scattering atmosphere above the surface.
const THICKNESS: f32 = 100000.0;
/// The sky mesh's radius as a multiple of the atmosphere's top: just enough for its flat
/// faces to clear the shell. While the camera is inside the mesh the sky draws only behind
/// everything (its far side), so the terrain hazes itself; outside, the mesh's near side
/// hazes it. `crate::atmosphere::SKY_MESH_SCALE` must match.
const SKY_MESH_SCALE: f32 = 1.002;
const PI: f32 = 3.141592653589793;

/// Light scattered more than once, as a multiple of single Rayleigh scattering. Without it
/// the daytime sky comes out too dark and too deep a blue.
const MULTIPLE_SCATTERING: f32 = 1.6;

/// Transmittance table size (sun angle, height) and how far below the mean surface it
/// starts: where sun rays are fully in the planet's shadow.
const LUT_SIZE: vec2<f32> = vec2<f32>(256.0, 64.0);
const LUT_DEPTH: f32 = 6000.0;

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

/// Where the optical depth from radius `r` towards the sun at `mu` (cosine from the zenith)
/// is stored in the transmittance table (Bruneton's parametrisation).
fn transmittance_uv(r_in: f32, mu: f32, radius: f32) -> vec2<f32> {
    let bottom = radius - LUT_DEPTH;
    let top = radius + THICKNESS;
    let r = clamp(r_in, bottom, top);
    let horizon = sqrt((top - bottom) * (top + bottom));
    let rho = sqrt(max((r - bottom) * (r + bottom), 0.0));
    let d = max(-r * mu + sqrt(max((top - r) * (top + r) + r * r * mu * mu, 0.0)), 0.0);
    let d_min = top - r;
    let d_max = rho + horizon;
    let x = vec2<f32>((d - d_min) / (d_max - d_min), rho / horizon);
    // End texel centres on 0 and 1.
    return (0.5 + x * (LUT_SIZE - 1.0)) / LUT_SIZE;
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
    let shadow = smoothstep(radius - LUT_DEPTH, radius - 1000.0, closest);
    if shadow <= 0.0 {
        return vec3<f32>(0.0);
    }
    let top = radius + THICKNESS;
    var height = r;
    var mu = b / max(r, 1.0);
    if r > top {
        // From outside, the light only dims from where the ray enters the atmosphere.
        let hits = sphere_hits(p, sun, top);
        if hits.x > hits.y || hits.x <= 0.0 {
            return vec3<f32>(shadow);
        }
        height = top;
        mu = dot(p + sun * hits.x, sun) / top;
    }
    let uv = transmittance_uv(height, mu, radius);
    let depth = textureSampleLevel(transmittance_lut, transmittance_sampler, uv, 0.0).rgb;
    return exp(-depth) * shadow;
}

struct Scattering {
    /// Light scattered towards the viewer (same units as `sun_color`).
    inscatter: vec3<f32>,
    /// Fraction of light from beyond the segment that gets through.
    transmittance: vec3<f32>,
};

/// A segment this long, or climbing this much, gets all of `scatter`'s steps, m.
const FULL_STEPS_LENGTH: f32 = 20000.0;
const FULL_STEPS_CLIMB: f32 = 2000.0;

/// Scattering along the ray segment `origin + dir * [t0, t1]`, integrated in up to `steps`
/// steps (the effects setting, see `atmosphere::view_steps`). Short, level segments (the
/// ground near the camera) cross air of nearly even density and get as few as two.
fn scatter(
    origin: vec3<f32>,
    dir: vec3<f32>,
    t0: f32,
    t1: f32,
    sun: vec3<f32>,
    sun_color: vec3<f32>,
    radius: f32,
    steps: i32,
) -> Scattering {
    var result: Scattering;
    result.inscatter = vec3<f32>(0.0);
    result.transmittance = vec3<f32>(1.0);
    if t1 <= t0 {
        return result;
    }
    let most = max(steps, 1);
    let climb = abs(length(origin + dir * t1) - length(origin + dir * t0));
    let share = max((t1 - t0) / FULL_STEPS_LENGTH, climb / FULL_STEPS_CLIMB);
    let n = clamp(i32(ceil(f32(most) * share)), min(2, most), most);
    let ds = (t1 - t0) / f32(n);
    var transmittance = vec3<f32>(1.0);
    var rayleigh = vec3<f32>(0.0);
    var mie = vec3<f32>(0.0);
    for (var i = 0; i < n; i++) {
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

/// The sun's illuminance above the air: `lux::RAW_SUNLIGHT`, which the CPU keeps as the
/// light's illuminance while its colour carries the dimming.
const SUNLIGHT: f32 = 130000.0;

/// How much the CPU dimmed the sun's light (`light_color`) for the air around the viewed
/// vessel (see `crate::atmosphere::dim_sunlight`), so each fragment can divide it out and
/// apply its own. Read from the light rather than recomputed: the vessel need not be at the
/// render origin, nor near this planet.
fn light_transmittance(light_color: vec3<f32>) -> vec3<f32> {
    return max(light_color / SUNLIGHT, vec3<f32>(1e-4));
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}
