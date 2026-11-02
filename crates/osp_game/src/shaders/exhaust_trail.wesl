// The smoke an engine leaves in the air (see `exhaust_trail.rs`): a soft round column drawn on
// a ribbon that faces the camera. How deep a ray goes through it follows from how far across it
// is; billows push its edge about and bump its light, and the sun and the sky light it as far
// as it hides what is behind. Output is premultiplied.

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings as bindings;

struct TrailParams {
    /// rgb: the smoke's albedo, w: seed of its billows.
    smoke: vec4<f32>,
    /// xyz: the local vertical (world axes).
    up: vec4<f32>,
};

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: TrailParams;

const PI: f32 = 3.14159265;
/// How far out the ribbon reaches, in radii (as in `exhaust_trail.rs`).
const MARGIN: f32 = 1.5;
/// How far the billows push the edge, as a share of the radius.
const BILLOWS: f32 = 0.3;

fn hash(p: vec3<f32>) -> f32 {
    let q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    let r = q + dot(q, q.yxz + 33.33);
    return fract((r.x + r.y) * r.z);
}

fn value_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
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

/// Billows at `p`, in radii, in about [-1, 1]: puffed-out lobes rather than a smooth wave.
fn billows(p: vec3<f32>) -> f32 {
    var sum = 0.0;
    var weight = 0.5;
    var q = p;
    for (var k = 0; k < 4; k++) {
        sum += weight * (1.0 - abs(2.0 * value_noise(q) - 1.0));
        weight *= 0.5;
        q = q * 2.03 + vec3<f32>(17.1, 3.7, 9.3);
    }
    return 2.0 * sum - 1.0;
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

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let u = in.uv.x * MARGIN;
    let along = in.uv.y;
    let depth = in.color.x;
    let age = in.color.y;
    let radius = in.color.z;

    let seed = params.smoke.w;
    let at = vec3<f32>(u * 1.3 + seed, along * 0.8, 0.012 * age + seed);
    let billow = billows(at);
    let edge = abs(u) * (1.0 - BILLOWS * billow);
    if edge >= 1.0 || depth <= 0.0 {
        discard;
    }
    let chord = sqrt(1.0 - edge * edge);

    let to_camera = bindings::view.world_position - in.world_position.xyz;
    let distance = length(to_camera);
    let facing = to_camera / max(distance, 1e-3);
    // Inside the column there is no telling a ribbon from fog: it fades instead of cutting.
    let near = smoothstep(0.1 * radius, 1.2 * radius, distance);
    let optical = depth * chord * (1.0 + 0.5 * billow);
    let thin = exp(-max(optical, 0.0));
    let alpha = (1.0 - thin) * near;
    if alpha < 0.002 {
        discard;
    }

    // The column's own surface, bumped by the lobes.
    let across = normalize(in.world_normal);
    let lengthwise = cross(across, facing);
    let bump = 0.6 * vec2<f32>(
        billows(at + vec3<f32>(0.15, 0.0, 0.0)) - billow,
        billows(at + vec3<f32>(0.0, 0.15, 0.0)) - billow,
    ) / 0.15;
    let normal = normalize(across * sign(u) * edge + facing * chord
        - 0.25 * (across * bump.x + lengthwise * bump.y));

    var sunlight = vec3<f32>(0.0);
    var sun = vec3<f32>(0.0, 1.0, 0.0);
    if bindings::lights.n_directional_lights > 0u {
        sun = bindings::lights.directional_lights[0].direction_to_light;
        sunlight = bindings::lights.directional_lights[0].color.rgb;
    }
    // Soft, as smoke is: light wraps round it, and it gets through where the column is thin.
    let wrap = saturate(0.5 + 0.5 * dot(normal, sun));
    let lit = wrap * wrap / PI;
    let forward = 0.25 * pow(saturate(dot(-facing, sun)), 6.0) * thin;
    let above = 0.75 + 0.25 * dot(normal, params.up.xyz);
    let light = params.smoke.rgb * (sunlight * (lit + forward) + surroundings() * above);
    return vec4<f32>(light * bindings::view.exposure * alpha, alpha);
}
