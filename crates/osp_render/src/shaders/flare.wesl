// What the Sun does to a lens: a soft glare, a starburst from the aperture, and a line of faint
// ghosts through the middle of the frame. It follows how much of the Sun's disc can be seen, read
// from the depth buffer around it, so it dies behind a vessel or a hill rather than popping.

import bevy_render::view::View;

struct Flare {
    /// xy: the Sun on screen (0 to 1), z: its radius on screen as a share of the height, w: aspect.
    place: vec4<f32>,
    /// rgb: the light getting through the air, w: depth of the Sun's disc.
    tint: vec4<f32>,
};

@group(0) @binding(0) var<uniform> flare: Flare;
@if(MULTISAMPLED)
@group(0) @binding(1) var depth_texture: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(0) @binding(1) var depth_texture: texture_depth_2d;
@group(0) @binding(2) var clouds_texture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    /// How much of the Sun's disc is in sight.
    @location(1) @interpolate(flat) seen: f32,
};

/// Points on the disc read for how much of it shows.
const SAMPLES: i32 = 32;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    var out: VertexOutput;
    out.uv = vec2<f32>(f32(index >> 1u), f32(index & 1u)) * 2.0;
    out.position = vec4<f32>(out.uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);

    let size = vec2<f32>(textureDimensions(depth_texture));
    let centre = flare.place.xy * size;
    // No smaller than a few pixels, so a far-off Sun doesn't flicker on one pixel of a tower.
    let radius = max(flare.place.z * size.y, 3.0);
    var seen = 0.0;
    for (var i = 0; i < SAMPLES; i++) {
        // A sunflower over the disc.
        let k = (f32(i) + 0.5) / f32(SAMPLES);
        let around = 2.399963 * f32(i);
        let at = centre + vec2<f32>(cos(around), sin(around)) * sqrt(k) * radius;
        if all(at >= vec2<f32>(0.0)) && all(at < size) {
            // Reversed depth: anything nearer than the Sun is larger.
            if textureLoad(depth_texture, vec2<i32>(at), 0) <= flare.tint.w * 1.5 {
                seen += 1.0;
            }
        }
    }
    let clouds = textureSampleLevel(clouds_texture, cloud_sampler, flare.place.xy, 0.0).a;
    out.seen = seen / f32(SAMPLES) * clouds;
    return out;
}

@group(0) @binding(3) var cloud_sampler: sampler;

fn ghost(at: vec2<f32>, centre: vec2<f32>, radius: f32, soft: f32) -> f32 {
    let d = distance(at, centre);
    // A disc with a brighter rim, as an aperture leaves it.
    let disc = 1.0 - smoothstep(radius * (1.0 - soft), radius, d);
    let rim = smoothstep(radius * 0.7, radius * 0.97, d);
    return disc * (0.4 + 0.6 * rim);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    if in.seen <= 0.002 {
        discard;
    }
    let aspect = flare.place.w;
    let at = (in.uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let sun = (flare.place.xy - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0);
    let from_sun = at - sun;
    let d = length(from_sun);

    // A wide soft glare, then six thin spikes from the aperture's blades.
    let glare = 0.012 / (d * d * 6.0 + 0.012) * 0.28;
    let angle = atan2(from_sun.y, from_sun.x);
    let spikes = pow(abs(cos(angle * 3.0 + 0.4)), 900.0) * exp(-d * 11.0) * 0.5;
    let fine = pow(abs(cos(angle * 3.0 + 1.45)), 1600.0) * exp(-d * 18.0) * 0.2;

    // Ghosts along the line through the middle of the frame.
    var ghosts = vec3<f32>(0.0);
    ghosts += vec3<f32>(0.35, 0.6, 1.0) * ghost(at, sun * -0.45, 0.055, 0.25) * 0.05;
    ghosts += vec3<f32>(1.0, 0.75, 0.4) * ghost(at, sun * -0.8, 0.09, 0.12) * 0.035;
    ghosts += vec3<f32>(0.5, 1.0, 0.7) * ghost(at, sun * 0.4, 0.03, 0.4) * 0.05;
    ghosts += vec3<f32>(0.75, 0.5, 1.0) * ghost(at, sun * -1.25, 0.14, 0.08) * 0.025;

    let color = flare.tint.rgb * in.seen * (vec3<f32>(1.0, 0.96, 0.9) * (glare + spikes + fine) + ghosts);
    return vec4<f32>(color, 0.0);
}
