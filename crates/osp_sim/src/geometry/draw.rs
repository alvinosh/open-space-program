//! Meshes to draw parts with: each wall, solid, skin and lattice of a part's geometry
//! tessellated with normals, surface coordinates and tangents, in the part's frame.
//!
//! Surface coordinates are in metres, so detail drawn in them keeps its size on any part:
//! around a wall of revolution `u` is the arc length around it at that radius and `v` the
//! length along its curve; on a loft `u` runs along its axis (a wing's span) and `v` around
//! its section. Tangents point along `u`.
//!
//! A wall has two faces, the front (out of a tank or shell) and the back, as far apart as
//! the wall is thick, joined round its open edges: an open skirt shows its inside, a
//! fairing half its cut. A solid is drawn by its surface, a cell not at all (its walls
//! are). Models are drawn from their own files, and their proxies and what is unseen
//! inside a part not at all.

use std::f64::consts::PI;

use glam::{DMat2, DQuat, DVec2, DVec3};

use super::{
    Geometry, Kind,
    curve::Curve,
    lattice::Lattice,
    lofted::{Lofted, Station},
    mesh::{signed_area, triangulate},
    revolved::Revolved,
};

/// Most a tessellated curve strays from the curve, as a share of its size and at most in
/// metres.
const CURVE_SHARE: f64 = 2e-3;
const CURVE_TOLERANCE: f64 = 2e-3;
/// Most the drawing turns between neighbouring columns around a wall, rad.
const ANGLE_STEP: f64 = 5.0 * PI / 180.0;
/// Most a loft's section twists between neighbouring rings, rad.
const TWIST_STEP: f64 = 2.0 * PI / 180.0;
/// Turns sharper than this, along a curve or round a section, are drawn as creases.
const CREASE: f64 = 35.0 * PI / 180.0;
/// Sides round a lattice member.
const MEMBER_SIDES: usize = 8;

/// A triangle mesh, its vertices each with a normal, surface coordinates (m) and a
/// tangent along `u` (`w` the handedness of tangent, bitangent along `v` and normal).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DrawMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub tangents: Vec<[f32; 4]>,
    /// Counter-clockwise seen from the side the normals face.
    pub indices: Vec<u32>,
}

/// One drawn piece of a part: the element it draws (a wall, solid or lattice), its front,
/// and for a wall its back with the edges between them.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawPiece {
    pub element: String,
    pub mesh: DrawMesh,
    pub back: DrawMesh,
}

impl DrawMesh {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// A vertex at `p`, facing `n`, at surface coordinates `uv`, its tangent `t` (along
    /// `u`) and bitangent `b` (along `v`) deciding the handedness.
    fn vertex(&mut self, p: DVec3, n: DVec3, uv: DVec2, t: DVec3, b: DVec3) -> u32 {
        let n = n.normalize_or(DVec3::Z);
        let t = (t - n * n.dot(t))
            .try_normalize()
            .unwrap_or_else(|| n.any_orthonormal_vector());
        let w = if t.cross(b).dot(n) < 0.0 { -1.0 } else { 1.0 };
        self.positions.push(p.as_vec3().to_array());
        self.normals.push(n.as_vec3().to_array());
        self.uvs.push(uv.as_vec2().to_array());
        self.tangents.push([t.x as f32, t.y as f32, t.z as f32, w]);
        (self.positions.len() - 1) as u32
    }

    /// A triangle, unless it has no area.
    fn triangle(&mut self, a: u32, b: u32, c: u32) {
        let p = |i: u32| DVec3::from(self.positions[i as usize].map(f64::from));
        if (p(b) - p(a)).cross(p(c) - p(a)).length_squared() > 1e-18 {
            self.indices.extend([a, b, c]);
        }
    }

    /// The other mesh's triangles added to this one's.
    fn append(&mut self, other: DrawMesh) {
        let base = self.positions.len() as u32;
        self.positions.extend(other.positions);
        self.normals.extend(other.normals);
        self.uvs.extend(other.uvs);
        self.tangents.extend(other.tangents);
        self.indices
            .extend(other.indices.into_iter().map(|i| i + base));
    }

