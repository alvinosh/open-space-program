//! What vessels collide with: convex pieces cut from the triangles they are drawn with.
//!
//! A part's collider ([`Mesh`]) starts from its geometry as [`crate::geometry::draw`] tessellates it
//! for the screen, the triangles of the models it is drawn from ([`model`]), and a strut and foot for
//! a landing leg. So anything that can be drawn collides, however it was made: nothing here knows
//! about a kind of part. The air's outline and a model's proxy are not used: they are rounder and
//! fatter than what the player sees, and leave out engines, legs and antennas.
//!
//! The solver ([`crate::contact`]) works on convex shapes, so the first time a part may be hit its
//! triangles are cut into convex hulls ([`cut`]). The cut follows the surface, so a closed tank is
//! one solid hull and an open nozzle or interstage is a ring of panels with nothing in the middle:
//! something can sit inside an adapter or fly into an open interstage.

mod cut;
pub mod model;

use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::{Arc, OnceLock},
};

use glam::{DQuat, DVec3};
use rapier3d_f64::{
    parry::{query, shape::SharedShape},
    prelude::Pose as ShapePose,
};
use serde::{Deserialize, Serialize};

use crate::{
    assembly::Pose,
    component::BuiltPart,
    gear::Leg,
    geometry::draw::DrawMesh,
    parts::{PartKind, PartTree},
};

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

/// A surface of triangles in a part's frame, and the convex pieces it is cut into when one is needed.
#[derive(Debug)]
pub struct Mesh {
    vertices: Vec<DVec3>,
    triangles: Vec<[u32; 3]>,
    bounds: Option<Bounds>,
    shape: OnceLock<Option<SharedShape>>,
}

impl Mesh {
    pub fn new(vertices: Vec<DVec3>, mut triangles: Vec<[u32; 3]>) -> Self {
        // Nothing to hit on one with no area, and it has no way to face.
        triangles.retain(|t| {
            let [a, b, c] = t.map(|i| vertices[i as usize]);
            (b - a).cross(c - a).length_squared() > 1e-20
        });
        let mut bounds = None;
        for &p in triangles.iter().flatten().map(|&i| &vertices[i as usize]) {
            bounds.get_or_insert(Bounds::EMPTY).grow(p);
        }
        Self {
            vertices,
            triangles,
            bounds,
            shape: OnceLock::new(),
        }
    }

    fn corners(&self, triangle: usize) -> [DVec3; 3] {
        self.triangles[triangle].map(|i| self.vertices[i as usize])
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
        self.bounds
    }

    /// What the solver sees of it, in its frame; cut the first time it is asked for.
    pub(crate) fn shape(&self) -> Option<&SharedShape> {
        self.shape
            .get_or_init(|| {
                let pieces: Vec<(ShapePose, SharedShape)> =
                    cut::convex_pieces(&self.vertices, &self.triangles)
                        .iter()
                        .filter_map(|corners| SharedShape::convex_hull(corners))
                        .map(|hull| (ShapePose::IDENTITY, hull))
                        .collect();
                (!pieces.is_empty()).then(|| SharedShape::compound(pieces))
            })
            .as_ref()
    }

    /// How many convex pieces it is cut into.
    pub fn piece_count(&self) -> usize {
        self.shape()
            .and_then(|shape| shape.as_compound())
            .map_or(0, |compound| compound.shapes().len())
    }

    /// The edges of its convex pieces, in its frame, if it has been cut yet: nothing here waits for
    /// that (see [`Colliders::prepare`]).
    pub fn piece_edges(&self) -> Vec<[DVec3; 2]> {
        let shape = self.shape.get().and_then(Option::as_ref);
        let Some(compound) = shape.and_then(|shape| shape.as_compound()) else {
            return Vec::new();
        };
        let mut lines = Vec::new();
        for (_, piece) in compound.shapes() {
            let Some(hull) = piece.as_convex_polyhedron() else {
                continue;
            };
            let points = hull.points();
            lines.extend(
                (hull.edges().iter()).map(|edge| edge.vertices.map(|v: u32| points[v as usize])),
            );
        }
        lines
    }
}

