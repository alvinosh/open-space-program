//! The shape of a body's surface: an optional height source (baked elevation data) with
//! launch-site earthworks (pads and roads) laid over it.
//!
//! This is the single definition of the ground used by both collision and rendering. The
//! sea surface is the floor: oceans are flat at the mean radius.

use std::{fmt, sync::Arc};

use glam::DVec3;

use crate::{body::Body, data::LaunchSiteDef, system::SolarSystem};

/// Elevation data for a body, e.g. a baked dataset.
pub trait HeightSource: Send + Sync {
    /// Height above the mean radius, m, in a body-fixed unit direction (negative below sea
    /// level). `max_level` limits the detail to that cube-sphere quadtree level, so coarse
    /// meshes can sample filtered data; [`FINEST`] asks for the best available.
    fn height(&self, dir: DVec3, max_level: u8) -> f64;
}

/// Ask a [`HeightSource`] for its most detailed data.
pub const FINEST: u8 = u8::MAX;

/// What a surface point is paved with, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paving {
    Concrete,
    Asphalt,
}

#[derive(Clone, Default)]
pub struct Terrain {
    source: Option<Arc<dyn HeightSource>>,
    features: Vec<Feature>,
}

impl fmt::Debug for Terrain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Terrain")
            .field("has_source", &self.source.is_some())
            .field("features", &self.features)
            .finish()
    }
}

/// A launch site's horizontal frame: `forward` follows the site heading, `right` is 90°
/// clockwise from it, both tangent to the surface at `center`.
#[derive(Debug, Clone, Copy)]
struct SiteFrame {
    center: DVec3,
    right: DVec3,
    forward: DVec3,
    radius: f64,
    /// Cosine of the angular radius beyond which the site has no influence.
    reach_cos: f64,
}

/// A launch site's body-fixed axes: the up direction at its centre, then `right` and
/// `forward` (along its heading), tangent to the surface. Site layouts (pads, roads and
/// the structures drawn on them) are given in this frame.
pub fn site_axes(site: &LaunchSiteDef) -> (DVec3, DVec3, DVec3) {
    let center =
        Body::surface_normal_fixed(site.latitude.to_radians(), site.longitude.to_radians());
    let east = DVec3::Z.cross(center).try_normalize().unwrap_or(DVec3::Y);
    let north = center.cross(east);
    let heading = site.heading.to_radians();
    (
        center,
        east * heading.cos() - north * heading.sin(),
        north * heading.cos() + east * heading.sin(),
    )
}

/// Body-fixed direction of a point `right`, `forward` metres from a site's centre.
pub fn site_point(site: &LaunchSiteDef, radius: f64, right: f64, forward: f64) -> DVec3 {
    let (center, r, f) = site_axes(site);
    (center + (r * right + f * forward) / radius).normalize()
}

impl SiteFrame {
    fn new(site: &LaunchSiteDef, radius: f64, reach: f64) -> Self {
        let (center, right, forward) = site_axes(site);
        Self {
            center,
            forward,
            right,
            radius,
            reach_cos: (reach / radius).min(std::f64::consts::PI).cos(),
        }
    }

    /// Metres (right, forward) in the tangent plane (gnomonic projection), or `None` when
    /// out of reach.
    fn local(&self, dir: DVec3) -> Option<(f64, f64)> {
        let d = dir.dot(self.center);
        if d < self.reach_cos {
            return None;
        }
        let p = dir / d - self.center;
        Some((
            p.dot(self.right) * self.radius,
            p.dot(self.forward) * self.radius,
        ))
    }

    fn direction(&self, x: f64, y: f64) -> DVec3 {
        (self.center + (self.right * x + self.forward * y) / self.radius).normalize()
    }
}

#[derive(Debug, Clone)]
enum Feature {
    /// A level deck (rounded rectangle) on an embankment.
    Pad {
        frame: SiteFrame,
        half: (f64, f64),
        corner: f64,
        elevation: f64,
        /// Horizontal width of the embankment.
        skirt: f64,
    },
    /// A level-across, paved strip along a polyline (site frame, metres).
    Road {
        frame: SiteFrame,
        points: Vec<(f64, f64)>,
        half_width: f64,
    },
}

