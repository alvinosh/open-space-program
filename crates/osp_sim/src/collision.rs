//! What vessels collide with: the triangles they are drawn with.
//!
//! A part's collider ([`Mesh`]) is its geometry as [`crate::geometry::draw`] tessellates it for the
//! screen, the triangles of the models it is drawn from ([`model`]), and a strut and foot for a
//! landing leg. The air's outline and a model's proxy are not used: they are rounder and fatter
//! than what the player sees, and leave out engines, legs and antennas. A bounding volume hierarchy
//! over each mesh keeps a query near the few triangles in reach.
//!
//! The meshes are surfaces, not solids. That suits a stack of shells: something can sit inside an
//! adapter or fly into an open interstage. Two surfaces touch where an edge of a triangle of one
//! pierces a triangle of the other, and that says where and which way ([`Contact`]). The contact
//! solver is [`crate::contact`].

pub mod model;

use std::{collections::HashMap, fmt, sync::Arc};

use glam::{DQuat, DVec3};
use serde::{Deserialize, Serialize};

use crate::{
    assembly::Pose,
    component::BuiltPart,
    gear::Leg,
    geometry::draw::DrawMesh,
    parts::{PartKind, PartTree},
};

/// Triangles in a leaf of the hierarchy.
const LEAF: usize = 4;
/// The most contact points kept between two vessels: enough to say how they lie on each other.
const MOST_CONTACTS: usize = 64;
/// How many sides a leg's strut and foot are given.
const LEG_SIDES: usize = 8;

/// A box along its frame's axes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub min: DVec3,
    pub max: DVec3,
}

impl Bounds {
    const EMPTY: Self = Self {
        min: DVec3::splat(f64::INFINITY),
        max: DVec3::splat(f64::NEG_INFINITY),
    };

    fn grow(&mut self, p: DVec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    fn center(&self) -> DVec3 {
        (self.min + self.max) / 2.0
    }

    fn half(&self) -> DVec3 {
        (self.max - self.min) / 2.0
    }

    /// This box seen from a frame `pose` turns it into, as a box along that frame's axes.
    fn seen_from(&self, pose: &Seen) -> Self {
        let center = pose.point(self.center());
        let half = self.half();
        let half = DVec3::new(
            pose.abs[0].dot(half),
            pose.abs[1].dot(half),
            pose.abs[2].dot(half),
        );
        Self {
            min: center - half,
            max: center + half,
        }
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.min.cmple(other.max).all() && other.min.cmple(self.max).all()
    }

    fn within(&self, other: &Self, margin: f64) -> bool {
        (self.min - DVec3::splat(margin)).cmple(other.max).all()
            && (other.min - DVec3::splat(margin)).cmple(self.max).all()
    }
}

/// A pose worked out for rotating boxes: the absolute values of its rows.
struct Seen {
    pose: Pose,
    abs: [DVec3; 3],
}

impl Seen {
    fn new(pose: Pose) -> Self {
        let (x, y, z) = (
            pose.rotation * DVec3::X,
            pose.rotation * DVec3::Y,
            pose.rotation * DVec3::Z,
        );
        Self {
            pose,
            abs: [
                DVec3::new(x.x, y.x, z.x).abs(),
                DVec3::new(x.y, y.y, z.y).abs(),
                DVec3::new(x.z, y.z, z.z).abs(),
            ],
        }
    }

