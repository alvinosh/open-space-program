// Average the half-resolution clouds over time: find where each pixel's cloud was last frame, and
// blend with what was drawn there. What the history says is kept within what this frame's
// neighbours allow (variance clipping, Salvi 2016), which stops ghosts when the view or the scene
// changes under it.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import bevy_render::view::View;
import osp::shaders::clouds_types::{CloudDraw, view_ray};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> draw: CloudDraw;
@group(0) @binding(2) var current_light: texture_2d<f32>;
@group(0) @binding(3) var current_distance: texture_2d<f32>;
@group(0) @binding(4) var history: texture_2d<f32>;
@group(0) @binding(5) var history_sampler: sampler;

/// How much of each new frame goes into the history.
const NEW_SHARE: f32 = 0.12;
/// How far the history may be from the neighbours' mean, in standard deviations.
const CLIP: f32 = 1.4;
/// Where a pixel with no cloud is taken to be, m: only matters for the sky behind.
const FAR: f32 = 1e8;

@fragment
fn main(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.position.xy);
    let size = vec2<i32>(textureDimensions(current_light));
    let now = textureLoad(current_light, px, 0);

    var sum = vec4<f32>(0.0);
    var sum2 = vec4<f32>(0.0);
    var nearest = textureLoad(current_distance, px, 0).r;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let at = clamp(px + vec2<i32>(x, y), vec2<i32>(0), size - 1);
            let c = textureLoad(current_light, at, 0);
            sum += c;
            sum2 += c * c;
            // The cloud closest to the camera moves most: its edge shouldn't smear.
            let d = textureLoad(current_distance, at, 0).r;
            if d > 0.0 && (nearest <= 0.0 || d < nearest) {
                nearest = d;
            }
        }
    }
    if draw.moved.w > 0.5 {
        return now;
    }
    let mean = sum / 9.0;
    let spread = sqrt(max(sum2 / 9.0 - mean * mean, vec4<f32>(0.0)));
    let low = mean - spread * CLIP;
    let high = mean + spread * CLIP;

    let scaled = view_ray(in.uv, view.clip_from_view, view.world_from_view);
    let ray = normalize(scaled);
    let distance = select(FAR, nearest, nearest > 0.0);
    let then = draw.prev_view_from_world * (ray * distance + draw.moved.xyz);
    let clip = draw.prev_clip_from_view * vec4<f32>(then, 1.0);
    if clip.w <= 0.0 {
        return now;
    }
    let ndc = clip.xy / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) {
        return now;
    }
    let old = clamp(textureSampleLevel(history, history_sampler, uv, 0.0), low, high);
    return mix(old, now, NEW_SHARE);
}
