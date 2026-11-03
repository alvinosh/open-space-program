//! A body's clouds: how much of them is where.
//!
//! The **map** ([`CloudMap`]) is what they look like from above: a cube of cloud imagery, in the format
//! of EnvironmentalVisualEnhancements, whose way of drawing clouds this follows (`crate::clouds` in
//! the game). The **weather** ([`Weather`]) is a coarser map built once per body from its data
//! ([`CloudsDef`]), shaped like real circulation: a climate by latitude, fronts swept into commas
//! around the lows of the storm tracks, and a few hurricanes. A body with a map of its own uses the
//! weather only for its thin high cloud; on any other, the weather says where the map shows.
//!
//! The sim's copy of the weather also says how much sunlight reaches a point under them.

use std::f64::consts::TAU;

use glam::{DQuat, DVec3};
use serde::{Deserialize, Serialize};

/// A body's clouds as data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudsDef {
    /// Bottom and top of the cloud above the mean radius, m. The layer is drawn a fifth of the way
    /// up: most clouds are low.
    pub base: f64,
    pub top: f64,
    /// A map of the clouds: `<map>P.png` and `<map>N.png` in a mod's files. Without one the shared map
    /// shows where the weather says.
    #[serde(default)]
    pub map: Option<String>,
    /// Share of the light each scattering keeps, by colour.
    pub albedo: [f64; 3],
    /// Share of the sky covered by latitude, as `[latitude, share]` from the equator to the pole
    /// (the same in both hemispheres), degrees.
    pub bands: Vec<[f64; 2]>,
    /// Lows with their fronts, spread over the latitudes of `storm_latitude`.
    #[serde(default)]
    pub storms: u32,
    /// The band of latitudes they keep to, degrees.
    #[serde(default = "default_storm_latitude")]
    pub storm_latitude: [f64; 2],
    /// Hurricanes, in the tropics.
    #[serde(default)]
    pub hurricanes: u32,
    /// Wind that moves the clouds' detail, m/s.
    #[serde(default)]
    pub wind: f64,
    /// Height of the thin high layer above the mean radius, m (0 for none), and how much of it
    /// there is, 0 to 1.
    #[serde(default)]
    pub cirrus_height: f64,
    #[serde(default)]
    pub cirrus: f64,
    /// For the pattern.
    #[serde(default)]
    pub seed: u32,
}

impl CloudsDef {
    /// The height at which sunlight is taken to cross the layer, for what it shades: most clouds
    /// are low in it.
    pub fn shadow_height(&self) -> f64 {
        self.base + (self.top - self.base) * 0.2
    }
}

fn default_storm_latitude() -> [f64; 2] {
    [35.0, 65.0]
}

/// Texels along each edge of a face of the weather map: about 10 km on Earth.
pub const WEATHER_SIZE: usize = 1024;
fn hash(x: i64, y: i64, z: i64) -> u64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (z as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    h
}

/// A number in 0..1 from a hash.
fn unit(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Smooth value noise in -1..1 with unit lattice spacing.
fn value_noise(p: DVec3, seed: i64) -> f64 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (DVec3::splat(3.0) - 2.0 * f);
    let (x, y, z) = (i.x as i64 + seed, i.y as i64, i.z as i64);
    let corner = |dx, dy, dz| unit(hash(x + dx, y + dy, z + dz));
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u.x);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u.x);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u.x);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u.x);
    lerp(lerp(x00, x10, u.y), lerp(x01, x11, u.y), u.z) * 2.0 - 1.0
}

/// Octaves of value noise, each twice as fine and half as strong, in about -1..1.
fn fbm(p: DVec3, octaves: u32, seed: i64) -> f64 {
    let (mut sum, mut weight, mut total, mut q) = (0.0, 1.0, 0.0, p);
    for k in 0..octaves {
        sum += weight * value_noise(q, seed + k as i64 * 17);
        total += weight;
        weight *= 0.5;
        q = q * 2.03 + DVec3::splat(11.3);
    }
    sum / total
}