    /// Moved from a subpart's frame into its part's.
    fn placed(mut self, position: DVec3, rotation: DQuat) -> Self {
        let turn = rotation.as_quat();
        let shift = position.as_vec3();
        for p in &mut self.positions {
            *p = (turn * glam::Vec3::from(*p) + shift).to_array();
        }
        for n in &mut self.normals {
            *n = (turn * glam::Vec3::from(*n)).to_array();
        }
        for t in &mut self.tangents {
            let turned = turn * glam::Vec3::new(t[0], t[1], t[2]);
            *t = [turned.x, turned.y, turned.z, t[3]];
        }
        self
    }

    /// The area of its triangles, m².
    pub fn area(&self) -> f64 {
        let p = |i: u32| DVec3::from(self.positions[i as usize].map(f64::from));
        self.indices
            .chunks(3)
            .map(|t| (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0])).length() / 2.0)
            .sum()
    }
}

impl Geometry {
    /// Meshes for each piece to draw: walls `thickness(element)` thick, lattice members
    /// `member` metres across.
    pub fn draw(&self, thickness: impl Fn(&str) -> f64, member: f64) -> Vec<DrawPiece> {
        self.pieces(thickness, member, false)
    }

    /// The surfaces the air meets, as [`Geometry::draw`] draws them, with the proxies
    /// standing in for models.
    pub fn outer(&self, thickness: impl Fn(&str) -> f64, member: f64) -> Vec<DrawPiece> {
        self.pieces(thickness, member, true)
    }

    fn pieces(
        &self,
        thickness: impl Fn(&str) -> f64,
        member: f64,
        proxies: bool,
    ) -> Vec<DrawPiece> {
        let mut out = Vec::new();
        for sub in (self.subparts.iter()).filter(|s| (proxies || !s.proxy) && !s.unseen) {
            let mut push = |element: String, (mesh, back): (DrawMesh, DrawMesh)| {
                if !(mesh.is_empty() && back.is_empty()) {
                    out.push(DrawPiece {
                        element,
                        mesh: mesh.placed(sub.position, sub.rotation),
                        back: back.placed(sub.position, sub.rotation),
                    });
                }
            };
            let front = |mesh| (mesh, DrawMesh::default());
            match &sub.kind {
                Kind::Revolved(r) => {
                    let (from, to) = r.span();
                    for wall in r.walls() {
                        let element = sub.named(&wall.name);
                        let thick = thickness(&element).max(0.0);
                        push(element, wall_mesh(&wall.curve, from, to, thick));
                    }
                    for region in r.regions() {
                        if region.fill == super::Fill::Solid {
                            push(
                                sub.named(&region.name),
                                front(solid_mesh(&region.boundary, r)),
                            );
                        }
                    }
                }
                Kind::Lofted(l) => {
                    push(sub.named(&l.name), front(loft_mesh(l, [0.0, 1.0], None)));
                }
                Kind::Lattice(l) => {
                    push(sub.named(&l.name), front(lattice_mesh(l, member / 2.0)));
                }
                Kind::Model(_) => {}
            }
        }
        out
    }

    /// The loft named `name` between shares `span` of its length and shares `chord` of its
    /// section's extent along x, in the part's frame: the fixed part of a wing and its
    /// control surface. `None` without such a loft.
    pub fn draw_loft(&self, name: &str, span: [f64; 2], chord: [f64; 2]) -> Option<DrawMesh> {
        let (sub, loft) = self.subparts.iter().find_map(|s| match &s.kind {
            Kind::Lofted(l) if s.named(&l.name) == name => Some((s, l)),
            _ => None,
        })?;
        Some(loft_mesh(loft, span, Some(chord)).placed(sub.position, sub.rotation))
    }
}

