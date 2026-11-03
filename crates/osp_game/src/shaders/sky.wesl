// The sky and the haze: one pass over the whole view per body with air in sight, after the
// opaque scene and before anything transparent. Each pixel's ray is integrated from where it
// enters the air to whatever the depth buffer says it hits, so the ground, vessels, trees and
// buildings all sit in the same haze, and a planet's glow from space is the same code as the
// sky from its surface.
//
// Output goes through dual-source blending: `inscatter + background * transmittance`, per colour.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import bevy_render::view::View;
import osp::shaders::atmosphere::{scatter, sphere_hits, air, luminance};

@if(DUAL_SOURCE_BLENDING)
enable dual_source_blending;

struct SkyDraw {
    /// xyz: the camera relative to the body's centre (render axes, m); w: ray-marching steps.
    eye: vec4<f32>,
    /// xyz: direction to the sun from the body; w: its illuminance above the air.
    sun: vec4<f32>,
};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> draw: SkyDraw;
@if(MULTISAMPLED)
@group(0) @binding(2) var depth_texture: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(0) @binding(2) var depth_texture: texture_depth_2d;

/// How fast stars disappear with the brightness of the sky in front of them, as seen after
/// exposure: they are drawn at a fixed screen brightness rather than their real one, and the
/// sky's own light would not hide them. A blue sky hides them at any height, and they come out
/// as it goes dark: half gone at 0.2% of screen white, which is about 35 km up by day.
const STARS_FADE: f32 = 300.0;

struct Output {
    @if(DUAL_SOURCE_BLENDING)
    @location(0) @blend_src(0) inscatter: vec4<f32>,
    @if(DUAL_SOURCE_BLENDING)
    @location(0) @blend_src(1) transmittance: vec4<f32>,
    @if(!DUAL_SOURCE_BLENDING)
    @location(0) inscatter: vec4<f32>,
};

fn shade(in: FullscreenVertexOutput, depth: f32) -> Output {
    // The ray through this pixel, through the projection so it follows TAA's jitter. The
    // projection is infinite and reversed: the depth is the near plane over the distance
    // along the view axis, which keeps its precision at any range.
    let clip = view.clip_from_view;
    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let through = vec3<f32>(
        (ndc.x + clip[2][0]) / clip[0][0],
        (ndc.y + clip[2][1]) / clip[1][1],
        -1.0,
    );
    let world_from_view = mat3x3<f32>(
        view.world_from_view[0].xyz,
        view.world_from_view[1].xyz,
        view.world_from_view[2].xyz,
    );
    let ray = normalize(world_from_view * through);

    let eye = draw.eye.xyz;
    let shell = sphere_hits(eye, ray, air.shape.y);
    if shell.x > shell.y || shell.y <= 0.0 {
        discard;
    }
    let t0 = max(shell.x, 0.0);
    var t1 = shell.y;
    var solid = depth > 0.0;
    if solid {
        t1 = min(t1, clip[3][2] / depth * length(through));
    } else {
        // Nothing drawn here, but the planet may still be in the way: while its terrain loads,
        // and past the edge of a coarse chunk, which cuts inside the true sphere.
        let ground = sphere_hits(eye, ray, air.shape.x);
        if ground.x <= ground.y && ground.x > 0.0 {
            t1 = min(t1, ground.x);
            solid = true;
        }
    }
    if t1 <= t0 {
        discard;
    }
    let sun = draw.sun.xyz;
    let s = scatter(eye, ray, t0, t1, sun, vec3<f32>(draw.sun.w), i32(draw.eye.w));
    let inscatter = s.inscatter * view.exposure;
    var transmittance = s.transmittance;
    if !solid {
        // What is behind is only the stars.
        transmittance *= exp(-STARS_FADE * luminance(inscatter));
    } else if depth <= 0.0 {
        transmittance = vec3<f32>(0.0);
    }

    var out: Output;
    @if(DUAL_SOURCE_BLENDING) {
        out.inscatter = vec4<f32>(inscatter, 0.0);
        out.transmittance = vec4<f32>(transmittance, 1.0);
    }
    @if(!DUAL_SOURCE_BLENDING) {
        out.inscatter = vec4<f32>(inscatter, (transmittance.r + transmittance.g + transmittance.b) / 3.0);
    }
    return out;
}

@if(MULTISAMPLED)
@fragment
fn main(in: FullscreenVertexOutput, @builtin(sample_index) sample: u32) -> Output {
    return shade(in, textureLoad(depth_texture, vec2<i32>(in.position.xy), i32(sample)));
}

@if(!MULTISAMPLED)
@fragment
fn main(in: FullscreenVertexOutput) -> Output {
    return shade(in, textureLoad(depth_texture, vec2<i32>(in.position.xy), 0));
}
