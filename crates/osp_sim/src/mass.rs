//! Mass distributions: how much mass there is, where its centre is and its inertia tensor.
//!
//! Parts are made of simple shapes with exact moments: points, rods, plates, solid
//! cylinders, thin tubes, solid and hollow spheres, and solid frustums (cones among them).
//! Each is placed in the assembly frame ([`crate::parts`]), and they add up with the
//! parallel-axis theorem. The sum's tensor is generally not diagonal in the vessel's axes;
//! its principal axes ([`principal_axes`]) are the body frame, in which it is.

use std::{f64::consts::PI, iter::Sum, ops::Add};

use glam::{DMat3, DQuat, DVec3};

/// A rigid mass distribution: kg, its centre (m) and its inertia tensor about that centre
/// (kg·m²), in some frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mass {
    pub mass: f64,
    pub center: DVec3,
    pub inertia: DMat3,
}

impl Mass {
    pub const ZERO: Self = Self {
        mass: 0.0,
        center: DVec3::ZERO,
        inertia: DMat3::ZERO,
    };

    /// All of it at one point.
    pub fn point(mass: f64, at: DVec3) -> Self {
        Self {
            mass,
            center: at,
            inertia: DMat3::ZERO,
        }
    }

    /// A thin rod from `a` to `b`.
    pub fn rod(mass: f64, a: DVec3, b: DVec3) -> Self {
        Self {
            mass,
            center: (a + b) / 2.0,
            inertia: offset(mass / 12.0, b - a),
        }
    }

    /// A thin plate around `center` with edges `u` and `v` (a parallelogram).
    pub fn plate(mass: f64, center: DVec3, u: DVec3, v: DVec3) -> Self {
        Self {
            mass,
            center,
            inertia: offset(mass / 12.0, u) + offset(mass / 12.0, v),
        }
    }

    /// A solid cylinder standing on the origin, up +Z.
    pub fn cylinder(mass: f64, radius: f64, length: f64) -> Self {
        let (r2, l2) = (radius * radius, length * length);
        let across = mass * (r2 / 4.0 + l2 / 12.0);
        Self {
            mass,
            center: DVec3::Z * length / 2.0,
            inertia: DMat3::from_diagonal(DVec3::new(across, across, mass * r2 / 2.0)),
        }
    }

    /// A thin-walled tube, open at the ends, standing on the origin, up +Z.
    pub fn tube(mass: f64, radius: f64, length: f64) -> Self {
        let (r2, l2) = (radius * radius, length * length);
        let across = mass * (r2 / 2.0 + l2 / 12.0);
        Self {
            mass,
            center: DVec3::Z * length / 2.0,
            inertia: DMat3::from_diagonal(DVec3::new(across, across, mass * r2)),
        }
    }

    /// The +X half of [`Self::tube`], split along its axis.
    pub fn half_tube(mass: f64, radius: f64, length: f64) -> Self {
        // About the axis it is the whole tube's; its centre is 2r/π out along X.
        let tube = Self::tube(mass, radius, length);
        let center = tube.center + DVec3::X * 2.0 * radius / PI;
        Self {
            mass,
            center,
            inertia: tube.inertia - offset(mass, center - tube.center),
        }
    }

    /// A solid sphere around the origin.
    pub fn sphere(mass: f64, radius: f64) -> Self {
        let moment = 0.4 * mass * radius * radius;
        Self {
            mass,
            center: DVec3::ZERO,
            inertia: DMat3::from_diagonal(DVec3::splat(moment)),
        }
    }

    /// A thin spherical shell around the origin.
    pub fn spherical_shell(mass: f64, radius: f64) -> Self {
        let moment = 2.0 / 3.0 * mass * radius * radius;
        Self {
            mass,
            center: DVec3::ZERO,
            inertia: DMat3::from_diagonal(DVec3::splat(moment)),
        }
    }