impl Geometry {
    /// The point of the loft named `name` `span` of the way along its length and `chord` of
    /// the way along its section's extent in x, on its section's x axis, in the part's
    /// frame: a point of a wing's hinge line.
    pub fn loft_point(&self, name: &str, span: f64, chord: f64) -> Option<DVec3> {
        let (sub, loft) = self.subparts.iter().find_map(|s| match &s.kind {
            Kind::Lofted(l) if s.named(&l.name) == name => Some((s, l)),
            _ => None,
        })?;
        let stations = loft.stations();
        let (z0, z1) = (stations[0].z, stations[stations.len() - 1].z);
        let (lo, hi) = loft
            .section()
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(p.x), hi.max(p.x))
            });
        let station = loft.at(z0 + (z1 - z0) * span);
        let local = place(&station, DVec2::new(lo + (hi - lo) * chord, 0.0));
        Some(sub.position + sub.rotation * local)
    }
}

/// A point of a tessellated curve: where it is, which way the curve runs there (unit), and
/// how far along it, m.
#[derive(Debug, Clone, Copy)]
struct Along {
    point: DVec2,
    direction: DVec2,
    length: f64,
}

/// `curve` as points along it, a point twice where it turns a corner (once for each way
/// it runs).
fn along(curve: &Curve) -> Vec<Along> {
    let tolerance = (CURVE_SHARE * curve.size()).min(CURVE_TOLERANCE);
    let points = curve.tessellate(tolerance);
    let segments = curve.segments();
    let direction = |segment: usize, t: f64| {
        let d = segments[segment].derivative(t);
        d.try_normalize().unwrap_or(DVec2::Y)
    };
    let mut out: Vec<Along> = Vec::with_capacity(points.len() + 4);
    let mut length = 0.0;
    for (k, p) in points.iter().enumerate() {
        if k > 0 {
            length += (p.point - points[k - 1].point).length();
        }
        let here = direction(p.segment, p.t);
        // Where one segment hands over to the next, the next's direction too.
        let next = points.get(k + 1).filter(|q| q.segment != p.segment);
        let after = next.map(|q| direction(q.segment, 0.0));
        out.push(Along {
            point: p.point,
            direction: here,
            length,
        });
        if let Some(after) = after {
            if here.angle_to(after).abs() > CREASE {
                out.push(Along {
                    point: p.point,
                    direction: after,
                    length,
                });
            } else {
                let mean = (here + after).normalize_or(here);
                out.last_mut().expect("just pushed").direction = mean;
            }
        }
    }
    out
}

/// Columns round a span from `from` to `to`, rad: their angles.
fn columns(from: f64, to: f64) -> Vec<f64> {
    let span = to - from;
    let full = (span - 2.0 * PI).abs() < 1e-9;
    let steps = (span / ANGLE_STEP).ceil().max(if full { 3.0 } else { 1.0 }) as usize;
    (0..=steps)
        .map(|j| from + span * j as f64 / steps as f64)
        .collect()
}

/// In 3D at angle `a`: a point `(r, z)` of the half-plane, and a direction in it.
fn at(p: DVec2, a: f64) -> DVec3 {
    let (sin, cos) = a.sin_cos();
    DVec3::new(p.x * cos, p.x * sin, p.y)
}

fn around(a: f64) -> DVec3 {
    let (sin, cos) = a.sin_cos();
    DVec3::new(-sin, cos, 0.0)
}

/// Grid of `rows` × `columns` vertices made into quads, from row `r` to `r + 1` and
/// column `c` to `c + 1`, facing the way `(c → c + 1) × (r → r + 1)` does; `flip` the
/// other way.
fn quads(mesh: &mut DrawMesh, grid: &[Vec<u32>], flip: bool) {
    for r in 0..grid.len().saturating_sub(1) {
        for c in 0..grid[r].len() - 1 {
            let (a, b) = (grid[r][c], grid[r][c + 1]);
            let (d, e) = (grid[r + 1][c], grid[r + 1][c + 1]);
            if flip {
                mesh.triangle(a, e, b);
                mesh.triangle(a, d, e);
            } else {
                mesh.triangle(a, b, e);
                mesh.triangle(a, e, d);
            }
        }
    }
}