    fn point(&self, p: DVec3) -> DVec3 {
        self.pose.point(p)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Node {
    bounds: Bounds,
    /// A leaf's first triangle and how many; or else its first child, the other right after.
    first: u32,
    count: u32,
}

/// A surface of triangles in a part's frame, with a hierarchy over them.
#[derive(Debug, Clone)]
pub struct Mesh {
    vertices: Vec<DVec3>,
    triangles: Vec<[u32; 3]>,
    nodes: Vec<Node>,
}

impl Mesh {
    pub fn new(vertices: Vec<DVec3>, mut triangles: Vec<[u32; 3]>) -> Self {
        // Nothing to hit on one with no area, and it has no way to face.
        triangles.retain(|t| {
            let [a, b, c] = t.map(|i| vertices[i as usize]);
            (b - a).cross(c - a).length_squared() > 1e-20
        });
        let mut mesh = Self {
            vertices,
            triangles,
            nodes: Vec::new(),
        };
        if !mesh.triangles.is_empty() {
            let end = mesh.triangles.len();
            mesh.nodes.push(Node {
                bounds: Bounds::EMPTY,
                first: 0,
                count: 0,
            });
            mesh.build(0, 0, end);
        }
        mesh
    }

    fn corners(&self, triangle: usize) -> [DVec3; 3] {
        self.triangles[triangle].map(|i| self.vertices[i as usize])
    }

    fn build(&mut self, node: usize, start: usize, end: usize) {
        let mut bounds = Bounds::EMPTY;
        let mut middles = Bounds::EMPTY;
        for k in start..end {
            let corners = self.corners(k);
            for &p in &corners {
                bounds.grow(p);
            }
            middles.grow((corners[0] + corners[1] + corners[2]) / 3.0);
        }
        self.nodes[node].bounds = bounds;
        let count = end - start;
        let spread = middles.max - middles.min;
        if count <= LEAF || spread.max_element() <= 0.0 {
            self.nodes[node].first = start as u32;
            self.nodes[node].count = count as u32;
            return;
        }
        // Halfway through them along the way they are most spread out.
        let axis = spread.max_position();
        let vertices = &self.vertices;
        let middle = |t: &[u32; 3]| t.iter().map(|&i| vertices[i as usize][axis]).sum::<f64>();
        let half = count / 2;
        self.triangles[start..end]
            .select_nth_unstable_by(half, |a, b| middle(a).total_cmp(&middle(b)));
        let left = self.nodes.len();
        for _ in 0..2 {
            self.nodes.push(Node {
                bounds: Bounds::EMPTY,
                first: 0,
                count: 0,
            });
        }
        self.nodes[node].first = left as u32;
        self.build(left, start, start + half);
        self.build(left + 1, start + half, end);
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// How many triangles it has.
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// Each triangle's corners.
    pub fn triangles(&self) -> impl Iterator<Item = [DVec3; 3]> + '_ {
        (0..self.triangles.len()).map(|k| self.corners(k))
    }

    pub fn bounds(&self) -> Option<Bounds> {
        self.nodes.first().map(|n| n.bounds)
    }

    /// Visit each pair of triangles of this mesh and `other` whose boxes overlap, `to_other` taking
    /// this mesh's frame to `other`'s. `visit` says whether to go on.
    fn pairs(&self, other: &Mesh, to_other: Pose, mut visit: impl FnMut(usize, usize) -> bool) {
        if self.is_empty() || other.is_empty() {
            return;
        }
        let seen = Seen::new(to_other);
        let mut stack = vec![(0usize, 0usize)];
        while let Some((a, b)) = stack.pop() {
            let (na, nb) = (&self.nodes[a], &other.nodes[b]);
            if !na.bounds.seen_from(&seen).overlaps(&nb.bounds) {
                continue;
            }
            match (na.count > 0, nb.count > 0) {
                (true, true) => {
                    for ta in na.first..na.first + na.count {
                        for tb in nb.first..nb.first + nb.count {
                            if !visit(ta as usize, tb as usize) {
                                return;
                            }
                        }
                    }
                }
                (true, false) => {
                    stack.push((a, nb.first as usize));
                    stack.push((a, nb.first as usize + 1));
                }
                (false, true) => {
                    stack.push((na.first as usize, b));
                    stack.push((na.first as usize + 1, b));
                }
                (false, false) => {
                    // The bigger one first: it narrows the search fastest.
                    if na.bounds.half().length_squared() >= nb.bounds.half().length_squared() {
                        stack.push((na.first as usize, b));
                        stack.push((na.first as usize + 1, b));
                    } else {
                        stack.push((a, nb.first as usize));
                        stack.push((a, nb.first as usize + 1));
                    }
                }
            }
        }
    }
}

/// Where two surfaces touch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contact {
    pub point: DVec3,
    /// Unit: the way the first is pushed from the second.
    pub normal: DVec3,
    /// How far the edge that found it pokes through, m.
    pub depth: f64,
}

/// Where the segment `p` to `q` goes through triangle `t`, as a share of the way along it.
fn pierces(p: DVec3, q: DVec3, t: &[DVec3; 3]) -> Option<f64> {
    let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
    let along = q - p;
    let h = along.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1e-14 {
        return None;
    }
    let f = 1.0 / det;
    let s = p - t[0];
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let w = s.cross(e1);
    let v = f * along.dot(w);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let share = f * e2.dot(w);
    (0.0..=1.0).contains(&share).then_some(share)
}

/// How `other` lies on the plane of `face`: how far it reaches through on its lesser side, and the
/// unit way from the plane to its greater side, where most of it is. `None` if it does not reach
/// through, or `face` has no way to face.
fn lies_across(face: &[DVec3; 3], other: &[DVec3; 3]) -> Option<(f64, DVec3)> {
    let normal = (face[1] - face[0])
        .cross(face[2] - face[0])
        .normalize_or_zero();
    if normal == DVec3::ZERO {
        return None;
    }
    let (mut above, mut below) = (0.0_f64, 0.0_f64);
    for &p in other {
        let height = (p - face[0]).dot(normal);
        above = above.max(height);
        below = below.max(-height);
    }
    if above <= below {
        Some((above, -normal))
    } else {
        Some((below, normal))
    }
}

/// Where triangles `a` of the first vessel and `b` of the second go through each other. The two
/// planes offer two ways to part them, and the one that takes the shorter move is the true one: an
/// edge going through a wall a long way in is only where a big flat face lies across it. Most of
/// the triangle that goes through lies on the side it came from, so this needs no telling which way
/// the triangles are wound, and works for walls hit from inside as well as out.
fn triangle_contacts(a: &[DVec3; 3], b: &[DVec3; 3], out: &mut Vec<Contact>) {
    let mut points = [DVec3::ZERO; 6];
    let mut count = 0;
    for (edges, face) in [(a, b), (b, a)] {
        for k in 0..3 {
            let (p, q) = (edges[k], edges[(k + 1) % 3]);
            if let Some(share) = pierces(p, q, face) {
                points[count] = p + (q - p) * share;
                count += 1;
            }
        }
    }
    if count == 0 {
        return;
    }
    // Through `b`'s plane, `a` is pushed to where most of `a` is; through `a`'s, `b` is pushed to
    // where most of `b` is, and `a` the other way.
    let through_b = lies_across(b, a);
    let through_a = lies_across(a, b).map(|(depth, way)| (depth, -way));
    let Some((depth, normal)) = [through_b, through_a]
        .into_iter()
        .flatten()
        .min_by(|x, y| x.0.total_cmp(&y.0))
    else {
        return;
    };
    out.extend(points[..count].iter().map(|&point| Contact {
        point,
        normal,
        depth,
    }));
}

/// Whether two triangles go through each other.
fn triangles_touch(a: &[DVec3; 3], b: &[DVec3; 3]) -> bool {
    (0..3).any(|k| {
        pierces(a[k], a[(k + 1) % 3], b).is_some() || pierces(b[k], b[(k + 1) % 3], a).is_some()
    })
}

/// A part's mesh, where it is on the vessel.
#[derive(Debug, Clone)]
pub struct PlacedMesh {
    pub mesh: Arc<Mesh>,
    /// In the assembly frame.
    pub pose: Pose,
    /// Round that pose's meshes, in the assembly frame.
    pub bounds: Bounds,
}

/// A vessel's collider: each of its parts' meshes, where they are on it.
#[derive(Debug, Clone, Default)]
pub struct Collider {
    pub parts: Vec<PlacedMesh>,
}

impl Collider {
    /// The box round all its parts, in the frame they are placed in, if it has any.
    pub fn bounds(&self) -> Option<Bounds> {
        let mut parts = self.parts.iter().map(|part| part.bounds);
        let first = parts.next()?;
        Some(parts.fold(first, |all, bounds| Bounds {
            min: all.min.min(bounds.min),
            max: all.max.max(bounds.max),
        }))
    }

