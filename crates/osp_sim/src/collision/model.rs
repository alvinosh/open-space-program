//! The triangles of a glTF model, as it is drawn.
//!
//! glTF is Y-up and the sim is Z-up. The game draws a model as it comes, under a part whose frame is
//! the sim's turned into the renderer's (X → X, Y → −Z, Z → Y), so what a model calls `(x, y, z)` is
//! `(x, −z, y)` in the part's frame.

use glam::{DMat4, DVec3};

/// A model's triangles, in the frame of the part it is on, before its scale.
#[derive(Debug, Clone, Default)]
pub struct Triangles {
    pub vertices: Vec<DVec3>,
    pub triangles: Vec<[u32; 3]>,
}

impl Triangles {
    /// The triangles of a binary glTF file; `None` if it is no such thing, or keeps its data
    /// anywhere but in the file.
    pub fn read(bytes: &[u8]) -> Option<Self> {
        let file = gltf::Gltf::from_slice(bytes).ok()?;
        let blob = file.blob.as_deref();
        let scene = file.default_scene().or_else(|| file.scenes().next())?;
        let mut out = Self::default();
        for node in scene.nodes() {
            out.add(&node, DMat4::IDENTITY, blob)?;
        }
        (!out.triangles.is_empty()).then_some(out)
    }

    fn add(&mut self, node: &gltf::Node, above: DMat4, blob: Option<&[u8]>) -> Option<()> {
        let local = DMat4::from_cols_array_2d(&node.transform().matrix().map(|c| c.map(f64::from)));
        let here = above * local;
        if let Some(mesh) = node.mesh() {
            for primitive in mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let reader = primitive.reader(|buffer| match buffer.source() {
                    gltf::buffer::Source::Bin => blob,
                    gltf::buffer::Source::Uri(_) => None,
                });
                let first = self.vertices.len() as u32;
                let positions = reader.read_positions()?;
                self.vertices.extend(positions.map(|p| {
                    let p = here.transform_point3(DVec3::from(p.map(f64::from)));
                    DVec3::new(p.x, -p.z, p.y)
                }));
                let count = self.vertices.len() as u32 - first;
                match reader.read_indices() {
                    Some(indices) => {
                        let indices: Vec<u32> = indices.into_u32().collect();
                        self.triangles.extend(
                            indices
                                .chunks_exact(3)
                                .map(|t| [t[0] + first, t[1] + first, t[2] + first]),
                        );
                    }
                    None => self
                        .triangles
                        .extend((0..count / 3).map(|k| [0, 1, 2].map(|i| first + 3 * k + i))),
                }
            }
        }
        for child in node.children() {
            self.add(&child, here, blob)?;
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn read(relative: &str) -> Triangles {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let bytes = std::fs::read(root.join(relative)).unwrap();
        Triangles::read(&bytes).unwrap()
    }

    fn bounds(t: &Triangles) -> (DVec3, DVec3) {
        let (mut low, mut high) = (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN));
        for &v in &t.vertices {
            low = low.min(v);
            high = high.max(v);
        }
        (low, high)
    }

    /// The real models, read the way the game draws them: up is +Z. The highest thing on the lunar
    /// module is an aerial well off to one side; on its axis is the tunnel, whose rim goes all the
    /// way round at the height of the craft's `dock` node, with its hatch down inside it.
    #[test]
    fn the_lunar_module_stands_up_with_a_tunnel_on_its_axis() {
        let t = read("mods/realism/models/pods/ROC-LEMAscentBDB.glb");
        assert!(t.triangles.len() > 10_000, "{}", t.triangles.len());
        let (low, high) = bounds(&t);
        assert!((low.z + 0.892).abs() < 1e-3, "{low}");
        assert!((high.z - 2.852).abs() < 1e-3, "{high}");
        let rim: Vec<&DVec3> = (t.vertices.iter())
            .filter(|v| (0.46..0.54).contains(&v.truncate().length()) && v.z > 1.9)
            .collect();
        let top = rim.iter().map(|v| v.z).fold(0.0, f64::max);
        assert!((top - 2.0).abs() < 2e-3, "{top}");
        for sector in 0..12 {
            let at = f64::from(sector) * std::f64::consts::TAU / 12.0;
            let way = glam::DVec2::from_angle(at);
            assert!(
                rim.iter().any(|v| v.truncate().normalize().dot(way) > 0.9),
                "nothing of the rim towards {at}"
            );
        }
        // Down the tunnel: nothing near the axis comes up to the rim.
        let inside = (t.vertices.iter())
            .filter(|v| v.truncate().length() < 0.44)
            .map(|v| v.z)
            .fold(f64::MIN, f64::max);
        assert!(inside < 1.75, "{inside}");
    }

    #[test]
    fn the_command_module_narrows_to_its_nose() {
        let t = read("mods/realism/models/pods/ROC-ApolloCMBDB.glb");
        let (low, high) = bounds(&t);
        assert!(low.z.abs() < 0.05, "{low}");
        assert!((high.z - 2.092).abs() < 1e-3, "{high}");
        let widest_at_nose = (t.vertices.iter())
            .filter(|v| v.z > 2.0)
            .map(|v| v.truncate().length())
            .fold(0.0, f64::max);
        assert!(widest_at_nose < 0.6, "{widest_at_nose}");
    }

    #[test]
    fn anything_else_is_not_a_model() {
        assert!(Triangles::read(b"not a model").is_none());
    }
}