/// A wall along `curve` revolved from `from` to `to`: its front out of the right of the
/// way the curve runs, and its back `thick` behind with the edges between them where it is
/// open.
fn wall_mesh(curve: &Curve, from: f64, to: f64, thick: f64) -> (DrawMesh, DrawMesh) {
    let points = along(curve);
    let angles = columns(from, to);
    let (mut front_mesh, mut mesh) = (DrawMesh::default(), DrawMesh::default());
    let near_axis = 1e-9 * curve.size();
    // The front on the curve, the back behind it.
    let face = |p: &Along, depth: f64| {
        let front = DVec2::new(p.direction.y, -p.direction.x);
        let point = p.point - front * depth;
        (DVec2::new(point.x.max(0.0), point.y), front)
    };
    for (depth, side) in [(0.0, 1.0), (thick, -1.0)] {
        let target = if side > 0.0 {
            &mut front_mesh
        } else {
            &mut mesh
        };
        let grid: Vec<Vec<u32>> = points
            .iter()
            .map(|p| {
                let (point, front) = face(p, depth);
                angles
                    .iter()
                    .map(|&a| {
                        let uv = DVec2::new(point.x * (a - from), p.length);
                        let (n, b) = (at(front * side, a), at(p.direction, a));
                        target.vertex(at(point, a), n, uv, around(a), b)
                    })
                    .collect()
            })
            .collect();
        quads(target, &grid, side < 0.0);
    }
    if thick <= 0.0 {
        return (front_mesh, mesh);
    }
    // Round its open ends, between its faces.
    for (p, out) in [(points[0], -1.0), (points[points.len() - 1], 1.0)] {
        if p.point.x <= near_axis {
            continue;
        }
        let rows: Vec<Vec<u32>> = [0.0, thick]
            .iter()
            .map(|&depth| {
                let (point, front) = face(&p, depth);
                angles
                    .iter()
                    .map(|&a| {
                        let uv = DVec2::new(point.x * (a - from), p.length + depth);
                        let n = at(p.direction * out, a);
                        mesh.vertex(at(point, a), n, uv, around(a), at(-front, a))
                    })
                    .collect()
            })
            .collect();
        quads(&mut mesh, &rows, out < 0.0);
    }
    // And along its cut edges, over part of a turn.
    if (to - from - 2.0 * PI).abs() > 1e-9 {
        for (a, out) in [(from, -1.0), (to, 1.0)] {
            let rows: Vec<Vec<u32>> = points
                .iter()
                .map(|p| {
                    [0.0, thick]
                        .iter()
                        .map(|&depth| {
                            let (point, front) = face(p, depth);
                            let uv = DVec2::new(depth, p.length);
                            let n = around(a) * out;
                            mesh.vertex(at(point, a), n, uv, at(-front, a), at(p.direction, a))
                        })
                        .collect()
                })
                .collect();
            quads(&mut mesh, &rows, out < 0.0);
        }
    }
    (front_mesh, mesh)
}

/// The surface of a solid region of `r`: its boundary revolved, and its flat sides over
/// part of a turn.
fn solid_mesh(boundary: &Curve, r: &Revolved) -> DrawMesh {
    let (from, to) = r.span();
    let mut points = along(boundary);
    let outline: Vec<DVec2> = points.iter().map(|p| p.point).collect();
    // Faces out of the right of the way it runs, so run it counter-clockwise.
    if signed_area(&outline) < 0.0 {
        points.reverse();
        let total = points[0].length;
        for p in &mut points {
            p.direction = -p.direction;
            p.length = total - p.length;
        }
    }
    let angles = columns(from, to);
    let mut mesh = DrawMesh::default();
    let grid: Vec<Vec<u32>> = points
        .iter()
        .map(|p| {
            let normal = DVec2::new(p.direction.y, -p.direction.x);
            angles
                .iter()
                .map(|&a| {
                    let uv = DVec2::new(p.point.x * (a - from), p.length);
                    mesh.vertex(
                        at(p.point, a),
                        at(normal, a),
                        uv,
                        around(a),
                        at(p.direction, a),
                    )
                })
                .collect()
        })
        .collect();
    quads(&mut mesh, &grid, false);
    if !r.is_full() {
        let mut outline: Vec<DVec2> = points.iter().map(|p| p.point).collect();
        outline.dedup_by(|a, b| (*a - *b).length() < 1e-12);
        if outline.len() > 2 && (outline[0] - outline[outline.len() - 1]).length() < 1e-12 {
            outline.pop();
        }
        for (a, out) in [(from, -1.0), (to, 1.0)] {
            let n = around(a) * out;
            let base = mesh.positions.len() as u32;
            for p in &outline {
                mesh.vertex(at(*p, a), n, *p, at(DVec2::X, a), DVec3::Z);
            }
            // Counter-clockwise in (r, z) faces back, against the turn.
            for [i, j, k] in triangulate(&outline) {
                let [i, j, k] = [i, j, k].map(|v| base + v as u32);
                if out < 0.0 {
                    mesh.triangle(i, j, k);
                } else {
                    mesh.triangle(i, k, j);
                }
            }
        }
    }
    mesh
}

