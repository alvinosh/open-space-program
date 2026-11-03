//! A body's clouds: where they are, and the noise they are carved from.
//!
//! Where is the **weather**: a map over the whole sphere of how much cloud there is and what
//! kind, built once per body from its data ([`CloudsDef`]). It is shaped like real circulation:
//! a climate by latitude (the rain belt at the equator, clear subtropics, stormy mid-latitudes),
//! fronts swept into commas around the lows of the storm tracks, and a few hurricanes.
//!
//! The shape of each cloud comes from two tiling 3D textures of **noise**, in the way of
//! Schneider's cloudscapes for Horizon (2015): billows of Perlin-Worley noise to carve a cloud
//! out of the weather, and finer Worley noise to wear away its edges.
//!
//! The game draws them (`crates/osp_game/src/clouds.rs`). The sim's copy of the weather also
//! says how much sunlight reaches a point under them.

use std::f64::consts::TAU;

use glam::DVec3;
use serde::{Deserialize, Serialize};

/// A body's clouds as data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudsDef {
    /// Bottom and top of the layer above the mean radius, m. Most clouds stay low in it; only
    /// the deepest storms reach the top.
    pub base: f64,
    pub top: f64,
    /// Extinction inside a dense cloud, per metre: about 0.05 for water clouds.
    pub density: f64,
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
    /// Wind that moves the clouds' fine structure, m/s.
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
/// Noise textures' edges, in texels (`BASE_NOISE_SIZE` and `DETAIL_NOISE_SIZE` in the shader).
pub const BASE_NOISE_SIZE: usize = 128;
pub const DETAIL_NOISE_SIZE: usize = 64;

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

fn remap(x: f64, a: f64, b: f64, c: f64, d: f64) -> f64 {
    c + (x - a) / (b - a) * (d - c)
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

/// Worley noise that tiles every `cells` cells: 1 at a feature point, falling to 0 a cell away.
fn worley(p: DVec3, cells: i64, seed: i64) -> f64 {
    let q = p * cells as f64;
    let i = q.floor();
    let (x, y, z) = (i.x as i64, i.y as i64, i.z as i64);
    let mut nearest = f64::MAX;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (cx, cy, cz) = (x + dx, y + dy, z + dz);
                let h = hash(
                    cx.rem_euclid(cells) + seed,
                    cy.rem_euclid(cells),
                    cz.rem_euclid(cells),
                );
                let h2 = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
                let h3 = h2.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
                let feature = DVec3::new(
                    cx as f64 + unit(h),
                    cy as f64 + unit(h2),
                    cz as f64 + unit(h3),
                );
                nearest = nearest.min(feature.distance_squared(q));
            }
        }
    }
    1.0 - nearest.sqrt().min(1.0)
}

/// Three octaves of Worley noise, tiling.
fn worley_fbm(p: DVec3, cells: i64, seed: i64) -> f64 {
    worley(p, cells, seed) * 0.625
        + worley(p, cells * 2, seed + 1) * 0.25
        + worley(p, cells * 4, seed + 2) * 0.125
}

/// Gradient noise that tiles every `cells` cells, in about -1..1.
fn perlin(p: DVec3, cells: i64, seed: i64) -> f64 {
    let q = p * cells as f64;
    let i = q.floor();
    let f = q - i;
    let u = f * f * f * (f * (f * 6.0 - DVec3::splat(15.0)) + DVec3::splat(10.0));
    let (x, y, z) = (i.x as i64, i.y as i64, i.z as i64);
    let corner = |dx: i64, dy: i64, dz: i64| {
        let h = hash(
            (x + dx).rem_euclid(cells) + seed,
            (y + dy).rem_euclid(cells),
            (z + dz).rem_euclid(cells),
        );
        // One of twelve edge directions of a cube, as Perlin's improved noise has it.
        let d = f - DVec3::new(dx as f64, dy as f64, dz as f64);
        match h % 12 {
            0 => d.x + d.y,
            1 => -d.x + d.y,
            2 => d.x - d.y,
            3 => -d.x - d.y,
            4 => d.x + d.z,
            5 => -d.x + d.z,
            6 => d.x - d.z,
            7 => -d.x - d.z,
            8 => d.y + d.z,
            9 => -d.y + d.z,
            10 => d.y - d.z,
            _ => -d.y - d.z,
        }
    };
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u.x);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u.x);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u.x);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u.x);
    lerp(lerp(x00, x10, u.y), lerp(x01, x11, u.y), u.z)
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

/// The billows, as a cube of RGBA8 that tiles: red is Perlin-Worley, the rest Worley at three
/// scales, each finer than the last.
pub fn base_noise() -> Vec<[u8; 4]> {
    let n = BASE_NOISE_SIZE;
    fill_rows(n * n, n, [0u8; 4], |row, out| {
        let (y, z) = (row % n, row / n);
        for (x, texel) in out.iter_mut().enumerate() {
            let p = DVec3::new(x as f64, y as f64, z as f64) / n as f64;
            let billows = perlin(p, 4, 0) * 0.6 + perlin(p, 8, 1) * 0.3 + perlin(p, 16, 2) * 0.15;
            let billows = (billows * 0.5 + 0.5).clamp(0.0, 1.0);
            let w0 = worley_fbm(p, 4, 10);
            let perlin_worley = remap(billows, 0.0, 1.0, w0, 1.0).clamp(0.0, 1.0);
            *texel = [
                (perlin_worley * 255.0) as u8,
                (w0 * 255.0) as u8,
                (worley_fbm(p, 8, 20) * 255.0) as u8,
                (worley_fbm(p, 16, 30) * 255.0) as u8,
            ];
        }
    })
}