/// Embankment of a runway, rise over run: graded gently, so a vessel running off the end
/// rolls onto it.
const RUNWAY_SLOPE: f64 = 0.05;
/// Ground kept clear of vegetation around a pad's deck and beside a road, m.
const PAD_APRON: f64 = 60.0;
const ROAD_VERGE: f64 = 4.0;
/// Width of a road's graded shoulder, m.
const SHOULDER: f64 = 4.0;
/// Width over which paving fades out at its edge, m.
const PAVING_EDGE: f64 = 0.4;

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Signed distance from a rounded rectangle centred on the origin (negative inside).
fn rounded_rect_distance(x: f64, y: f64, half: (f64, f64), corner: f64) -> f64 {
    let qx = x.abs() - (half.0 - corner);
    let qy = y.abs() - (half.1 - corner);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - corner
}

/// Distance to a polyline and the closest point on it.
fn polyline_distance(points: &[(f64, f64)], x: f64, y: f64) -> (f64, (f64, f64)) {
    let mut best = (f64::INFINITY, points[0]);
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 {
            (((x - a.0) * dx + (y - a.1) * dy) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let p = (a.0 + dx * t, a.1 + dy * t);
        let d = ((x - p.0).powi(2) + (y - p.1).powi(2)).sqrt();
        if d < best.0 {
            best = (d, p);
        }
    }
    best
}

impl Terrain {
    /// Terrain from an optional height source plus the earthworks of `sites` (which must
    /// all be on this body).
    pub fn new(
        radius: f64,
        source: Option<Arc<dyn HeightSource>>,
        sites: &[&LaunchSiteDef],
    ) -> Self {
        let mut terrain = Self {
            source,
            features: Vec::new(),
        };
        // Roads first, so pads are laid over them.
        for site in sites {
            for road in &site.roads {
                if road.points.len() < 2 {
                    continue;
                }
                let points: Vec<(f64, f64)> = road.points.iter().map(|p| (p[0], p[1])).collect();
                let reach = points.iter().map(|p| p.0.hypot(p.1)).fold(0.0, f64::max)
                    + road.width
                    + SHOULDER
                    + ROAD_VERGE;
                terrain.features.push(Feature::Road {
                    frame: SiteFrame::new(site, radius, reach),
                    points,
                    half_width: road.width / 2.0,
                });
            }
        }
        // Pads and runways: level decks, `size` [along the heading, across], on embankments.
        for site in sites {
            let center_dir =
                Body::surface_normal_fixed(site.latitude.to_radians(), site.longitude.to_radians());
            let ground = terrain.height_lod(center_dir, FINEST);
            let decks = site
                .pad
                .iter()
                .map(|p| (p.size, p.corner_radius, p.elevation, p.slope))
                .chain(
                    site.runway
                        .iter()
                        .map(|r| (r.size, 0.0, r.elevation, RUNWAY_SLOPE)),
                );
            for (size, corner, elevation, slope) in decks.collect::<Vec<_>>() {
                let elevation = elevation.unwrap_or(ground + 1.0);
                let skirt = ((elevation - ground).abs() / slope.max(0.05)).max(3.0);
                // Reach covers the apron too, for `is_cleared`. `half` is (right, forward).
                let half = (size[1] / 2.0, size[0] / 2.0);
                let corner = corner.clamp(0.0, half.0.min(half.1));
                terrain.features.push(Feature::Pad {
                    frame: SiteFrame::new(
                        site,
                        radius,
                        half.0.hypot(half.1) + skirt + PAD_APRON + 10.0,
                    ),
                    half,
                    corner,
                    elevation,
                    skirt,
                });
            }
        }
        terrain
    }

    pub fn source(&self) -> Option<&Arc<dyn HeightSource>> {
        self.source.as_ref()
    }

    /// Ground or sea-surface height above the mean radius in a body-fixed unit direction,
    /// using elevation data no deeper than `max_level` (see [`HeightSource::height`]).
    pub fn height_lod(&self, dir: DVec3, max_level: u8) -> f64 {
        let raw = self
            .source
            .as_ref()
            .map_or(0.0, |s| s.height(dir, max_level));
        self.height_over(dir, raw, max_level)
    }

    /// Like [`Self::height_lod`], for a caller that already has the source's height at
    /// `dir` (e.g. a renderer reading a whole tile at once).
    pub fn height_over(&self, dir: DVec3, raw: f64, max_level: u8) -> f64 {
        let natural = |dir: DVec3| {
            self.source
                .as_ref()
                .map_or(0.0, |s| s.height(dir, max_level))
                .max(0.0)
        };
        let mut height = raw.max(0.0);
        for feature in &self.features {
            match feature {
                Feature::Pad {
                    frame,
                    half,
                    corner,
                    elevation,
                    skirt,
                } => {
                    let Some((x, y)) = frame.local(dir) else {
                        continue;
                    };
                    let d = rounded_rect_distance(x, y, *half, *corner);
                    let w = 1.0 - smoothstep(d / skirt);
                    height += (elevation - height) * w;
                }
                Feature::Road {
                    frame,
                    points,
                    half_width,
                } => {
                    let Some((x, y)) = frame.local(dir) else {
                        continue;
                    };
                    let (d, nearest) = polyline_distance(points, x, y);
                    if d > half_width + SHOULDER {
                        continue;
                    }
                    let road = natural(frame.direction(nearest.0, nearest.1)).max(0.5);
                    let w = 1.0 - smoothstep((d - half_width) / SHOULDER);
                    height += (road - height) * w;
                }
            }
        }
        height
    }

    /// Ground height from the most detailed data.
    pub fn height(&self, dir: DVec3) -> f64 {
        self.height_lod(dir, FINEST)
    }

    /// Paving at a body-fixed direction and how fully it covers the point (0..1); the
    /// last feature laid wins.
    pub fn paving(&self, dir: DVec3) -> Option<(Paving, f64)> {
        let mut result = None;
        for feature in &self.features {
            let (kind, d) = match feature {
                Feature::Pad {
                    frame,
                    half,
                    corner,
                    ..
                } => {
                    let Some((x, y)) = frame.local(dir) else {
                        continue;
                    };
                    (
                        Paving::Concrete,
                        rounded_rect_distance(x, y, *half, *corner),
                    )
                }
                Feature::Road {
                    frame,
                    points,
                    half_width,
                } => {
                    let Some((x, y)) = frame.local(dir) else {
                        continue;
                    };
                    (
                        Paving::Asphalt,
                        polyline_distance(points, x, y).0 - half_width,
                    )
                }
            };
            let cover = 1.0 - smoothstep(d / PAVING_EDGE + 0.5);
            if cover > 0.0 {
                result = Some((kind, cover));
            }
        }
        result
    }

    /// True inside the cleared ground around a site's pad and along its roads, where
    /// nothing grows.
    pub fn is_cleared(&self, dir: DVec3) -> bool {
        self.features.iter().any(|feature| match feature {
            Feature::Pad {
                frame,
                half,
                corner,
                skirt,
                ..
            } => frame.local(dir).is_some_and(|(x, y)| {
                rounded_rect_distance(x, y, *half, *corner) < skirt + PAD_APRON
            }),
            Feature::Road {
                frame,
                points,
                half_width,
            } => frame.local(dir).is_some_and(|(x, y)| {
                polyline_distance(points, x, y).0 < half_width + SHOULDER + ROAD_VERGE
            }),
        })
    }

    /// Highest point the earthworks raise the ground to, m.
    pub fn max_feature_height(&self) -> f64 {
        self.features
            .iter()
            .map(|f| match f {
                Feature::Pad { elevation, .. } => *elevation,
                Feature::Road { .. } => 0.0,
            })
            .fold(0.0, f64::max)
    }
}

/// Give every body its terrain: a height source where one is provided, and the earthworks
/// of the launch sites on it. Returns the ids of sites on unknown bodies.
pub fn install(
    system: &mut SolarSystem,
    sources: Vec<(crate::body::BodyId, Arc<dyn HeightSource>)>,
    sites: &[LaunchSiteDef],
) -> Vec<String> {
    let unknown: Vec<String> = sites
        .iter()
        .filter(|s| system.find(&s.body).is_none())
        .map(|s| s.id.clone())
        .collect();
    let ids: Vec<_> = system.bodies().iter().map(|b| b.id).collect();
    for id in ids {
        let body = system.body(id);
        let on_body: Vec<&LaunchSiteDef> = sites
            .iter()
            .filter(|s| system.find(&s.body) == Some(id))
            .collect();
        let source = sources
            .iter()
            .find(|(b, _)| *b == id)
            .map(|(_, s)| s.clone());
        if source.is_none() && on_body.is_empty() {
            continue;
        }
        let terrain = Terrain::new(body.radius, source, &on_body);
        system.set_terrain(id, terrain);
    }
    unknown
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{PadDef, RoadDef, RunwayDef};

    struct Slope;
    impl HeightSource for Slope {
        // Rises 10 m per kilometre northwards from 2 m at 28.6°N; ocean south of 28.5°N.
        fn height(&self, dir: DVec3, _: u8) -> f64 {
            let lat = dir.z.asin().to_degrees();
            if lat < 28.5 {
                -20.0
            } else {
                2.0 + (lat - 28.6) * 111.0 * 10.0
            }
        }
    }

    fn site() -> LaunchSiteDef {
        LaunchSiteDef {
            id: "test".into(),
            name: String::new(),
            body: "Earth".into(),
            latitude: 28.6,
            longitude: -80.6,
            heading: 30.0,
            pad: Some(PadDef {
                size: [60.0, 40.0],
                corner_radius: 5.0,
                elevation: Some(8.0),
                slope: 0.5,
                mount_height: 0.0,
            }),
            roads: vec![RoadDef {
                points: vec![[0.0, -20.0], [0.0, -500.0]],
                width: 8.0,
            }],
            runway: None,
        }
    }

    const R: f64 = 6_371_000.0;

    fn at(frame_site: &LaunchSiteDef, x: f64, y: f64) -> DVec3 {
        SiteFrame::new(frame_site, R, 1e4).direction(x, y)
    }

    #[test]
    fn pad_is_level_and_blends_into_the_ground() {
        let s = site();
        let terrain = Terrain::new(R, Some(Arc::new(Slope)), &[&s]);
        // Level deck, including near its corners (heading-rotated frame).
        // 60 m along the heading (forward), 40 m across (right).
        for (x, y) in [(0.0, 0.0), (18.0, 28.0), (-18.0, -28.0), (-19.0, 10.0)] {
            assert!((terrain.height(at(&s, x, y)) - 8.0).abs() < 1e-9, "{x} {y}");
        }
        // Natural ground well away from it.
        let far = at(&s, 400.0, 0.0);
        let natural = Slope.height(far, FINEST);
        assert!((terrain.height(far) - natural).abs() < 1e-9);
        // Embankment in between, monotonic.
        let mut previous = 8.0;
        for k in 0..20 {
            let h = terrain.height(at(&s, 20.0 + k as f64, 0.0));
            assert!(h <= previous + 1e-9);
            previous = h;
        }
        assert_eq!(
            terrain.paving(at(&s, 0.0, 0.0)),
            Some((Paving::Concrete, 1.0))
        );
        assert_eq!(terrain.paving(at(&s, 60.0, 0.0)), None);
        assert!(terrain.max_feature_height() == 8.0);
        assert!(terrain.is_cleared(at(&s, 0.0, 0.0)) && terrain.is_cleared(at(&s, 60.0, 0.0)));
        assert!(!terrain.is_cleared(at(&s, 300.0, 300.0)));
    }

    #[test]
    fn runway_is_level_concrete_along_its_length() {
        let mut s = site();
        (s.pad, s.roads) = (None, Vec::new());
        s.runway = Some(RunwayDef {
            size: [3000.0, 60.0],
            elevation: Some(5.0),
        });
        let terrain = Terrain::new(R, Some(Arc::new(Slope)), &[&s]);
        for (x, y) in [(0.0, 0.0), (25.0, 1450.0), (-25.0, -1450.0)] {
            assert!((terrain.height(at(&s, x, y)) - 5.0).abs() < 1e-9, "{x} {y}");
            assert_eq!(terrain.paving(at(&s, x, y)), Some((Paving::Concrete, 1.0)));
        }
        assert_eq!(terrain.paving(at(&s, 60.0, 0.0)), None);
    }

    #[test]
    fn road_is_level_across_and_above_the_sea() {
        let s = site();
        let terrain = Terrain::new(R, Some(Arc::new(Slope)), &[&s]);
        let left = terrain.height(at(&s, -3.0, -300.0));
        let right = terrain.height(at(&s, 3.0, -300.0));
        assert!((left - right).abs() < 1e-9);
        assert_eq!(
            terrain.paving(at(&s, 0.0, -300.0)).unwrap().0,
            Paving::Asphalt
        );
        // The sea is the floor everywhere.
        let sea = Body::surface_normal_fixed(20f64.to_radians(), 0.0);
        assert_eq!(terrain.height(sea), 0.0);
    }

    #[test]
    fn pad_defaults_to_just_above_the_ground() {
        let mut s = site();
        s.pad.as_mut().unwrap().elevation = None;
        s.roads.clear();
        let terrain = Terrain::new(R, Some(Arc::new(Slope)), &[&s]);
        let ground = Slope.height(at(&s, 0.0, 0.0), FINEST);
        assert!((terrain.height(at(&s, 0.0, 0.0)) - (ground + 1.0)).abs() < 1e-9);
    }
}