/// Where `p` of the section is at station `s`.
fn place(s: &Station, p: DVec2) -> DVec3 {
    (DMat2::from_angle(s.twist) * (s.scale * p) + s.offset).extend(s.z)
}

/// `polygon` (counter-clockwise) cut to `lo <= x <= hi`.
fn clip(polygon: &[DVec2], lo: f64, hi: f64) -> Vec<DVec2> {
    let mut out = polygon.to_vec();
    for (keep, edge) in [(1.0, lo), (-1.0, -hi)] {
        let inside = |p: DVec2| keep * p.x >= edge;
        let input = std::mem::take(&mut out);
        for (i, &p) in input.iter().enumerate() {
            let q = input[(i + 1) % input.len()];
            if inside(p) {
                out.push(p);
            }
            if inside(p) != inside(q) {
                let x = edge * keep;
                let t = (x - p.x) / (q.x - p.x);
                out.push(p.lerp(q, t));
            }
        }
        if out.is_empty() {
            return out;
        }
    }
    out
}

/// A loft between shares `span` of its length, its section cut to shares `chord` of its
/// extent along x if given: the skin through rings at each station (and between them
/// where it twists), sharp at the section's corners, and flat ends.
fn loft_mesh(loft: &Lofted, span: [f64; 2], chord: Option<[f64; 2]>) -> DrawMesh {
    let stations = loft.stations();
    let (z0, z1) = (stations[0].z, stations[stations.len() - 1].z);
    let (from, to) = (z0 + (z1 - z0) * span[0], z0 + (z1 - z0) * span[1]);
    let mut section = loft.section().to_vec();
    if let Some([a, b]) = chord {
        let (lo, hi) = section
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(p.x), hi.max(p.x))
            });
        let (x0, x1) = (lo + (hi - lo) * a, lo + (hi - lo) * b);
        section = clip(&section, x0, x1);
    }
    let mut mesh = DrawMesh::default();
    if section.len() < 3 || to <= from {
        return mesh;
    }
    // Rings: the ends, the stations between, and more where it twists.
    let mut heights = vec![from];
    heights.extend(stations.iter().map(|s| s.z).filter(|&z| z > from && z < to));
    heights.push(to);
    let mut rings = Vec::new();
    for w in heights.windows(2) {
        let (a, b) = (loft.at(w[0]), loft.at(w[1]));
        let steps = ((b.twist - a.twist).abs() / TWIST_STEP).ceil().max(1.0) as usize;
        if rings.is_empty() {
            rings.push(a);
        }
        rings.extend((1..=steps).map(|k| loft.at(w[0] + (w[1] - w[0]) * k as f64 / steps as f64)));
    }
    // Columns round the section: one at each smooth point, two at each corner (into it
    // and out of it), and one more at the first point to close the section.
    let n = section.len();
    let edge = |i: usize| (section[(i + 1) % n] - section[i]).normalize_or(DVec2::X);
    let corner: Vec<bool> = (0..n)
        .map(|i| edge((i + n - 1) % n).angle_to(edge(i)).abs() > CREASE)
        .collect();
    let (mut into, mut out_of, mut point_of) = (vec![0; n], vec![0; n], Vec::new());
    for i in 0..n {
        into[i] = point_of.len();
        point_of.push(i);
        out_of[i] = if corner[i] {
            point_of.push(i);
            point_of.len() - 1
        } else {
            into[i]
        };
    }
    let close = point_of.len();
    point_of.push(0);
    let columns = point_of.len();
    // Edge i runs from column `out_of[i]` to `ends[i]`.
    let ends: Vec<usize> = (0..n)
        .map(|i| if i + 1 == n { close } else { into[i + 1] })
        .collect();
    let ring_points: Vec<Vec<DVec3>> = rings
        .iter()
        .map(|s| point_of.iter().map(|&i| place(s, section[i])).collect())
        .collect();
    let round: Vec<Vec<f64>> = ring_points
        .iter()
        .map(|pts| {
            let mut v = vec![0.0; columns];
            for c in 1..columns {
                v[c] = v[c - 1] + (pts[c] - pts[c - 1]).length();
            }
            v
        })
        .collect();
    // Normals summed from the faces each vertex touches; the closing column and the first
    // point's are one where the section is smooth there.
    let mut normals = vec![vec![DVec3::ZERO; columns]; rings.len()];
    for r in 0..rings.len() - 1 {
        for i in 0..n {
            let (c0, c1) = (out_of[i], ends[i]);
            let (a, b) = (ring_points[r][c0], ring_points[r][c1]);
            let (d, e) = (ring_points[r + 1][c0], ring_points[r + 1][c1]);
            let face = (b - a).cross(e - a) + (e - a).cross(d - a);
            for (rr, cc) in [(r, c0), (r, c1), (r + 1, c0), (r + 1, c1)] {
                normals[rr][cc] += face;
            }
        }
    }
    if !corner[0] {
        for ring in &mut normals {
            let both = ring[into[0]] + ring[close];
            (ring[into[0]], ring[close]) = (both, both);
        }
    }
    let grid: Vec<Vec<u32>> = (0..rings.len())
        .map(|r| {
            (0..columns)
                .map(|c| {
                    let i = point_of[c];
                    let up = ring_points[(r + 1).min(rings.len() - 1)][c]
                        - ring_points[r.saturating_sub(1)][c];
                    let ahead = place(&rings[r], section[(i + 1) % n])
                        - place(&rings[r], section[(i + n - 1) % n]);
                    let uv = DVec2::new(rings[r].z, round[r][c]);
                    mesh.vertex(ring_points[r][c], normals[r][c], uv, up, ahead)
                })
                .collect()
        })
        .collect();
    for r in 0..rings.len() - 1 {
        for i in 0..n {
            let (c0, c1) = (out_of[i], ends[i]);
            let (a, b) = (grid[r][c0], grid[r][c1]);
            let (d, e) = (grid[r + 1][c0], grid[r + 1][c1]);
            mesh.triangle(a, b, e);
            mesh.triangle(a, e, d);
        }
    }
    // The flat ends, where they have some size.
    for (ring, out) in [(&rings[0], -1.0), (&rings[rings.len() - 1], 1.0)] {
        if ring.scale.min_element() <= 0.0 {
            continue;
        }
        let normal = DVec3::Z * out;
        let base = mesh.positions.len() as u32;
        for p in &section {
            let q = place(ring, *p);
            mesh.vertex(q, normal, q.truncate(), DVec3::X, DVec3::Y * out);
        }
        for [i, j, k] in triangulate(&section) {
            let [i, j, k] = [i, j, k].map(|v| base + v as u32);
            if out > 0.0 {
                mesh.triangle(i, j, k);
            } else {
                mesh.triangle(i, k, j);
            }
        }
    }
    mesh
}

