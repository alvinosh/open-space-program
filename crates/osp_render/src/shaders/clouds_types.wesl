// What the cloud passes share (`shaders/clouds*.wesl`, `crate::clouds`).

struct CloudDraw {
    /// xyz: the camera relative to the body's centre (render axes, m).
    eye: vec4<f32>,
    /// xyz: direction to the sun; w: its illuminance above the air.
    sun: vec4<f32>,
    /// The body's fixed axes (Y to its pole) from render axes.
    local_from_render: mat3x3<f32>,
    /// The map's axes from the body's, and its second copy's from those (`osp_sim::clouds::map_turns`).
    map_from_local: mat3x3<f32>,
    copy_from_map: mat3x3<f32>,
    /// x: radius of the layer, y: of the thin high layer (0 for none), z: size of a texel of the map
    /// here, m, w: 1 if the map is the body's own.
    shell: vec4<f32>,
    /// rgb: the share of light each scattering keeps; w: how much thin high cloud, 0 to 1.
    albedo: vec4<f32>,
    /// How far the wind has carried the detail, in its periods: xy for its big copy, zw for its fine one.
    detail_wind: vec4<f32>,
    /// xy: how far the UV noise has moved, in its periods; zw: the same for the high layer.
    wobble: vec4<f32>,
};

/// The ray through a pixel in render axes, unnormalised, without TAA's jitter: clouds are drawn at
/// half resolution. Its length is the distance per unit of view depth.
fn view_ray(uv: vec2<f32>, clip_from_view: mat4x4<f32>, world_from_view: mat4x4<f32>) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let through = vec3<f32>(ndc.x / clip_from_view[0][0], ndc.y / clip_from_view[1][1], -1.0);
    return mat3x3<f32>(world_from_view[0].xyz, world_from_view[1].xyz, world_from_view[2].xyz) * through;
}