    /// A solid frustum standing on the origin, up +Z: radius `bottom` at its base, `top` at
    /// its top (a cone when that is 0).
    pub fn frustum(mass: f64, bottom: f64, top: f64, height: f64) -> Self {
        let (a, b, h) = (bottom, top, height);
        // Integrals of r², r⁴ and r²z² up the height, without the common factors.
        let s2 = a * a + a * b + b * b;
        if s2 <= 0.0 {
            return Self::rod(mass, DVec3::ZERO, DVec3::Z * h);
        }
        let s4 = a.powi(4) + a.powi(3) * b + a * a * b * b + a * b.powi(3) + b.powi(4);
        let t = a * a + 3.0 * a * b + 6.0 * b * b;
        let centroid = h * (a * a + 2.0 * a * b + 3.0 * b * b) / (4.0 * s2);
        let axial = 0.3 * mass * s4 / s2;
        let across_base = mass * (0.15 * s4 / s2 + h * h * t / (10.0 * s2));
        let across = across_base - mass * centroid * centroid;
        Self {
            mass,
            center: DVec3::Z * centroid,
            inertia: DMat3::from_diagonal(DVec3::new(across, across, axial)),
        }
    }

    /// A thin wall of revolution about +Z through `outline`, (height, radius) points, open
    /// at the ends: the mass spread evenly over its area, band by conical band.
    pub fn revolved_shell(mass: f64, outline: &[(f64, f64)]) -> Self {
        // Integrals over the area of r, r·z, r·z² and r³ (r² about the axis), without 2π.
        let (mut area, mut z1, mut z2, mut r3) = (0.0, 0.0, 0.0, 0.0);
        for w in outline.windows(2) {
            let [(c, a), (z, r)] = [w[0], w[1]];
            // r = a + b·t and z = c + d·t along a band of slant length s, t from 0 to 1.
            let (b, d) = (r - a, z - c);
            let s = b.hypot(d);
            area += s * (a + b / 2.0);
            z1 += s * (a * c + (a * d + b * c) / 2.0 + b * d / 3.0);
            z2 += s
                * (a * (c * c + c * d + d * d / 3.0)
                    + b * (c * c / 2.0 + 2.0 * c * d / 3.0 + d * d / 4.0));
            r3 += s * (a.powi(3) + 1.5 * a * a * b + a * b * b + b.powi(3) / 4.0);
        }
        if area <= 0.0 {
            let at = outline.first().map_or(0.0, |p| p.0);
            return Self::point(mass, DVec3::Z * at);
        }
        let per = mass / area;
        let height = z1 / area;
        let across = per * (r3 / 2.0 + z2) - mass * height * height;
        Self {
            mass,
            center: DVec3::Z * height,
            inertia: DMat3::from_diagonal(DVec3::new(across, across, per * r3)),
        }
    }

    /// Turned by `rotation` about the origin, then moved to `position`.
    pub fn placed(self, position: DVec3, rotation: DQuat) -> Self {
        let turn = DMat3::from_quat(rotation);
        Self {
            mass: self.mass,
            center: position + rotation * self.center,
            inertia: turn * self.inertia * turn.transpose(),
        }
    }

    /// The inertia tensor about `point` (parallel-axis theorem).
    pub fn inertia_about(&self, point: DVec3) -> DMat3 {
        self.inertia + offset(self.mass, self.center - point)
    }
}

impl Add for Mass {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        let mass = self.mass + other.mass;
        if mass <= 0.0 {
            return Self::ZERO;
        }
        let center = (self.center * self.mass + other.center * other.mass) / mass;
        Self {
            mass,
            center,
            inertia: self.inertia_about(center) + other.inertia_about(center),
        }
    }
}

impl Sum for Mass {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, Add::add)
    }
}

/// What `mass` at `d` from a point adds to the inertia about it: m (|d|² E − d dᵀ).
fn offset(mass: f64, d: DVec3) -> DMat3 {
    let outer = DMat3::from_cols(d * d.x, d * d.y, d * d.z);
    (DMat3::from_diagonal(DVec3::splat(d.length_squared())) - outer) * mass
}