/// Each member of `l` as a tube `radius` round.
fn lattice_mesh(l: &Lattice, radius: f64) -> DrawMesh {
    let mut mesh = DrawMesh::default();
    if radius <= 0.0 {
        return mesh;
    }
    for &[a, b] in l.members() {
        let (p, q) = (l.nodes()[a], l.nodes()[b]);
        let axis = (q - p).normalize();
        let side = axis.any_orthonormal_vector();
        let other = axis.cross(side);
        let length = (q - p).length();
        let mut tube = DrawMesh::default();
        let grid: Vec<Vec<u32>> = [0.0, 1.0]
            .iter()
            .map(|&t| {
                (0..=MEMBER_SIDES)
                    .map(|k| {
                        let a = 2.0 * PI * k as f64 / MEMBER_SIDES as f64;
                        let out = side * a.cos() + other * a.sin();
                        let point = p + (q - p) * t + out * radius;
                        let tangent = other * a.cos() - side * a.sin();
                        let uv = DVec2::new(radius * a, length * t);
                        tube.vertex(point, out, uv, tangent, axis)
                    })
                    .collect()
            })
            .collect();
        quads(&mut tube, &grid, false);
        mesh.append(tube);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{geometry::ShapeDef, test_data::records};

    fn shape(text: &str) -> Geometry {
        let def: ShapeDef = records(text).remove(0);
        def.build().unwrap_or_else(|e| panic!("{e}"))
    }

    fn pieces(g: &Geometry, thick: f64) -> Vec<DrawPiece> {
        g.draw(|_| thick, 0.05)
    }

    /// Normals and tangents unit and square; every triangle wound to face its normals.
    fn assert_well_formed(mesh: &DrawMesh) {
        let v = |a: [f32; 3]| DVec3::from(a.map(f64::from));
        for (n, t) in mesh.normals.iter().zip(&mesh.tangents) {
            let (n, t) = (v(*n), DVec3::new(t[0].into(), t[1].into(), t[2].into()));
            assert!((n.length() - 1.0).abs() < 1e-5 && (t.length() - 1.0).abs() < 1e-5);
            assert!(n.dot(t).abs() < 1e-5, "{n} {t}");
        }
        for tri in mesh.indices.chunks(3) {
            let p = |k: usize| v(mesh.positions[tri[k] as usize]);
            let face = (p(1) - p(0)).cross(p(2) - p(0));
            let normal: DVec3 = (0..3).map(|k| v(mesh.normals[tri[k] as usize])).sum();
            assert!(
                face.dot(normal) > 0.0,
                "a triangle faces away from its normals"
            );
        }
    }

    #[test]
    fn a_tank_draws_its_walls_both_sides() {
        let g = shape("{ cylinder_tank: { diameter: 3, length: 9, aft_skirt: 2 } }");
        let thick = 0.004;
        let drawn = pieces(&g, thick);
        let mut names: Vec<_> = drawn.iter().map(|p| p.element.as_str()).collect();
        names.sort();
        assert_eq!(names, ["aft_dome", "aft_skirt", "barrel", "forward_dome"]);
        for p in &drawn {
            assert_well_formed(&p.mesh);
            assert_well_formed(&p.back);
            // Both faces of the wall: twice its area, and the rims round open ends.
            let area = g.element(&p.element).unwrap().moments.measure;
            let rims = 2.0 * 2.0 * PI * 1.5 * thick;
            let both = p.mesh.area() + p.back.area();
            assert!(
                (both - 2.0 * area).abs() < 0.01 * area + rims,
                "{}",
                p.element
            );
        }
        // The barrel's front faces out, its coordinates in metres round and along it.
        let whole = drawn.iter().find(|p| p.element == "barrel").unwrap();
        let mut barrel = whole.mesh.clone();
        barrel.append(whole.back.clone());
        let barrel = &barrel;
        let facing = |out: bool| {
            barrel
                .positions
                .iter()
                .zip(&barrel.normals)
                .filter(|(p, n)| {
                    let (p, n) = (DVec3::from(p.map(f64::from)), DVec3::from(n.map(f64::from)));
                    let radial = p.truncate().normalize().dot(n.truncate());
                    if out { radial > 0.99 } else { radial < -0.99 }
                })
                .count()
        };
        assert!(facing(true) > 0 && facing(true) == facing(false));
        let (u, v) = barrel
            .uvs
            .iter()
            .fold((0.0f32, 0.0f32), |(u, v), t| (u.max(t[0]), v.max(t[1])));
        assert!((u - 2.0 * PI as f32 * 1.5).abs() < 0.02, "{u}");
        let barrel_length = 9.0 - 2.0 * 1.5 * std::f32::consts::FRAC_1_SQRT_2;
        // The rim round its top end runs on through the wall's thickness.
        assert!((v - barrel_length - thick as f32).abs() < 1e-4, "{v}");
    }

    #[test]
    fn a_fairing_half_shows_its_cut() {
        let g = shape(
            "{ shell: { bottom_diameter: 3, sections: [{ length: 6 }, { length: 3, \
             top_diameter: 0, profile: 'ogive' }], span: [-90, 90] } }",
        );
        let thick = 0.05;
        let drawn = &pieces(&g, thick)[0];
        assert_well_formed(&drawn.mesh);
        assert_well_formed(&drawn.back);
        let mut wall = drawn.mesh.clone();
        wall.append(drawn.back.clone());
        // Outside, inside, and the two cut edges along its whole length.
        let area = g.element("wall").unwrap().moments.measure;
        let Kind::Revolved(r) = &g.subparts[0].kind else {
            panic!("a shell is revolved");
        };
        let length = r.walls()[0].curve.length();
        assert!(wall.area() > 2.0 * area * 0.98 + 2.0 * length * thick * 0.9);
    }

    #[test]
    fn solids_and_lofts_are_closed_and_face_out() {
        for text in [
            "{ ring: { diameter: 2, height: 0.3, thickness: 0.2 } }",
            "{ wing: { root_chord: 2, tip_chord: 0.8, span: 4, sweep: 1, twist: -4 } }",
            "{ box: { size: [1, 2, 0.5] } }",
            "{ revolved: { span: [0, 120], regions: [{ name: 'sector', boundary: { start: [0, 0], \
               segments: [{ line: [1, 0] }, { line: [1, 1] }, { line: [0, 1] }] } }] } }",
        ] {
            let g = shape(text);
            let drawn = pieces(&g, 0.0);
            let mesh = &drawn[0].mesh;
            assert_well_formed(mesh);
            // The divergence theorem gives back its volume: closed, and facing out.
            let p = |i: u32| DVec3::from(mesh.positions[i as usize].map(f64::from));
            let volume: f64 = mesh
                .indices
                .chunks(3)
                .map(|t| p(t[0]).dot(p(t[1]).cross(p(t[2]))) / 6.0)
                .sum();
            let exact = g
                .elements()
                .iter()
                .filter(|e| e.fill.is_some())
                .map(|e| e.moments.measure)
                .sum::<f64>();
            assert!(
                (volume - exact).abs() < 0.01 * exact,
                "{text}: {volume} vs {exact}"
            );
        }
    }

    #[test]
    fn a_wing_parts_at_its_hinge() {
        let g = shape("{ wing: { root_chord: 2, tip_chord: 1, span: 4 } }");
        let fixed = g.draw_loft("wing", [0.2, 0.8], [0.0, 0.75]).unwrap();
        let flap = g.draw_loft("wing", [0.2, 0.8], [0.75, 1.0]).unwrap();
        assert_well_formed(&fixed);
        assert_well_formed(&flap);
        // The wing's chord runs back down −Z from its leading edge: the flap is the aft
        // quarter, the fixed part ahead of it, both over the middle of the span.
        let centre = |m: &DrawMesh| {
            m.positions
                .iter()
                .map(|p| DVec3::from(p.map(f64::from)))
                .sum::<DVec3>()
                / m.positions.len() as f64
        };
        assert!(centre(&flap).z < centre(&fixed).z - 0.5);
        for m in [&fixed, &flap] {
            let (lo, hi) = m
                .positions
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| {
                    (lo.min(p[0]), hi.max(p[0]))
                });
            assert!(
                (lo - 0.8).abs() < 1e-3 && (hi - 3.2).abs() < 1e-3,
                "{lo} {hi}"
            );
        }
    }

    #[test]
    fn a_truss_draws_its_members() {
        let g = shape("{ truss: { bottom_diameter: 2, length: 1, around: 6 } }");
        let mesh = &g.draw(|_| 0.0, 0.04)[0].mesh;
        assert_well_formed(mesh);
        let length = g.elements()[0].moments.measure;
        let area = PI * 0.04 * length;
        assert!(
            (mesh.area() - area).abs() < 0.05 * area,
            "{} {area}",
            mesh.area()
        );
    }
}
