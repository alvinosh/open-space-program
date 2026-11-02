//! Liquid in a cylindrical tank under thrust: it settles against the aft end of the vessel
//! with its surface level, whichever way the tank points.
//!
//! The liquid fills the part of the cylinder on the low side of a plane that cuts it
//! (`p·up ≤ w`), with `up` the vessel's +Z in the tank's frame. Each slice across the
//! cylinder is a circular segment, so the volume has a closed form and the plane is found by
//! bisection; the moments are integrated over the angle of the chord, where the integrand
//! is smooth.

use std::f64::consts::PI;

use glam::{DMat3, DQuat, DVec3};

use super::{Mass, offset};

/// Gauss–Legendre nodes and weights of 8 points on −1..1, the positive half.
const NODES: [f64; 4] = [
    0.183_434_642_495_649_8,
    0.525_532_409_916_329,
    0.796_666_477_413_626_7,
    0.960_289_856_497_536_3,
];
const WEIGHTS: [f64; 4] = [
    0.362_683_783_378_362,
    0.313_706_645_877_887_3,
    0.222_381_034_453_374_5,
    0.101_228_536_290_376_3,
];
/// Panels the angle is split in before the 8 points are put on each.
const PANELS: usize = 4;
/// Steps of the bisection for the surface.
const BISECTIONS: usize = 60;
/// A tank whose axis is closer than this (the sine of the angle) to the vessel's is along it.
const ALONG: f64 = 1e-9;
/// A tank whose axis is closer than this (the cosine of the angle) to level is level: the
/// height along the axis is found by dividing by the cosine, which loses digits below it.
const LEVEL: f64 = 1e-7;

impl Mass {
    /// `mass` of liquid in a cylinder `radius` wide and `length` long, standing on the origin
    /// up +Z, that would fill it to `column` standing upright, and how deep it is. It lies on
    /// the low side of a level surface, `up` (a unit vector in the cylinder's frame) being the
    /// way away from the aft end. The depth is measured along `up`, from the cylinder's
    /// lowest point to the surface, m. The domes are not told apart from the barrel.
    pub fn settled(mass: f64, radius: f64, length: f64, column: f64, up: DVec3) -> (Self, f64) {
        let pool = Pool::new(radius, length, column, up);
        if mass <= 0.0 || pool.fraction <= 0.0 {
            return (Self::cylinder(0.0, radius, 0.0), 0.0);
        }
        if pool.fraction >= 1.0 {
            return (Self::cylinder(mass, radius, length), pool.depth());
        }
        let along = up.truncate().length() < ALONG;
        let liquid = if along && up.z > 0.0 {
            Self::cylinder(mass, radius, pool.fraction * length)
        } else if along {
            let lower = pool.fraction * length;
            Self::cylinder(mass, radius, lower).placed(DVec3::Z * (length - lower), DQuat::IDENTITY)
        } else {
            pool.moments(mass)
        };
        (liquid, pool.depth())
    }

    /// How deep the liquid is: [`Self::settled`]'s second figure alone.
    pub fn settled_depth(radius: f64, length: f64, column: f64, up: DVec3) -> f64 {
        Pool::new(radius, length, column, up).depth()
    }
}

/// A cylinder's liquid, in the frame where the axis of the tank is turned about +Z to put
/// `up` in the XZ plane, leaning towards +X, and mirrored end for end where it points down.
struct Pool {
    radius: f64,
    length: f64,
    /// The share of the cylinder it fills.
    fraction: f64,
    /// `up` turned to this frame: sine and cosine of its angle from the axis.
    sin: f64,
    cos: f64,
    /// The liquid is on the other side of the cylinder than the canonical frame has it.
    mirrored: bool,
    /// The turn about +Z from this frame to the cylinder's.
    yaw: f64,
    /// Where the surface is: `x sin + z cos = w`.
    w: f64,
}