/// A square table filled on all the cores, one row at a time.
fn fill_rows<T: Copy + Send>(
    rows: usize,
    row_size: usize,
    zero: T,
    f: impl Fn(usize, &mut [T]) + Sync,
) -> Vec<T> {
    let mut data = vec![zero; rows * row_size];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let chunk_rows = rows.div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (k, chunk) in data.chunks_mut(chunk_rows * row_size).enumerate() {
            let f = &f;
            scope.spawn(move || {
                for (r, row) in chunk.chunks_mut(row_size).enumerate() {
                    f(k * chunk_rows + r, row);
                }
            });
        }
    });
    data
}

/// A low with its fronts, or a hurricane, on the sphere.
#[derive(Clone, Copy)]
struct Storm {
    centre: DVec3,
    /// Its size, as the angle from the centre (radians).
    size: f64,
    /// Which way the air turns: counter-clockwise (north) or clockwise (south).
    turn: f64,
}

fn latitude_direction(lat: f64, lon: f64) -> DVec3 {
    // The map's axes follow the render's: +Y to the north pole.
    DVec3::new(lat.cos() * lon.cos(), lat.sin(), lat.cos() * lon.sin())
}

/// Weather over a whole sphere: for each texel, the share of the sky covered, the kind of cloud
/// (0 low and flat, 1 deep and tall), the thin high cloud, and how dark the cloud's underside is.
/// Six faces, in the order and orientation of a GPU cubemap, in the body's fixed axes as the game
/// draws them (Y to the pole).
pub struct Weather {
    /// `WEATHER_SIZE` squared per face.
    pub data: Vec<[u8; 4]>,
}

/// The direction through face `face` at `(s, t)`, each in -1..1, as a GPU samples a cubemap.
fn face_direction(face: usize, s: f64, t: f64) -> DVec3 {
    match face {
        0 => DVec3::new(1.0, -t, -s),
        1 => DVec3::new(-1.0, -t, s),
        2 => DVec3::new(s, 1.0, t),
        3 => DVec3::new(s, -1.0, -t),
        4 => DVec3::new(s, -t, 1.0),
        _ => DVec3::new(-s, -t, -1.0),
    }
    .normalize()
}

/// The face and `(s, t)` a direction falls on, as a GPU picks them.
fn face_of(d: DVec3) -> (usize, f64, f64) {
    let a = d.abs();
    if a.x >= a.y && a.x >= a.z {
        if d.x > 0.0 {
            (0, -d.z / a.x, -d.y / a.x)
        } else {
            (1, d.z / a.x, -d.y / a.x)
        }
    } else if a.y >= a.z {
        if d.y > 0.0 {
            (2, d.x / a.y, d.z / a.y)
        } else {
            (3, d.x / a.y, -d.z / a.y)
        }
    } else if d.z > 0.0 {
        (4, d.x / a.z, -d.y / a.z)
    } else {
        (5, -d.x / a.z, -d.y / a.z)
    }
}

fn bands_at(bands: &[[f64; 2]], latitude: f64) -> f64 {
    let lat = latitude.abs();
    let Some(first) = bands.first() else {
        return 0.0;
    };
    if lat <= first[0] {
        return first[1];
    }
    for pair in bands.windows(2) {
        if lat <= pair[1][0] {
            let k = (lat - pair[0][0]) / (pair[1][0] - pair[0][0]).max(1e-9);
            return pair[0][1] + (pair[1][1] - pair[0][1]) * k;
        }
    }
    bands.last().map_or(0.0, |b| b[1])
}

