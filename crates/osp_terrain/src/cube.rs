//! Cube-sphere addressing, shared by the bake tool, the simulation and the renderer.
//!
//! A body's surface is split into six cube faces, each addressed by face coordinates
//! `(s, t)` in `0..1` and subdivided as a quadtree. A [`TileKey`] names one quadtree node;
//! the renderer's terrain chunks and the baked data tiles use the same keys, so a chunk at
//! a given level maps onto exactly one data tile.
//!
//! Directions are body-fixed: +Z is the north pole and +X the prime meridian.

use std::f64::consts::FRAC_PI_4;

use glam::DVec3;

/// Cells along a tile edge. Tiles are grid-registered: they store `TILE_SAMPLES` samples per
/// edge, and neighbouring tiles share their edge samples.
pub const TILE_CELLS: usize = 128;
pub const TILE_SAMPLES: usize = TILE_CELLS + 1;
/// Deepest quadtree level that keys can address.
pub const MAX_LEVEL: u8 = 24;

/// Outward normal and the directions of increasing `s` and `t` for a cube face.
pub fn face_basis(face: u8) -> (DVec3, DVec3, DVec3) {
    match face {
        0 => (DVec3::X, DVec3::Y, DVec3::Z),
        1 => (DVec3::NEG_X, DVec3::NEG_Y, DVec3::Z),
        2 => (DVec3::Y, DVec3::NEG_X, DVec3::Z),
        3 => (DVec3::NEG_Y, DVec3::X, DVec3::Z),
        4 => (DVec3::Z, DVec3::Y, DVec3::NEG_X),
        _ => (DVec3::NEG_Z, DVec3::Y, DVec3::X),
    }
}

/// Map face coordinates to a unit vector. Uses the equal-angle cube mapping, which spreads
/// samples far more evenly than plain normalisation. Coordinates slightly outside `0..1`
/// extrapolate smoothly onto the neighbouring faces.
pub fn cube_to_sphere(face: u8, s: f64, t: f64) -> DVec3 {
    let (normal, u, v) = face_basis(face);
    let a = (FRAC_PI_4 * (2.0 * s - 1.0)).tan();
    let b = (FRAC_PI_4 * (2.0 * t - 1.0)).tan();
    (normal + u * a + v * b).normalize()
}

/// Inverse of [`cube_to_sphere`]: the face containing `dir` and the face coordinates.
pub fn sphere_to_cube(dir: DVec3) -> (u8, f64, f64) {
    let a = dir.abs();
    let face = if a.x >= a.y && a.x >= a.z {
        if dir.x >= 0.0 { 0 } else { 1 }
    } else if a.y >= a.z {
        if dir.y >= 0.0 { 2 } else { 3 }
    } else if dir.z >= 0.0 {
        4
    } else {
        5
    };
    let (normal, u, v) = face_basis(face);
    let d = dir.dot(normal);
    let s = ((dir.dot(u) / d).atan() / FRAC_PI_4 + 1.0) * 0.5;
    let t = ((dir.dot(v) / d).atan() / FRAC_PI_4 + 1.0) * 0.5;
    (face, s.clamp(0.0, 1.0), t.clamp(0.0, 1.0))
}

/// Latitude and longitude in radians of a body-fixed direction.
pub fn lat_lon(dir: DVec3) -> (f64, f64) {
    let dir = dir.normalize();
    (dir.z.clamp(-1.0, 1.0).asin(), dir.y.atan2(dir.x))
}

/// Body-fixed unit vector for a latitude and longitude in radians.
pub fn from_lat_lon(lat: f64, lon: f64) -> DVec3 {
    DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin())
}

/// Local east and north unit vectors at a body-fixed direction (east is arbitrary but
/// continuous at the poles).
pub fn east_north(dir: DVec3) -> (DVec3, DVec3) {
    let east = DVec3::Z.cross(dir).try_normalize().unwrap_or(DVec3::Y);
    (east, dir.cross(east))
}

/// One node of a face quadtree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub face: u8,
    pub level: u8,
    pub x: u32,
    pub y: u32,
}

impl TileKey {
    pub const fn root(face: u8) -> Self {
        Self {
            face,
            level: 0,
            x: 0,
            y: 0,
        }
    }

    /// The key at `level` containing face coordinates `(s, t)`.
    pub fn containing(face: u8, s: f64, t: f64, level: u8) -> Self {
        let n = 1u32 << level;
        let cell = |c: f64| ((c * n as f64).floor() as i64).clamp(0, n as i64 - 1) as u32;
        Self {
            face,
            level,
            x: cell(s),
            y: cell(t),
        }
    }

    pub fn children(self) -> [TileKey; 4] {
        let child = |dx, dy| TileKey {
            face: self.face,
            level: self.level + 1,
            x: self.x * 2 + dx,
            y: self.y * 2 + dy,
        };
        [child(0, 0), child(1, 0), child(0, 1), child(1, 1)]
    }

