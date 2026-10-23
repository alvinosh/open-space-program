// The sky and the ground seen from the floating origin, rendered into a cubemap that Bevy
// filters into image-based lighting (see `crate::sky_light`). Radiance is in the light's
// units (cd/m² for a sun given in lux) and not yet exposed. The sun's disc is left out:
// the directional light carries it.

import bevy_pbr::render::utils::sample_cube_dir;
import osp::shaders::atmosphere::{scatter, sphere_hits, sun_transmittance, PI, THICKNESS};

struct SkyLightParams {
    /// xyz: planet centre relative to the probe (render axes, m); w: mean radius.
    planet: vec4<f32>,
    /// xyz: direction to the sun; w: its illuminance above the atmosphere.
    sun: vec4<f32>,
    /// rgb: ground albedo; w: 1 for an Earth-like atmosphere.
    ground: vec4<f32>,
    /// x: ray-marching steps.
    options: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: SkyLightParams;
@group(0) @binding(1) var output: texture_storage_2d_array<rgba16float, write>;

/// Skylight on level ground with the sun high, as a share of the sunlight above the air.
const SKYLIGHT: vec3<f32> = vec3<f32>(0.05, 0.08, 0.13);

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
    let earthlike = params.ground.w > 0.5;
    // Where the terrain lies below the mean radius, the ground sphere still has to be
    // below the probe.
    let ground_radius = min(radius, length(eye) - 1.0);
    let ground = sphere_hits(eye, ray, ground_radius);
    let hits_ground = ground.x <= ground.y && ground.x > 0.0;

    var inscatter = vec3<f32>(0.0);
    var transmittance = vec3<f32>(1.0);
    if earthlike {
        let shell = sphere_hits(eye, ray, radius + THICKNESS);
        if shell.x <= shell.y && shell.y > 0.0 {
            var t1 = shell.y;
            if hits_ground {
                t1 = min(t1, ground.x);
            }
            let s = scatter(eye, ray, max(shell.x, 0.0), t1, sun, sun_color, radius, i32(params.options.x));
            inscatter = s.inscatter;
            transmittance = s.transmittance;
        }
    }

    var color = inscatter;
    if hits_ground {
        let p = eye + ray * ground.x;
        let mu = dot(normalize(p), sun);
        var light = vec3<f32>(max(mu, 0.0));
        if earthlike {
            light = sun_transmittance(p, sun, radius) * max(mu, 0.0)
                + SKYLIGHT * smoothstep(-0.1, 0.25, mu);
        }
        color += params.ground.rgb / PI * sun_color * light * transmittance;
    }
    textureStore(output, vec2<i32>(id.xy), i32(id.z), vec4<f32>(color, 1.0));
}