impl Weather {
    pub fn new(def: &CloudsDef) -> Self {
        let n = WEATHER_SIZE;
        let storms = storms(def);
        let seed = def.seed as i64 * 101;
        let data = fill_rows(6 * n, n, [0u8; 4], |row, out| {
            let (face, j) = (row / n, row % n);
            for (i, texel) in out.iter_mut().enumerate() {
                let s = 2.0 * (i as f64 + 0.5) / n as f64 - 1.0;
                let t = 2.0 * (j as f64 + 0.5) / n as f64 - 1.0;
                *texel = weather_at(def, &storms, seed, face_direction(face, s, t));
            }
        });
        Self { data }
    }

    /// Take the cover from a map as a body with this seed draws it, for the sunlight under what is
    /// actually there.
    pub fn take_cover(&mut self, map: &CloudMap, seed: u32) {
        let n = WEATHER_SIZE;
        let (first, second) = map_turns(seed);
        for (index, texel) in self.data.iter_mut().enumerate() {
            let (face, row) = (index / (n * n), index % (n * n));
            let s = 2.0 * ((row % n) as f64 + 0.5) / n as f64 - 1.0;
            let t = 2.0 * ((row / n) as f64 + 0.5) / n as f64 - 1.0;
            let dir = first * face_direction(face, s, t);
            texel[0] = (stacked(map.at(dir), map.at(second * dir)) * 255.0) as u8;
        }
    }

    /// Bilinear read of the map at a direction in the body's fixed axes (Y to the pole), per
    /// channel in 0..1, within a face.
    pub fn at(&self, dir: DVec3) -> [f64; 4] {
        let n = WEATHER_SIZE;
        let (face, s, t) = face_of(dir);
        let x = ((s * 0.5 + 0.5) * n as f64 - 0.5).clamp(0.0, (n - 1) as f64);
        let y = ((t * 0.5 + 0.5) * n as f64 - 0.5).clamp(0.0, (n - 1) as f64);
        let (i, j) = ((x as usize).min(n - 2), (y as usize).min(n - 2));
        let (fx, fy) = (x - i as f64, y - j as f64);
        let at = |i: usize, j: usize| self.data[face * n * n + j * n + i].map(|v| v as f64 / 255.0);
        let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * fx;
            let bottom = c[k] + (d[k] - c[k]) * fx;
            top + (bottom - top) * fy
        })
    }

    /// The share of direct sunlight that gets through the clouds to `p` (relative to the body's
    /// centre, in its fixed axes, Y to the pole), with the sun in direction `sun` in those axes.
    /// Mirrors the shader's `cloud_shadow`.
    pub fn sunlight(&self, def: &CloudsDef, radius: f64, p: DVec3, sun: DVec3) -> f64 {
        let middle = radius + def.shadow_height();
        let Some((_, far)) = crate::scattering::sphere_hits(p, sun, middle) else {
            return 1.0;
        };
        if far <= 0.0 || p.length() > middle {
            return 1.0;
        }
        let cover = self.at((p + sun * far).normalize())[0];
        (-SHADOW_DEPTH * cover).exp()
    }
}

/// How the map is turned on a body, by its seed, and how its second copy is turned from that. A map
/// made for a small world in fair weather has too little cloud on it for most planets; layers of the
/// same map, turned, are how packs of EnvironmentalVisualEnhancements build up more.
pub fn map_turns(seed: u32) -> (DQuat, DQuat) {
    (
        DQuat::from_axis_angle(DVec3::new(0.48, 0.72, 0.5).normalize(), seed as f64 * 1.7),
        DQuat::from_axis_angle(DVec3::new(0.3, -0.8, 0.5).normalize(), 2.1),
    )
}

/// How much cloud two readings of the map come to (`stacked` in the shaders).
pub fn stacked(first: f64, second: f64) -> f64 {
    1.0 - (1.0 - first.powf(MAP_LIFT)) * (1.0 - second.powf(MAP_LIFT))
}

/// Brings up the thin parts of a map, as a power of it.
const MAP_LIFT: f64 = 0.75;

/// The size of a texel of a map of 2048 on a face, on a body of this radius, m.
pub fn map_texel(radius: f64) -> f32 {
    (2.0 * radius / 2048.0) as f32
}

