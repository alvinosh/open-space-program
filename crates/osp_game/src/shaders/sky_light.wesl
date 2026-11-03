// The sky and the ground seen from the viewed vessel, rendered into a cubemap that Bevy
// filters into image-based lighting (see `crate::sky_light`). Radiance is in the light's
// units (cd/m² for a sun given in lux) and not yet exposed. The sun's disc is left out:
// the directional light carries it.

import bevy_pbr::render::utils::sample_cube_dir;
import osp::shaders::atmosphere::{
    air, clouds, weather, lut_sampler, scatter, sky_irradiance, sphere_hits, sun_transmittance,
    CLOUD_THROUGH, PI,
};

struct SkyLightParams {
    /// xyz: planet centre relative to the probe (render axes, m); w: mean radius.
    planet: vec4<f32>,
    /// xyz: direction to the sun; w: its illuminance above the atmosphere.
    sun: vec4<f32>,
    /// rgb: ground albedo; w: 1 if the body has air.
    ground: vec4<f32>,
    /// x: ray-marching steps.
    options: vec4<f32>,
    /// The body's fixed axes from render axes.
    local_from_render: mat3x3<f32>,
};

/// What a sunlit cloud top reflects, seen from above.
const CLOUD_TOP_ALBEDO: f32 = 0.7;

@group(0) @binding(0) var<uniform> params: SkyLightParams;
@group(0) @binding(1) var output: texture_storage_2d_array<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(output);
    if id.x >= size.x || id.y >= size.y || id.z >= 6u {
        return;
    }
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(size);
    var ray = sample_cube_dir(uv, id.z);
    // Bevy's cubemaps are left-handed.
    ray.z = -ray.z;

    let radius = params.planet.w;
    let eye = -params.planet.xyz;
    let sun = params.sun.xyz;
    let sun_color = vec3<f32>(params.sun.w);
    let has_air = params.ground.w > 0.5;
    // Where the terrain lies below the mean radius, the ground sphere still has to be
    // below the probe.
    let ground_radius = min(radius, length(eye) - 1.0);
    let ground = sphere_hits(eye, ray, ground_radius);
    let hits_ground = ground.x <= ground.y && ground.x > 0.0;

    var inscatter = vec3<f32>(0.0);
    var transmittance = vec3<f32>(1.0);
    if has_air {
        let shell = sphere_hits(eye, ray, air.shape.y);
        if shell.x <= shell.y && shell.y > 0.0 {
            var t1 = shell.y;
            if hits_ground {
                t1 = min(t1, ground.x);
            }
            let s = scatter(eye, ray, max(shell.x, 0.0), t1, sun, sun_color, i32(params.options.x));
            inscatter = s.inscatter;
            transmittance = s.transmittance;
        }
    }

    var color = inscatter;
    // From under the clouds, they are what fills the sky: the sunlight they scatter down.
    if has_air && !hits_ground && clouds.shell.y > 0.0 && length(eye) < clouds.shell.x {
        let hit = eye + ray * sphere_hits(eye, ray, clouds.shell.x).y;
        let cover = textureSampleLevel(weather, lut_sampler, params.local_from_render * hit, 2.0).r;
        let mu = max(dot(normalize(hit), sun), 0.0);
        let underside = sun_transmittance(hit, sun) * mu * CLOUD_THROUGH / PI * sun_color;
        color = mix(color, underside + sky_irradiance(hit, sun) * sun_color / PI * 0.6, cover);
    }
    if hits_ground {
        let p = eye + ray * ground.x;
        let mu = dot(normalize(p), sun);
        var light = vec3<f32>(max(mu, 0.0));
        if has_air {
            light = sun_transmittance(p, sun) * max(mu, 0.0) + sky_irradiance(p, sun);
        }
        // From above the clouds, what looks like ground is largely their tops.
        var albedo = params.ground.rgb;
        if clouds.shell.y > 0.0 && length(eye) > clouds.shell.x {
            let cover = textureSampleLevel(weather, lut_sampler, params.local_from_render * p, 2.0).r;
            albedo = mix(albedo, vec3<f32>(CLOUD_TOP_ALBEDO), cover);
        }
        color += albedo / PI * sun_color * light * transmittance;
    }
    textureStore(output, vec2<i32>(id.xy), i32(id.z), vec4<f32>(color, 1.0));
}
