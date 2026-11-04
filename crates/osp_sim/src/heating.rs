//! Which of a vessel's parts the air's heat falls on.
//!
//! The vessel's outer surfaces are scattered with samples once, and looked at along the way it is
//! moving through the air: a sample facing that way, with nothing upstream of it, takes the heat.
//! How much is Sutton and Graves's stagnation heating ([`crate::plasma::stagnation_heating`]) over
//! its area, less as it leans away from the flow. A part with a heat shield takes what falls on the
//! surfaces the shield covers on the shield. The thermal systems do the rest
//! ([`crate::systems::Thermal`], [`crate::systems::HeatShield`]).

use std::collections::BTreeMap;

use glam::DVec3;

use crate::{
    aero::outer_triangles,
    component::Component,
    parts::{PartId, PartTree},
};

/// How far apart the samples on a surface are, about, m.
const SPACING: f64 = 0.4;
/// The most a triangle is cut into along each side.
const MOST_CUTS: usize = 8;
/// Cells along each side of the map of what is furthest upstream.
const CELLS: usize = 96;
/// A surface this nearly edge-on to the flow takes no heat: the cosine.
const EDGE_ON: f64 = 0.02;
/// How far behind the front a sample still counts as the front, in cells.
const DEPTH_SLACK: f64 = 2.0;
/// The flow has to turn this far before the map is worked out again, rad.
const TURN_TO_REMAP: f64 = 0.05;

#[derive(Debug, Clone, Copy)]
struct Sample {
    part: PartId,
    at: DVec3,
    normal: DVec3,
    area: f64,
    /// It is on a surface a heat shield covers.
    shielded: bool,
}

/// What of a part faces the flow, m², as seen along it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Exposed {
    pub bare: f64,
    pub shielded: f64,
}

/// The samples of a vessel's parts, and what they came to for the flow last asked about.
#[derive(Debug, Clone, Default)]
pub struct HeatHull {
    samples: Vec<Sample>,
    built_for: usize,
    toward: DVec3,
    exposed: BTreeMap<PartId, Exposed>,
}

impl HeatHull {
    /// What `tree`'s parts expose moving `toward` (unit, vessel axes). Cheap unless the parts have
    /// changed or the flow has turned.
    pub fn exposure(&mut self, tree: &PartTree, toward: DVec3) -> &BTreeMap<PartId, Exposed> {
        let count = tree.parts().len();
        let stale = self.built_for != count || self.samples.is_empty();
        if stale {
            self.samples = samples(tree);
            self.built_for = count;
        }
        if stale || self.toward.angle_between(toward) > TURN_TO_REMAP {
            self.exposed = exposed(&self.samples, toward);
            self.toward = toward;
        }
        &self.exposed
    }
}

fn samples(tree: &PartTree) -> Vec<Sample> {
    let mut out = Vec::new();
    let mut triangles = Vec::new();
    for part in tree.parts() {
        let Some(built) = &part.built else { continue };
        // The way the shield faces and how far round it goes, as a cosine.
        let shield = built.components.iter().find_map(|c| match c {
            Component::HeatShield(def) => Some((
                part.rotation * DVec3::from(def.faces).normalize_or(DVec3::NEG_Z),
                def.half_angle.to_radians().cos(),
            )),
            _ => None,
        });
        triangles.clear();
        let place = |p: DVec3| part.position + part.rotation * p;
        outer_triangles(built, place, &mut triangles);
        for &[a, b, c] in &triangles {
            let cross = (b - a).cross(c - a);
            let area = cross.length() / 2.0;
            let Some(normal) = cross.try_normalize() else {
                continue;
            };
            let shielded = shield.is_some_and(|(faces, cos)| normal.dot(faces) >= cos);
            let longest = (b - a).length().max((c - a).length()).max((c - b).length());
            let cuts = ((longest / SPACING).ceil() as usize).clamp(1, MOST_CUTS);
            let each = area / (cuts * cuts) as f64;
            // The middles of the triangles it cuts into, both ways up.
            for i in 0..cuts {
                for j in 0..(cuts - i) {
                    let mut add = |u: f64, v: f64| {
                        let (u, v) = (u / cuts as f64, v / cuts as f64);
                        out.push(Sample {
                            part: part.id,
                            at: a + (b - a) * u + (c - a) * v,
                            normal,
                            area: each,
                            shielded,
                        });
                    };
                    add(i as f64 + 1.0 / 3.0, j as f64 + 1.0 / 3.0);
                    if i + j + 1 < cuts {
                        add(i as f64 + 2.0 / 3.0, j as f64 + 2.0 / 3.0);
                    }
                }
            }
        }
    }
    out
}

fn exposed(samples: &[Sample], toward: DVec3) -> BTreeMap<PartId, Exposed> {
    let mut out: BTreeMap<PartId, Exposed> = BTreeMap::new();
    let (x, y) = toward.any_orthonormal_pair();
    let facing = || samples.iter().filter(|s| s.normal.dot(toward) > EDGE_ON);
    let (mut low, mut high) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for sample in facing() {
        let p = DVec3::new(sample.at.dot(x), sample.at.dot(y), 0.0);
        low = low.min(p);
        high = high.max(p);
    }
    if low.x > high.x {
        return out;
    }
    let span = (high.x - low.x).max(high.y - low.y).max(0.01);
    let size = span / (CELLS - 1) as f64;
    let cell = |at: DVec3| {
        let u = ((at.dot(x) - low.x) / size) as usize;
        let v = ((at.dot(y) - low.y) / size) as usize;
        u.min(CELLS - 1) + CELLS * v.min(CELLS - 1)
    };
    let mut front = vec![f64::NEG_INFINITY; CELLS * CELLS];
    for sample in facing() {
        let k = cell(sample.at);
        front[k] = front[k].max(sample.at.dot(toward));
    }
    let slack = DEPTH_SLACK * size;
    for sample in facing() {
        if sample.at.dot(toward) < front[cell(sample.at)] - slack {
            continue;
        }
        let seen = sample.area * sample.normal.dot(toward);
        let entry = out.entry(sample.part).or_default();
        if sample.shielded {
            entry.shielded += seen;
        } else {
            entry.bare += seen;
        }
    }
    out
}
