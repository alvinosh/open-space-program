//! Revolved subparts: walls and regions in the (radius, height) half-plane, turned about
//! the part's axis all the way round or over a span of angles.
//!
//! A wall is thin and measured by area: its area element is r ds dφ. A region is closed
//! (its boundary ends where it starts, or starts and ends on the axis) and measured by
//! volume, r dr dz dφ. By Green's theorem a region's integrals become integrals around its
//! boundary, ∫∫ f dr dz = ∮ F dz with ∂F/∂r = f. Every F here vanishes on the axis, so the
//! stretch of axis that closes a boundary adds nothing, and neither does a liquid's flat
//! surface (dz = 0 along it): what lies below a height is the boundary below it, integrated
//! the same way. Everything is exact to the quadrature, whatever the curves.
//!
//! A region turned through less than a full turn also has two flat sides, at the ends of
//! its span; they are measured by area like walls, so a sector tank's walls are its curved
//! walls and its sides.

use std::{
    f64::consts::{PI, TAU},
    sync::OnceLock,
};

use glam::{DVec2, DVec3};

use super::{
    Fill, SurfacePoint,
    curve::Curve,
    mesh::{TriMesh, is_simple, signed_area, solve_level, triangulate},
    moments::{Moments, symmetric},
};

/// Largest angle between neighbouring columns of a region's mesh, rad. The mesh is for
/// liquid settled off the axis, scaled to the exact volume; renderers make their own.
const MESH_STEP: f64 = 5.0 * PI / 180.0;
/// How far a region's mesh outline may stray from its boundary, as a share of its size.
const MESH_TOLERANCE: f64 = 2.5e-4;

/// A thin wall along a curve.
#[derive(Debug, Clone)]
pub struct Wall {
    pub name: String,
    pub curve: Curve,
}

/// A closed region.
#[derive(Debug, Clone)]
pub struct Region {
    pub name: String,
    pub boundary: Curve,
    pub fill: Fill,
}

#[derive(Debug, Clone)]
pub struct Revolved {
    walls: Vec<Wall>,
    regions: Vec<Region>,
    turn: Turn,
    /// +1 for a region whose boundary runs counter-clockwise, −1 clockwise.
    signs: Vec<f64>,
    wall_moments: Vec<Moments>,
    region_moments: Vec<Moments>,
    side_moments: Vec<Moments>,
    meshes: Vec<OnceLock<TriMesh>>,
}

impl Revolved {
    /// Walls and regions turned from angle `from` to `to`, rad, from +X towards +Y.
    pub fn new(from: f64, to: f64, walls: Vec<Wall>, regions: Vec<Region>) -> Result<Self, String> {
        if !(from.is_finite() && to > from && to - from <= TAU + 1e-9) {
            return Err(
                "a revolved span runs from one angle to a larger one, at most a full turn".into(),
            );
        }
        if walls.is_empty() && regions.is_empty() {
            return Err("a revolved subpart needs a wall or a region".into());
        }
        let turn = Turn::new(from, to.min(from + TAU));
        let mut signs = Vec::new();
        for region in &regions {
            let name = &region.name;
            if !region.boundary.is_closed() {
                return Err(format!(
                    "region `{name}` must close: end where it starts, or start and end on the axis"
                ));
            }
            let polygon = polygon(&region.boundary, 1e-3);
            if !is_simple(&polygon) {
                return Err(format!("region `{name}` crosses itself"));
            }
            let area = signed_area(&polygon);
            if area.abs() <= 1e-9 * region.boundary.size().powi(2) {
                return Err(format!("region `{name}` encloses nothing"));
            }
            signs.push(area.signum());
        }
        let wall_moments = walls
            .iter()
            .map(|w| turn.revolve(w.curve.integrate(wall_integrand)))
            .collect();
        let region_moments = regions
            .iter()
            .zip(&signs)
            .map(|(r, s)| turn.revolve(r.boundary.integrate(region_integrand).map(|x| x * s)))
            .collect();
        let side_moments = regions
            .iter()
            .zip(&signs)
            .map(|(r, s)| {
                if turn.full {
                    return Moments::ZERO;
                }
                let p = r.boundary.integrate(side_integrand).map(|x| x * s);
                side(turn.from, p) + side(turn.to, p)
            })
            .collect();
        let meshes = regions.iter().map(|_| OnceLock::new()).collect();
        Ok(Self {
            walls,
            regions,
            turn,
            signs,
            wall_moments,
            region_moments,
            side_moments,
            meshes,
        })
    }

    /// The angles it is turned between, rad.
    pub fn span(&self) -> (f64, f64) {
        (self.turn.from, self.turn.to)
    }

    pub fn is_full(&self) -> bool {
        self.turn.full
    }

    pub fn walls(&self) -> &[Wall] {
        &self.walls
    }

    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// Wall `i`, by area.
    pub fn wall_moments(&self, i: usize) -> Moments {
        self.wall_moments[i]
    }

