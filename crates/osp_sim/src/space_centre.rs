//! A space centre's buildings as they stand on the ground: where each is and the box round it, to
//! draw it and to find the one under the pointer, and the earthworks under them, a level apron for
//! each and the roads, which the terrain lays out as it does a launch site's pad and roads
//! ([`crate::terrain::install`]).

use glam::DVec3;

use crate::{
    body::Body,
    data::{BuildingDef, LaunchSiteDef, PadDef, SpaceCentreDef},
    terrain,
};

/// Concrete round a building's walls, m.
const APRON: f64 = 25.0;
/// How steep the bank round an apron is, rise over run.
const APRON_SLOPE: f64 = 0.3;
const APRON_CORNER: f64 = 8.0;

/// The centre's layout as a launch site's, with its roads and nothing to launch from: its middle
/// and heading place things in it ([`terrain::site_point`], [`terrain::site_axes`]).
pub fn layout(def: &SpaceCentreDef) -> LaunchSiteDef {
    LaunchSiteDef {
        id: def.id.clone(),
        name: def.name.clone(),
        body: def.body.clone(),
        latitude: def.latitude,
        longitude: def.longitude,
        heading: def.heading,
        pad: None,
        roads: def.roads.clone(),
        runway: None,
    }
}

/// `building` of `def`, on a body of `radius`, as a launch site in its own right: its middle and
/// the way its front faces, and its apron as a pad.
fn as_site(def: &SpaceCentreDef, building: &BuildingDef, radius: f64) -> LaunchSiteDef {
    let middle = terrain::site_point(&layout(def), radius, building.at[0], building.at[1]);
    LaunchSiteDef {
        id: format!("{}.{}", def.id, building.id),
        name: building.display_name().to_owned(),
        body: def.body.clone(),
        latitude: middle.z.asin().to_degrees(),
        longitude: middle.y.atan2(middle.x).to_degrees(),
        heading: def.heading + building.turn,
        pad: Some(PadDef {
            size: [
                building.size[0] + 2.0 * APRON,
                building.size[1] + 2.0 * APRON,
            ],
            corner_radius: APRON_CORNER,
            elevation: None,
            slope: APRON_SLOPE,
            mount_height: 0.0,
        }),
        roads: Vec::new(),
        runway: None,
    }
}

/// The earthworks of `def` on a body of `radius`, as launch sites for the terrain to lay out: its
/// roads, then a level apron under each building. Nothing is launched from them: they go to
/// [`terrain::install`] only.
pub fn grounds(def: &SpaceCentreDef, radius: f64) -> Vec<LaunchSiteDef> {
    let mut out = vec![layout(def)];
    out.extend((def.buildings.iter()).map(|building| as_site(def, building, radius)));
    out
}

/// A building as it stands, body-fixed: the middle of its floor, its axes (right, forward to its
/// front, up), and half its size along each, m.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Standing {
    pub base: DVec3,
    pub right: DVec3,
    pub forward: DVec3,
    pub up: DVec3,
    pub half: DVec3,
}

impl Standing {
    /// The middle of the box round it.
    pub fn middle(&self) -> DVec3 {
        self.base + self.up * self.half.z
    }

    /// The middle of its roof.
    pub fn top(&self) -> DVec3 {
        self.base + self.up * (2.0 * self.half.z)
    }

    /// `at`, body-fixed, in its own axes from the middle of its box.
    fn local(&self, at: DVec3) -> DVec3 {
        DVec3::new(at.dot(self.right), at.dot(self.forward), at.dot(self.up))
    }

    /// How far along the ray from `from` going `way` (both body-fixed, `way` of unit length) it
    /// first meets the box round the building, if it does ahead of `from`.
    pub fn hit(&self, from: DVec3, way: DVec3) -> Option<f64> {
        let from = self.local(from - self.middle());
        let way = self.local(way);
        let (mut near, mut far) = (0.0_f64, f64::INFINITY);
        for axis in 0..3 {
            let (o, d, h) = (from[axis], way[axis], self.half[axis]);
            if d.abs() < 1e-12 {
                if o.abs() > h {
                    return None;
                }
                continue;
            }
            let (a, b) = ((-h - o) / d, (h - o) / d);
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if near > far {
                return None;
            }
        }
        Some(near)
    }
}

/// Where each building of `def` stands on `body`, in the order of `def.buildings`, each at its level
/// in `levels` (none past its end: the first): on its apron, which has to be in the body's terrain
/// ([`grounds`]) for it to stand on it.
pub fn standing(def: &SpaceCentreDef, body: &Body, levels: &[usize]) -> Vec<Standing> {
    (def.buildings.iter().enumerate())
        .map(|(index, building)| {
            let size = building.size_at(levels.get(index).copied().unwrap_or(0));
            let site = as_site(def, building, body.radius);
            let (center, right, forward) = terrain::site_axes(&site);
            let floor = body.radius + body.terrain.height(center);
            Standing {
                base: center * floor,
                right,
                forward,
                up: center,
                half: DVec3::new(size[1], size[0], size[2]) / 2.0,
            }
        })
        .collect()
}

/// The middle of `def`'s layout on `body`, on its ground, body-fixed.
pub fn middle(def: &SpaceCentreDef, body: &Body) -> DVec3 {
    let (center, ..) = terrain::site_axes(&layout(def));
    center * (body.radius + body.terrain.height(center))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_meets_the_box_it_goes_through_and_misses_one_beside_it() {
        let standing = Standing {
            base: DVec3::ZERO,
            right: DVec3::X,
            forward: DVec3::Y,
            up: DVec3::Z,
            half: DVec3::new(10.0, 20.0, 5.0),
        };
        let from = DVec3::new(-100.0, 0.0, 5.0);
        assert_eq!(standing.hit(from, DVec3::X), Some(90.0));
        assert_eq!(standing.hit(from, -DVec3::X), None);
        assert_eq!(standing.hit(from + DVec3::Y * 30.0, DVec3::X), None);
    }
}