/// A cube of cloud imagery: how much cloud there is over each point, as a satellite would see it.
pub struct CloudMap {
    /// Texels along each edge of a face.
    pub size: usize,
    /// One byte a texel, `size` squared per face, in the order and orientation of a GPU cubemap, in
    /// the body's fixed axes as the game draws them (Y to the pole).
    pub data: Vec<u8>,
}

/// Where the map's format keeps a direction: in the texture for the positive faces or the negative
/// ones, which of its channels (X faces in red, Y in green, Z in blue), and where, with `v` up from
/// the bottom of the image.
fn place_in_two_textures(d: DVec3) -> (bool, usize, f64, f64) {
    let a = d.abs();
    let (on_z, on_sides) = (a.z >= a.x, a.x.max(a.z) >= a.y);
    let positive = if on_sides {
        if on_z { d.z >= 0.0 } else { d.x >= 0.0 }
    } else {
        d.y >= 0.0
    };
    let s = if positive { 1.0 } else { -1.0 };
    let (channel, u, v) = if !on_sides {
        (1, d.x / a.y, s * d.z / a.y)
    } else if on_z {
        (2, s * d.x / a.z, -d.y / a.z)
    } else {
        (0, -s * d.z / a.x, -d.y / a.x)
    };
    (positive, channel, u * 0.5 + 0.5, v * 0.5 + 0.5)
}

impl CloudMap {
    /// From the format of EnvironmentalVisualEnhancements: six faces in the channels of two square
    /// images (see [`place_in_two_textures`]), each `size` across and `stride` bytes a pixel, rows
    /// from the top.
    pub fn from_two_textures(size: usize, stride: usize, positive: &[u8], negative: &[u8]) -> Self {
        let n = size as f64;
        let data = fill_rows(6 * size, size, 0u8, |row, out| {
            let (face, j) = (row / size, row % size);
            for (i, texel) in out.iter_mut().enumerate() {
                let s = 2.0 * (i as f64 + 0.5) / n - 1.0;
                let t = 2.0 * (j as f64 + 0.5) / n - 1.0;
                let (pos, channel, u, v) = place_in_two_textures(face_direction(face, s, t));
                let x = ((u * n) as usize).min(size - 1);
                let y = ((v * n) as usize).min(size - 1);
                let texture = if pos { positive } else { negative };
                *texel = texture[(size - 1 - y) * size * stride + x * stride + channel];
            }
        });
        Self { size, data }
    }

    /// How much cloud there is in a direction, 0 to 1, to the nearest texel.
    pub fn at(&self, dir: DVec3) -> f64 {
        let (face, s, t) = face_of(dir);
        let n = self.size as f64;
        let x = (((s * 0.5 + 0.5) * n) as usize).min(self.size - 1);
        let y = (((t * 0.5 + 0.5) * n) as usize).min(self.size - 1);
        self.data[(face * self.size + y) * self.size + x] as f64 / 255.0
    }
}

/// Optical depth of full cover, for sunlight on what is below (`CLOUD_SHADOW_DEPTH` in the shader).
/// Real overcast lets through little direct sun, and the sky's light stays.
const SHADOW_DEPTH: f64 = 3.0;

fn storms(def: &CloudsDef) -> Vec<Storm> {
    let seed = def.seed as i64;
    let mut out = Vec::new();
    let pick = |k: i64, channel: i64| unit(hash(k, channel, seed + 7));
    for k in 0..def.storms as i64 {
        let lat = (def.storm_latitude[0]
            + (def.storm_latitude[1] - def.storm_latitude[0]) * pick(k, 0))
        .to_radians();
        let north = pick(k, 1) < 0.5;
        let lat = if north { lat } else { -lat };
        let lon = pick(k, 2) * TAU;
        out.push(Storm {
            centre: latitude_direction(lat, lon),
            size: (10.0 + 9.0 * pick(k, 3)).to_radians(),
            turn: if north { 1.0 } else { -1.0 },
        });
    }
    out
}

