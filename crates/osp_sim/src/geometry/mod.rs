//! Part geometry: the one description of a part's shape that its mass, volumes,
//! attachment surfaces, aerodynamics and meshes all come from (see
//! `docs/vessel-overhaul.md`).
//!
//! A part's geometry is a list of subparts, each placed in the part's frame and of one of
//! four kinds:
//!
//! - [`Revolved`]: walls and regions in the (radius, height) half-plane turned about the
//!   subpart's axis, all the way round or over a span of angles: tanks with domes, skirts
//!   and common bulkheads, spheres, tori, sector tanks, rings, nose cones, adapters,
//!   interstages, fairing panels.
//! - [`Lofted`]: a closed section swept through stations: wings, fins, boxes, beams, prisms.
//! - [`Lattice`]: straight members between nodes: trusses.
//! - [`ModelRef`]: a glTF model, whose shape for the physics comes from proxy subparts.
//!
//! Geometry knows nothing of materials; it measures. Every wall, region, skin and member is
//! an [`Element`] with its [`Moments`] at unit density, by area, volume or length, and the
//! part multiplies them by what it is made of. A region or loft can also hold liquid
//! settled in any direction ([`Geometry::settle`]), and walls and skins have coordinates on
//! them for mounting things ([`Geometry::surface_point`]) that stay put when the part is
//! resized.
//!
//! A shape also has natural nodes ([`Node`]), where other parts stack on it: the ends of a
//! tank, shell or truss, a wing's root and tip, the nodes a model declares.
//!
//! Craft files describe shapes with presets ([`ShapeDef`]: `cylinder_tank`, `sphere`,
//! `torus`, `shell`, `wing`, `truss`, ...), which become these four kinds when loaded, so
//! nothing downstream knows about them.
//!
//! Frames: a subpart's axis is its +Z and angles about it run from +X towards +Y, as in the
//! assembly frame ([`crate::parts`]). Presets put the part's base at z = 0.

pub mod curve;
pub mod draw;
pub mod lattice;
pub mod lofted;
pub mod mesh;
pub mod moments;
pub mod preset;
mod quadrature;
pub mod revolved;

use glam::{DQuat, DVec3};
use serde::{Deserialize, Serialize};

pub use self::{
    lattice::Lattice,
    lofted::{Lofted, Station},
    moments::Moments,
    preset::ShapeDef,
    revolved::{Region, Revolved, Wall},
};

/// What a closed region or loft is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fill {
    /// Filled with what the part is made of: a ring, a wing, a battery.
    #[default]
    Solid,
    /// Empty space that can hold resources: a tank.
    Cell,
}

/// How an element is measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measure {
    Length,
    Area,
    Volume,
}

/// A measured piece of a part: a wall, a region, a loft's solid or skin, a lattice.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// Unique within the part: the subpart's name, a slash and the piece's, or the piece's
    /// alone for an unnamed subpart.
    pub name: String,
    pub measure: Measure,
    /// For volumes.
    pub fill: Option<Fill>,
    /// Part of a model's stand-in shape, not of the part's mass.
    pub proxy: bool,
    /// At unit density, in the part's frame.
    pub moments: Moments,
}

/// A point on a surface, in the part's frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfacePoint {
    pub position: DVec3,
    /// Unit, out of the front of the surface.
    pub normal: DVec3,
    /// How the point moves with its surface coordinates (not unit).
    pub along_u: DVec3,
    pub along_v: DVec3,
}

/// Liquid settled in a region or loft.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settled {
    /// Height of its surface along `up`, in the part's frame, m.
    pub level: f64,
    /// By volume, in the part's frame.
    pub moments: Moments,
}

/// A place on a part where another part's node meets it, node to node.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Unique within the part: `bottom` and `top` for the ends of a stack, `root` and `tip`
    /// for a wing.
    pub name: String,
    /// In the part's frame, m.
    pub position: DVec3,
    /// Unit, out of the part: the way the part joined there lies.
    pub direction: DVec3,
    /// Diameter of what fits there, m; 0 for a point, like the tip of a nose cone.
    pub size: f64,
}

/// A glTF model, relative to its mod's root.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelRef {
    pub path: String,
    pub scale: f64,
}

#[derive(Debug, Clone)]
pub enum Kind {
    Revolved(Revolved),
    Lofted(Lofted),
    Lattice(Lattice),
    Model(ModelRef),
}

#[derive(Debug, Clone)]
pub struct Subpart {
    /// Unique within the part, or empty.
    pub name: String,
    /// Where its origin is in the part's frame, and how its axes are turned there.
    pub position: DVec3,
    pub rotation: DQuat,
    /// Stands in for a model: it shapes the air and collisions but has no mass of its own.
    pub proxy: bool,
    /// Inside the part and not drawn: room its equipment takes, which weighs.
    pub unseen: bool,
    pub kind: Kind,
}

impl Subpart {
    pub fn new(name: impl Into<String>, kind: Kind) -> Self {
        Self {
            name: name.into(),
            position: DVec3::ZERO,
            rotation: DQuat::IDENTITY,
            proxy: false,
            unseen: false,
            kind,
        }
    }

