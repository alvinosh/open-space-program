//! Closed triangle meshes, and the polygon work behind them.
//!
//! A closed mesh with its triangles counter-clockwise seen from outside measures exactly
//! the polyhedron it bounds: its volume integrals are sums over the tetrahedra from any one
//! point to its triangles (the divergence theorem). Cut by a plane, the same sum over the
//! clipped triangles, from a point on the plane, measures the part below it: the cut face
//! adds nothing, as it lies in the plane with that point. That is how liquid settles in a
//! tank of any shape, in any direction.

use std::collections::HashSet;

use glam::{DVec2, DVec3};

use super::moments::Moments;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriMesh {
    pub positions: Vec<DVec3>,
    /// Counter-clockwise seen from outside.
    pub triangles: Vec<[u32; 3]>,
}

impl TriMesh {
    fn corners(&self, triangle: [u32; 3]) -> [DVec3; 3] {
        triangle.map(|i| self.positions[i as usize])
    }

    /// What it encloses, by volume.
    pub fn volume_moments(&self) -> Moments {
        self.triangles
            .iter()
            .map(|&t| {
                let [a, b, c] = self.corners(t);
                Moments::tetrahedron(DVec3::ZERO, a, b, c)
            })
            .sum()
    }

    /// Its surface, by area.
    pub fn surface_moments(&self) -> Moments {
        self.triangles
            .iter()
            .map(|&t| {
                let [a, b, c] = self.corners(t);
                Moments::triangle(a, b, c)
            })
            .sum()
    }

    /// Turn its triangles inside out if they face in.
    pub fn orient_outward(&mut self) {
        if self.volume_moments().measure < 0.0 {
            for t in &mut self.triangles {
                t.swap(1, 2);
            }
        }
    }

    /// Whether every edge joins two triangles that run along it in opposite directions: a
    /// watertight surface with a consistent inside and outside.
    pub fn is_closed(&self) -> bool {
        let mut edges = HashSet::new();
        for &[a, b, c] in &self.triangles {
            for edge in [(a, b), (b, c), (c, a)] {
                if !edges.insert(edge) {
                    return false;
                }
            }
        }
        edges.iter().all(|&(a, b)| edges.contains(&(b, a)))
    }

    /// The lowest and highest it reaches along `up`.
    pub fn extent(&self, up: DVec3) -> (f64, f64) {
        self.positions
            .iter()
            .map(|p| up.dot(*p))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), h| {
                (lo.min(h), hi.max(h))
            })
    }

    /// Each piece of its triangles no higher than `level` along the unit vector `up`, as
    /// triangles.
    fn clipped(&self, up: DVec3, level: f64, mut each: impl FnMut(DVec3, DVec3, DVec3)) {
        for &t in &self.triangles {
            let corners = self.corners(t);
            let heights = corners.map(|p| up.dot(p) - level);
            if heights.iter().all(|h| *h > 0.0) {
                continue;
            }
            let mut clipped = [DVec3::ZERO; 4];
            let mut n = 0;
            for k in 0..3 {
                let (a, b) = (corners[k], corners[(k + 1) % 3]);
                let (ha, hb) = (heights[k], heights[(k + 1) % 3]);
                if ha <= 0.0 {
                    clipped[n] = a;
                    n += 1;
                }
                if (ha < 0.0 && hb > 0.0) || (ha > 0.0 && hb < 0.0) {
                    clipped[n] = a + (b - a) * (ha / (ha - hb));
                    n += 1;
                }
            }
            for k in 1..n.saturating_sub(1) {
                each(clipped[0], clipped[k], clipped[k + 1]);
            }
        }
    }

    /// What it encloses no higher than `level` along the unit vector `up`, by volume.
    pub fn below(&self, up: DVec3, level: f64) -> Moments {
        let apex = up * level;
        let mut moments = Moments::ZERO;
        self.clipped(up, level, |a, b, c| {
            moments = moments + Moments::tetrahedron(apex, a, b, c);
        });
        moments
    }

    /// The volume it encloses no higher than `level` along the unit vector `up`, and the
    /// area of its cross-section there.
    pub fn volume_below(&self, up: DVec3, level: f64) -> (f64, f64) {
        let apex = up * level;
        let (mut volume, mut faces) = (0.0, DVec3::ZERO);
        self.clipped(up, level, |a, b, c| {
            let face = (b - a).cross(c - a);
            volume += (a - apex).dot(face) / 6.0;
            faces += face / 2.0;
        });
        // A closed surface's faces add up to nothing, so the cut face is minus the rest.
        (volume, (-faces).dot(up).max(0.0))
    }

    /// The level along the unit vector `up` below which it encloses `volume` of the `total`
    /// it encloses ([`Self::volume_moments`]).
    pub fn level(&self, up: DVec3, volume: f64, total: f64) -> f64 {
        let (lo, hi) = self.extent(up);
        solve_level(lo, hi, total, volume, |h| self.volume_below(up, h))
    }
}

