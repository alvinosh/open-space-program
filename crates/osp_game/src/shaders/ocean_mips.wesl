// One level of the maps' mip chain from the one above: a plain average, which is what makes the
// slope variance in the third channel come out right at every level.

@group(0) @binding(0) var above: texture_2d_array<f32>;
@group(0) @binding(1) var below: texture_storage_2d_array<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(below);
    if id.x >= size.x || id.y >= size.y || id.z >= 3u {
        return;
    }
    let at = vec2<i32>(id.xy) * 2;
    let layer = i32(id.z);
    let sum = textureLoad(above, at, layer, 0)
        + textureLoad(above, at + vec2<i32>(1, 0), layer, 0)
        + textureLoad(above, at + vec2<i32>(0, 1), layer, 0)
        + textureLoad(above, at + vec2<i32>(1, 1), layer, 0);
    textureStore(below, vec2<i32>(id.xy), layer, sum * 0.25);
}
