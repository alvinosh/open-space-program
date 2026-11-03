// What the cloud passes share (`shaders/clouds*.wesl`, `crate::clouds`).

struct CloudDraw {
    /// xyz: the camera relative to the body's centre (render axes, m); w: most steps along a ray.
    eye: vec4<f32>,
    /// xyz: direction to the sun; w: its illuminance above the air.
    sun: vec4<f32>,
    /// The body's fixed axes (Y to its pole) from render axes.
    local_from_render: mat3x3<f32>,
    /// xyz: how far the wind has carried the noise (m, wrapped to its period); w: steps towards the
    /// sun.
    tile: vec4<f32>,
    /// x: radius of the bottom of the layer, y: of its top, z: extinction in dense cloud, per
    /// metre, w: radius of the thin high layer (0 for none).
    shell: vec4<f32>,
    /// rgb: the share of light each scattering keeps; w: ground reflectance for the undersides.
    albedo: vec4<f32>,
    /// How last frame's camera turned and projected, for finding what it drew.
    prev_view_from_world: mat3x3<f32>,
    prev_clip_from_view: mat4x4<f32>,
    /// xyz: how far the camera moved since last frame, relative to the body (render axes, m);
    /// w: 1 if the view was cut and last frame is no use.
    moved: vec4<f32>,
    /// x: frame number, y: distance beyond which clouds lose their fine detail, m, z: 1 for
    /// detail at all, w: length of a step as a share of the distance.
    frame: vec4<f32>,
};

/// The ray through a pixel in render axes, unnormalised, without TAA's jitter: clouds are drawn at
/// half resolution and keep their own history. Its length is the distance per unit of view depth.
fn view_ray(uv: vec2<f32>, clip_from_view: mat4x4<f32>, world_from_view: mat4x4<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let through = vec3<f32>(ndc.x / clip_from_view[0][0], ndc.y / clip_from_view[1][1], -1.0);
    return mat3x3<f32>(world_from_view[0].xyz, world_from_view[1].xyz, world_from_view[2].xyz) * through;
}
