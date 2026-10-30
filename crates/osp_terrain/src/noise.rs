//! Deterministic value noise, used to break up the outlines of upsampled land cover and to
//! scatter vegetation. Lives here (not in the game) so it is optimised in dev builds too.

use glam::DVec3;

fn hash(x: i64, y: i64, z: i64) -> f64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (z as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Smooth value noise in [-1, 1] with unit lattice spacing.
pub fn value_noise(p: DVec3) -> f64 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (DVec3::splat(3.0) - 2.0 * f);
    let (x, y, z) = (i.x as i64, i.y as i64, i.z as i64);
    let corner = |dx, dy, dz| hash(x + dx, y + dy, z + dz);
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u.x);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u.x);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u.x);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u.x);
    lerp(lerp(x00, x10, u.y), lerp(x01, x11, u.y), u.z) * 2.0 - 1.0
}

/// Two octaves of value noise at lattice spacing `scale`.
pub fn fbm(p: DVec3, scale: f64) -> f64 {
    let q = p / scale;
    (value_noise(q) + 0.5 * value_noise(q * 2.03 + 11.0)) / 1.5
}

/// Deterministic random number in 0..1 for an integer cell and channel.
pub fn random(cell: (i64, i64, i64), channel: i64) -> f64 {
    hash(
        cell.0 ^ (channel << 40),
        cell.1,
        cell.2.wrapping_add(channel),
    )
}

/// Offset in texels for sampling land cover at `p` (a position in units of data texels), so
/// class boundaries follow ragged natural outlines instead of the texel grid.
pub fn cover_warp(p: DVec3) -> (f64, f64) {
    let warp =
        |offset: f64| 1.1 * (value_noise(p + offset) + 0.4 * value_noise(p * 3.0 + offset + 17.0));
    (warp(0.0), warp(41.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_bounded_and_continuous() {
        let mut previous = value_noise(DVec3::new(0.0, 0.0, 3.3));
        for k in 1..2000 {
            let p = DVec3::new(k as f64 * 0.01, k as f64 * 0.007, 3.3);
            let n = value_noise(p);
            assert!((-1.0..=1.0).contains(&n));
            assert!((n - previous).abs() < 0.1);
            previous = n;
        }
    }
}
