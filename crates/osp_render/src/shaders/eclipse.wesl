// Darken what is in another body's shadow: everything opaque, by where the depth buffer says it
// is. A multiply over the scene, before the sky is laid on it.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import bevy_render::view::View;
import osp_render::shaders::eclipse_common::{Eclipse, eclipse_visibility};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> eclipse: Eclipse;
@group(0) @binding(2) var<uniform> sun: vec4<f32>;
@if(MULTISAMPLED)
@group(0) @binding(3) var depth_texture: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(0) @binding(3) var depth_texture: texture_depth_2d;

/// Sky light and what scatters around the shadow never quite goes out.
const LEAST: f32 = 0.004;

@fragment
fn main(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let depth = textureLoad(depth_texture, vec2<i32>(in.position.xy), 0);
    if depth <= 0.0 {
        discard;
    }
    let clip = view.clip_from_view;
    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let through = vec3<f32>((ndc.x + clip[2][0]) / clip[0][0], (ndc.y + clip[2][1]) / clip[1][1], -1.0);
    let world_from_view = mat3x3<f32>(
        view.world_from_view[0].xyz,
        view.world_from_view[1].xyz,
        view.world_from_view[2].xyz,
    );
    // Relative to the camera, as the occluders are here.
    let p = world_from_view * through * (clip[3][2] / depth);
    return vec4<f32>(max(eclipse_visibility(p, sun.xyz, eclipse), vec3<f32>(LEAST)), 1.0);
}