    fn in_space(part: &PlacedMesh, at: Pose) -> Bounds {
        part.bounds.seen_from(&Seen::new(at))
    }

    /// Whether any of this collider's parts is within `margin` of `other`'s, by their boxes.
    pub fn within(&self, at: Pose, other: &Collider, at_other: Pose, margin: f64) -> bool {
        let theirs: Vec<Bounds> = (other.parts.iter())
            .map(|p| Self::in_space(p, at_other))
            .collect();
        self.parts.iter().any(|part| {
            let mine = Self::in_space(part, at);
            theirs.iter().any(|other| mine.within(other, margin))
        })
    }

    /// Whether the two go through each other anywhere.
    pub fn touches(&self, at: Pose, other: &Collider, at_other: Pose) -> bool {
        let theirs: Vec<Bounds> = (other.parts.iter())
            .map(|p| Self::in_space(p, at_other))
            .collect();
        for part in &self.parts {
            let mine = Self::in_space(part, at);
            for (far, box_far) in other.parts.iter().zip(&theirs) {
                if !mine.overlaps(box_far) {
                    continue;
                }
                let (world_a, world_b) = (at * part.pose, at_other * far.pose);
                let to_far = world_b.inverse() * world_a;
                let mut found = false;
                part.mesh.pairs(&far.mesh, to_far, |ta, tb| {
                    let a = part.mesh.corners(ta).map(|p| to_far.point(p));
                    found = triangles_touch(&a, &far.mesh.corners(tb));
                    !found
                });
                if found {
                    return true;
                }
            }
        }
        false
    }

