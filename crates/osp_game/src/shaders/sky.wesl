// The sky, and the atmosphere's glow and haze seen from space, drawn on a sphere just
// enclosing the atmosphere. From inside the sphere, only its far side shows (behind
// everything else), so it fills the sky. From outside, its near side is blended over the
// planet and space. Either way the light is integrated over the part of the view ray inside
// the atmosphere, which starts at the camera or where the ray enters it.
// Output is premultiplied: inscattered light plus how much of the background it hides.

import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::{view, lights},
    mesh_functions::get_world_from_local,
};
import osp::shaders::atmosphere::{
    scatter, sphere_hits, origin_sun_transmittance, luminance, THICKNESS, SKY_MESH_SCALE,
};

/// x: planet mean radius, y: ray-marching steps.
@group(constants::MATERIAL_BIND_GROUP) @binding(0) var<uniform> params: vec4<f32>;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> @location(0) vec4<f32> {
    let center = get_world_from_local(in.instance_index)[3].xyz;
    let radius = params.x;
    let top = radius + THICKNESS;
    let camera = view.world_position;
    let eye = camera - center;
    let ray = normalize(in.world_position.xyz - camera);
    // Which side of the mesh draws depends on where the camera is relative to the mesh,
    // not to the atmosphere: between the two (just above its top) the camera is outside the
    // atmosphere but still sees only the mesh's far side. From outside the mesh, the near
    // side does the whole ray; the far side would count it twice.
    let inside_mesh = length(eye) < top * SKY_MESH_SCALE;
    if !inside_mesh && !is_front {
        discard;
    }
    let shell = sphere_hits(eye, ray, top);
    if shell.x > shell.y || shell.y <= 0.0 {
        discard;
    }
    let t0 = max(shell.x, 0.0);
    var t1 = shell.y;
    let ground = sphere_hits(eye, ray, radius);
    if ground.x <= ground.y && ground.x > 0.0 {
        t1 = min(t1, ground.x);
    }
    let sun = lights.directional_lights[0].direction_to_light;
    let sun_color = lights.directional_lights[0].color.rgb
        / origin_sun_transmittance(center, sun, radius);
    let s = scatter(eye, ray, t0, t1, sun, sun_color, radius, i32(params.y));
    let coverage = clamp(1.0 - luminance(s.transmittance), 0.0, 1.0);
    return vec4<f32>(s.inscatter * view.exposure, coverage);
}
