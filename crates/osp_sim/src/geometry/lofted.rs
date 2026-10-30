//! Lofted subparts: one closed polygonal section swept up the subpart's +Z through
//! stations, each with its own offset, scale and twist. Between two stations these change
//! linearly with height, so the section at any height is the same polygon stretched,
//! turned and moved ([`Lofted::at`]). It stays square to the axis: a swept wing keeps its
//! airfoil along the flow.
//!
//! Lofts are measured exactly from that description: the volume slice by slice up the axis
//! (the section's own moments, transformed), the skin over each edge of the section and
//! height. The triangle mesh, with extra stations where the section twists, is for
//! rendering and for liquid settled off the axis.

use std::f64::consts::FRAC_PI_2;

use glam::{DMat2, DVec2, DVec3};

use super::{
    Fill, SurfacePoint,
    mesh::{TriMesh, is_simple, signed_area, triangulate},
    moments::{Moments, symmetric},
    quadrature::{fixed, integrate},
};

/// Most the mesh lets the section twist between neighbouring rings, rad.
const MESH_TWIST: f64 = 1.0 * std::f64::consts::PI / 180.0;

/// Where the section is at one height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Station {
    /// Height up the subpart's axis, m.
    pub z: f64,
    /// Where the section's origin is, m.
    pub offset: DVec2,
    /// How much the section is stretched along x and y.
    pub scale: DVec2,
    /// How far it is turned about its origin, from +X towards +Y, rad.
    pub twist: f64,
}

impl Station {
    /// The linear map of the section: stretch, then turn.
    fn map(&self) -> DMat2 {
        DMat2::from_angle(self.twist) * DMat2::from_diagonal(self.scale)
    }

    fn place(&self, p: DVec2) -> DVec3 {
        (self.map() * p + self.offset).extend(self.z)
    }

    fn lerp(&self, other: &Station, w: f64) -> Station {
        Station {
            z: self.z + (other.z - self.z) * w,
            offset: self.offset.lerp(other.offset, w),
            scale: self.scale.lerp(other.scale, w),
            twist: self.twist + (other.twist - self.twist) * w,
        }
    }
}

/// A polygon's own moments: its area, ∫x, ∫y, ∫x², ∫y² and ∫xy.
#[derive(Debug, Clone, Copy)]
struct Section {
    area: f64,
    first: DVec2,
    xx: f64,
    yy: f64,
    xy: f64,
}

impl Section {
    fn of(polygon: &[DVec2]) -> Self {
        let n = polygon.len();
        let mut s = Section {
            area: 0.0,
            first: DVec2::ZERO,
            xx: 0.0,
            yy: 0.0,
            xy: 0.0,
        };
        for i in 0..n {
            let (a, b) = (polygon[i], polygon[(i + 1) % n]);
            let cross = a.perp_dot(b);
            s.area += cross / 2.0;
            s.first += (a + b) * cross / 6.0;
            s.xx += (a.x * a.x + a.x * b.x + b.x * b.x) * cross / 12.0;
            s.yy += (a.y * a.y + a.y * b.y + b.y * b.y) * cross / 12.0;
            s.xy += (a.x * b.y + 2.0 * a.x * a.y + 2.0 * b.x * b.y + b.x * a.y) * cross / 24.0;
        }
        s
    }

    /// The moments, per metre of height, of the slice `station` places at its height.
    fn slice(&self, station: &Station) -> [f64; 10] {
        let (map, o, z) = (station.map(), station.offset, station.z);
        let det = station.scale.x * station.scale.y;
        let area = det * self.area;
        let mapped = map * self.first;
        let first = (mapped + o * self.area) * det;
        let own = DMat2::from_cols(DVec2::new(self.xx, self.xy), DVec2::new(self.xy, self.yy));
        let outer = |a: DVec2, b: DVec2| DMat2::from_cols(a * b.x, a * b.y);
        let second = (map * own * map.transpose()
            + outer(mapped, o)
            + outer(o, mapped)
            + outer(o, o) * self.area)
            * det;
        [
            area,
            first.x,
            first.y,
            z * area,
            second.x_axis.x,
            second.y_axis.y,
            z * z * area,
            second.x_axis.y,
            z * first.x,
            z * first.y,
        ]
    }
}

/// Moments from their ten integrals: measure, ∫x, ∫y, ∫z, ∫x², ∫y², ∫z², ∫xy, ∫xz, ∫yz.
fn moments([m, x, y, z, xx, yy, zz, xy, xz, yz]: [f64; 10]) -> Moments {
    Moments {
        measure: m,
        first: DVec3::new(x, y, z),
        second: symmetric(xx, yy, zz, xy, xz, yz),
    }
}