    /// Where the two touch, in space: at most [`MOST_CONTACTS`] points, spread over the touch.
    pub fn contacts(&self, at: Pose, other: &Collider, at_other: Pose) -> Vec<Contact> {
        let theirs: Vec<Bounds> = (other.parts.iter())
            .map(|p| Self::in_space(p, at_other))
            .collect();
        let mut out = Vec::new();
        for part in &self.parts {
            let mine = Self::in_space(part, at);
            for (far, box_far) in other.parts.iter().zip(&theirs) {
                if !mine.overlaps(box_far) {
                    continue;
                }
                let (world_a, world_b) = (at * part.pose, at_other * far.pose);
                let to_far = world_b.inverse() * world_a;
                part.mesh.pairs(&far.mesh, to_far, |ta, tb| {
                    let a = part.mesh.corners(ta).map(|p| world_a.point(p));
                    let b = far.mesh.corners(tb).map(|p| world_b.point(p));
                    triangle_contacts(&a, &b, &mut out);
                    true
                });
            }
        }
        if out.len() > MOST_CONTACTS {
            let step = out.len() as f64 / MOST_CONTACTS as f64;
            out = (0..MOST_CONTACTS)
                .map(|k| out[(k as f64 * step) as usize])
                .collect();
        }
        out
    }
}

/// Where a part's model files come from: its bytes, given the path a record gives it. Absent in
/// tests with no mods, where the model's proxy stands in.
pub type ModelFiles = Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

/// Colliders already built, so that each part and model file is only read once.
#[derive(Default)]
pub struct Colliders {
    files: Option<ModelFiles>,
    models: HashMap<String, Option<Arc<model::Triangles>>>,
    /// By the part as built: it is kept here so that its address stays its own.
    parts: HashMap<usize, (Arc<BuiltPart>, Arc<Mesh>)>,
}

impl fmt::Debug for Colliders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Colliders")
            .field("models", &self.models.len())
            .field("parts", &self.parts.len())
            .finish()
    }
}

impl Colliders {
    /// Read models through `files` from now on.
    pub fn set_files(&mut self, files: ModelFiles) {
        self.files = Some(files);
        self.models.clear();
        self.parts.clear();
    }

    fn model(&mut self, path: &str) -> Option<Arc<model::Triangles>> {
        if let Some(found) = self.models.get(path) {
            return found.clone();
        }
        let read = self
            .files
            .as_ref()
            .and_then(|files| files(path))
            .and_then(|bytes| model::Triangles::read(&bytes).map(Arc::new));
        self.models.insert(path.to_owned(), read.clone());
        read
    }

