// Lay the half-resolution clouds over the full-resolution scene. A pixel takes its four nearest
// cloud samples, weighted by how close they are and by how similar a depth each saw: a sample that
// stopped at the vessel says nothing about the sky just past its edge, and the other way round.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

@group(0) @binding(0) var clouds_texture: texture_2d<f32>;
@if(MULTISAMPLED)
@group(0) @binding(1) var depth_texture: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(0) @binding(1) var depth_texture: texture_depth_2d;

/// How quickly a sample loses weight as its depth differs, by relative difference.
const DEPTH_SHARPNESS: f32 = 40.0;

@fragment
fn main(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let full = vec2<i32>(in.position.xy);
    let size = vec2<i32>(textureDimensions(clouds_texture));
    let here = textureLoad(depth_texture, full, 0);

    let st = in.position.xy * 0.5 - 0.5;
    let base = floor(st);
    let f = st - base;
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    var best = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    var best_score = -1.0;
    for (var y = 0; y <= 1; y++) {
        for (var x = 0; x <= 1; x++) {
            let at = clamp(vec2<i32>(base) + vec2<i32>(x, y), vec2<i32>(0), size - 1);
            let d = textureLoad(depth_texture, at * 2, 0);
            let wx = select(1.0 - f.x, f.x, x == 1);
            let wy = select(1.0 - f.y, f.y, y == 1);
            let different = abs(d - here) / max(max(d, here), 1e-12);
            let similar = exp(-different * DEPTH_SHARPNESS);
            let c = textureLoad(clouds_texture, at, 0);
            let weight = wx * wy * similar + 1e-5 * wx * wy;
            sum += c * weight;
            total += weight;
            if similar > best_score {
                best_score = similar;
                best = c;
            }
        }
    }
    // Nothing nearby saw this depth: a sliver thinner than a cloud pixel.
    if best_score < 0.05 {
        return best;
    }
    return sum / total;
}