    /// Region `i`, by volume.
    pub fn region_moments(&self, i: usize) -> Moments {
        self.region_moments[i]
    }

    /// The flat sides of region `i` at the ends of a span short of a full turn, by area.
    pub fn side_moments(&self, i: usize) -> Moments {
        self.side_moments[i]
    }

    /// The lowest and highest heights of region `i`.
    pub fn heights(&self, i: usize) -> (f64, f64) {
        self.regions[i].boundary.heights()
    }

    /// The part of region `i` no higher than `height`, by volume.
    pub fn below(&self, i: usize, height: f64) -> Moments {
        let integrals = self.regions[i]
            .boundary
            .integrate_below(height, region_integrand);
        self.turn.revolve(integrals.map(|x| x * self.signs[i]))
    }

    /// The area of region `i`'s cross-section at `height`: how fast the volume below it grows
    /// with the height.
    pub fn area_at(&self, i: usize, height: f64) -> f64 {
        let squares: f64 = self.regions[i]
            .boundary
            .crossings(height)
            .iter()
            .map(|(r, rising)| rising * r * r)
            .sum();
        (self.turn.span / 2.0 * squares * self.signs[i]).max(0.0)
    }

    /// The height below which region `i` holds `volume`.
    pub fn level(&self, i: usize, volume: f64) -> f64 {
        let (lo, hi) = self.heights(i);
        let total = self.region_moments[i].measure;
        solve_level(lo, hi, total, volume, |h| {
            (self.below(i, h).measure, self.area_at(i, h))
        })
    }

    /// Region `i` as a closed triangle mesh: its outline within 1/4000 of its size, turned
    /// in steps of at most 5°, enclosing about its volume.
    pub fn region_mesh(&self, i: usize) -> &TriMesh {
        self.meshes[i].get_or_init(|| self.build_mesh(i))
    }

    fn build_mesh(&self, i: usize) -> TriMesh {
        let boundary = &self.regions[i].boundary;
        let size = boundary.size();
        let mut ring = polygon(boundary, MESH_TOLERANCE);
        if self.signs[i] < 0.0 {
            ring.reverse();
        }
        for p in &mut ring {
            if p.x < 1e-9 * size {
                p.x = 0.0;
            }
        }
        let full = self.turn.full;
        let steps = (self.turn.span / MESH_STEP)
            .ceil()
            .max(if full { 3.0 } else { 1.0 }) as usize;
        let columns = if full { steps } else { steps + 1 };
        // Rings pushed out so that each of their flat facets sweeps the area the curved
        // surface does: the mesh then holds the region's volume, however coarse.
        let step = self.turn.span / steps as f64;
        let swell = (step / step.sin()).sqrt();
        // One vertex per point and column, and one per point on the axis.
        let mut positions = Vec::new();
        let mut index = Vec::with_capacity(ring.len());
        for p in &ring {
            if p.x == 0.0 {
                index.push(vec![positions.len() as u32; columns]);
                positions.push(DVec3::new(0.0, 0.0, p.y));
            } else {
                let start = positions.len() as u32;
                index.push((0..columns as u32).map(|j| start + j).collect());
                for j in 0..columns {
                    let angle = self.turn.from + self.turn.span * j as f64 / steps as f64;
                    let (sin, cos) = angle.sin_cos();
                    positions.push(DVec3::new(p.x * swell * cos, p.x * swell * sin, p.y));
                }
            }
        }
        let column = |j: usize| if full { j % steps } else { j };
        let mut triangles = Vec::new();
        let n = ring.len();
        for k in 0..n {
            let next = (k + 1) % n;
            for j in 0..steps {
                let (a, b) = (index[k][column(j)], index[next][column(j)]);
                let (c, d) = (index[next][column(j + 1)], index[k][column(j + 1)]);
                for t in [[a, c, b], [a, d, c]] {
                    if t[0] != t[1] && t[1] != t[2] && t[2] != t[0] {
                        triangles.push(t);
                    }
                }
            }
        }
        if !full {
            // The flat sides: facing back at the start of the span, forward at its end.
            for [a, b, c] in triangulate(&ring) {
                triangles.push([index[a][0], index[b][0], index[c][0]]);
                triangles.push([index[a][steps], index[c][steps], index[b][steps]]);
            }
        }
        let mut mesh = TriMesh {
            positions,
            triangles,
        };
        mesh.orient_outward();
        mesh
    }