/// The principal moments of a symmetric inertia tensor, smallest first, and the rotation
/// that takes the principal axes (body frame) into the tensor's frame. Cyclic Jacobi
/// rotations, exact to rounding.
pub fn principal_axes(tensor: &DMat3) -> (DVec3, DQuat) {
    let mut a = tensor.to_cols_array_2d();
    let mut v = DMat3::IDENTITY.to_cols_array_2d();
    let scale: f64 = (0..3).map(|i| a[i][i].abs()).sum();
    for _ in 0..32 {
        let off = a[0][1].powi(2) + a[0][2].powi(2) + a[1][2].powi(2);
        if off <= (1e-15 * scale).powi(2) {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q] == 0.0 {
                continue;
            }
            // Zero a[p][q] with a rotation in the p–q plane.
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for row in &mut a {
                let (akp, akq) = (row[p], row[q]);
                row[p] = c * akp - s * akq;
                row[q] = s * akp + c * akq;
            }
            let (rp, rq) = (a[p], a[q]);
            a[p] = std::array::from_fn(|k| c * rp[k] - s * rq[k]);
            a[q] = std::array::from_fn(|k| s * rp[k] + c * rq[k]);
            // Gather the rotations: `v[row][column]`, its columns become the axes.
            for row in &mut v {
                let (vp, vq) = (row[p], row[q]);
                row[p] = c * vp - s * vq;
                row[q] = s * vp + c * vq;
            }
        }
    }
    let axes = DMat3::from_cols_array_2d(&v).transpose();
    let mut order = [0, 1, 2];
    order.sort_by(|&i, &j| a[i][i].total_cmp(&a[j][j]));
    let moments = DVec3::from_array(order.map(|i| a[i][i]));
    let [x, y, _] = order.map(|i| axes.col(i));
    // A right-handed frame, so that it is a rotation.
    let rotation = DQuat::from_mat3(&DMat3::from_cols(x, y, x.cross(y)));
    (moments, rotation.normalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    fn close_mat(a: DMat3, b: DMat3) -> bool {
        a.to_cols_array()
            .iter()
            .zip(b.to_cols_array())
            .all(|(x, y)| (x - y).abs() <= 1e-9 * b.abs().to_cols_array().iter().sum::<f64>())
    }

    #[test]
    fn a_cone_and_a_frustum_have_their_textbook_moments() {
        let (m, r, h) = (12.0, 1.5, 4.0);
        let cone = Mass::frustum(m, r, 0.0, h);
        assert!(close(cone.center.z, h / 4.0));
        assert!(close(cone.inertia.z_axis.z, 0.3 * m * r * r));
        // About its centre: 3/20 m r² + 3/80 m h².
        assert!(close(
            cone.inertia.x_axis.x,
            m * (0.15 * r * r + 0.0375 * h * h)
        ));
        // Equal radii make a cylinder.
        let even = Mass::frustum(m, r, r, h);
        let cylinder = Mass::cylinder(m, r, h);
        assert!(close(even.center.z, cylinder.center.z));
        assert!(close_mat(even.inertia, cylinder.inertia));
    }

    #[test]
    fn walls_of_revolution_have_their_textbook_moments() {
        // Straight up, it is a tube, wherever it stands.
        let (m, r, h) = (12.0, 1.5, 4.0);
        let wall = Mass::revolved_shell(m, &[(2.0, r), (2.0 + h, r)]);
        let tube = Mass::tube(m, r, h);
        assert!(close(wall.center.z, 2.0 + h / 2.0));
        assert!(close_mat(wall.inertia, tube.inertia));
        // A cone's lateral surface: its centre a third of the way up, m r²/2 about the
        // axis and m (r²/4 + h²/18) across.
        let cone = Mass::revolved_shell(m, &[(0.0, r), (h, 0.0)]);
        assert!(close(cone.center.z, h / 3.0));
        assert!(close(cone.inertia.z_axis.z, m * r * r / 2.0));
        assert!(close(
            cone.inertia.x_axis.x,
            m * (r * r / 4.0 + h * h / 18.0)
        ));
        // Cut in two bands, it adds back up.
        let split = Mass::revolved_shell(m, &[(0.0, r), (h / 2.0, r / 2.0), (h, 0.0)]);
        assert!(close(split.center.z, cone.center.z));
        assert!(close_mat(split.inertia, cone.inertia));
        // A flat ring: an annulus.
        let disc = Mass::revolved_shell(m, &[(0.0, 0.0), (0.0, r)]);
        assert!(close(disc.inertia.z_axis.z, m * r * r / 2.0));
        assert!(close(disc.inertia.x_axis.x, m * r * r / 4.0));
    }

    #[test]
    fn adding_follows_the_parallel_axis_theorem() {
        // Two points 2 m apart: 2 kg each, 1 m from the middle.
        let pair = Mass::point(2.0, DVec3::X) + Mass::point(2.0, -DVec3::X);
        assert_eq!(pair.center, DVec3::ZERO);
        assert!(close_mat(
            pair.inertia,
            DMat3::from_diagonal(DVec3::new(0.0, 4.0, 4.0))
        ));
        // A rod is a line of points, and cut in two its halves add back up to it.
        let rod = Mass::rod(6.0, DVec3::ZERO, DVec3::new(3.0, 4.0, 0.0));
        let halves = Mass::rod(3.0, DVec3::ZERO, DVec3::new(1.5, 2.0, 0.0))
            + Mass::rod(3.0, DVec3::new(1.5, 2.0, 0.0), DVec3::new(3.0, 4.0, 0.0));
        assert!(close_mat(halves.inertia, rod.inertia));
        assert!(close(rod.inertia.z_axis.z, 6.0 * 25.0 / 12.0));
        // Two half tubes make a tube.
        let half = Mass::half_tube(5.0, 2.0, 6.0);
        let both = half + half.placed(DVec3::ZERO, DQuat::from_rotation_z(PI));
        let tube = Mass::tube(10.0, 2.0, 6.0);
        assert!(close(both.center.z, 3.0) && both.center.truncate().length() < 1e-12);
        assert!(close_mat(both.inertia, tube.inertia));
    }

    #[test]
    fn turning_turns_the_tensor() {
        // A rod along X, turned to lie along Y.
        let rod = Mass::rod(1.0, -DVec3::X, DVec3::X);
        let turned = rod.placed(DVec3::Z, DQuat::from_rotation_z(PI / 2.0));
        assert!(close_mat(
            turned.inertia,
            Mass::rod(1.0, -DVec3::Y, DVec3::Y).inertia
        ));
        assert!((turned.center - DVec3::Z).length() < 1e-12);
    }

    #[test]
    fn principal_axes_diagonalise_the_tensor() {
        // A plate tilted every which way.
        let tilt = DQuat::from_euler(glam::EulerRot::XYZ, 0.4, -0.9, 1.3);
        let plate =
            Mass::plate(3.0, DVec3::ZERO, DVec3::X * 4.0, DVec3::Y * 1.0).placed(DVec3::ZERO, tilt);
        let (moments, axes) = principal_axes(&plate.inertia);
        // m/12 (b²), m/12 (a²), m/12 (a² + b²), smallest first.
        let expected = DVec3::new(0.25, 4.0, 4.25);
        assert!((moments - expected).length() < 1e-12, "{moments}");
        let body = DMat3::from_quat(axes);
        let diagonal = body.transpose() * plate.inertia * body;
        assert!(close_mat(diagonal, DMat3::from_diagonal(expected)));
        // Already diagonal: the axes stay put, ordered by moment.
        let (moments, axes) = principal_axes(&DMat3::from_diagonal(DVec3::new(1.0, 2.0, 3.0)));
        assert_eq!(moments, DVec3::new(1.0, 2.0, 3.0));
        assert!(axes.angle_between(DQuat::IDENTITY) < 1e-12);
    }
}