    pub fn parent(self) -> Option<TileKey> {
        (self.level > 0).then(|| TileKey {
            face: self.face,
            level: self.level - 1,
            x: self.x / 2,
            y: self.y / 2,
        })
    }

    /// The ancestor (or self) at `level`, which must not be deeper than this key.
    pub fn ancestor(self, level: u8) -> TileKey {
        let shift = self.level - level;
        TileKey {
            face: self.face,
            level,
            x: self.x >> shift,
            y: self.y >> shift,
        }
    }

    /// Edge length in face coordinates.
    pub fn size(self) -> f64 {
        1.0 / (1u64 << self.level) as f64
    }

    /// Face coordinates of grid sample `(i, j)` (fractional indices allowed).
    pub fn sample_st(self, i: f64, j: f64) -> (f64, f64) {
        let size = self.size();
        (
            (self.x as f64 + i / TILE_CELLS as f64) * size,
            (self.y as f64 + j / TILE_CELLS as f64) * size,
        )
    }

    /// Body-fixed direction of grid sample `(i, j)`.
    pub fn sample_dir(self, i: f64, j: f64) -> DVec3 {
        let (s, t) = self.sample_st(i, j);
        cube_to_sphere(self.face, s, t)
    }

    pub fn center_dir(self) -> DVec3 {
        self.sample_dir(TILE_CELLS as f64 / 2.0, TILE_CELLS as f64 / 2.0)
    }

    /// Approximate edge length on a sphere of `radius`, m.
    pub fn edge_length(self, radius: f64) -> f64 {
        radius * std::f64::consts::FRAC_PI_2 * self.size()
    }

    /// Continuous sample coordinates inside `self` of face coordinates `(s, t)`.
    pub fn local_sample(self, s: f64, t: f64) -> (f64, f64) {
        let n = (1u64 << self.level) as f64;
        (
            (s * n - self.x as f64) * TILE_CELLS as f64,
            (t * n - self.y as f64) * TILE_CELLS as f64,
        )
    }

    /// Packed into a `u64` (used as the on-disk index key).
    pub fn to_bits(self) -> u64 {
        (self.face as u64) << 61 | (self.level as u64) << 56 | (self.x as u64) << 28 | self.y as u64
    }

    pub fn from_bits(bits: u64) -> Self {
        Self {
            face: (bits >> 61) as u8,
            level: ((bits >> 56) & 0x1f) as u8,
            x: ((bits >> 28) & 0x0fff_ffff) as u32,
            y: (bits & 0x0fff_ffff) as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_round_trip() {
        for face in 0..6 {
            for (s, t) in [(0.1, 0.2), (0.5, 0.5), (0.93, 0.01), (0.37, 0.88)] {
                let dir = cube_to_sphere(face, s, t);
                let (f, s2, t2) = sphere_to_cube(dir);
                assert_eq!(f, face);
                assert!((s - s2).abs() < 1e-12 && (t - t2).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn faces_tile_the_sphere() {
        // Every direction maps to a face whose image is the same direction.
        for k in 0..2000 {
            let x = (k as f64 * 0.7548776662).fract() * 2.0 - 1.0;
            let y = (k as f64 * 0.5698402910).fract() * 2.0 - 1.0;
            let z = (k as f64 * 0.3141592653).fract() * 2.0 - 1.0;
            let dir = DVec3::new(x, y, z).normalize();
            let (f, s, t) = sphere_to_cube(dir);
            assert!(cube_to_sphere(f, s, t).distance(dir) < 1e-12);
        }
    }

    #[test]
    fn keys() {
        let key = TileKey::containing(3, 0.3, 0.7, 5);
        assert_eq!(TileKey::from_bits(key.to_bits()), key);
        assert_eq!(
            key.ancestor(2),
            key.parent().unwrap().parent().unwrap().parent().unwrap()
        );
        assert!(key.parent().unwrap().children().contains(&key));
        let (u, v) = key.local_sample(0.3, 0.7);
        assert!((0.0..=TILE_CELLS as f64).contains(&u) && (0.0..=TILE_CELLS as f64).contains(&v));
        let (s, t) = key.sample_st(u, v);
        assert!((s - 0.3).abs() < 1e-12 && (t - 0.7).abs() < 1e-12);
    }

    #[test]
    fn lat_lon_round_trip() {
        let dir = from_lat_lon(28.6f64.to_radians(), -80.6f64.to_radians());
        let (lat, lon) = lat_lon(dir);
        assert!((lat.to_degrees() - 28.6).abs() < 1e-12);
        assert!((lon.to_degrees() + 80.6).abs() < 1e-12);
    }
}