    /// The point of wall `i` `u` of the way around its span and `v` of the way along its
    /// curve by length.
    pub fn wall_point(&self, i: usize, u: f64, v: f64) -> SurfacePoint {
        let (p, along) = self.walls[i].curve.at_share(v);
        let angle = self.turn.from + self.turn.span * u.clamp(0.0, 1.0);
        let (sin, cos) = angle.sin_cos();
        let radial = DVec3::new(cos, sin, 0.0);
        SurfacePoint {
            position: radial * p.x + DVec3::Z * p.y,
            normal: (radial * along.y - DVec3::Z * along.x).normalize(),
            along_u: DVec3::new(-sin, cos, 0.0),
            along_v: radial * along.x + DVec3::Z * along.y,
        }
    }
}

/// A region's boundary as a polygon, closed from its last point back to its first (along the
/// axis, for one whose ends lie on it).
fn polygon(boundary: &Curve, tolerance: f64) -> Vec<DVec2> {
    let mut points: Vec<DVec2> = boundary
        .tessellate(tolerance * boundary.size())
        .iter()
        .map(|p| p.point)
        .collect();
    if points.len() > 1 && (points[points.len() - 1] - points[0]).length() <= 1e-9 * boundary.size()
    {
        points.pop();
    }
    points
}

/// Integrals turned through a span of angles.
#[derive(Debug, Clone, Copy)]
struct Turn {
    from: f64,
    to: f64,
    span: f64,
    full: bool,
    /// ∫ cos φ, ∫ sin φ, ∫ cos² φ, ∫ sin² φ and ∫ sin φ cos φ over the span.
    cos: f64,
    sin: f64,
    cos2: f64,
    sin2: f64,
    sincos: f64,
}

impl Turn {
    fn new(from: f64, to: f64) -> Self {
        let span = to - from;
        if (span - TAU).abs() < 1e-12 {
            return Self {
                from,
                to,
                span: TAU,
                full: true,
                cos: 0.0,
                sin: 0.0,
                cos2: PI,
                sin2: PI,
                sincos: 0.0,
            };
        }
        let ((s0, c0), (s1, c1)) = (from.sin_cos(), to.sin_cos());
        let double = ((2.0 * to).sin() - (2.0 * from).sin()) / 4.0;
        Self {
            from,
            to,
            span,
            full: false,
            cos: s1 - s0,
            sin: c0 - c1,
            cos2: span / 2.0 + double,
            sin2: span / 2.0 - double,
            sincos: (s1 * s1 - s0 * s0) / 2.0,
        }
    }

    /// The moments of something whose integrals over the half-plane, with the r of its area
    /// or volume element, are `[∫ r, ∫ r², ∫ r³, ∫ z r, ∫ z r², ∫ z² r]`.
    fn revolve(&self, [a1, a2, a3, b1, b2, b3]: [f64; 6]) -> Moments {
        Moments {
            measure: a1 * self.span,
            first: DVec3::new(a2 * self.cos, a2 * self.sin, b1 * self.span),
            second: symmetric(
                a3 * self.cos2,
                a3 * self.sin2,
                b3 * self.span,
                a3 * self.sincos,
                b2 * self.cos,
                b2 * self.sin,
            ),
        }
    }
}

/// A wall's integrals along its curve, ds = |rate of change|.
fn wall_integrand(p: DVec2, d: DVec2) -> [f64; 6] {
    let (r, z, ds) = (p.x, p.y, d.length());
    [
        r * ds,
        r * r * ds,
        r * r * r * ds,
        z * r * ds,
        z * r * r * ds,
        z * z * r * ds,
    ]
}

/// A region's integrals ∫∫ r, r², r³, z r, z r², z² r dr dz, as ∮ F dz.
fn region_integrand(p: DVec2, d: DVec2) -> [f64; 6] {
    let (r, z, dz) = (p.x, p.y, d.y);
    let r2 = r * r;
    [
        r2 / 2.0 * dz,
        r2 * r / 3.0 * dz,
        r2 * r2 / 4.0 * dz,
        r2 * z / 2.0 * dz,
        r2 * r * z / 3.0 * dz,
        r2 * z * z / 2.0 * dz,
    ]
}

/// A region's flat integrals ∫∫ 1, r, z, r², r z, z² dr dz, as ∮ F dz.
fn side_integrand(p: DVec2, d: DVec2) -> [f64; 6] {
    let (r, z, dz) = (p.x, p.y, d.y);
    [
        r * dz,
        r * r / 2.0 * dz,
        r * z * dz,
        r * r * r / 3.0 * dz,
        r * r * z / 2.0 * dz,
        r * z * z * dz,
    ]
}

/// The flat side of a region at `angle`, from its flat integrals.
fn side(angle: f64, [p0, p1, p2, p3, p4, p5]: [f64; 6]) -> Moments {
    let (sin, cos) = angle.sin_cos();
    Moments {
        measure: p0,
        first: DVec3::new(p1 * cos, p1 * sin, p2),
        second: symmetric(
            p3 * cos * cos,
            p3 * sin * sin,
            p5,
            p3 * sin * cos,
            p4 * cos,
            p4 * sin,
        ),
    }
}
