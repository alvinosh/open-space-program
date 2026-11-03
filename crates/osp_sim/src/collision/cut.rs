//! Cutting a part's surface into convex pieces that follow it.
//!
//! The contact solver works on convex shapes. A piece is good enough when no point of its hull's
//! surface stands further off the part's own surface than a few centimetres. So a closed tank is
//! one hull, because its hull is its surface; and the end of an open tube is far from anything, so
//! it is cut and cut again until it is a ring of panels with nothing across the middle.
//!
//! That is a distance on purpose. Measured by volume, as the usual decompositions do it, a thin disc
//! right across a hollow is nearly no error at all, and yet nothing can get past it.
//!
//! Each cut goes through the point standing furthest off, which is where a cut does the most good,
//! along whichever way leaves the least empty space in the hulls of the two sides.

use glam::{DMat3, DVec3};
use rapier3d_f64::parry::{query::PointQuery, shape::TriMesh, transformation::try_convex_hull};

use crate::mass;

/// How far a piece may stand off the surface, as a share of its part's size, and within what, m:
/// more than anyone can see, and well inside the 15 cm docking ports capture at.
const STAND_OFF: f64 = 0.006;
const LEAST_STAND_OFF: f64 = 0.03;
const MOST_STAND_OFF: f64 = 0.05;
/// The most pieces in a part, the worst of them being cut first.
const MOST_PIECES: usize = 96;
/// How many points of a piece's hull are measured.
const SAMPLES: f64 = 400.0;
/// How many corners a side is judged by, when choosing between cuts.
const CORNERS_TO_JUDGE: usize = 256;
/// How thick a piece that is flat is made, each way, m: a hull needs some depth to have an inside.
const FLAT: f64 = 0.01;
/// The least there can be of a side of a cut, along the way it is cut, m.
const LEAST_SIDE: f64 = 1e-3;
/// A hull with less volume than this has no inside, m³.
const NO_VOLUME: f64 = 1e-10;

type Triangle = [DVec3; 3];

/// A convex hull: its corners and its faces.
struct Hull {
    points: Vec<DVec3>,
    faces: Vec<[u32; 3]>,
}

impl Hull {
    fn volume(&self) -> f64 {
        let about = self.points.first().copied().unwrap_or(DVec3::ZERO);
        let corner = |i: u32| self.points[i as usize] - about;
        (self.faces.iter())
            .map(|f| corner(f[0]).dot(corner(f[1]).cross(corner(f[2]))))
            .sum::<f64>()
            .abs()
            / 6.0
    }

    /// The hull of `points`, given some depth if they all lie in a plane: a floor, a flat fin.
    fn of(points: &[DVec3]) -> Option<Self> {
        let solid = |points: &[DVec3]| {
            let (points, faces) = try_convex_hull(points).ok()?;
            Some(Self { points, faces }).filter(|hull| hull.volume() > NO_VOLUME)
        };
        solid(points).or_else(|| {
            let facing = spread_axes(points)[0];
            let thick: Vec<DVec3> = points
                .iter()
                .flat_map(|&p| [p + facing * FLAT, p - facing * FLAT])
                .collect();
            solid(&thick)
        })
    }

    /// How far it stands off `surface`, and the point of it that does so most.
    fn standing_off(&self, surface: &TriMesh) -> (f64, DVec3) {
        let corners = |face: &[u32; 3]| face.map(|i| self.points[i as usize]);
        let area = |[a, b, c]: [DVec3; 3]| (b - a).cross(c - a).length() / 2.0;
        let total: f64 = self.faces.iter().map(|f| area(corners(f))).sum();
        let mut worst = (0.0, self.points.first().copied().unwrap_or(DVec3::ZERO));
        for face in &self.faces {
            let [a, b, c] = corners(face);
            // Evenly over the hull by area, and the middle of even the smallest face.
            let wanted = SAMPLES * area([a, b, c]) / total.max(1e-12);
            let rows = (2.0 * wanted).sqrt().ceil().max(1.0) as usize;
            let step = 1.0 / rows as f64;
            for i in 0..rows {
                for j in 0..rows - i {
                    let u = (i as f64 + 1.0 / 3.0) * step;
                    let v = (j as f64 + 1.0 / 3.0) * step;
                    let point = a + (b - a) * u + (c - a) * v;
                    let off = surface.distance_to_local_point(point, false);
                    if off > worst.0 {
                        worst = (off, point);
                    }
                }
            }
        }
        worst
    }
}

