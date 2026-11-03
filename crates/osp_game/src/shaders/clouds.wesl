// The clouds, ray-marched at half resolution through a shell around the planet.
//
// How much cloud is where comes from the body's weather map; the shape of each cloud is carved
// from tiling 3D noise in the manner of Schneider's Horizon cloudscapes (2015). Light is a march
// towards the sun for self-shadowing, Wrenninge's octaves for what scattered many times inside
// (2013), and the sky's own irradiance. Sunlight is taken through the air at each sample, so
// clouds go orange at sunset and dark in the planet's shadow by themselves.
//
// The result is what must be added over the scene, air in front of the cloud included, and how
// much of what is behind still shows. Writing the sky pass's output as `S`, the air between the
// eye and the cloud as `A` with transmittance `T`, and the cloud as `C` with transmittance `t`,
// the true picture is `S * t + T * C + (1 - t) * A`. So the composite is a plain blend, and the sky
// pass didn't have to know about any of this. Second target: how far the light came from, for
// reprojection.

import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import bevy_render::view::View;
import osp::shaders::atmosphere::{
    air, clouds, eclipse_at, weather, lut_sampler, scatter, sky_irradiance, sphere_hits,
    sun_transmittance, PI,
};
import osp::shaders::clouds_types::{CloudDraw, view_ray};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> draw: CloudDraw;
@if(MULTISAMPLED)
@group(0) @binding(2) var depth_texture: texture_depth_multisampled_2d;
@if(!MULTISAMPLED)
@group(0) @binding(2) var depth_texture: texture_depth_2d;
@group(0) @binding(3) var base_noise: texture_3d<f32>;
@group(0) @binding(4) var detail_noise: texture_3d<f32>;
@group(0) @binding(5) var noise_sampler: sampler;

/// The tiling noises' periods, m (`osp_sim::clouds::BASE_TILE`), and the size of a texel of each.
const BASE_TILE: f32 = 12000.0;
const DETAIL_TILE: f32 = 1700.0;
const BASE_TEXEL: f32 = 93.75;
const DETAIL_TEXEL: f32 = 26.6;
/// The large noise's period, no multiple of the small one's, and the size of its texels, m.
const LARGE_TILE: f32 = 63700.0;
const LARGE_TEXEL: f32 = 498.0;
/// How far it pushes the small noise, m: more than its period, to leave no trace of the grid, and
/// over a long enough distance not to stretch the clouds out of shape.
const WARP: f32 = 9000.0;
/// Size of a weather map's texel on Earth, and how many mip levels it has below the top.
const WEATHER_TEXEL: f32 = 9800.0;
const WEATHER_MIPS: f32 = 3.0;
/// Where the noise's billows mostly fall, measured over the texture.
const BILLOWS_LOW: f32 = 0.74;
const BILLOWS_WIDTH: f32 = 0.16;
/// How soft a cloud's edge is, as a share of the billows' range.
const CLOUD_EDGE: f32 = 0.14;
/// How deep the fine noise bites into a cloud's surface, in the billows' terms.
const DETAIL_REACH: f32 = 0.34;
/// Between these pixel sizes (m) the cloud cells change from the small noise's to the large one's.
const FAR_NOISE_FROM: f32 = 250.0;
const FAR_NOISE_TO: f32 = 900.0;
/// Between these pixel sizes (m) the march gives way to a flat sheet read from the weather map:
/// past the first, a pixel is too big for the noise to be anything but aliasing.
const SHEET_FROM: f32 = 1400.0;
const SHEET_TO: f32 = 4000.0;
/// What a thick cloud reflects of the light on it.
const THICK_ALBEDO: f32 = 0.8;
/// How much of a cloud faces a sun on the horizon, as a share of what faces one overhead.
const SIDES_LIT: f32 = 0.4;
/// Empty rays are stepped this many times faster, and go back once they find cloud.
const EMPTY_STRIDE: f32 = 4.0;
/// The shortest step, m: inside a cloud, around the camera.
const SHORTEST_STEP: f32 = 30.0;
/// Steps for the air in front of the cloud.
const AIR_STEPS: i32 = 10;
/// How far a sample looks towards the sun, in the first and each following step's length, m.
const LIGHT_FIRST_STEP: f32 = 70.0;
/// Each order of scattering: a share as bright (a), as easily through (b), as forward (c).
const OCTAVES: i32 = 3;
const OCTAVE_A: f32 = 0.5;
const OCTAVE_B: f32 = 0.45;
const OCTAVE_C: f32 = 0.5;
/// How much of the light a cloud hides at optical depth 1 it still lets down as diffuse light,
/// after the two-stream solution for a thick layer, `1 / (1 + 0.75 depth (1 - g))`, with g about
/// 0.65 once the higher orders are spread out.
const TWO_STREAM: f32 = 0.28;
/// The forward and back lobes of a water cloud's phase function, and their shares.
const FORWARD_G: f32 = 0.8;
const BACK_G: f32 = -0.3;
const FORWARD_SHARE: f32 = 0.7;