    /// The name of one of its pieces within the part.
    pub(crate) fn named(&self, local: &str) -> String {
        match (self.name.is_empty(), local.is_empty()) {
            (true, _) => local.to_owned(),
            (false, true) => self.name.clone(),
            (false, false) => format!("{}/{local}", self.name),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Geometry {
    pub subparts: Vec<Subpart>,
    pub nodes: Vec<Node>,
}

impl Geometry {
    /// Every measured piece of every subpart.
    pub fn elements(&self) -> Vec<Element> {
        let mut out = Vec::new();
        for sub in &self.subparts {
            let mut push = |name: String, measure, fill, moments: Moments| {
                out.push(Element {
                    name: sub.named(&name),
                    measure,
                    fill,
                    proxy: sub.proxy,
                    moments: moments.placed(sub.position, sub.rotation),
                })
            };
            match &sub.kind {
                Kind::Revolved(r) => {
                    for (i, wall) in r.walls().iter().enumerate() {
                        push(wall.name.clone(), Measure::Area, None, r.wall_moments(i));
                    }
                    for (i, region) in r.regions().iter().enumerate() {
                        let name = region.name.clone();
                        push(
                            name.clone(),
                            Measure::Volume,
                            Some(region.fill),
                            r.region_moments(i),
                        );
                        if !r.is_full() {
                            push(
                                format!("{name}/sides"),
                                Measure::Area,
                                None,
                                r.side_moments(i),
                            );
                        }
                    }
                }
                Kind::Lofted(l) => {
                    push(
                        l.name.clone(),
                        Measure::Volume,
                        Some(l.fill),
                        l.solid_moments(),
                    );
                    push(
                        format!("{}/skin", l.name),
                        Measure::Area,
                        None,
                        l.skin_moments(),
                    );
                }
                Kind::Lattice(l) => push(l.name.clone(), Measure::Length, None, l.moments()),
                Kind::Model(_) => {}
            }
        }
        out
    }

    pub fn node(&self, name: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.name == name)
    }

    pub fn element(&self, name: &str) -> Option<Element> {
        self.elements().into_iter().find(|e| e.name == name)
    }

    /// The surfaces things can be mounted on: walls and skins, by element name.
    pub fn surfaces(&self) -> Vec<String> {
        let mut out = Vec::new();
        for sub in &self.subparts {
            match &sub.kind {
                Kind::Revolved(r) => out.extend(r.walls().iter().map(|w| sub.named(&w.name))),
                Kind::Lofted(l) => out.push(sub.named(&format!("{}/skin", l.name))),
                Kind::Lattice(_) | Kind::Model(_) => {}
            }
        }
        out
    }

    /// The point at coordinates `u` and `v` (each 0 to 1) on the surface named `surface`: on
    /// a revolved wall `u` runs around its span and `v` along its curve; on a loft's skin `u`
    /// runs up its stations and `v` around its section.
    pub fn surface_point(&self, surface: &str, u: f64, v: f64) -> Option<SurfacePoint> {
        for sub in &self.subparts {
            let found = match &sub.kind {
                Kind::Revolved(r) => r
                    .walls()
                    .iter()
                    .position(|w| sub.named(&w.name) == surface)
                    .map(|i| r.wall_point(i, u, v)),
                Kind::Lofted(l) if sub.named(&format!("{}/skin", l.name)) == surface => {
                    Some(l.surface_point(u, v))
                }
                _ => None,
            };
            if let Some(p) = found {
                let turn = sub.rotation;
                return Some(SurfacePoint {
                    position: sub.position + turn * p.position,
                    normal: turn * p.normal,
                    along_u: turn * p.along_u,
                    along_v: turn * p.along_v,
                });
            }
        }
        None
    }

    /// `volume` of liquid settled in the region or loft named `name` under an acceleration
    /// that pushes it away from `up` (a direction in the part's frame): it fills the space
    /// up to a flat surface square to `up`. Along a revolved region's own axis this is
    /// exact; otherwise it is measured on the region's mesh and scaled to its exact volume.
    pub fn settle(&self, name: &str, up: DVec3, volume: f64) -> Option<Settled> {
        let up = up.try_normalize()?;
        for sub in &self.subparts {
            // `up` in the subpart's frame, and the height of its origin along it.
            let local = sub.rotation.inverse() * up;
            let base = up.dot(sub.position);
            let placed = |level: f64, m: Moments| Settled {
                level: base + level,
                moments: m.placed(sub.position, sub.rotation),
            };
            match &sub.kind {
                Kind::Revolved(r) => {
                    let Some(i) = r.regions().iter().position(|g| sub.named(&g.name) == name)
                    else {
                        continue;
                    };
                    let total = r.region_moments(i);
                    let volume = volume.clamp(0.0, total.measure);
                    if local.z.abs() >= 1.0 - 1e-12 {
                        let settled = if local.z > 0.0 {
                            let h = r.level(i, volume);
                            (h, r.below(i, h))
                        } else {
                            // Upside down: the liquid fills from the top.
                            let h = r.level(i, total.measure - volume);
                            (-h, total - r.below(i, h))
                        };
                        return Some(placed(settled.0, settled.1));
                    }
                    let (level, m) = placed_on_mesh(r.region_mesh(i), local, volume, total.measure);
                    return Some(placed(level, m));
                }
                Kind::Lofted(l) if sub.named(&l.name) == name => {
                    let total = l.solid_moments().measure;
                    let (level, m) =
                        placed_on_mesh(l.mesh(), local, volume.clamp(0.0, total), total);
                    return Some(placed(level, m));
                }
                _ => {}
            }
        }
        None
    }
}

/// `volume` of liquid settled on `mesh` against `up`, where the shape's exact volume is
/// `exact`: the mesh is filled to the same share of its own volume, and what it holds scaled
/// up to `volume`.
fn placed_on_mesh(mesh: &mesh::TriMesh, up: DVec3, volume: f64, exact: f64) -> (f64, Moments) {
    let own = mesh.volume_moments().measure;
    let share = if exact > 0.0 { volume / exact } else { 0.0 };
    let level = mesh.level(up, share * own, own);
    let moments = mesh.below(up, level);
    let scale = if moments.measure > 0.0 {
        volume / moments.measure
    } else {
        0.0
    };
    (level, moments.scaled(scale))
}

#[cfg(test)]
mod tests;