    fn part_mesh(&mut self, part: &crate::parts::Part) -> Option<Arc<Mesh>> {
        let built = part.built.as_ref()?;
        let key = Arc::as_ptr(built) as usize;
        if let Some((_, mesh)) = self.parts.get(&key) {
            return Some(mesh.clone());
        }
        let mut soup = Soup::default();
        let thickness = |element: &str| {
            (built.pieces.iter())
                .find(|p| p.element == element)
                .map_or(0.0, |p| p.thickness)
        };
        // Its own surfaces, and the models it is drawn from if they can be read; without them, what
        // stands in for them.
        let models: Vec<_> = built.geometry.models().collect();
        let mut drawn_all = true;
        for (shape, at, turn) in &models {
            match self.model(&shape.path) {
                Some(triangles) => soup.add_model(&triangles, *at, *turn, shape.scale),
                None => drawn_all = false,
            }
        }
        let pieces = if drawn_all {
            built.geometry.draw(thickness, built.member)
        } else {
            built.geometry.outer(thickness, built.member)
        };
        for piece in pieces {
            soup.add_drawn(&piece.mesh);
            soup.add_drawn(&piece.back);
        }
        // A leg is drawn from its record.
        for kind in &part.kinds {
            if let PartKind::Gear(def) = kind {
                for leg in Leg::at(def, DVec3::ZERO, DQuat::IDENTITY) {
                    soup.add_leg(&leg);
                }
            }
        }
        let mesh = Arc::new(Mesh::new(soup.vertices, soup.triangles));
        self.parts.insert(key, (built.clone(), mesh.clone()));
        Some(mesh)
    }

    /// The collider of `tree`'s parts.
    pub fn of(&mut self, tree: &PartTree) -> Collider {
        let mut parts = Vec::new();
        for part in tree.parts() {
            let Some(mesh) = self.part_mesh(part) else {
                continue;
            };
            let Some(bounds) = mesh.bounds() else {
                continue;
            };
            let pose = Pose::new(part.position, part.rotation);
            parts.push(PlacedMesh {
                bounds: bounds.seen_from(&Seen::new(pose)),
                mesh,
                pose,
            });
        }
        Collider { parts }
    }
}

/// Triangles on their way to being a mesh.
#[derive(Default)]
struct Soup {
    vertices: Vec<DVec3>,
    triangles: Vec<[u32; 3]>,
}

impl Soup {
    fn add_drawn(&mut self, mesh: &DrawMesh) {
        let first = self.vertices.len() as u32;
        self.vertices
            .extend(mesh.positions.iter().map(|p| DVec3::from(p.map(f64::from))));
        self.triangles.extend(
            (mesh.indices.chunks_exact(3)).map(|t| [t[0] + first, t[1] + first, t[2] + first]),
        );
    }

    fn add_model(&mut self, model: &model::Triangles, at: DVec3, turn: DQuat, scale: f64) {
        let first = self.vertices.len() as u32;
        self.vertices
            .extend(model.vertices.iter().map(|&p| at + turn * (p * scale)));
        self.triangles
            .extend(model.triangles.iter().map(|t| t.map(|i| i + first)));
    }

    /// A strut from the mount to the axle, and a foot or wheel at its end, as they are drawn.
    fn add_leg(&mut self, leg: &Leg) {
        let (mount, axle) = (leg.mount, leg.axle(0.0));
        let along = (axle - mount).normalize_or(DVec3::NEG_Z);
        let side = along.any_orthonormal_vector();
        let other = along.cross(side);
        let ring = |center: DVec3, radius: f64, k: usize| {
            let angle = std::f64::consts::TAU * k as f64 / LEG_SIDES as f64;
            center + (side * angle.cos() + other * angle.sin()) * radius
        };
        let strut = 0.125 * leg.wheel_radius;
        self.add_tube(
            LEG_SIDES,
            |k| ring(mount, strut, k),
            |k| ring(axle, strut, k),
        );
        let thick = if leg.foot { 0.1 } else { 0.35 } * leg.wheel_radius;
        let (near, far) = (axle - along * thick, axle + along * thick);
        self.add_tube(
            LEG_SIDES,
            |k| ring(near, leg.wheel_radius, k),
            |k| ring(far, leg.wheel_radius, k),
        );
        // The foot's end, flat.
        let middle = self.vertices.len() as u32;
        self.vertices.push(far);
        let first = self.vertices.len() as u32;
        self.vertices
            .extend((0..LEG_SIDES).map(|k| ring(far, leg.wheel_radius, k)));
        for k in 0..LEG_SIDES as u32 {
            self.triangles
                .push([middle, first + k, first + (k + 1) % LEG_SIDES as u32]);
        }
    }

