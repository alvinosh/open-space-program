//! Moments of a shape at unit density: what a wall, region, skin or member measures.

use std::{
    iter::Sum,
    ops::{Add, Sub},
};

use glam::{DMat3, DQuat, DVec3};

use crate::mass::Mass;

/// A shape's measure (an area, volume or length), first moment ∫x and second moment
/// ∫x xᵀ, about the origin of its frame. Unlike [`Mass`] these add up directly, and
/// become a mass distribution for any density ([`Self::mass`]).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Moments {
    pub measure: f64,
    pub first: DVec3,
    pub second: DMat3,
}

impl Moments {
    pub const ZERO: Self = Self {
        measure: 0.0,
        first: DVec3::ZERO,
        second: DMat3::ZERO,
    };

    /// The centre of the shape, if it has a measure.
    pub fn centroid(&self) -> Option<DVec3> {
        (self.measure != 0.0).then(|| self.first / self.measure)
    }

    /// The mass distribution of the shape made of something `density` per unit of its
    /// measure (kg/m³ for a volume, kg/m² for a wall, kg/m for a member).
    pub fn mass(&self, density: f64) -> Mass {
        let mass = self.measure * density;
        let Some(center) = self.centroid().filter(|_| mass != 0.0) else {
            return Mass::ZERO;
        };
        let second = self.second * density;
        let trace = second.x_axis.x + second.y_axis.y + second.z_axis.z;
        // Inertia about the origin, tr(S) E − S, moved to the centre.
        let about_origin = DMat3::from_diagonal(DVec3::splat(trace)) - second;
        let shift =
            DMat3::from_diagonal(DVec3::splat(center.length_squared())) - outer(center, center);
        Mass {
            mass,
            center,
            inertia: about_origin - shift * mass,
        }
    }

    /// Turned by `rotation` about the origin, then moved to `position`.
    pub fn placed(self, position: DVec3, rotation: DQuat) -> Self {
        let turn = DMat3::from_quat(rotation);
        let first = turn * self.first;
        Self {
            measure: self.measure,
            first: first + position * self.measure,
            second: turn * self.second * turn.transpose()
                + outer(first, position)
                + outer(position, first)
                + outer(position, position) * self.measure,
        }
    }

    pub fn scaled(self, factor: f64) -> Self {
        Self {
            measure: self.measure * factor,
            first: self.first * factor,
            second: self.second * factor,
        }
    }

    /// A triangle, by area.
    pub fn triangle(a: DVec3, b: DVec3, c: DVec3) -> Self {
        let area = (b - a).cross(c - a).length() / 2.0;
        let sum = a + b + c;
        Self {
            measure: area,
            first: sum * area / 3.0,
            second: (outer(a, a) + outer(b, b) + outer(c, c) + outer(sum, sum)) * (area / 12.0),
        }
    }

    /// The tetrahedron from `apex` to the triangle `a b c`, by volume: positive when the
    /// triangle runs counter-clockwise seen from outside, away from the apex.
    pub fn tetrahedron(apex: DVec3, a: DVec3, b: DVec3, c: DVec3) -> Self {
        let volume = (a - apex).dot((b - apex).cross(c - apex)) / 6.0;
        let sum = apex + a + b + c;
        let squares = outer(apex, apex) + outer(a, a) + outer(b, b) + outer(c, c);
        Self {
            measure: volume,
            first: sum * volume / 4.0,
            second: (squares + outer(sum, sum)) * (volume / 20.0),
        }
    }

    /// A straight member from `a` to `b`, by length.
    pub fn segment(a: DVec3, b: DVec3) -> Self {
        let d = b - a;
        let length = d.length();
        Self {
            measure: length,
            first: (a + b) * length / 2.0,
            second: (outer(a, a) + (outer(a, d) + outer(d, a)) / 2.0 + outer(d, d) / 3.0) * length,
        }
    }
}

impl Add for Moments {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            measure: self.measure + other.measure,
            first: self.first + other.first,
            second: self.second + other.second,
        }
    }
}

impl Sub for Moments {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        self + other.scaled(-1.0)
    }
}

impl Sum for Moments {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, Add::add)
    }
}

/// a bᵀ.
pub fn outer(a: DVec3, b: DVec3) -> DMat3 {
    DMat3::from_cols(a * b.x, a * b.y, a * b.z)
}

/// The symmetric matrix with these diagonal and off-diagonal entries.
pub fn symmetric(xx: f64, yy: f64, zz: f64, xy: f64, xz: f64, yz: f64) -> DMat3 {
    DMat3::from_cols(
        DVec3::new(xx, xy, xz),
        DVec3::new(xy, yy, yz),
        DVec3::new(xz, yz, zz),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: DMat3, b: DMat3, tolerance: f64) -> bool {
        (a - b).to_cols_array().iter().all(|x| x.abs() <= tolerance)
    }

    #[test]
    fn a_member_has_the_mass_of_a_rod() {
        let (a, b) = (DVec3::new(1.0, 2.0, 3.0), DVec3::new(-2.0, 0.5, 4.0));
        let m = Moments::segment(a, b).mass(3.0);
        let rod = Mass::rod(3.0 * (b - a).length(), a, b);
        assert!((m.mass - rod.mass).abs() < 1e-12);
        assert!((m.center - rod.center).length() < 1e-12);
        assert!(close(m.inertia, rod.inertia, 1e-11));
    }

    #[test]
    fn placing_moments_places_their_mass() {
        let m = Moments::segment(DVec3::ZERO, DVec3::new(0.0, 0.0, 2.0))
            + Moments::triangle(DVec3::X, DVec3::Y, DVec3::new(0.3, 0.2, 1.0));
        let (p, q) = (
            DVec3::new(4.0, -1.0, 2.0),
            DQuat::from_euler(glam::EulerRot::XYZ, 0.3, -0.7, 1.1),
        );
        let direct = m.placed(p, q).mass(2.5);
        let moved = m.mass(2.5).placed(p, q);
        assert!((direct.center - moved.center).length() < 1e-12);
        assert!(close(direct.inertia, moved.inertia, 1e-11));
    }

    #[test]
    fn tetrahedra_from_any_apex_add_up_to_the_solid() {
        // A unit cube's six faces as triangles, counter-clockwise from outside.
        let v = |x: f64, y: f64, z: f64| DVec3::new(x, y, z);
        let faces = [
            [v(0., 0., 0.), v(0., 1., 0.), v(1., 1., 0.), v(1., 0., 0.)],
            [v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)],
            [v(0., 0., 0.), v(1., 0., 0.), v(1., 0., 1.), v(0., 0., 1.)],
            [v(0., 1., 0.), v(0., 1., 1.), v(1., 1., 1.), v(1., 1., 0.)],
            [v(0., 0., 0.), v(0., 0., 1.), v(0., 1., 1.), v(0., 1., 0.)],
            [v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 1.), v(1., 0., 1.)],
        ];
        for apex in [DVec3::ZERO, v(5.0, -3.0, 2.0)] {
            let cube: Moments = faces
                .iter()
                .flat_map(|[a, b, c, d]| {
                    [
                        Moments::tetrahedron(apex, *a, *b, *c),
                        Moments::tetrahedron(apex, *a, *c, *d),
                    ]
                })
                .sum();
            let m = cube.mass(6.0);
            assert!((m.mass - 6.0).abs() < 1e-12);
            assert!((m.center - DVec3::splat(0.5)).length() < 1e-12);
            // A cube's inertia about its centre: m a² / 6 on every axis.
            assert!(close(m.inertia, DMat3::from_diagonal(DVec3::ONE), 1e-11));
        }
    }
}
