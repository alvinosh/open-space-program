// From the transformed fields to the maps the terrain reads: slope, its square (whose average over
// a mip level, less the average's square, is the slope's variance, what a coarse pixel sees as
// roughness), and foam. Foam marks where the surface folds over on itself, which is where the
// Jacobian of its horizontal displacement goes down (Tessendorf), and then lingers and fades.

struct Params {
    /// x: seconds since the last frame, y: how long foam lasts, s.
    frame: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var first: texture_2d_array<f32>;
@group(0) @binding(2) var second: texture_2d_array<f32>;
@group(0) @binding(3) var foam_before: texture_2d_array<f32>;
@group(0) @binding(4) var foam_after: texture_storage_2d_array<r32float, write>;
@group(0) @binding(5) var maps: texture_storage_2d_array<rgba16float, write>;

const N: i32 = 256;
/// The horizontal squeeze at which the surface starts to break, and how fast that turns to foam.
const FOLD: f32 = 0.55;
const FOAM_GAIN: f32 = 2.2;

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let at = vec2<i32>(id.xy);
    if at.x >= N || at.y >= N || id.z >= 3u {
        return;
    }
    let layer = i32(id.z);
    let a = textureLoad(first, at, layer, 0);
    let b = textureLoad(second, at, layer, 0);
    let slope = a.xy;
    let dxx = b.y;
    let dzz = b.z;
    let dxz = b.w;
    let jacobian = (1.0 + dxx) * (1.0 + dzz) - dxz * dxz;
    let breaking = clamp((FOLD - jacobian) * FOAM_GAIN, 0.0, 1.0);
    let before = textureLoad(foam_before, at, layer, 0).r;
    let lasting = before * exp(-params.frame.x / params.frame.y);
    let foam = max(lasting, breaking);
    textureStore(foam_after, at, layer, vec4<f32>(foam, 0.0, 0.0, 1.0));
    textureStore(maps, at, layer, vec4<f32>(slope, dot(slope, slope), foam));
}