fn saturate(x: f32) -> f32 {
    return clamp(x, 0.0, 1.0);
}

fn remap(x: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    return c + (x - a) / (b - a) * (d - c);
}

fn henyey_greenstein(mu: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / (4.0 * PI * pow(max(1.0 + g2 - 2.0 * g * mu, 1e-5), 1.5));
}

fn phase(mu: f32, scale: f32) -> f32 {
    return mix(henyey_greenstein(mu, BACK_G * scale), henyey_greenstein(mu, FORWARD_G * scale), FORWARD_SHARE);
}

/// How high up in the layer a radius is, 0 to 1.
fn height_in_layer(r: f32) -> f32 {
    return (r - draw.shell.x) / (draw.shell.y - draw.shell.x);
}

/// How much cloud a kind of cloud has at a height in the layer. Flat decks are low and thin, deep
/// ones fill it.
fn profile(h: f32, deep: f32) -> f32 {
    let top = mix(0.24, 1.0, deep * deep);
    let rise = smoothstep(0.0, 0.025 + 0.06 * deep, h);
    let fall = 1.0 - smoothstep(top * 0.35, top, h);
    return rise * fall;
}

/// The weather over a point, read as coarsely as a pixel's footprint there asks for.
fn weather_at(p: vec3<f32>, footprint: f32) -> vec4<f32> {
    let dir = draw.local_from_render * p;
    let lod = clamp(log2(max(footprint, 1.0) / WEATHER_TEXEL), 0.0, WEATHER_MIPS);
    return textureSampleLevel(weather, lut_sampler, dir, lod);
}

/// Where a point falls in the noise, on charts wrapped around the planet.
///
/// The noise can't simply be read at the point's position in space. The layer is curved and the
/// noise's lattice is flat, so the layer would slice down through the lattice with distance, and
/// each period of the noise would come out as a ring hundreds of kilometres across, like Newton's
/// rings. So it is read by distance along the surface and height above it: the charts are those of
/// a cube around the planet, each flat on its face, overlapping where they meet.
struct Charts {
    /// How much of each chart (with x, y or z as its axis) there is here; they add up to 1.
    weights: vec3<f32>,
    /// Noise coordinates, m, in each chart.
    x: vec3<f32>,
    y: vec3<f32>,
    z: vec3<f32>,
};

/// How wide the overlap between charts is, as a difference of direction components.
const CHART_OVERLAP: f32 = 0.03;

fn charts_at(local: vec3<f32>, height: f32) -> Charts {
    var out: Charts;
    let d = normalize(local);
    let a = abs(d);
    // Distance along the surface from the middle of the face.
    let scale = draw.shell.x;
    let wind = draw.tile.xyz;
    let w = vec3<f32>(
        clamp((a.x - max(a.y, a.z)) / CHART_OVERLAP + 0.5, 0.0, 1.0),
        clamp((a.y - max(a.x, a.z)) / CHART_OVERLAP + 0.5, 0.0, 1.0),
        clamp((a.z - max(a.x, a.y)) / CHART_OVERLAP + 0.5, 0.0, 1.0),
    );
    out.weights = w / max(w.x + w.y + w.z, 1e-5);
    if w.x > 0.0 {
        out.x = vec3<f32>(atan2(d.y, a.x) * scale + wind.x, height + wind.y, atan2(d.z, a.x) * scale + wind.z);
    }
    if w.y > 0.0 {
        out.y = vec3<f32>(atan2(d.x, a.y) * scale + wind.x, height + wind.y, atan2(d.z, a.y) * scale + wind.z);
    }
    if w.z > 0.0 {
        out.z = vec3<f32>(atan2(d.x, a.z) * scale + wind.x, height + wind.y, atan2(d.y, a.z) * scale + wind.z);
    }
    return out;
}