/// The fine structure, as a cube of RGBA8 that tiles: Worley at three scales.
pub fn detail_noise() -> Vec<[u8; 4]> {
    let n = DETAIL_NOISE_SIZE;
    fill_rows(n * n, n, [0u8; 4], |row, out| {
        let (y, z) = (row % n, row / n);
        for (x, texel) in out.iter_mut().enumerate() {
            let p = DVec3::new(x as f64, y as f64, z as f64) / n as f64;
            *texel = [
                (worley_fbm(p, 2, 40) * 255.0) as u8,
                (worley_fbm(p, 4, 50) * 255.0) as u8,
                (worley_fbm(p, 8, 60) * 255.0) as u8,
                255,
            ];
        }
    })
}

/// A cube of noise with its mip chain under it, every texel averaged with its tiling neighbours.
pub fn with_mips(top: &[[u8; 4]], size: usize) -> Vec<[u8; 4]> {
    let mut all = top.to_vec();
    let (mut level, mut n) = (top.to_vec(), size);
    while n > 1 {
        let half = n / 2;
        let mut next = vec![[0u8; 4]; half * half * half];
        for z in 0..half {
            for y in 0..half {
                for x in 0..half {
                    let mut sum = [0u32; 4];
                    for (dx, dy, dz) in [
                        (0, 0, 0),
                        (1, 0, 0),
                        (0, 1, 0),
                        (1, 1, 0),
                        (0, 0, 1),
                        (1, 0, 1),
                        (0, 1, 1),
                        (1, 1, 1),
                    ] {
                        let i = (2 * x + dx) + (2 * y + dy) * n + (2 * z + dz) * n * n;
                        for (s, v) in sum.iter_mut().zip(level[i]) {
                            *s += v as u32;
                        }
                    }
                    next[x + y * half + z * half * half] = sum.map(|s| (s / 8) as u8);
                }
            }
        }
        all.extend_from_slice(&next);
        level = next;
        n = half;
    }
    all
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
    for low in lows {
        wound = swirl(wound, low);
        let k = dir.dot(low.centre).clamp(-1.0, 1.0).acos() / low.size;
        storminess = storminess.max((-k * k * 0.5).exp());
    }
    let stretch = DVec3::new(1.0, 1.7, 1.0);
    let warp = DVec3::new(
        fbm(wound * 3.0 + DVec3::splat(4.1), 4, seed + 1),
        fbm(wound * 3.0 + DVec3::splat(9.7), 4, seed + 2),
        fbm(wound * 3.0 + DVec3::splat(17.3), 4, seed + 3),
    );
    // Large systems, then the cells inside them down to about 40 km.
    let large = fbm((wound + warp * 0.22) * stretch * 3.2, 4, seed + 4);
    let cells = fbm((wound + warp * 0.12) * 26.0, 4, seed + 5);
    let field = large * 0.66 + cells * 0.34;
    // The noise spreads about 0.2 either side of 0, so this asks for about `share` of the sphere.
    let share = (climate + storminess * 0.22).clamp(0.0, 1.0);
    let threshold = (0.5 - share) * 0.5;
    // A wide ramp, not an edge: most of a system is broken cloud, which the 3D noise carves into
    // cells, and only its core is solid.
    let mut cover = smoothstep(threshold - 0.03, threshold + 0.3, field);
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

/// The noise textures tile every this many metres at the largest scale (`BASE_TILE` in the
/// shader), so an offset into them can be sent as a small number.
pub const BASE_TILE: f64 = 12_000.0;

/// An offset into the tiling noise, wrapped to stay accurate in f32.
pub fn tile_offset(p: DVec3) -> DVec3 {
    DVec3::new(
        p.x.rem_euclid(BASE_TILE),
        p.y.rem_euclid(BASE_TILE),
        p.z.rem_euclid(BASE_TILE),
    )
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
            density: 0.05,
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

    fn tiles(data: &[[u8; 4]], n: usize, channel: usize) {
        // Across a tile's edge the noise changes no more than it does anywhere else.
        let at = |x: usize, y: usize, z: usize| data[x + y * n + z * n * n][channel] as i32;
        let mut inner = 0;
        let mut across = 0;
        for z in 0..n {
            for y in 0..n {
                for x in 1..n {
                    inner = inner.max((at(x, y, z) - at(x - 1, y, z)).abs());
                }
                across = across.max((at(0, y, z) - at(n - 1, y, z)).abs());
            }
        }
        assert!(
            across <= inner + 8,
            "channel {channel}: {across} vs {inner}"
        );
    }

    #[test]
    fn the_noise_tiles_and_has_range() {
        for (data, n) in [
            (base_noise(), BASE_NOISE_SIZE),
            (detail_noise(), DETAIL_NOISE_SIZE),
        ] {
            for c in 0..3 {
                tiles(&data, n, c);
                let (lo, hi) = data
                    .iter()
                    .fold((255u8, 0u8), |(lo, hi), t| (lo.min(t[c]), hi.max(t[c])));
                assert!(hi as i32 - lo as i32 > 100, "channel {c}: {lo}..{hi}");
            }
        }
    }

    #[test]
    fn mips_go_down_to_one_texel_and_keep_the_average() {
        let n = 8;
        let top: Vec<[u8; 4]> = (0..n * n * n)
            .map(|i| [(i % 256) as u8, 40, 0, 255])
            .collect();
        let all = with_mips(&top, n);
        assert_eq!(all.len(), 512 + 64 + 8 + 1);
        assert_eq!(all[all.len() - 1][1], 40);
        assert_eq!(all[all.len() - 1][3], 255);
    }
}