fn hurricanes(def: &CloudsDef) -> Vec<Storm> {
    let seed = def.seed as i64;
    let pick = |k: i64, channel: i64| unit(hash(k + 1000, channel, seed + 11));
    (0..def.hurricanes as i64)
        .map(|k| {
            let lat = (10.0 + 15.0 * pick(k, 0)).to_radians();
            let north = pick(k, 1) < 0.65;
            Storm {
                centre: latitude_direction(if north { lat } else { -lat }, pick(k, 2) * TAU),
                size: (5.0 + 3.0 * pick(k, 3)).to_radians(),
                turn: if north { 1.0 } else { -1.0 },
            }
        })
        .collect()
}

/// Turn `dir` around a storm's centre, more the nearer it is, which winds whatever pattern is
/// read from the result into a spiral.
fn swirl(dir: DVec3, storm: &Storm) -> DVec3 {
    let cosine = dir.dot(storm.centre).clamp(-1.0, 1.0);
    if cosine < (storm.size * 4.0).cos() {
        return dir;
    }
    let distance = cosine.acos();
    let k = distance / storm.size;
    // About a turn at the centre, winding down gently: more would draw the pattern into threads.
    let angle = storm.turn * 2.2 * (-k * k * 0.6).exp();
    let axis = storm.centre;
    // Rodrigues' rotation about the storm's centre.
    dir * angle.cos() + axis.cross(dir) * angle.sin() + axis * axis.dot(dir) * (1.0 - angle.cos())
}