fn sample_noise(charts: Charts, tile: f32, big: bool, lod: f32, offset: vec3<f32>) -> vec4<f32> {
    var sum = vec4<f32>(0.0);
    if charts.weights.x > 0.0 {
        sum += read_noise((charts.x + offset) / tile, big, lod) * charts.weights.x;
    }
    if charts.weights.y > 0.0 {
        sum += read_noise((charts.y + offset) / tile, big, lod) * charts.weights.y;
    }
    if charts.weights.z > 0.0 {
        sum += read_noise((charts.z + offset) / tile, big, lod) * charts.weights.z;
    }
    return sum;
}

fn read_noise(q: vec3<f32>, big: bool, lod: f32) -> vec4<f32> {
    if big {
        return textureSampleLevel(base_noise, noise_sampler, q, lod);
    }
    return textureSampleLevel(detail_noise, noise_sampler, q, lod);
}

/// The same noise far larger and turned, whose period has nothing to do with the small one's. It
/// pushes the small one's coordinates around, or that would show from orbit as an endless grid of
/// the same few clouds.
fn sample_large(charts: Charts, footprint: f32) -> vec4<f32> {
    let lod = clamp(log2(max(footprint, 1.0) / LARGE_TEXEL), 0.0, 6.0);
    var sum = vec4<f32>(0.0);
    if charts.weights.x > 0.0 {
        sum += textureSampleLevel(base_noise, noise_sampler, turned(charts.x) / LARGE_TILE, lod) * charts.weights.x;
    }
    if charts.weights.y > 0.0 {
        sum += textureSampleLevel(base_noise, noise_sampler, turned(charts.y) / LARGE_TILE, lod) * charts.weights.y;
    }
    if charts.weights.z > 0.0 {
        sum += textureSampleLevel(base_noise, noise_sampler, turned(charts.z) / LARGE_TILE, lod) * charts.weights.z;
    }
    return sum;
}

fn turned(q: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.8290 * q.x - 0.5592 * q.z, q.y, 0.5592 * q.x + 0.8290 * q.z);
}

/// The cloud's density at a point `rel` from the camera, as a share of the dense cloud's, before
/// the fine detail wears at it, and its place in the noise.
struct Shape {
    density: f32,
    height: f32,
    charts: Charts,
    /// How far the large noise pushes the small one here, m.
    warp: vec3<f32>,
    /// How far inside the cloud's surface this is, in the billows' terms; below 0 it is outside.
    above: f32,
    /// Whether the weather allows any cloud here at all. A ray only takes long strides where it
    /// doesn't: a layer of cloud can be thinner than a stride, and rays that stride over it or
    /// not, depending on how its height lines up, would draw its contours as bands.
    possible: bool,
};

fn shape_at(rel: vec3<f32>, footprint: f32, weather_sample: vec4<f32>) -> Shape {
    var out: Shape;
    let p = draw.eye.xyz + rel;
    let r = length(p);
    out.height = height_in_layer(r);
    out.density = 0.0;
    out.possible = false;
    let cover = weather_sample.r;
    if cover <= 0.004 || out.height <= 0.0 || out.height >= 1.0 {
        return out;
    }
    let shaped = profile(out.height, weather_sample.g);
    if shaped <= 0.0 {
        return out;
    }
    out.possible = true;
    out.charts = charts_at(draw.local_from_render * p, r - draw.shell.x);
    let lod = clamp(log2(max(footprint, 1.0) / BASE_TEXEL), 0.0, 6.0);
    let large = sample_large(out.charts, footprint);
    out.warp = (large.gba - 0.5) * vec3<f32>(WARP, 0.0, WARP);
    let n = sample_noise(out.charts, BASE_TILE, true, lod, out.warp);
    let cells = n.g * 0.625 + n.b * 0.25 + n.a * 0.125;
    let billows = saturate(remap(n.r, cells - 1.0, 1.0, 0.0, 1.0));
    // Spread over the whole range, so `cover` is about the share of the sky with cloud in it. The
    // profile pulls that in at the bottom and the top, which rounds the clouds off.
    // From far away cells of a few kilometres are all a pixel can show, and they would look like
    // scales; the larger noise's are the size real cloud fields show from orbit.
    let near = saturate((billows - BILLOWS_LOW) / BILLOWS_WIDTH);
    let large_billows = saturate(remap(large.r, large.g * 0.625 + large.b * 0.25 + large.a * 0.125 - 1.0, 1.0, 0.0, 1.0));
    let far = saturate((large_billows - BILLOWS_LOW) / BILLOWS_WIDTH);
    let even = mix(near * 0.8 + large.r * 0.2, far, smoothstep(FAR_NOISE_FROM, FAR_NOISE_TO, footprint));
    out.above = even - (1.0 - cover) - (1.0 - shaped) * 0.55;
    out.density = saturate(out.above / CLOUD_EDGE);
    return out;
}

