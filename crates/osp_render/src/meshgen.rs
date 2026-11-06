//! A small builder for low-poly procedural meshes with vertex colours (trees, launch-site
//! structures). Shapes are appended into one mesh so a whole group draws in one call.

use std::f32::consts::TAU;

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};

#[derive(Default)]
pub struct MeshBuilder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl MeshBuilder {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    fn vertex(&mut self, p: Vec3, n: Vec3, color: LinearRgba) -> u32 {
        self.positions.push(p.to_array());
        self.normals.push(n.to_array());
        self.colors.push(color.to_f32_array());
        (self.positions.len() - 1) as u32
    }

    /// A flat-shaded triangle (counter-clockwise seen from the front).
    pub fn triangle(&mut self, a: Vec3, b: Vec3, c: Vec3, color: LinearRgba) {
        let n = (b - a).cross(c - a).normalize_or_zero();
        let i = self.vertex(a, n, color);
        self.vertex(b, n, color);
        self.vertex(c, n, color);
        self.indices.extend([i, i + 1, i + 2]);
    }

    /// A triangle with given per-vertex normals (for smooth-looking foliage).
    pub fn triangle_smooth(&mut self, v: [(Vec3, Vec3); 3], color: LinearRgba) {
        let i = self.vertex(v[0].0, v[0].1, color);
        self.vertex(v[1].0, v[1].1, color);
        self.vertex(v[2].0, v[2].1, color);
        self.indices.extend([i, i + 1, i + 2]);
    }

    pub fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3, color: LinearRgba) {
        self.triangle(a, b, c, color);
        self.triangle(a, c, d, color);
    }

    /// An oriented box: `center`, half extents along the rotated axes.
    pub fn cuboid(&mut self, center: Vec3, half: Vec3, rotation: Quat, color: LinearRgba) {
        let corner = |x: f32, y: f32, z: f32| center + rotation * (half * Vec3::new(x, y, z));
        let faces = [
            // +X, -X, +Y, -Y, +Z, -Z; each counter-clockwise from outside.
            [(1., -1., -1.), (1., 1., -1.), (1., 1., 1.), (1., -1., 1.)],
            [
                (-1., -1., 1.),
                (-1., 1., 1.),
                (-1., 1., -1.),
                (-1., -1., -1.),
            ],
            [(-1., 1., -1.), (-1., 1., 1.), (1., 1., 1.), (1., 1., -1.)],
            [
                (-1., -1., 1.),
                (-1., -1., -1.),
                (1., -1., -1.),
                (1., -1., 1.),
            ],
            [(-1., -1., 1.), (1., -1., 1.), (1., 1., 1.), (-1., 1., 1.)],
            [
                (1., -1., -1.),
                (-1., -1., -1.),
                (-1., 1., -1.),
                (1., 1., -1.),
            ],
        ];
        for face in faces {
            let [a, b, c, d] = face.map(|(x, y, z)| corner(x, y, z));
            self.quad(a, b, c, d, color);
        }
    }

    /// A square-section beam of width `width` from `a` to `b`.
    pub fn beam(&mut self, a: Vec3, b: Vec3, width: f32, color: LinearRgba) {
        let axis = b - a;
        let length = axis.length();
        if length < 1e-4 {
            return;
        }
        let rotation = Quat::from_rotation_arc(Vec3::Y, axis / length);
        self.cuboid(
            (a + b) / 2.0,
            Vec3::new(width / 2.0, length / 2.0, width / 2.0),
            rotation,
            color,
        );
    }

    /// A cylinder (or truncated cone) from `base` along `axis`, smooth-sided.
    pub fn frustum(
        &mut self,
        base: Vec3,
        axis: Vec3,
        radii: (f32, f32),
        segments: usize,
        caps: bool,
        color: LinearRgba,
    ) {
        let up = axis.normalize_or(Vec3::Y);
        let side = up.any_orthonormal_vector();
        let other = up.cross(side);
        let ring = |k: usize| {
            let a = TAU * k as f32 / segments as f32;
            side * a.cos() + other * a.sin()
        };
        let slope = (radii.0 - radii.1) / axis.length().max(1e-4);
        for k in 0..segments {
            let (r0, r1) = (ring(k), ring(k + 1));
            let (n0, n1) = ((r0 + up * slope).normalize(), (r1 + up * slope).normalize());
            let (a, b) = (base + r0 * radii.0, base + r1 * radii.0);
            let (c, d) = (base + axis + r1 * radii.1, base + axis + r0 * radii.1);
            if radii.1 > 1e-4 {
                self.triangle_smooth([(a, n0), (b, n1), (c, n1)], color);
                self.triangle_smooth([(a, n0), (c, n1), (d, n0)], color);
            } else {
                self.triangle_smooth([(a, n0), (b, n1), (c, (n0 + n1).normalize())], color);
            }
            if caps {
                self.triangle(base, b, a, color);
                if radii.1 > 1e-4 {
                    self.triangle(base + axis, d, c, color);
                }
            }
        }
    }

    /// A lumpy ellipsoid (foliage, bushes): an octahedron subdivided `detail` times, with
    /// normals pointing away from its centre so it shades softly. `jitter(dir)` scales the
    /// radius in each direction.
    pub fn blob(
        &mut self,
        center: Vec3,
        radii: Vec3,
        rotation: Quat,
        detail: u32,
        color: LinearRgba,
        jitter: impl Fn(Vec3) -> f32,
    ) {
        let mut faces: Vec<[Vec3; 3]> = Vec::new();
        let (x, y, z) = (Vec3::X, Vec3::Y, Vec3::Z);
        // Counter-clockwise seen from outside.
        for (a, b) in [(x, z), (z, -x), (-x, -z), (-z, x)] {
            faces.push([b, a, y]);
            faces.push([a, b, -y]);
        }
        for _ in 0..detail {
            let mut next = Vec::with_capacity(faces.len() * 4);
            for [a, b, c] in faces {
                let (ab, bc, ca) = (
                    (a + b).normalize(),
                    (b + c).normalize(),
                    (c + a).normalize(),
                );
                next.extend([[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
            }
            faces = next;
        }
        let place = |d: Vec3| center + rotation * (d * radii * jitter(d));
        let normal = |d: Vec3| (rotation * (d / radii)).normalize();
        for [a, b, c] in faces {
            self.triangle_smooth(
                [
                    (place(a), normal(a)),
                    (place(b), normal(b)),
                    (place(c), normal(c)),
                ],
                color,
            );
        }
    }

    /// A drooping leaf or frond: a strip from `base` in direction `out`, bending down.
    pub fn frond(
        &mut self,
        base: Vec3,
        out: Vec3,
        up: Vec3,
        length: f32,
        width: f32,
        color: LinearRgba,
    ) {
        let side = out.cross(up).normalize_or(Vec3::X) * (width / 2.0);
        let mid = base + out * (length * 0.5) + up * (length * 0.12);
        let tip = base + out * length - up * (length * 0.35);
        let n = up;
        // Visible from both sides: the material is double-sided.
        self.triangle_smooth([(base, n), (mid - side, n), (mid + side, n)], color);
        self.triangle_smooth([(mid - side, n), (tip, n), (mid + side, n)], color);
    }

    pub fn build(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
        .with_inserted_indices(Indices::U32(self.indices))
    }
}