fn weather_at(def: &CloudsDef, lows: &[Storm], seed: i64, dir: DVec3) -> [u8; 4] {
    let latitude = dir.y.clamp(-1.0, 1.0).asin().to_degrees();
    let climate = bands_at(&def.bands, latitude);

    // The lows wind the fronts into commas; fronts themselves are stretched east-west.
    let mut wound = dir;
    let mut storminess = 0.0f64;
    // How much of the sky a low covers by itself, nearer than its fronts reach.
    let mut overcast = 0.0f64;
    for low in lows {
        wound = swirl(wound, low);
        let k = dir.dot(low.centre).clamp(-1.0, 1.0).acos() / low.size;
        storminess = storminess.max((-k * k * 0.5).exp());
        overcast = overcast.max((-k * k * 2.0).exp());
    }
    let stretch = DVec3::new(1.0, 1.7, 1.0);
    let warp = DVec3::new(
        fbm(wound * 3.0 + DVec3::splat(4.1), 4, seed + 1),
        fbm(wound * 3.0 + DVec3::splat(9.7), 4, seed + 2),
        fbm(wound * 3.0 + DVec3::splat(17.3), 4, seed + 3),
    );
    // Large systems, and the broad cells inside them. Anything finer is the noise's, in the shader:
    // this says how much cloud there is around, and a map thresholded here would draw puddles.
    let large = fbm((wound + warp * 0.22) * stretch * 3.2, 4, seed + 4);
    let cells = fbm((wound + warp * 0.12) * 11.0, 3, seed + 5);
    let field = large * 0.82 + cells * 0.18;
    // The noise spreads about 0.2 either side of 0, so this asks for about `share` of the sphere.
    let share = climate.clamp(0.0, 1.0);
    let threshold = (0.5 - share) * 0.5;
    // A wide ramp, not an edge: most of a system is broken cloud, which the 3D noise carves into
    // cells, and only its core is solid.
    let mut cover = smoothstep(threshold - 0.35, threshold + 0.35, field) + overcast * 0.35;
    cover = cover.min(1.0);
    if share >= 0.999 {
        cover = (0.8 + field).clamp(0.6, 1.0);
    }
    let mut deep = smoothstep(0.08, 0.42, field - threshold) * (0.2 + 0.65 * storminess);
    // The tropics build towers wherever there is cloud at all; the subtropics stay flat.
    deep = deep.max(
        smoothstep(25.0, 5.0, latitude.abs()) * smoothstep(0.02, 0.3, field - threshold) * 0.9,
    );

    // Hurricanes: spiral arms around a clear eye, over whatever was there.
    for storm in hurricanes(def) {
        let cosine = dir.dot(storm.centre).clamp(-1.0, 1.0);
        if cosine < (storm.size * 2.5).cos() {
            continue;
        }
        let distance = cosine.acos() / storm.size;
        let east = storm.centre.cross(DVec3::Y).normalize_or(DVec3::X);
        let north = east.cross(storm.centre);
        let around = (dir.dot(north)).atan2(dir.dot(east)) * storm.turn;
        let arms =
            (around * 2.0 + 4.5 * (distance + 0.1).ln() + 1.2 * fbm(dir * 24.0, 3, seed + 6)).sin()
                * 0.5
                + 0.5;
        let eye = smoothstep(0.05, 0.11, distance);
        let wall = 1.0 - smoothstep(0.1, 0.35, distance);
        let body = (1.0 - smoothstep(0.7, 2.4, distance)) * (arms * (1.0 - wall) + wall) * eye;
        cover = (cover * smoothstep(0.0, 0.2, distance)).max(body);
        deep = deep.max(body * 0.95);
    }

    // Thin high cloud in long streaks, where the jets and the rain belt are: patches of it, not a
    // veil. `cirrus` is about the share of the sphere they cover.
    let cirrus = if def.cirrus > 0.0 {
        let patches = fbm((wound + warp * 0.2) * 5.0 + DVec3::splat(3.3), 3, seed + 8);
        let streaks = fbm(
            (wound + warp * 0.25) * DVec3::new(12.0, 34.0, 12.0),
            5,
            seed + 7,
        );
        let edge = 0.28 - 0.5 * def.cirrus;
        smoothstep(edge, edge + 0.3, patches)
            * smoothstep(-0.2, 0.4, streaks)
            * (0.4 + 0.6 * climate)
    } else {
        0.0
    };
    let dark = (cover * deep).clamp(0.0, 1.0);
    let to_byte = |x: f64| (x.clamp(0.0, 1.0) * 255.0) as u8;
    [
        to_byte(cover),
        to_byte(deep),
        to_byte(cirrus),
        to_byte(dark),
    ]
}