/// The ways `points` are spread out, least first.
fn spread_axes(points: &[DVec3]) -> [DVec3; 3] {
    let middle = points.iter().copied().sum::<DVec3>() / points.len().max(1) as f64;
    let spread = points.iter().fold(DMat3::ZERO, |all, &p| {
        let off = p - middle;
        all + DMat3::from_cols(off * off.x, off * off.y, off * off.z)
    });
    let (_, axes) = mass::principal_axes(&spread);
    [axes * DVec3::X, axes * DVec3::Y, axes * DVec3::Z]
}

fn corners_of(triangles: &[Triangle]) -> Vec<DVec3> {
    let mut seen = std::collections::HashSet::new();
    triangles
        .iter()
        .flatten()
        .copied()
        .filter(|p| seen.insert(p.to_array().map(f64::to_bits)))
        .collect()
}

/// `triangles` on either side of the plane through `point` facing `normal`, cut where they cross it.
fn cut_by_plane(
    triangles: &[Triangle],
    point: DVec3,
    normal: DVec3,
) -> (Vec<Triangle>, Vec<Triangle>) {
    let (mut above, mut below) = (Vec::new(), Vec::new());
    for triangle in triangles {
        let heights = triangle.map(|p| (p - point).dot(normal));
        if heights.iter().all(|&h| h >= 0.0) {
            above.push(*triangle);
            continue;
        }
        if heights.iter().all(|&h| h <= 0.0) {
            below.push(*triangle);
            continue;
        }
        // Round its edges, sharing out its corners and adding one where an edge crosses.
        let (mut up, mut down) = (Vec::with_capacity(4), Vec::with_capacity(4));
        for k in 0..3 {
            let (p, q) = (triangle[k], triangle[(k + 1) % 3]);
            let (hp, hq) = (heights[k], heights[(k + 1) % 3]);
            if hp >= 0.0 {
                up.push(p);
            }
            if hp <= 0.0 {
                down.push(p);
            }
            if (hp > 0.0 && hq < 0.0) || (hp < 0.0 && hq > 0.0) {
                let crossing = p + (q - p) * (hp / (hp - hq));
                up.push(crossing);
                down.push(crossing);
            }
        }
        for (polygon, side) in [(&up, &mut above), (&down, &mut below)] {
            for k in 1..polygon.len().saturating_sub(1) {
                side.push([polygon[0], polygon[k], polygon[k + 1]]);
            }
        }
    }
    (above, below)
}

/// The volume of the hull of `triangles`, near enough to choose between cuts by.
fn rough_volume(triangles: &[Triangle]) -> Option<f64> {
    let corners = corners_of(triangles);
    let step = (corners.len() / CORNERS_TO_JUDGE).max(1);
    let few: Vec<DVec3> = corners.into_iter().step_by(step).collect();
    Hull::of(&few).map(|hull| hull.volume())
}

/// How far `triangles` reach along `normal`.
fn reach(triangles: &[Triangle], normal: DVec3) -> f64 {
    let along = triangles.iter().flatten().map(|p| p.dot(normal));
    let (low, high) = along.fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), h| {
        (low.min(h), high.max(h))
    });
    high - low
}

/// A convex piece of a surface, and how far short of good enough it is.
struct Piece {
    triangles: Vec<Triangle>,
    points: Vec<DVec3>,
    off: f64,
    worst: DVec3,
}

impl Piece {
    fn new(triangles: Vec<Triangle>, surface: &TriMesh) -> Option<Self> {
        let hull = Hull::of(&corners_of(&triangles))?;
        let (off, worst) = hull.standing_off(surface);
        Some(Self {
            triangles,
            points: hull.points,
            off,
            worst,
        })
    }