/// Wear the cloud's surface with the fine noise: this is what makes a cauliflower of it. Wispy near
/// the base, billowy near the top.
fn with_detail(shape: Shape, footprint: f32, distance: f32) -> f32 {
    if draw.frame.z <= 0.0 {
        return shape.density;
    }
    let fade = 1.0 - smoothstep(draw.frame.y * 0.5, draw.frame.y, distance);
    if fade <= 0.0 || shape.above > DETAIL_REACH * 1.5 {
        return shape.density;
    }
    let lod = clamp(log2(max(footprint, 1.0) / DETAIL_TEXEL), 0.0, 5.0);
    let d = sample_noise(shape.charts, DETAIL_TILE, false, lod, shape.warp).rgb;
    let f = d.r * 0.625 + d.g * 0.25 + d.b * 0.125;
    let wear = mix(1.0 - f, f, saturate(shape.height * 8.0)) * fade * smoothstep(0.0, 0.1, shape.above + 0.1);
    return saturate((shape.above - wear * DETAIL_REACH) / CLOUD_EDGE);
}

/// Optical depth towards the sun from a point `rel` from the camera, in steps that double. The
/// noise stays at the level the view reads it at: blurred further, thin clouds would average away
/// and let the sun through. `jitter` keeps the steps from lining up: they would draw terraces at
/// fixed heights below the tops of clouds.
fn optical_depth_to_sun(rel: vec3<f32>, footprint: f32, jitter: f32) -> f32 {
    let sun = draw.sun.xyz;
    var depth = 0.0;
    var along = 0.0;
    var step = LIGHT_FIRST_STEP;
    let steps = i32(draw.tile.w);
    for (var i = 0; i < steps; i++) {
        let q = rel + sun * (along + step * jitter);
        let w = weather_at(draw.eye.xyz + q, footprint);
        depth += shape_at(q, footprint, w).density * step;
        along += step;
        step *= 2.0;
    }
    return depth * draw.shell.z;
}

struct Output {
    /// rgb: what to add over the scene, after exposure; a: how much of the scene shows through.
    @location(0) light: vec4<f32>,
    /// How far along the ray the light came from, m; 0 for none.
    @location(1) distance: f32,
};

/// The interleaved gradient noise of Jimenez (2014), moved along every frame so TAA and the
/// clouds' own history average it out.
fn dither(px: vec2<f32>) -> f32 {
    let frame = draw.frame.x % 64.0;
    let moved = px + vec2<f32>(5.588238, 5.588238) * frame;
    return fract(52.9829189 * fract(dot(moved, vec2<f32>(0.06711056, 0.00583715))));
}

/// Where a ray crosses the layer: `x` to `y`, or `y <= x` if it doesn't.
fn layer_span(eye: vec3<f32>, ray: vec3<f32>) -> vec2<f32> {
    let bottom = draw.shell.x;
    let top = draw.shell.y;
    let r = length(eye);
    let outer = sphere_hits(eye, ray, top);
    if outer.x > outer.y || outer.y <= 0.0 {
        return vec2<f32>(1.0, 0.0);
    }
    var t0 = max(outer.x, 0.0);
    var t1 = outer.y;
    let inner = sphere_hits(eye, ray, bottom);
    let hits_inner = inner.x <= inner.y;
    if r < bottom {
        // From under the layer, up through it.
        t0 = max(inner.y, 0.0);
    } else if hits_inner && inner.x > 0.0 {
        // Down onto its bottom, and what comes back up on the far side is behind the planet or
        // lost in the haze.
        t1 = inner.x;
    }
    return vec2<f32>(t0, t1);
}

