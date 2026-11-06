// Light in a body's air: exponential layers of molecules and aerosols plus a band of ozone, lit
// by one sun. Shared by the sky and haze (`sky.wesl`), the terrain and the light probe.
//
// Positions are relative to the planet's centre, in metres. Everything about the air comes from
// the body's data through `air`, and three tables built by `osp_sim::scattering::Air` stand in
// for the slow integrals. Every material or pass that imports this module binds them at 110 to
// 114 in its material group. The CPU mirrors `sun_transmittance` and `scatter` in that module,
// where they are tested against a fine integral; keep the two in sync.

import osp_render::shaders::eclipse_common::{Eclipse, eclipse_visibility};

struct Air {
    /// rgb: scattering by molecules at the surface, per metre; w: 1 / its scale height.
    rayleigh: vec4<f32>,
    /// rgb: scattering by aerosols at the surface, per metre; w: 1 / its scale height.
    mie: vec4<f32>,
    /// rgb: aerosol scattering and absorption together.
    mie_extinction: vec4<f32>,
    /// rgb: how strongly aerosols scatter forward.
    mie_g: vec4<f32>,
    /// rgb: ozone absorption at its peak, per metre; w: the height of the peak.
    ozone: vec4<f32>,
    /// x: mean radius, y: radius of the top of the air, z: 1 / how far the ozone band reaches.
    shape: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(110) var transmittance_lut: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(111) var lut_sampler: sampler;
@group(constants::MATERIAL_BIND_GROUP) @binding(112) var multiple_lut: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(113) var irradiance_lut: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(114) var<uniform> air: Air;
/// How much cloud is where, in the body's fixed axes: cover, kind, high cloud, darkness.
@group(constants::MATERIAL_BIND_GROUP) @binding(115) var weather: texture_cube<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(116) var<uniform> clouds: Clouds;
/// The sky's march darkens the air under clouds, for shafts of light, which needs to know how the
/// body is turned this frame.
@if(CLOUD_RAYS)
@group(constants::MATERIAL_BIND_GROUP) @binding(117) var<uniform> air_local_from_render: mat3x3<f32>;
/// And so does what other bodies' shadows do to the air, relative to this body's centre.
@if(CLOUD_RAYS)
@group(constants::MATERIAL_BIND_GROUP) @binding(118) var<uniform> eclipse: Eclipse;
@if(CLOUD_RAYS)
@group(constants::MATERIAL_BIND_GROUP) @binding(119) var<uniform> eclipse_sun_direction: vec4<f32>;

struct Clouds {
    /// x: radius at which sunlight is taken to cross the layer, y: 1 if there are clouds.
    shell: vec4<f32>,
};

const PI: f32 = 3.141592653589793;
/// Optical depth of full cover, for sunlight on what is below (`osp_sim::clouds`).
const CLOUD_SHADOW_DEPTH: f32 = 3.0;
/// Of the sunlight clouds take out of the beam, how much still reaches the ground as diffuse light.
const CLOUD_THROUGH: f32 = 0.35;
/// The weather's mip level for shadows: their edges are soft at this height.
const CLOUD_SHADOW_LOD: f32 = 1.0;

/// Transmittance table size (sun angle, height) and how far below the mean surface it starts:
/// where sun rays are fully in the planet's shadow. `osp_sim::scattering` must match.
const TRANSMITTANCE_SIZE: vec2<f32> = vec2<f32>(256.0, 64.0);
const LUT_DEPTH: f32 = 6000.0;
const SHADOW_EDGE: f32 = 1000.0;
/// Size of the multiple scattering and sky irradiance tables.
const SKY_TABLE_SIZE: vec2<f32> = vec2<f32>(32.0, 32.0);

/// Relative densities of molecules, aerosols and ozone at a height above the surface.
fn densities(height: f32) -> vec3<f32> {
    let h = max(height, 0.0);
    let ozone = max(0.0, 1.0 - abs(h - air.ozone.w) * air.shape.z);
    return vec3<f32>(exp(-h * air.rayleigh.w), exp(-h * air.mie.w), ozone);
}

fn extinction(d: vec3<f32>) -> vec3<f32> {
    return air.rayleigh.rgb * d.x + air.mie_extinction.rgb * d.y + air.ozone.rgb * d.z;
}

/// Distances along a ray to where it enters and leaves a sphere centred on the origin; `x > y`
/// when it misses. Written to stay accurate for points near the sphere.
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

/// Where the optical depth from radius `r` towards the sun at `mu` (cosine from the zenith) is
/// stored in the transmittance table (Bruneton's parametrisation).
fn transmittance_uv(r_in: f32, mu: f32) -> vec2<f32> {
    let bottom = air.shape.x - LUT_DEPTH;
    let top = air.shape.y;
    let r = clamp(r_in, bottom, top);
    let horizon = sqrt((top - bottom) * (top + bottom));
    let rho = sqrt(max((r - bottom) * (r + bottom), 0.0));
    let d = max(-r * mu + sqrt(max((top - r) * (top + r) + r * r * mu * mu, 0.0)), 0.0);
    let d_min = top - r;
    let d_max = rho + horizon;
    let x = vec2<f32>((d - d_min) / (d_max - d_min), rho / horizon);
    // End texel centres on 0 and 1.
    return (0.5 + x * (TRANSMITTANCE_SIZE - 1.0)) / TRANSMITTANCE_SIZE;
}

/// Where the sky tables store a point at `p` with the sun in direction `sun`: uniform in the
/// sun's cosine, and the root of the height so low air gets more rows.
fn sky_table_uv(p: vec3<f32>, sun: vec3<f32>) -> vec2<f32> {
    let r = length(p);
    let v = sqrt(clamp((r - air.shape.x) / (air.shape.y - air.shape.x), 0.0, 1.0));
    let u = clamp(dot(p, sun) / max(r, 1.0), -1.0, 1.0) * 0.5 + 0.5;
    return (0.5 + vec2<f32>(u, v) * (SKY_TABLE_SIZE - 1.0)) / SKY_TABLE_SIZE;
}

/// Fraction of sunlight reaching a point: atmospheric extinction towards the sun, and the
/// planet's shadow. The shadow edge is softened over sun rays grazing 1 to 6 km below the mean
/// surface, which stands in for the sun's disc and refraction (and keeps ground-level points
/// fully lit while the sun is above their horizon).
fn sun_transmittance(p: vec3<f32>, sun: vec3<f32>) -> vec3<f32> {
    let radius = air.shape.x;
    let r = length(p);
    let b = dot(p, sun);
    // Lowest height of the sun ray above the centre, when the sun is below the horizon.
    let along = max(-b, 0.0);
    let closest = sqrt(max((r - along) * (r + along), 0.0));
    let shadow = smoothstep(radius - LUT_DEPTH, radius - SHADOW_EDGE, closest);
    if shadow <= 0.0 {
        return vec3<f32>(0.0);
    }
    let top = air.shape.y;
    var height = r;
    var mu = b / max(r, 1.0);
    if r > top {
        // From outside, the light only dims from where the ray enters the air.
        let hits = sphere_hits(p, sun, top);
        if hits.x > hits.y || hits.x <= 0.0 {
            return vec3<f32>(shadow);
        }
        height = top;
        mu = dot(p + sun * hits.x, sun) / top;
    }
    let uv = transmittance_uv(height, mu);
    let depth = textureSampleLevel(transmittance_lut, lut_sampler, uv, 0.0).rgb;
    return exp(-depth) * shadow;
}

/// The light the whole sky puts on level ground at `p`, per unit of sunlight.
fn sky_irradiance(p: vec3<f32>, sun: vec3<f32>) -> vec3<f32> {
    return textureSampleLevel(irradiance_lut, lut_sampler, sky_table_uv(p, sun), 0.0).rgb;
}

/// The share of direct sunlight that gets through the clouds to `p`. `local_from_world` turns
/// render axes to the body's fixed axes.
fn cloud_shadow(p: vec3<f32>, sun: vec3<f32>, local_from_world: mat3x3<f32>) -> f32 {
    let middle = clouds.shell.x;
    if clouds.shell.y <= 0.0 || dot(p, p) > middle * middle {
        return 1.0;
    }
    let hits = sphere_hits(p, sun, middle);
    if hits.y <= 0.0 {
        return 1.0;
    }
    let dir = local_from_world * (p + sun * hits.y);
    let cover = textureSampleLevel(weather, lut_sampler, dir, CLOUD_SHADOW_LOD).r;
    return exp(-CLOUD_SHADOW_DEPTH * cover);
}

/// What other bodies leave of the sunlight at a point.
@if(CLOUD_RAYS)
fn eclipse_at(p: vec3<f32>) -> vec3<f32> {
    return eclipse_visibility(p, eclipse_sun(), eclipse);
}

/// The Sun's direction for that: the same for all of a body, near enough.
@if(CLOUD_RAYS)
fn eclipse_sun() -> vec3<f32> {
    return eclipse_sun_direction.xyz;
}

struct Scattering {
    /// Light scattered towards the viewer (same units as `sun_color`).
    inscatter: vec3<f32>,
    /// Fraction of light from beyond the segment that gets through.
    transmittance: vec3<f32>,
};

/// What every step of a march needs and keeps adding to.
struct March {
    origin: vec3<f32>,
    dir: vec3<f32>,
    sun: vec3<f32>,
    /// Phase functions towards the eye, for molecules and aerosols.
    rayleigh: f32,
    mie: vec3<f32>,
    light: vec3<f32>,
    through: vec3<f32>,
};

/// One step of a march, from `a` to `b` along the ray. The source is taken at the middle and
/// integrated exactly against extinction over the step (Hillaire 2015), so thick air stays stable
/// however long the step is; thin air takes the series for it, since `1 - exp(-x)` loses all
/// its digits in f32.
fn march_step(march: ptr<function, March>, a: f32, b: f32) {
    let ds = b - a;
    let p = (*march).origin + (*march).dir * ((a + b) * 0.5);
    let d = densities(length(p) - air.shape.x);
    let e = extinction(d);
    let molecules = air.rayleigh.rgb * d.x;
    let aerosols = air.mie.rgb * d.y;
    var lit = sun_transmittance(p, (*march).sun);
    // All the orders above the first (Hillaire 2020).
    var more = textureSampleLevel(multiple_lut, lut_sampler, sky_table_uv(p, (*march).sun), 0.0).rgb;
    @if(CLOUD_RAYS) {
        let left = eclipse_at(p);
        lit *= cloud_shadow(p, (*march).sun, air_local_from_render) * left;
        more *= left;
    }
    let source = lit * (molecules * (*march).rayleigh + aerosols * (*march).mie)
        + more * (molecules + aerosols);
    let x = e * ds;
    let through = exp(-x);
    let integral = select(
        (1.0 - through) / max(e, vec3<f32>(1e-30)),
        ds * (1.0 - x * 0.5),
        x < vec3<f32>(1e-3),
    );
    (*march).light += (*march).through * source * integral;
    (*march).through *= through;
}

/// Scattering along the ray segment `origin + dir * [t0, t1]`, in up to `steps` steps (the effects
/// setting, see `atmosphere::view_steps`). Air is densest where a ray comes closest to the
/// planet and thins out both ways, so the segment is split there, half the steps to each side,
/// and they grow with the cube of their distance from it. Rays only leaving that point, like
/// those looking up from the ground, have just the one side to do.
fn scatter(
    origin: vec3<f32>,
    dir: vec3<f32>,
    t0: f32,
    t1: f32,
    sun: vec3<f32>,
    sun_color: vec3<f32>,
    steps: i32,
) -> Scattering {
    var result: Scattering;
    result.inscatter = vec3<f32>(0.0);
    result.transmittance = vec3<f32>(1.0);
    if t1 <= t0 {
        return result;
    }
    let mu = dot(dir, sun);
    let g = air.mie_g.rgb;
    let g2 = g * g;
    var march: March;
    march.origin = origin;
    march.dir = dir;
    march.sun = sun;
    march.rayleigh = 3.0 / (16.0 * PI) * (1.0 + mu * mu);
    march.mie = 3.0 / (8.0 * PI) * ((1.0 - g2) * (1.0 + mu * mu))
        / ((2.0 + g2) * pow(max(1.0 + g2 - 2.0 * g * mu, vec3<f32>(1e-9)), vec3<f32>(1.5)));
    march.light = vec3<f32>(0.0);
    march.through = vec3<f32>(1.0);

    let closest = clamp(-dot(origin, dir), t0, t1);
    let toward = closest - t0;
    let away = t1 - closest;
    let half = max(steps / 2, 1);
    let n = f32(half);
    if toward > 0.0 {
        for (var k = 0; k < half; k++) {
            let a = f32(half - k) / n;
            let b = f32(half - k - 1) / n;
            march_step(&march, closest - a * a * a * toward, closest - b * b * b * toward);
        }
    }
    if away > 0.0 {
        for (var k = 0; k < half; k++) {
            let a = f32(k) / n;
            let b = f32(k + 1) / n;
            march_step(&march, closest + a * a * a * away, closest + b * b * b * away);
        }
    }
    result.inscatter = sun_color * march.light;
    result.transmittance = march.through;
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