/// The ten integrands of a point `p` weighted by `w`.
fn weighed(p: DVec3, w: f64) -> [f64; 10] {
    [
        w,
        w * p.x,
        w * p.y,
        w * p.z,
        w * p.x * p.x,
        w * p.y * p.y,
        w * p.z * p.z,
        w * p.x * p.y,
        w * p.x * p.z,
        w * p.y * p.z,
    ]
}

#[derive(Debug, Clone)]
pub struct Lofted {
    pub name: String,
    pub fill: Fill,
    /// Counter-clockwise.
    section: Vec<DVec2>,
    stations: Vec<Station>,
    mesh: TriMesh,
    solid: Moments,
    skin: Moments,
    /// Distance around the section to each of its points, and back to the first.
    perimeter: Vec<f64>,
}

impl Lofted {
    pub fn new(
        name: String,
        section: Vec<DVec2>,
        stations: Vec<Station>,
        fill: Fill,
    ) -> Result<Self, String> {
        if section.len() < 3 || !section.iter().all(|p| p.is_finite()) {
            return Err(format!("`{name}`: a section needs at least three points"));
        }
        if !is_simple(&section) || signed_area(&section).abs() <= 1e-12 {
            return Err(format!(
                "`{name}`: its section must enclose an area without crossing itself"
            ));
        }
        if stations.len() < 2 || stations.windows(2).any(|w| w[1].z <= w[0].z) {
            return Err(format!(
                "`{name}`: it needs two or more stations, each higher than the last"
            ));
        }
        if stations.iter().any(|s| {
            !(s.scale.min_element() >= 0.0 && s.scale.is_finite())
                || !s.offset.is_finite()
                || !s.twist.is_finite()
        }) {
            return Err(format!("`{name}`: station scales cannot be negative"));
        }
        if stations.iter().all(|s| s.scale.min_element() == 0.0) {
            return Err(format!(
                "`{name}`: at least one station must have some size"
            ));
        }
        let mut section = section;
        if signed_area(&section) < 0.0 {
            section.reverse();
        }
        let mut perimeter = vec![0.0];
        for i in 0..section.len() {
            let edge = (section[(i + 1) % section.len()] - section[i]).length();
            perimeter.push(perimeter[i] + edge);
        }
        let mut loft = Self {
            name,
            fill,
            section,
            stations,
            mesh: TriMesh::default(),
            solid: Moments::ZERO,
            skin: Moments::ZERO,
            perimeter,
        };
        loft.solid = loft.measure_solid();
        loft.skin = loft.measure_skin();
        loft.mesh = loft.build_mesh();
        Ok(loft)
    }

    /// The section's place at height `z`, between the stations around it (clamped to the
    /// ends).
    pub fn at(&self, z: f64) -> Station {
        let s = self
            .stations
            .windows(2)
            .position(|w| z <= w[1].z)
            .unwrap_or(self.stations.len() - 2);
        let (low, high) = (&self.stations[s], &self.stations[s + 1]);
        let w = ((z - low.z) / (high.z - low.z)).clamp(0.0, 1.0);
        low.lerp(high, w)
    }

    /// How the section's place changes with height between stations `s` and `s + 1`: the
    /// rates of change of its offset, scale and twist.
    fn rates(&self, s: usize) -> (DVec2, DVec2, f64) {
        let (low, high) = (&self.stations[s], &self.stations[s + 1]);
        let h = high.z - low.z;
        (
            (high.offset - low.offset) / h,
            (high.scale - low.scale) / h,
            (high.twist - low.twist) / h,
        )
    }

    fn measure_solid(&self) -> Moments {
        let section = Section::of(&self.section);
        let mut sum = Moments::ZERO;
        for w in self.stations.windows(2) {
            let (low, high) = (w[0], w[1]);
            let slices = integrate(
                |z| section.slice(&low.lerp(&high, (z - low.z) / (high.z - low.z))),
                low.z,
                high.z,
            );
            sum = sum + moments(slices);
        }
        sum
    }