impl Pool {
    fn new(radius: f64, length: f64, column: f64, up: DVec3) -> Self {
        let up = up.normalize_or(DVec3::Z);
        let fraction = if length > 0.0 {
            (column / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut pool = Self {
            radius,
            length,
            fraction,
            sin: up.truncate().length(),
            cos: up.z.abs(),
            mirrored: up.z < 0.0,
            yaw: up.y.atan2(up.x),
            w: 0.0,
        };
        pool.w = pool.surface();
        pool
    }

    /// Lowest point along the axis of the tank's lean, `x sin + z cos` at its lowest.
    fn lowest(&self) -> f64 {
        -self.radius * self.sin
    }

    fn depth(&self) -> f64 {
        if self.fraction >= 1.0 {
            return self.length * self.cos + 2.0 * self.radius * self.sin;
        }
        if self.sin < ALONG {
            return self.fraction * self.length;
        }
        self.w - self.lowest()
    }

    /// The level of the surface that holds `fraction` of the cylinder.
    fn surface(&self) -> f64 {
        if self.sin < ALONG {
            return 0.0;
        }
        let want = self.fraction * PI * self.radius * self.radius * self.length;
        let (mut low, mut high) = (
            self.lowest(),
            self.radius * self.sin + self.length * self.cos,
        );
        for _ in 0..BISECTIONS {
            let middle = (low + high) / 2.0;
            if self.volume(middle) < want {
                low = middle;
            } else {
                high = middle;
            }
        }
        (low + high) / 2.0
    }

    /// Where the angle of the chord is at height `z`: the chord of the cut across the disc
    /// there lies at `x = −r cos β`.
    fn angle(&self, z: f64) -> f64 {
        ((z * self.cos - self.w) / (self.sin * self.radius))
            .clamp(-1.0, 1.0)
            .acos()
    }

    /// Where the cut meets the cylinder's wall: whole discs below the first, none above the
    /// second, along the axis and clipped to its length.
    fn reach(&self) -> (f64, f64) {
        let spread = self.radius * self.sin;
        let clip = |z: f64| z.clamp(0.0, self.length);
        (
            clip((self.w - spread) / self.cos),
            clip((self.w + spread) / self.cos),
        )
    }

    /// The volume on the low side of the surface at level `w`.
    fn volume(&self, w: f64) -> f64 {
        let (r, l) = (self.radius, self.length);
        let here = Self { w, ..*self };
        if self.cos < LEVEL {
            return l * segment(r, here.angle(0.0)).0;
        }
        let (full, empty) = here.reach();
        let wedge =
            r.powi(3) * self.sin / self.cos * (wedge(here.angle(full)) - wedge(here.angle(empty)));
        PI * r * r * full + wedge
    }

    /// The liquid's mass distribution in the cylinder's frame, `mass` of it.
    fn moments(&self, mass: f64) -> Mass {
        let (r, l) = (self.radius, self.length);
        // Volume, first moments in x and z, and second moments xx, yy, zz and xz.
        let mut sums = [0.0; 7];
        if self.cos < LEVEL {
            let (area, ax, axx, ayy) = segment(r, self.angle(0.0));
            sums = [
                area * l,
                ax * l,
                area * l * l / 2.0,
                axx * l,
                ayy * l,
                area * l.powi(3) / 3.0,
                ax * l * l / 2.0,
            ];
        } else {
            let (full, empty) = self.reach();
            // Whole discs from the base.
            let disc = PI * r * r;
            let around = disc * r * r / 4.0;
            sums[0] += disc * full;
            sums[2] += disc * full * full / 2.0;
            sums[3] += around * full;
            sums[4] += around * full;
            sums[5] += disc * full.powi(3) / 3.0;
            // Segments between, summed over the angle of the chord.
            let (from, to) = (self.angle(empty), self.angle(full));
            let step = (to - from) / PANELS as f64;
            for panel in 0..PANELS {
                let (a, b) = (from + step * panel as f64, from + step * (panel + 1) as f64);
                let (half, middle) = ((b - a) / 2.0, (a + b) / 2.0);
                for (&node, &weight) in NODES.iter().zip(&WEIGHTS) {
                    for sign in [-1.0, 1.0] {
                        let beta = middle + sign * node * half;
                        let slice = self.slice(beta);
                        for (sum, term) in sums.iter_mut().zip(slice) {
                            *sum += weight * half * term;
                        }
                    }
                }
            }
        }
        let [volume, sx, sz, qxx, qyy, qzz, qxz] = sums;
        let density = mass / volume;
        let center = DVec3::new(sx / volume, 0.0, sz / volume);
        let about_origin = DMat3::from_cols(
            DVec3::new(qyy + qzz, 0.0, -qxz),
            DVec3::new(0.0, qxx + qzz, 0.0),
            DVec3::new(-qxz, 0.0, qxx + qyy),
        ) * density;
        let mut liquid = Mass {
            mass,
            center,
            inertia: about_origin - offset(mass, center),
        };
        if self.mirrored {
            // End for end: z → length − z.
            let flip = DMat3::from_diagonal(DVec3::new(1.0, 1.0, -1.0));
            liquid.center.z = l - liquid.center.z;
            liquid.inertia = flip * liquid.inertia * flip;
        }
        liquid.placed(DVec3::ZERO, DQuat::from_rotation_z(self.yaw))
    }

    /// What the slice with chord angle `beta` adds, per radian of it, to the sums of
    /// [`Self::moments`].
    fn slice(&self, beta: f64) -> [f64; 7] {
        let (r, s, c, w) = (self.radius, self.sin, self.cos, self.w);
        let (area, ax, axx, ayy) = segment(r, beta);
        let z = (w + r * s * beta.cos()) / c;
        // dz = r sin β sin / cos · dβ
        let rate = r * s * beta.sin() / c;
        [area, ax, z * area, axx, ayy, z * z * area, z * ax].map(|term| term * rate)
    }
}

/// A disc of radius `r` cut by the chord at `x = −r cos β`: the area of the segment on the
/// side of −x, and the integrals over it of x, x² and y².
fn segment(r: f64, beta: f64) -> (f64, f64, f64, f64) {
    let (sin, cos) = beta.sin_cos();
    let r2 = r * r;
    (
        r2 * (beta - sin * cos),
        -2.0 / 3.0 * r2 * r * sin.powi(3),
        r2 * r2 * (beta / 4.0 - (4.0 * beta).sin() / 16.0),
        r2 * r2 * (beta / 4.0 - (2.0 * beta).sin() / 6.0 + (4.0 * beta).sin() / 48.0),
    )
}

/// The integral of `segment`'s area over the angle, weighted by sin β: the volume of the
/// wedge a slanting plane takes out of a cylinder, per `r³ sin / cos`.
fn wedge(beta: f64) -> f64 {
    let sin = beta.sin();
    sin - beta * beta.cos() - sin.powi(3) / 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    fn close_mat(a: DMat3, b: DMat3) -> bool {
        let size: f64 = b.abs().to_cols_array().iter().sum();
        a.to_cols_array()
            .iter()
            .zip(b.to_cols_array())
            .all(|(x, y)| (x - y).abs() <= 1e-9 * size.max(1.0))
    }

    /// The same liquid by counting small cubes of the cylinder on the low side of the
    /// surface at `w` (found to hold `fraction`): a slow, blunt, independent answer.
    fn counted(mass: f64, radius: f64, length: f64, fraction: f64, up: DVec3) -> Mass {
        let n = 90;
        let cube = 2.0 * radius / n as f64;
        let mut points = Vec::new();
        for i in 0..n {
            for j in 0..n {
                for k in 0..(length / cube).ceil() as usize {
                    let p = DVec3::new(
                        -radius + (i as f64 + 0.5) * cube,
                        -radius + (j as f64 + 0.5) * cube,
                        (k as f64 + 0.5) * cube,
                    );
                    if p.truncate().length() <= radius && p.z <= length {
                        points.push((p.dot(up), p));
                    }
                }
            }
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        let keep = (points.len() as f64 * fraction).round() as usize;
        let each = mass / keep as f64;
        points[..keep]
            .iter()
            .map(|&(_, p)| Mass::point(each, p))
            .sum()
    }

    #[test]
    fn upright_it_is_a_column_on_the_aft_dome_and_upside_down_on_the_other() {
        let (m, r, l) = (30.0, 0.5, 2.0);
        let (up, depth) = Mass::settled(m, r, l, 0.5, DVec3::Z);
        let column = Mass::cylinder(m, r, 0.5);
        assert!(close(up.center.z, column.center.z) && close_mat(up.inertia, column.inertia));
        assert!(close(depth, 0.5));
        let (down, depth) = Mass::settled(m, r, l, 0.5, -DVec3::Z);
        assert!(close(down.center.z, l - 0.25) && close_mat(down.inertia, column.inertia));
        assert!(close(depth, 0.5));
    }

    #[test]
    fn full_is_the_whole_cylinder_however_it_leans() {
        let (m, r, l) = (30.0, 0.5, 2.0);
        let whole = Mass::cylinder(m, r, l);
        for up in [DVec3::X, DVec3::new(1.0, 2.0, 0.5).normalize()] {
            let (liquid, depth) = Mass::settled(m, r, l, l, up);
            assert!(close_mat(liquid.inertia, whole.inertia));
            assert!(close(
                depth,
                l * up.z.abs() + 2.0 * r * up.truncate().length()
            ));
            // Just short of full, the general working gives the same.
            let (nearly, _) = Mass::settled(m, r, l, l * (1.0 - 1e-9), up);
            assert!((nearly.center - whole.center).length() < 1e-6);
            let size: f64 = whole.inertia.abs().to_cols_array().iter().sum();
            let diff: f64 = (nearly.inertia - whole.inertia)
                .abs()
                .to_cols_array()
                .iter()
                .sum();
            assert!(diff < 1e-6 * size);
        }
    }

    #[test]
    fn half_full_on_its_side_is_a_half_cylinder() {
        let (m, r, l) = (30.0, 0.5, 2.0);
        let (liquid, depth) = Mass::settled(m, r, l, l / 2.0, DVec3::X);
        // The flat face is level through the axis; the middle sits 4r/3π below it.
        assert!(close(depth, r));
        let drop = 4.0 * r / (3.0 * PI);
        assert!((liquid.center - DVec3::new(-drop, 0.0, l / 2.0)).length() < 1e-9);
        // About the axis it is half of a cylinder's m r²/2, less the offset of its middle.
        let axial = m * r * r / 2.0 - m * drop * drop;
        assert!(close(liquid.inertia.z_axis.z, axial));
        // Across (about X, level and across the axis) it is a cylinder's m (r²/4 + l²/12).
        let across = m * (r * r / 4.0 + l * l / 12.0);
        assert!(close(liquid.inertia.x_axis.x, across));
        // Turned the other way about the axis, the same at the other angle.
        let (turned, _) = Mass::settled(m, r, l, l / 2.0, DVec3::Y);
        assert!((turned.center - DVec3::new(0.0, -drop, l / 2.0)).length() < 1e-9);
    }

    #[test]
    fn a_leaning_tank_matches_counting_cubes() {
        let (m, r, l) = (30.0, 0.5, 2.0);
        for (up, fraction) in [
            (DVec3::new(0.6, 0.0, 0.8), 0.3),
            (DVec3::new(0.6, 0.0, 0.8), 0.9),
            (DVec3::new(-0.3, 0.5, -0.8124038).normalize(), 0.55),
            (DVec3::new(0.99, 0.1, 0.05).normalize(), 0.2),
            (DVec3::new(0.0, 1.0, 0.0), 0.7),
        ] {
            let (liquid, depth) = Mass::settled(m, r, l, fraction * l, up);
            let blunt = counted(m, r, l, fraction, up);
            let size = (blunt.inertia.abs().to_cols_array().iter().sum::<f64>()).max(1.0);
            assert!(close(liquid.mass, m));
            assert!((liquid.center - blunt.center).length() < 5e-3, "{up}");
            let diff: f64 = (liquid.inertia - blunt.inertia)
                .abs()
                .to_cols_array()
                .iter()
                .sum();
            assert!(diff < 0.02 * size, "{up} {fraction}: {diff} of {size}");
            assert!(depth > 0.0 && depth <= l * up.z.abs() + 2.0 * r * up.truncate().length());
        }
    }

    #[test]
    fn more_liquid_is_deeper_and_the_surface_holds_what_was_asked() {
        let (r, l) = (0.4, 1.5);
        let up = DVec3::new(0.5, 0.2, 0.7).normalize();
        let mut last = 0.0;
        for k in 1..=20 {
            let fraction = k as f64 / 21.0;
            let pool = Pool::new(r, l, fraction * l, up);
            let depth = pool.depth();
            assert!(depth > last);
            last = depth;
            // The closed form and the integrated moments agree on the volume.
            let held = pool.volume(pool.w) / (PI * r * r * l);
            assert!((held - fraction).abs() < 1e-9, "{held} {fraction}");
            let moments = pool.moments(1.0);
            assert!(close(moments.mass, 1.0));
            assert!(close(Mass::settled_depth(r, l, fraction * l, up), depth));
        }
    }

    #[test]
    fn level_and_nearly_level_axes_agree() {
        let (m, r, l) = (10.0, 0.3, 1.2);
        let level = Mass::settled(m, r, l, 0.4 * l, DVec3::X).0;
        let nearly = Mass::settled(m, r, l, 0.4 * l, DVec3::new(1.0, 0.0, 1e-5).normalize()).0;
        assert!((level.center - nearly.center).length() < 1e-4 * l);
        let alongside = Mass::settled(m, r, l, 0.4 * l, DVec3::new(1e-8, 0.0, 1.0).normalize()).0;
        let column = Mass::cylinder(m, r, 0.4 * l);
        assert!((alongside.center - column.center).length() < 1e-6);
    }
}