fn shade(in: FullscreenVertexOutput, depth: f32) -> Output {
    var out: Output;
    out.light = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    out.distance = 0.0;

    let scaled = view_ray(in.uv, view.clip_from_view, view.world_from_view);
    let per_depth = length(scaled);
    let ray = scaled / per_depth;
    let eye = draw.eye.xyz;
    let sun = draw.sun.xyz;
    let sun_color = vec3<f32>(draw.sun.w);
    let radius = air.shape.x;

    var span = layer_span(eye, ray);
    var cirrus_t = -1.0;
    if draw.shell.w > 0.0 {
        let high = sphere_hits(eye, ray, draw.shell.w);
        if high.x <= high.y {
            cirrus_t = select(high.y, high.x, high.x > 0.0);
        }
    }

    // What the ray hits: drawn scene, or the planet if nothing has been drawn there.
    var scene = 1e12;
    if depth > 0.0 {
        scene = view.clip_from_view[3][2] / depth * per_depth;
    } else {
        let ground = sphere_hits(eye, ray, radius);
        if ground.x <= ground.y && ground.x > 0.0 {
            scene = ground.x;
        }
    }
    span.y = min(span.y, scene);
    if cirrus_t > scene {
        cirrus_t = -1.0;
    }

    var light = vec3<f32>(0.0);
    var through = 1.0;
    // Of the light's distance, weighted by how much each sample hid.
    var weight = 0.0;
    var mean_t = 0.0;

    // A half-resolution pixel's size per metre of distance, and how much of a sheet this is.
    let per_metre = 4.0 / (view.clip_from_view[1][1] * view.viewport.w);
    let sheet = smoothstep(SHEET_FROM, SHEET_TO, max(span.x, 0.0) * per_metre);

    var march_end = 0.0;
    if span.y > span.x && sheet < 1.0 {
        // Steps in proportion to the distance, so a pixel's depth is about as fine near as far.
        let steps = i32(draw.eye.w);
        let growth = draw.frame.w;
        let jitter = dither(in.position.xy);
        let mu = dot(ray, sun);
        let sky_above = draw.albedo.w;

        var t = span.x;
        var striding = false;
        // After going back for something a stride went past, no striding until past it again.
        var careful_until = span.x;
        for (var i = 0; i < steps; i++) {
            if t >= span.y {
                break;
            }
            let fine = max(t * growth, SHORTEST_STEP);
            let long = striding && t >= careful_until;
            let stride = select(fine, fine * EMPTY_STRIDE, long);
            let at = min(t + jitter * stride, span.y);
            let rel = ray * at;
            let footprint = at * per_metre;
            let w = weather_at(eye + rel, footprint);
            let shape = shape_at(rel, max(footprint, stride * 0.5), w);
            if !shape.possible {
                striding = true;
                t += stride;
                continue;
            }
            if long {
                // Went past where cloud may start: back to where there was none, at the finer pace.
                t = max(t - stride, span.x);
                careful_until = t + stride;
                striding = false;
                continue;
            }
            striding = false;
            if shape.density <= 0.0 {
                t += stride;
                continue;
            }
            let density = with_detail(shape, footprint, at);
            if density > 0.001 {
                let p = eye + rel;
                let extinction = density * draw.shell.z;
                let ds = min(stride, span.y - t);
                let step_through = exp(-extinction * ds);
                let depth_to_sun = optical_depth_to_sun(rel, footprint, fract(jitter + 0.618034 * f32(i)));
                let reached = sun_transmittance(p, sun) * eclipse_at(p);
                // Light that has scattered many times comes out in all directions, whatever the
                // sun's angle to the view. It makes up most of a thick cloud's brightness: a white
                // surface under sunlight, as bright as paper from above and greyer from below.
                // Clouds have sides, so a low sun still lights them.
                let height_of_sun = max(dot(p / length(p), sun), 0.0);
                let lit_area = mix(SIDES_LIT, 1.0, height_of_sun);
                var sum = THICK_ALBEDO * lit_area / PI / (1.0 + TWO_STREAM * depth_to_sun);
                // Sunlight scattered a few times, with the higher orders getting through more
                // easily and more evenly: the shading on the sides, the silver lining.
                var a = 1.0;
                var b = 1.0;
                var c = 1.0;
                for (var o = 0; o < OCTAVES; o++) {
                    sum += a * exp(-depth_to_sun * b) * phase(mu, c);
                    a *= OCTAVE_A;
                    b *= OCTAVE_B;
                    c *= OCTAVE_C;
                }
                // The sky from above, a little of the lit ground from below, both dimmer deep in.
                let irradiance = sky_irradiance(p, sun) * sun_color / PI;
                let open = mix(0.35, 1.0, saturate(shape.height * 5.0)) * (1.0 - density * 0.5);
                let bounce = sky_above * max(dot(p / length(p), sun), 0.0) * reached * sun_color / PI;
                let source = draw.albedo.rgb
                    * (reached * sun_color * sum + irradiance * open + bounce * (1.0 - shape.height) * 0.5);
                let hidden = through * (1.0 - step_through) * (1.0 - sheet);
                light += source * hidden;
                weight += hidden;
                mean_t += at * hidden;
                through *= mix(step_through, 1.0, sheet);
                if through < 0.01 {
                    break;
                }
            }
            t += stride;
        }
        march_end = t;
    }

    // From far away, and past where a low ray's steps run out, one surface at the height most
    // clouds are, as a satellite sees them: no further than the march got.
    if through > 0.01 {
        let hit = sphere_hits(eye, ray, clouds.shell.x);
        if hit.x <= hit.y {
            let hit_t = select(hit.y, hit.x, hit.x > 0.0);
            // Fading in past the march's end, so nothing shows where one gives way to the other.
            let past = smoothstep(march_end, march_end * 1.4 + 1.0, hit_t);
            let flat_share = sheet + (1.0 - sheet) * past;
            if flat_share > 0.0 && hit_t > 0.0 && hit_t < scene {
                let p = eye + ray * hit_t;
                // Seen edge-on a pixel covers a long stretch of the map; read it that coarsely.
                let facing = max(abs(dot(ray, p / length(p))), 0.02);
                let w = weather_at(p, hit_t * per_metre / facing);
                let share = w.r * w.r * (3.0 - 2.0 * w.r) * flat_share;
                let height_of_sun = max(dot(p / length(p), sun), 0.0);
                let lit_area = mix(SIDES_LIT, 1.0, height_of_sun);
                let source = draw.albedo.rgb
                    * (sun_transmittance(p, sun) * eclipse_at(p) * sun_color * (THICK_ALBEDO * lit_area / PI + phase(dot(ray, sun), 0.5) * 0.5)
                        + sky_irradiance(p, sun) * sun_color / PI);
                let hidden = through * share;
                light += source * hidden;
                through *= 1.0 - share;
                weight += hidden;
                mean_t += hit_t * hidden;
            }
        }
    }

    // The thin high layer, drawn as a single sheet, in front of or behind the rest.
    if cirrus_t > 0.0 {
        let p = eye + ray * cirrus_t;
        let footprint = cirrus_t * 0.001;
        let w = weather_at(p, footprint);
        if w.b > 0.002 {
            // Long fibres along the wind, faint: this is a veil, not a layer.
            var charts = charts_at(draw.local_from_render * p, 0.0);
            let along = vec3<f32>(0.22, 1.0, 1.0);
            charts.x *= along;
            charts.y *= along;
            charts.z *= along;
            let fibres = sample_noise(charts, BASE_TILE, true, 1.0, vec3<f32>(0.0)).b;
            let sheet = saturate(w.b * smoothstep(0.25, 0.85, fibres)) * 0.28;
            let mu = dot(ray, sun);
            let source = draw.albedo.rgb
                * (sun_transmittance(p, sun) * sun_color * phase(mu, 0.6) * 1.4
                    + sky_irradiance(p, sun) * sun_color / PI);
            let front = weight <= 0.0 || cirrus_t < mean_t / weight;
            if front {
                light = source * sheet + light * (1.0 - sheet);
            } else {
                light += source * sheet * through;
            }
            let hidden = sheet * through;
            through *= 1.0 - sheet;
            weight += hidden;
            mean_t += cirrus_t * hidden;
        }
    }

    if weight <= 0.0 {
        return out;
    }
    let distance = mean_t / weight;
    let entry = sphere_hits(eye, ray, air.shape.y);
    let front_air = scatter(eye, ray, max(entry.x, 0.0), distance, sun, sun_color, AIR_STEPS);
    let total = front_air.transmittance * light + (1.0 - through) * front_air.inscatter;
    out.light = vec4<f32>(total * view.exposure, through);
    out.distance = distance;
    return out;
}

@if(MULTISAMPLED)
@fragment
fn main(in: FullscreenVertexOutput) -> Output {
    let px = vec2<i32>(in.position.xy) * 2;
    return shade(in, textureLoad(depth_texture, px, 0));
}

@if(!MULTISAMPLED)
@fragment
fn main(in: FullscreenVertexOutput) -> Output {
    // The full-resolution pixel the composite also compares against.
    let px = vec2<i32>(in.position.xy) * 2;
    return shade(in, textureLoad(depth_texture, px, 0));
}
