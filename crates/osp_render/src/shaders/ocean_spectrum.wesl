// Each wave mode at this instant, ready for the inverse FFT (Tessendorf 2001). Two pairs of real
// fields are packed per texture: a complex field `a + ib` turns back into two real ones, a in the
// real part and b in the imaginary, as long as a and b each have the conjugate symmetry of a real
// signal.

struct Params {
    /// x: seconds, y: gravity.
    time: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var amplitudes: texture_2d_array<f32>;
/// Slope in x and z, then displacement in x and z.
@group(0) @binding(2) var first: texture_storage_2d_array<rgba32float, write>;
/// Height and dDx/dx, then dDz/dz and dDx/dz.
@group(0) @binding(3) var second: texture_storage_2d_array<rgba32float, write>;

const N: i32 = 256;
const CASCADES: vec3<f32> = vec3<f32>(4096.0, 512.0, 64.0);
const TAU: f32 = 6.283185307179586;

fn multiply(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}

/// i times a complex number.
fn times_i(a: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(-a.y, a.x);
}

@compute @workgroup_size(16, 16, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let at = vec2<i32>(id.xy);
    if at.x >= N || at.y >= N || id.z >= 3u {
        return;
    }
    let layer = i32(id.z);
    let wave = select(at, at - N, at >= vec2<i32>(N / 2));
    let k = vec2<f32>(wave) * TAU / CASCADES[layer];
    let length_k = max(length(k), 1e-6);
    let omega = sqrt(params.time.y * length_k);
    // Wrapped so its precision survives a long session; a mode's period is what counts, and it
    // needn't line up over minutes on end.
    let phase = omega * params.time.x;
    let turn = vec2<f32>(cos(phase), sin(phase));
    let h0 = textureLoad(amplitudes, at, layer, 0);
    let h = multiply(h0.xy, turn) + multiply(h0.zw, vec2<f32>(turn.x, -turn.y));

    let i_h = times_i(h);
    let slope_x = i_h * k.x;
    let slope_z = i_h * k.y;
    let along = k / length_k;
    let disp_x = i_h * along.x;
    let disp_z = i_h * along.y;
    let dxx = -h * k.x * along.x;
    let dzz = -h * k.y * along.y;
    let dxz = -h * k.x * along.y;

    // a + ib for each pair.
    textureStore(first, at, layer, vec4<f32>(slope_x + times_i(slope_z), disp_x + times_i(disp_z)));
    textureStore(second, at, layer, vec4<f32>(h + times_i(dxx), dzz + times_i(dxz)));
}