/// The placing of a shape, from one of ours.
pub(crate) fn shape_pose(pose: Pose) -> ShapePose {
    ShapePose::from_parts(pose.position, pose.rotation)
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

    /// Whether the two overlap anywhere, as the solver sees them.
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
                let (Some(near), Some(away)) = (part.mesh.shape(), far.mesh.shape()) else {
                    continue;
                };
                let hit = query::intersection_test(
                    &shape_pose(at * part.pose),
                    near.as_ref(),
                    &shape_pose(at_other * far.pose),
                    away.as_ref(),
                );
                if hit.is_ok_and(|hit| hit.intersecting) {
                    return true;
                }
            }
        }
        false
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
    /// The meshes that have been sent off to be cut.
    cutting: HashSet<usize>,
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
        self.cutting.clear();
    }

    /// Start cutting `collider`'s parts into convex pieces, in the background: a big vessel takes a
    /// second or so, and that should not all fall on the tick it is hit. If they are needed before
    /// that is done, whoever needs them waits for it.
    pub fn prepare(&mut self, collider: &Collider) {
        let meshes: Vec<Arc<Mesh>> = (collider.parts.iter())
            .filter(|part| self.cutting.insert(Arc::as_ptr(&part.mesh) as usize))
            .map(|part| part.mesh.clone())
            .collect();
        if meshes.is_empty() {
            return;
        }
        let cut = move || {
            for mesh in meshes {
                mesh.shape();
            }
        };
        // Without a thread to do it on, it is done when it is needed.
        let _ = std::thread::Builder::new()
            .name("collider cutting".into())
            .spawn(cut);
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

    fn tube(radius: f64, height: f64) -> Mesh {
        let mut soup = Soup::default();
        let circle = |z: f64, k: usize| {
            let angle = std::f64::consts::TAU * k as f64 / 32.0;
            DVec3::new(angle.cos() * radius, angle.sin() * radius, z)
        };
        soup.add_tube(32, |k| circle(0.0, k), |k| circle(height, k));
        Mesh::new(soup.vertices, soup.triangles)
    }

    #[test]
    fn solids_touch_only_where_they_meet() {
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

    /// What is closed is solid, all the way through.
    #[test]
    fn a_closed_box_is_solid() {
        let closed = collider(cuboid(DVec3::splat(3.0)));
        let block = collider(cuboid(DVec3::splat(0.2)));
        assert!(block.touches(at(DVec3::ZERO), &closed, at(DVec3::ZERO)));
        assert!(block.touches(at(DVec3::new(1.5, -1.5, 1.5)), &closed, at(DVec3::ZERO)));
    }

    /// Something can fly into a hollow thing, and only its walls stop it.
    #[test]
    fn a_hollow_tube_is_hollow() {
        let tube = collider(tube(2.0, 6.0));
        let block = collider(cuboid(DVec3::splat(0.5)));
        // Inside it, clear of its walls, and through its open end.
        assert!(!block.touches(at(DVec3::Z * 3.0), &tube, at(DVec3::ZERO)));
        assert!(!block.touches(at(DVec3::Z * 6.0), &tube, at(DVec3::ZERO)));
        // Against its wall.
        assert!(block.touches(at(DVec3::new(1.8, 0.0, 3.0)), &tube, at(DVec3::ZERO)));
    }

    /// As wide as it is tall or wider, with room all the way up it.
    #[test]
    fn a_wide_tube_has_nothing_across_it() {
        let tube = collider(tube(5.0, 5.6));
        let block = collider(cuboid(DVec3::splat(0.5)));
        for k in 0..=12 {
            let z = 0.5 * k as f64;
            assert!(
                !block.touches(at(DVec3::Z * z), &tube, at(DVec3::ZERO)),
                "{z}"
            );
        }
    }

    /// A flat plate, which no hull can be made of as it is, still gets in the way.
    #[test]
    fn a_flat_plate_is_there() {
        let plate = Mesh::new(
            vec![
                DVec3::new(-5.0, -5.0, 0.0),
                DVec3::new(5.0, -5.0, 0.0),
                DVec3::new(5.0, 5.0, 0.0),
                DVec3::new(-5.0, 5.0, 0.0),
            ],
            vec![[0, 1, 2], [0, 2, 3]],
        );
        let plate = collider(plate);
        let block = collider(cuboid(DVec3::splat(1.0)));
        assert!(block.touches(at(DVec3::Z * 0.5), &plate, at(DVec3::ZERO)));
        assert!(!block.touches(at(DVec3::Z * 1.5), &plate, at(DVec3::ZERO)));
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