    fn measure_skin(&self) -> Moments {
        let n = self.section.len();
        let mut sum = Moments::ZERO;
        for s in 0..self.stations.len() - 1 {
            let (low, high) = (self.stations[s], self.stations[s + 1]);
            let (doffset, dscale, dtwist) = self.rates(s);
            for i in 0..n {
                let (p, e) = (self.section[i], self.section[(i + 1) % n] - self.section[i]);
                let side = integrate(
                    |z| {
                        let at = low.lerp(&high, (z - low.z) / (high.z - low.z));
                        let map = at.map();
                        let turn = DMat2::from_angle(at.twist + FRAC_PI_2) * dtwist;
                        let along_s = (map * e).extend(0.0);
                        // Along the edge the integrand is smooth (the root of a quadratic),
                        // and a fixed rule keeps the outer integrand smooth too.
                        fixed(
                            |t| {
                                let q = p + e * t;
                                let up = (turn * DMat2::from_diagonal(at.scale) * q
                                    + DMat2::from_angle(at.twist) * (dscale * q)
                                    + doffset)
                                    .extend(1.0);
                                weighed(at.place(q), along_s.cross(up).length())
                            },
                            0.0,
                            1.0,
                            2,
                        )
                    },
                    low.z,
                    high.z,
                );
                sum = sum + moments(side);
            }
        }
        // The flat ends.
        let ends = [self.stations[0], self.stations[self.stations.len() - 1]];
        for end in ends {
            for [a, b, c] in triangulate(&self.section) {
                let corner = |k: usize| end.place(self.section[k]);
                sum = sum + Moments::triangle(corner(a), corner(b), corner(c));
            }
        }
        sum
    }

    /// The mesh through the stations, with rings in between where the section twists.
    fn build_mesh(&self) -> TriMesh {
        let mut rings = vec![self.stations[0]];
        for w in self.stations.windows(2) {
            let steps = ((w[1].twist - w[0].twist).abs() / MESH_TWIST)
                .ceil()
                .max(1.0) as usize;
            rings.extend((1..=steps).map(|k| w[0].lerp(&w[1], k as f64 / steps as f64)));
        }
        let n = self.section.len();
        let positions = rings
            .iter()
            .flat_map(|s| self.section.iter().map(move |p| s.place(*p)))
            .collect();
        let at = |s: usize, i: usize| (s * n + i % n) as u32;
        let mut triangles = Vec::new();
        for s in 0..rings.len() - 1 {
            for i in 0..n {
                let (a, b, c, d) = (at(s, i), at(s, i + 1), at(s + 1, i + 1), at(s + 1, i));
                triangles.push([a, b, c]);
                triangles.push([a, c, d]);
            }
        }
        let last = rings.len() - 1;
        for [a, b, c] in triangulate(&self.section) {
            triangles.push([at(0, a), at(0, c), at(0, b)]);
            triangles.push([at(last, a), at(last, b), at(last, c)]);
        }
        let mut mesh = TriMesh {
            positions,
            triangles,
        };
        mesh.orient_outward();
        mesh
    }

    pub fn mesh(&self) -> &TriMesh {
        &self.mesh
    }

    pub fn stations(&self) -> &[Station] {
        &self.stations
    }

    /// The section, counter-clockwise, before any station places it.
    pub fn section(&self) -> &[DVec2] {
        &self.section
    }

    /// What it encloses, by volume.
    pub fn solid_moments(&self) -> Moments {
        self.solid
    }

    /// Its whole surface, ends included, by area.
    pub fn skin_moments(&self) -> Moments {
        self.skin
    }

    /// The point of its side `u` of the way up its stations by height and `v` of the way
    /// around its section by length.
    pub fn surface_point(&self, u: f64, v: f64) -> SurfacePoint {
        let (first, last) = (self.stations[0].z, self.stations[self.stations.len() - 1].z);
        let z = first + (last - first) * u.clamp(0.0, 1.0);
        let s = self
            .stations
            .windows(2)
            .position(|w| z <= w[1].z)
            .unwrap_or(self.stations.len() - 2);
        let at = self.at(z);
        let (doffset, dscale, dtwist) = self.rates(s);
        let n = self.section.len();
        let target = v.clamp(0.0, 1.0) * self.perimeter[n];
        let i = self
            .perimeter
            .windows(2)
            .position(|p| target <= p[1])
            .unwrap_or(n - 1);
        let f = (target - self.perimeter[i]) / (self.perimeter[i + 1] - self.perimeter[i]);
        let (p, e) = (self.section[i], self.section[(i + 1) % n] - self.section[i]);
        let q = p + e * f;
        let up = DMat2::from_angle(at.twist + FRAC_PI_2) * (at.scale * q) * dtwist
            + DMat2::from_angle(at.twist) * (dscale * q)
            + doffset;
        let along_u = up.extend(1.0) * (last - first);
        let along_v = (at.map() * e).extend(0.0) * (self.perimeter[n] / e.length());
        SurfacePoint {
            position: at.place(q),
            normal: along_v.cross(along_u).normalize_or_zero(),
            along_u,
            along_v,
        }
    }
}