/// The angle a storm's centre is from `dir` in radians; here so the tests can find them.
#[cfg(test)]
fn nearest_storm(def: &CloudsDef, dir: DVec3) -> f64 {
    storms(def)
        .iter()
        .map(|s| dir.dot(s.centre).clamp(-1.0, 1.0).acos())
        .fold(std::f64::consts::PI, f64::min)
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use super::*;

    fn earth() -> CloudsDef {
        CloudsDef {
            base: 1000.0,
            top: 12_000.0,
            map: None,
            albedo: [0.99; 3],
            bands: vec![
                [0.0, 0.62],
                [8.0, 0.68],
                [25.0, 0.36],
                [45.0, 0.62],
                [60.0, 0.8],
                [75.0, 0.62],
                [90.0, 0.5],
            ],
            storms: 16,
            storm_latitude: [35.0, 65.0],
            hurricanes: 3,
            wind: 15.0,
            cirrus_height: 10_000.0,
            cirrus: 0.5,
            seed: 1,
        }
    }

    static WEATHER: LazyLock<Weather> = LazyLock::new(|| Weather::new(&earth()));

    fn mean_cover(from: f64, to: f64) -> f64 {
        let (mut sum, mut weight) = (0.0, 0.0);
        for i in 0..4000 {
            let lat = from + (to - from) * (i as f64 + 0.5) / 4000.0;
            let lon = i as f64 * 2.399963;
            let dir = latitude_direction(lat.to_radians(), lon);
            sum += WEATHER.at(dir)[0];
            weight += 1.0;
            sum += WEATHER.at(DVec3::new(dir.x, -dir.y, dir.z))[0];
            weight += 1.0;
        }
        sum / weight
    }

    #[test]
    fn the_cover_follows_the_climate() {
        let subtropics = mean_cover(20.0, 30.0);
        let storm_track = mean_cover(50.0, 65.0);
        let equator = mean_cover(0.0, 10.0);
        assert!(
            storm_track > subtropics + 0.15,
            "{storm_track} vs {subtropics}"
        );
        assert!(equator > subtropics + 0.1, "{equator} vs {subtropics}");
        // And it is about what the data asks for, within what a single pattern scatters.
        let all = mean_cover(0.0, 90.0);
        assert!(all > 0.4 && all < 0.8, "{all}");
    }

    #[test]
    fn the_map_reads_back_what_the_cubemap_stores() {
        // Every face, at a texel's centre: the direction falls on that face at that spot.
        let n = WEATHER_SIZE;
        for face in 0..6 {
            for (i, j) in [(3, 10), (500, 700), (1000, 20)] {
                let s = 2.0 * (i as f64 + 0.5) / n as f64 - 1.0;
                let t = 2.0 * (j as f64 + 0.5) / n as f64 - 1.0;
                let (f, s2, t2) = face_of(face_direction(face, s, t));
                assert_eq!(f, face);
                assert!((s2 - s).abs() < 1e-9 && (t2 - t).abs() < 1e-9, "{face}");
            }
        }
    }

    #[test]
    fn the_map_is_continuous_across_the_faces_of_the_cube() {
        // Walk a great circle: no jumps bigger than the pattern's sharpest edge allows.
        let mut last = WEATHER.at(DVec3::new(1.0, 0.2, 0.0).normalize())[0];
        let mut worst = 0.0f64;
        for k in 1..20000 {
            let a = k as f64 / 20000.0 * TAU;
            let dir = DVec3::new(a.cos(), 0.0, a.sin()) + DVec3::new(0.0, 0.2, 0.0);
            let now = WEATHER.at(dir.normalize())[0];
            worst = worst.max((now - last).abs());
            last = now;
        }
        assert!(worst < 0.2, "{worst}");
    }

    #[test]
    fn lows_are_cloudier_than_their_latitude() {
        let def = earth();
        let (mut near, mut far, mut n_near, mut n_far) = (0.0, 0.0, 0, 0);
        for i in 0..20000 {
            let lat = 35.0 + 30.0 * (i as f64 / 20000.0);
            let dir = latitude_direction(lat.to_radians(), i as f64 * 2.399963);
            let distance = nearest_storm(&def, dir);
            let cover = WEATHER.at(dir)[0];
            if distance < 0.15 {
                near += cover;
                n_near += 1;
            } else if distance > 0.5 {
                far += cover;
                n_far += 1;
            }
        }
        assert!(n_near > 50 && n_far > 50);
        assert!(near / n_near as f64 > far / n_far as f64, "{near} {far}");
    }

    #[test]
    fn a_clear_day_gets_all_the_sun_and_overcast_little() {
        let def = earth();
        let (radius, sun) = (6_371_000.0, DVec3::Y);
        let clear = Weather {
            data: vec![[0, 0, 0, 0]; 6 * WEATHER_SIZE * WEATHER_SIZE],
        };
        let full = Weather {
            data: vec![[255, 0, 0, 0]; 6 * WEATHER_SIZE * WEATHER_SIZE],
        };
        let p = DVec3::new(0.0, radius + 2.0, 0.0);
        assert_eq!(clear.sunlight(&def, radius, p, sun), 1.0);
        assert!(full.sunlight(&def, radius, p, sun) < 0.1);
        // Above the layer, nothing is in the way.
        let high = DVec3::new(0.0, radius + 50_000.0, 0.0);
        assert_eq!(full.sunlight(&def, radius, high, sun), 1.0);
    }

    /// Two textures in the map's format, holding a pattern that is smooth over the sphere.
    fn two_textures(size: usize, pattern: impl Fn(DVec3) -> f64) -> (Vec<u8>, Vec<u8>) {
        let mut textures = (vec![0u8; size * size * 3], vec![0u8; size * size * 3]);
        for j in 0..size {
            // The rows run from the top of the image; `v` from its bottom.
            let v = 1.0 - (j as f64 + 0.5) / size as f64;
            for i in 0..size {
                let u = (i as f64 + 0.5) / size as f64;
                let (a, b) = (2.0 * u - 1.0, 2.0 * v - 1.0);
                for (index, s) in [1.0, -1.0].into_iter().enumerate() {
                    let faces = [
                        DVec3::new(s, -b, -a * s),
                        DVec3::new(a, s, b * s),
                        DVec3::new(a * s, -b, s),
                    ];
                    for (channel, dir) in faces.into_iter().enumerate() {
                        let value = (pattern(dir.normalize()) * 255.0) as u8;
                        let texture = if index == 0 {
                            &mut textures.0
                        } else {
                            &mut textures.1
                        };
                        texture[(j * size + i) * 3 + channel] = value;
                    }
                }
            }
        }
        textures
    }

    #[test]
    fn a_map_comes_out_of_two_textures_the_right_way_round() {
        let pattern = |d: DVec3| 0.5 + 0.3 * d.x - 0.15 * d.y + 0.05 * d.z * d.x;
        let size = 64;
        let (positive, negative) = two_textures(size, pattern);
        let map = CloudMap::from_two_textures(size, 3, &positive, &negative);
        for k in 0..2000 {
            let dir =
                latitude_direction(((k * 37 % 180) as f64 - 89.5).to_radians(), k as f64 * 2.4);
            assert!(
                (map.at(dir) - pattern(dir)).abs() < 0.03,
                "{dir}: {}",
                map.at(dir)
            );
        }
    }

    #[test]
    fn the_weather_takes_its_cover_from_a_map() {
        let size = 64;
        // Solid on one side of the map and clear on the other.
        let map = CloudMap {
            size,
            data: (0..6 * size * size)
                .map(|index| {
                    let (face, row) = (index / (size * size), index % (size * size));
                    let t = 2.0 * ((row / size) as f64 + 0.5) / size as f64 - 1.0;
                    let s = 2.0 * ((row % size) as f64 + 0.5) / size as f64 - 1.0;
                    if face_direction(face, s, t).y > 0.0 {
                        255
                    } else {
                        0
                    }
                })
                .collect(),
        };
        let mut weather = Weather {
            data: vec![[0, 0, 7, 0]; 6 * WEATHER_SIZE * WEATHER_SIZE],
        };
        weather.take_cover(&map, 3);
        let (first, second) = map_turns(3);
        for k in 0..400 {
            let dir =
                latitude_direction(((k * 13 % 170) as f64 - 85.0).to_radians(), k as f64 * 2.4);
            let turned = first * dir;
            let [cover, _, thin, _] = weather.at(dir);
            // Away from the map's edge between solid and clear, so that the read is of one or the other.
            if turned.y.abs() > 0.05 && (second * turned).y.abs() > 0.05 {
                let expected = stacked(map.at(turned), map.at(second * turned));
                assert!(
                    (cover - expected).abs() < 0.02,
                    "{dir}: {cover} vs {expected}"
                );
            }
            assert!(thin > 0.0, "the rest stays");
        }
    }

    #[test]
    fn stacking_only_adds_cloud() {
        assert_eq!(stacked(0.0, 0.0), 0.0);
        assert_eq!(stacked(1.0, 0.2), 1.0);
        assert!(stacked(0.3, 0.0) > 0.3);
        assert!(stacked(0.3, 0.3) > stacked(0.3, 0.0));
    }
}