    fn add_tube(
        &mut self,
        sides: usize,
        start: impl Fn(usize) -> DVec3,
        end: impl Fn(usize) -> DVec3,
    ) {
        let first = self.vertices.len() as u32;
        for k in 0..sides {
            self.vertices.push(start(k));
            self.vertices.push(end(k));
        }
        for k in 0..sides as u32 {
            let next = (k + 1) % sides as u32;
            let (a, b, c, d) = (
                first + 2 * k,
                first + 2 * k + 1,
                first + 2 * next,
                first + 2 * next + 1,
            );
            self.triangles.push([a, b, c]);
            self.triangles.push([c, b, d]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two triangles making a square of side `side` across X and Y, at height `z`.
    fn square(side: f64, z: f64) -> Mesh {
        let h = side / 2.0;
        Mesh::new(
            vec![
                DVec3::new(-h, -h, z),
                DVec3::new(h, -h, z),
                DVec3::new(h, h, z),
                DVec3::new(-h, h, z),
            ],
            vec![[0, 1, 2], [0, 2, 3]],
        )
    }

    /// A closed box, centred on the origin.
    fn cuboid(half: DVec3) -> Mesh {
        let corners = (0..8)
            .map(|k| {
                DVec3::new(
                    if k & 1 == 0 { -half.x } else { half.x },
                    if k & 2 == 0 { -half.y } else { half.y },
                    if k & 4 == 0 { -half.z } else { half.z },
                )
            })
            .collect();
        Mesh::new(
            corners,
            vec![
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
            ],
        )
    }

    fn collider(mesh: Mesh) -> Collider {
        let bounds = mesh.bounds().unwrap();
        Collider {
            parts: vec![PlacedMesh {
                mesh: Arc::new(mesh),
                pose: Pose::IDENTITY,
                bounds,
            }],
        }
    }

    fn at(position: DVec3) -> Pose {
        Pose::new(position, DQuat::IDENTITY)
    }

    /// A small generator, so that a failure comes back the same.
    struct Dice(u64);

    impl Dice {
        fn next(&mut self) -> f64 {
            self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005);
            self.0 = self.0.wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 33) as f64) / ((1u64 << 31) as f64)
        }

        fn point(&mut self, size: f64) -> DVec3 {
            DVec3::new(self.next() - 0.5, self.next() - 0.5, self.next() - 0.5) * size
        }
    }

    fn scattered(dice: &mut Dice, count: u32) -> Mesh {
        let mut vertices = Vec::new();
        let mut triangles = Vec::new();
        for k in 0..count {
            let middle = dice.point(10.0);
            for _ in 0..3 {
                vertices.push(middle + dice.point(1.0));
            }
            triangles.push([3 * k, 3 * k + 1, 3 * k + 2]);
        }
        Mesh::new(vertices, triangles)
    }

    /// The hierarchy finds exactly what looking at every pair finds.
    #[test]
    fn the_hierarchy_misses_nothing() {
        let mut dice = Dice(5);
        for _ in 0..20 {
            let (a, b) = (scattered(&mut dice, 300), scattered(&mut dice, 300));
            let pose = Pose::new(
                dice.point(4.0),
                DQuat::from_scaled_axis(dice.point(3.0)).normalize(),
            );
            let mut brute = Vec::new();
            for ta in 0..a.triangle_count() {
                for tb in 0..b.triangle_count() {
                    let moved = a.corners(ta).map(|p| pose.point(p));
                    if triangles_touch(&moved, &b.corners(tb)) {
                        brute.push((ta, tb));
                    }
                }
            }
            let mut found = Vec::new();
            a.pairs(&b, pose, |ta, tb| {
                let moved = a.corners(ta).map(|p| pose.point(p));
                if triangles_touch(&moved, &b.corners(tb)) {
                    found.push((ta, tb));
                }
                true
            });
            brute.sort_unstable();
            found.sort_unstable();
            assert_eq!(brute, found);
        }
    }

