// One body's shadow on another, as a share of the Sun's disc still in sight from a point.
// `crate::eclipse` mirrors `eclipse_visibility` for the exposure; keep the two in sync.

struct Eclipse {
    /// xyz: an occluder's centre, relative to whatever the positions are relative to (render
    /// axes, m); w: its radius. Radius 0 for none.
    first: vec4<f32>,
    second: vec4<f32>,
    /// rgb: the light that still gets into the first occluder's umbra; w: the Sun's angular
    /// radius, rad.
    umbra: vec4<f32>,
};

/// The share of the Sun's disc in sight past an occluder, from angles alone. Exact where the two
/// discs don't overlap or one is inside the other, and a smooth curve in between.
fn disc_in_sight(p: vec3<f32>, sun: vec3<f32>, occluder: vec4<f32>, sun_radius: f32) -> f32 {
    if occluder.w <= 0.0 {
        return 1.0;
    }
    let to_it = occluder.xyz - p;
    let distance = length(to_it);
    if distance <= occluder.w {
        return 1.0;
    }
    let toward = dot(to_it, sun);
    if toward <= 0.0 {
        return 1.0;
    }
    let its_radius = asin(occluder.w / distance);
    let apart = atan2(length(cross(to_it, sun)), toward);
    let least = max(1.0 - (its_radius * its_radius) / (sun_radius * sun_radius), 0.0);
    if its_radius >= sun_radius {
        return smoothstep(its_radius - sun_radius, its_radius + sun_radius, apart);
    }
    return mix(least, 1.0, smoothstep(sun_radius - its_radius, sun_radius + its_radius, apart));
}

/// How much sunlight reaches a point past the occluders, by colour: what gets into an umbra is not
/// nothing, around an Earth with air it is the red of every sunset at once.
fn eclipse_visibility(p: vec3<f32>, sun: vec3<f32>, eclipse: Eclipse) -> vec3<f32> {
    let first = disc_in_sight(p, sun, eclipse.first, eclipse.umbra.w);
    let second = disc_in_sight(p, sun, eclipse.second, eclipse.umbra.w);
    return mix(eclipse.umbra.rgb, vec3<f32>(1.0), first) * second;
}