    /// The two sides of its best cut, if it has one.
    fn cut(&self, surface: &TriMesh) -> Option<(Self, Self)> {
        let spread = spread_axes(&self.points);
        let ways = [
            DVec3::X,
            DVec3::Y,
            DVec3::Z,
            spread[0],
            spread[1],
            spread[2],
        ];
        let mut best = None;
        let mut least = f64::INFINITY;
        for way in ways {
            let (above, below) = cut_by_plane(&self.triangles, self.worst, way);
            if above.is_empty()
                || below.is_empty()
                || reach(&above, way) < LEAST_SIDE
                || reach(&below, way) < LEAST_SIDE
            {
                continue;
            }
            let (Some(a), Some(b)) = (rough_volume(&above), rough_volume(&below)) else {
                continue;
            };
            if a + b < least {
                least = a + b;
                best = Some((above, below));
            }
        }
        let (above, below) = best?;
        Some((Self::new(above, surface)?, Self::new(below, surface)?))
    }
}

/// The corners of the convex hulls `triangles` are cut into, which follow their surface.
pub(super) fn convex_pieces(vertices: &[DVec3], triangles: &[[u32; 3]]) -> Vec<Vec<DVec3>> {
    if triangles.is_empty() {
        return Vec::new();
    }
    let Ok(surface) = TriMesh::new(vertices.to_vec(), triangles.to_vec()) else {
        return Vec::new();
    };
    let whole: Vec<Triangle> = (triangles.iter())
        .map(|t| t.map(|i| vertices[i as usize]))
        .collect();
    let size = {
        let corners = whole.iter().flatten();
        let (low, high) = corners.fold((DVec3::INFINITY, DVec3::NEG_INFINITY), |(low, high), p| {
            (low.min(*p), high.max(*p))
        });
        (high - low).length()
    };
    let enough = (STAND_OFF * size).clamp(LEAST_STAND_OFF, MOST_STAND_OFF);
    let Some(first) = Piece::new(whole, &surface) else {
        return Vec::new();
    };
    let mut pieces = vec![first];
    // The worst first, for as long as there is a worst and room for more.
    while pieces.len() < MOST_PIECES {
        let Some(k) = (0..pieces.len())
            .filter(|&k| pieces[k].off > enough)
            .max_by(|&a, &b| pieces[a].off.total_cmp(&pieces[b].off))
        else {
            break;
        };
        match pieces[k].cut(&surface) {
            Some((above, below)) => {
                pieces[k] = above;
                pieces.push(below);
            }
            // It cannot be cut any better than it is.
            None => pieces[k].off = 0.0,
        }
    }
    pieces.into_iter().map(|piece| piece.points).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tube(radius: f64, height: f64, sides: usize) -> (Vec<DVec3>, Vec<[u32; 3]>) {
        let (mut vertices, mut triangles) = (Vec::new(), Vec::new());
        for k in 0..sides {
            let angle = std::f64::consts::TAU * k as f64 / sides as f64;
            let at = DVec3::new(angle.cos() * radius, angle.sin() * radius, 0.0);
            vertices.push(at);
            vertices.push(at + DVec3::Z * height);
        }
        for k in 0..sides as u32 {
            let next = (k + 1) % sides as u32;
            let (a, b, c, d) = (2 * k, 2 * k + 1, 2 * next, 2 * next + 1);
            triangles.push([a, b, c]);
            triangles.push([c, b, d]);
        }
        (vertices, triangles)
    }

    fn inside(corners: &[DVec3], point: DVec3) -> bool {
        let hull = Hull::of(corners).unwrap();
        hull.faces.iter().all(|face| {
            let [a, b, c] = face.map(|i| hull.points[i as usize]);
            (point - a).dot((b - a).cross(c - a)) <= 0.0
        })
    }

    /// As wide as it is tall or wider, it stays hollow at every height: no disc across it.
    #[test]
    fn a_wide_short_tube_has_nothing_across_it() {
        let (vertices, triangles) = tube(5.05, 5.6, 72);
        let pieces = convex_pieces(&vertices, &triangles);
        assert!(pieces.len() < MOST_PIECES, "{} pieces", pieces.len());
        for piece in &pieces {
            for k in 0..=56 {
                let z = 0.1 * k as f64;
                for radius in [0.0, 2.5, 4.0] {
                    let at = DVec3::new(radius, 0.0, z);
                    assert!(!inside(piece, at), "{at} is inside a piece");
                }
            }
        }
    }

    /// A closed box is its own hull.
    #[test]
    fn what_is_convex_and_closed_is_one_piece() {
        let mut vertices = Vec::new();
        for k in 0..8 {
            vertices.push(DVec3::new(
                if k & 1 == 0 { -1.0 } else { 1.0 },
                if k & 2 == 0 { -2.0 } else { 2.0 },
                if k & 4 == 0 { -3.0 } else { 3.0 },
            ));
        }
        let triangles = [
            [0, 2, 1],
            [1, 2, 3],
            [4, 5, 6],
            [5, 7, 6],
            [0, 1, 4],
            [1, 5, 4],
            [2, 6, 3],
            [3, 6, 7],
            [0, 4, 2],
            [2, 4, 6],
            [1, 3, 5],
            [3, 7, 5],
        ];
        assert_eq!(convex_pieces(&vertices, &triangles).len(), 1);
    }

    /// A flat ring keeps its hole, and is not lost for having no depth.
    #[test]
    fn a_flat_ring_has_a_hole() {
        let sides = 48;
        let (mut vertices, mut triangles) = (Vec::new(), Vec::new());
        for k in 0..sides {
            let angle = std::f64::consts::TAU * k as f64 / sides as f64;
            let way = DVec3::new(angle.cos(), angle.sin(), 0.0);
            vertices.push(way * 2.0);
            vertices.push(way * 4.0);
        }
        for k in 0..sides as u32 {
            let next = (k + 1) % sides as u32;
            triangles.push([2 * k, 2 * k + 1, 2 * next]);
            triangles.push([2 * next, 2 * k + 1, 2 * next + 1]);
        }
        let pieces = convex_pieces(&vertices, &triangles);
        assert!(pieces.len() > 3, "{} pieces", pieces.len());
        assert!(pieces.iter().all(|piece| !inside(piece, DVec3::ZERO)));
        assert!(pieces.iter().any(|piece| inside(piece, DVec3::X * 3.0)));
    }

    /// Cutting loses nothing of a triangle and adds nothing to it.
    #[test]
    fn cutting_a_triangle_keeps_its_area() {
        let triangle = [DVec3::ZERO, DVec3::X * 4.0, DVec3::Y * 3.0];
        let area = |side: &[Triangle]| -> f64 {
            (side.iter())
                .map(|[a, b, c]| (*b - *a).cross(*c - *a).length() / 2.0)
                .sum()
        };
        for (point, normal) in [
            (DVec3::X, DVec3::X),
            (DVec3::Y, DVec3::Y),
            (
                DVec3::new(1.0, 1.0, 0.0),
                DVec3::new(1.0, 1.0, 0.0).normalize(),
            ),
            (DVec3::ZERO, DVec3::X),
        ] {
            let (above, below) = cut_by_plane(&[triangle], point, normal);
            assert!((area(&above) + area(&below) - 6.0).abs() < 1e-9);
        }
    }

    /// Every hull stays within what it was asked to of the surface.
    #[test]
    fn pieces_stay_close_to_the_surface() {
        let (vertices, triangles) = tube(2.0, 6.0, 64);
        let surface = TriMesh::new(vertices.clone(), triangles.clone()).unwrap();
        for piece in convex_pieces(&vertices, &triangles) {
            let (off, at) = Hull::of(&piece).unwrap().standing_off(&surface);
            assert!(off < 0.06, "{off} m off at {at}");
        }
    }
}