    #[test]
    fn surfaces_touch_only_where_they_cross() {
        let (a, b) = (
            collider(cuboid(DVec3::splat(1.0))),
            collider(cuboid(DVec3::splat(1.0))),
        );
        assert!(!a.touches(at(DVec3::Z * 2.01), &b, at(DVec3::ZERO)));
        assert!(a.touches(at(DVec3::Z * 1.99), &b, at(DVec3::ZERO)));
        // Boxes that only overlap as boxes do not touch: a long thin one past the corner of a
        // wide one, turned so that its box takes in the other's.
        let thin = collider(cuboid(DVec3::new(5.0, 0.05, 0.05)));
        let turned = Pose::new(
            DVec3::new(3.0, 3.0, 0.0),
            DQuat::from_rotation_z(-std::f64::consts::FRAC_PI_4),
        );
        let wide = collider(cuboid(DVec3::splat(1.0)));
        assert!(!thin.touches(turned, &wide, at(DVec3::ZERO)));
    }

    /// A contact pushes the first away from the second, whichever way the triangles are wound.
    #[test]
    fn a_contact_pushes_them_apart_however_they_are_wound() {
        let wound = |flip: bool| {
            let mut mesh = square(10.0, 0.0);
            if flip {
                mesh = Mesh::new(mesh.vertices.clone(), vec![[0, 2, 1], [0, 3, 2]]);
            }
            collider(mesh)
        };
        let block = collider(cuboid(DVec3::splat(1.0)));
        for flip in [false, true] {
            let contacts = block.contacts(at(DVec3::Z * 0.99), &wound(flip), at(DVec3::ZERO));
            assert!(!contacts.is_empty());
            for c in &contacts {
                assert!((c.normal - DVec3::Z).length() < 1e-9, "{c:?}");
                assert!(c.depth <= 0.011, "{c:?}");
            }
            // And the same with the two the other way round.
            let contacts = wound(flip).contacts(at(DVec3::ZERO), &block, at(DVec3::Z * 0.99));
            for c in &contacts {
                assert!((c.normal + DVec3::Z).length() < 1e-9, "{c:?}");
            }
        }
    }

    /// Something can fly into a hollow thing, and only its walls stop it.
    #[test]
    fn a_hollow_tube_is_hollow() {
        let mut soup = Soup::default();
        let circle = |z: f64, k: usize| {
            let angle = std::f64::consts::TAU * k as f64 / 32.0;
            DVec3::new(angle.cos() * 2.0, angle.sin() * 2.0, z)
        };
        soup.add_tube(32, |k| circle(0.0, k), |k| circle(6.0, k));
        let tube = collider(Mesh::new(soup.vertices, soup.triangles));
        let block = collider(cuboid(DVec3::splat(0.5)));
        // Inside it, clear of its walls, and through its open end.
        assert!(!block.touches(at(DVec3::Z * 3.0), &tube, at(DVec3::ZERO)));
        assert!(!block.touches(at(DVec3::Z * 6.0), &tube, at(DVec3::ZERO)));
        // Against its wall.
        assert!(block.touches(at(DVec3::new(1.8, 0.0, 3.0)), &tube, at(DVec3::ZERO)));
    }

    #[test]
    fn a_leg_has_a_strut_and_a_foot_where_it_is_drawn() {
        let def: crate::data::GearDef = crate::test_data::records(
            "{ name: 'leg', position: 0, angle: 0, length: 2, wheel_radius: 0.4, stroke: 0.3, \
             load: 1000, mass: 10, foot: true }",
        )
        .remove(0);
        let mut soup = Soup::default();
        for leg in Leg::at(&def, DVec3::ZERO, DQuat::IDENTITY) {
            soup.add_leg(&leg);
        }
        let mesh = Mesh::new(soup.vertices, soup.triangles);
        let bounds = mesh.bounds().unwrap();
        // Out along −Z, as far as the leg reaches, and as wide as its foot.
        assert!(bounds.max.z < 0.1, "{bounds:?}");
        assert!(bounds.min.z < -1.9, "{bounds:?}");
        assert!((bounds.max.x - 0.4).abs() < 1e-9, "{bounds:?}");
    }
}
