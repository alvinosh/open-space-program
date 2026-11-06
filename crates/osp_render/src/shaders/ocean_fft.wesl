// One stage of an inverse FFT over every line of a cascade: each output reads the two inputs that
// make it, so a stage is a plain dispatch between two textures. Stockham's autosort, which needs no
// bit reversal; `osp_sim::ocean::inverse_fft` is the same thing on the CPU, tested against the sum
// it stands for. Keep the two in sync.

struct Stage {
    /// x: size of the sub-transforms this stage makes twice as long; y: 1 along columns.
    size: vec4<u32>,
};

@group(0) @binding(0) var<uniform> stage: Stage;
@group(0) @binding(1) var source: texture_2d_array<f32>;
@group(0) @binding(2) var target_lines: texture_storage_2d_array<rgba32float, write>;

const N: u32 = 256u;
const PI: f32 = 3.141592653589793;

fn multiply(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= N || id.y >= N || id.z >= 3u {
        return;
    }
    let layer = i32(id.z);
    let columns = stage.size.y != 0u;
    let output = select(id.x, id.y, columns);
    let p = stage.size.x;
    let k = output & (p - 1u);
    let second_half = (output & p) != 0u;
    let first = k + ((output & ~(2u * p - 1u)) >> 1u);
    let other = first + N / 2u;
    let at_first = select(vec2<i32>(i32(first), i32(id.y)), vec2<i32>(i32(id.x), i32(first)), columns);
    let at_other = select(vec2<i32>(i32(other), i32(id.y)), vec2<i32>(i32(id.x), i32(other)), columns);
    let u = textureLoad(source, at_first, layer, 0);
    let v = textureLoad(source, at_other, layer, 0);
    // Inverse: the positive sign.
    let angle = PI * f32(k) / f32(p);
    let w = vec2<f32>(cos(angle), sin(angle));
    let turned = vec4<f32>(multiply(v.xy, w), multiply(v.zw, w));
    let out = select(u + turned, u - turned, second_half);
    textureStore(target_lines, vec2<i32>(id.xy), layer, out);
}