/// The level between `lo` and `hi` at which `below` (volume below a level, and the
/// cross-section there) reaches `volume` of `total`: Newton's method on the cross-section,
/// kept within a shrinking bracket.
pub fn solve_level(
    mut lo: f64,
    mut hi: f64,
    total: f64,
    volume: f64,
    below: impl Fn(f64) -> (f64, f64),
) -> f64 {
    let volume = volume.clamp(0.0, total);
    if volume <= 0.0 {
        return lo;
    }
    if volume >= total {
        return hi;
    }
    let span = hi - lo;
    let mut level = lo + span * volume / total;
    for _ in 0..200 {
        let (v, area) = below(level);
        let error = v - volume;
        if error.abs() <= 1e-13 * total {
            break;
        }
        if error > 0.0 {
            hi = level;
        } else {
            lo = level;
        }
        let step = level - error / area;
        level = if area > 0.0 && step > lo && step < hi {
            step
        } else {
            (lo + hi) / 2.0
        };
        if hi - lo <= 1e-15 * span {
            break;
        }
    }
    level
}

/// Twice the signed area of a polygon: positive when it runs counter-clockwise.
pub fn signed_area(polygon: &[DVec2]) -> f64 {
    let n = polygon.len();
    (0..n)
        .map(|i| polygon[i].perp_dot(polygon[(i + 1) % n]))
        .sum::<f64>()
        / 2.0
}

/// Whether a polygon (closed from its last point back to its first) has no two edges that
/// cross.
pub fn is_simple(polygon: &[DVec2]) -> bool {
    let n = polygon.len();
    let size = polygon
        .iter()
        .fold(0.0f64, |m, p| m.max(p.x.abs()).max(p.y.abs()));
    let eps = 1e-12 * size * size;
    let orient = |a: DVec2, b: DVec2, c: DVec2| {
        let o = (b - a).perp_dot(c - a);
        if o.abs() <= eps { 0.0 } else { o.signum() }
    };
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        for j in i + 2..n {
            if i == 0 && j == n - 1 {
                continue;
            }
            let (c, d) = (polygon[j], polygon[(j + 1) % n]);
            if orient(a, b, c) * orient(a, b, d) < 0.0 && orient(c, d, a) * orient(c, d, b) < 0.0 {
                return false;
            }
        }
    }
    true
}

/// Triangles covering a simple polygon, as indices into it, counter-clockwise. Ear
/// clipping; a corner where the polygon runs straight on becomes a flat triangle, so that
/// every edge of the polygon is still an edge of a triangle.
pub fn triangulate(polygon: &[DVec2]) -> Vec<[usize; 3]> {
    let n = polygon.len();
    if n < 3 {
        return Vec::new();
    }
    let mut ring: Vec<usize> = (0..n).collect();
    if signed_area(polygon) < 0.0 {
        ring.reverse();
    }
    let size = polygon
        .iter()
        .fold(0.0f64, |m, p| m.max(p.x.abs()).max(p.y.abs()));
    let eps = 1e-12 * size * size;
    let p = |i: usize| polygon[i];
    let turn = |a: usize, b: usize, c: usize| (p(b) - p(a)).perp_dot(p(c) - p(b));
    let mut out = Vec::with_capacity(n - 2);
    while ring.len() > 3 {
        let m = ring.len();
        let corner = |k: usize| (ring[(k + m - 1) % m], ring[k], ring[(k + 1) % m]);
        let flat = (0..m).find(|&k| {
            let (a, b, c) = corner(k);
            turn(a, b, c).abs() <= eps
        });
        let ear = flat.or_else(|| {
            (0..m).find(|&k| {
                let (a, b, c) = corner(k);
                turn(a, b, c) > eps
                    && !ring
                        .iter()
                        .any(|&q| q != a && q != b && q != c && inside(p(q), p(a), p(b), p(c), eps))
            })
        });
        let Some(k) = ear else {
            // Numerically stuck: fan what is left.
            for k in 1..ring.len() - 1 {
                out.push([ring[0], ring[k], ring[k + 1]]);
            }
            return out;
        };
        let (a, b, c) = corner(k);
        out.push([a, b, c]);
        ring.remove(k);
    }
    out.push([ring[0], ring[1], ring[2]]);
    out
}

/// Whether `q` is inside or on the counter-clockwise triangle `a b c`.
fn inside(q: DVec2, a: DVec2, b: DVec2, c: DVec2, eps: f64) -> bool {
    (b - a).perp_dot(q - a) >= -eps
        && (c - b).perp_dot(q - b) >= -eps
        && (a - c).perp_dot(q - c) >= -eps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygons_triangulate_with_their_area() {
        // An L shape, clockwise, with a straight run along its bottom.
        let l = [
            DVec2::new(0.0, 0.0),
            DVec2::new(0.0, 2.0),
            DVec2::new(1.0, 2.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(2.0, 1.0),
            DVec2::new(2.0, 0.0),
            DVec2::new(1.0, 0.0),
        ];
        assert!(is_simple(&l));
        let triangles = triangulate(&l);
        assert_eq!(triangles.len(), l.len() - 2);
        let area: f64 = triangles
            .iter()
            .map(|t| signed_area(&t.map(|i| l[i])))
            .sum();
        assert!((area - 3.0).abs() < 1e-12, "{area}");
        assert!(
            triangles
                .iter()
                .all(|t| signed_area(&t.map(|i| l[i])) >= -1e-12)
        );
        let bow_tie = [DVec2::ZERO, DVec2::ONE, DVec2::X, DVec2::Y];
        assert!(!is_simple(&bow_tie));
    }
}
