// The sky, and the atmosphere's glow and haze seen from space, drawn on a sphere enclosing
// the atmosphere. From inside, only its far side shows (behind everything else), so it
// fills the sky. From outside, its near side is blended over the planet and space.
// Output is premultiplied: inscattered light plus how much of the background it hides.

import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::{view, lights},
    mesh_functions::get_world_from_local,
};
import osp::shaders::atmosphere::{
    scatter, sphere_hits, origin_sun_transmittance, luminance, THICKNESS,
};

/// x: planet mean radius.
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: vec4<f32>;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> @location(0) vec4<f32> {
    let center = get_world_from_local(in.instance_index)[3].xyz;
    let radius = params.x;
    let top = radius + THICKNESS;
    let camera = view.world_position;
    let eye = camera - center;
    let ray = normalize(in.world_position.xyz - camera);
    let inside = length(eye) < top;
    // From outside, the near side does the whole ray; the far side would count it twice.
    if !inside && !is_front {
        discard;
    }
    let shell = sphere_hits(eye, ray, top);
    if shell.x > shell.y || shell.y <= 0.0 {
        discard;
    }
    let t0 = select(shell.x, 0.0, inside);
    var t1 = shell.y;
    let ground = sphere_hits(eye, ray, radius);
    if ground.x <= ground.y && ground.x > 0.0 {
        t1 = min(t1, ground.x);
    }
    let sun = lights.directional_lights[0].direction_to_light;
    let sun_color = lights.directional_lights[0].color.rgb
        / origin_sun_transmittance(center, sun, radius);
    let s = scatter(eye, ray, t0, t1, sun, sun_color, radius);
    let coverage = clamp(1.0 - luminance(s.transmittance), 0.0, 1.0);
    return vec4<f32>(s.inscatter * view.exposure, coverage);
}
