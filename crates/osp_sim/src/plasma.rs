//! What entering an atmosphere looks like, as far as the sim can say: how hot the air gets, what
//! that does to the look of the plasma, and which of the hull the air reaches.
//!
//! The heating is Sutton and Graves's formula for a blunt nose. The look follows Lindner's breakdown
//! of entry effects: fast in thin air the streaks are long, faint and blue to violet; slower in
//! thicker air they are shorter, hotter and orange; slower still they go white, which is the Mach
//! cone. The hull is a set of points scattered evenly over the vessel's own meshes, so that the
//! plasma doesn't depend on how the meshes happen to be divided, and a depth map of them looking
//! along the flow (the **airstream map**): only what the air reaches gives off plasma.

use glam::{DVec3, Vec3};

/// Texels along each side of the airstream map (`textureDimensions` in the shader).
pub const AIRSTREAM_SIZE: usize = 128;
/// In the airstream map where no hull is.
pub const NOTHING_THERE: f32 = 1.0e9;
/// Sutton-Graves constant for air, kg^0.5/m, with a nose radius in metres and heating in W/m².
const SUTTON_GRAVES: f64 = 1.74153e-4;
/// A surface more edge-on to the flow than this has no plane worth following across the map: the
/// cosine of the angle between its normal and the flow (`EDGE_ON` in the shader).
const EDGE_ON: f32 = 0.15;
/// How far upstream of a point its surface is taken to go across the map, and how far behind the
/// map a point may be and still be reached, in texels (`UPSTREAM_TEXELS` in the shader). The plane
/// a point is on doesn't really go on for ever: at a corner it would hide what is on the other side.
const UPSTREAM_TEXELS: f32 = 2.0;

/// How wide the band around the hull's outline that streaks come off is, as a share of its reach
/// (`RIM_BAND` in the shader).
const RIM_BAND: f32 = 0.14;
/// A surface facing the air more squarely than this gives no streak (`LEAST_LEAN` in the shader).
const LEAST_LEAN: f32 = 0.02;

/// Stagnation-point heating, W/m², of a nose of this radius in air of this density.
pub fn stagnation_heating(density: f64, speed: f64, nose_radius: f64) -> f64 {
    SUTTON_GRAVES * (density / nose_radius.max(0.05)).sqrt() * speed.powi(3)
}

/// How strong the show is, 0 to 1, at this heating: a dim glow from about 30 kW/m², and all there
/// is around 3 MW/m², which is about the peak coming back from the Moon. On a log scale, as the
/// light of a gas that hot climbs very steeply.
pub fn glow_level(heating: f64) -> f64 {
    let t = ((heating.max(1.0).log10() - 4.5) / 2.0).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How the plasma around a vessel looks at a moment of its flight.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EntryLook {
    /// How strong it all is, 0 to 1.
    pub strength: f64,
    /// How long the streaks are, m, before each one's own variation.
    pub length: f64,
    /// How far the streaks flare out from the hull by their ends, m.
    pub spread: f64,
    /// How thin the air is, 0 to 1: a long, faint, blue to violet stream at 1.
    pub thin: f64,
    /// How much of it is the white of a Mach cone rather than plasma, 0 to 1.
    pub white: f64,
}

/// The look at this air density (kg/m³), airspeed (m/s), hull radius (m) and rate of climb (m/s).
pub fn entry_look(density: f64, speed: f64, radius: f64, climbing: f64) -> EntryLook {
    let heat = glow_level(stagnation_heating(density, speed, radius));
    // A shock in thick air shows from about Mach 1.2. Less of it on the way up, where it would
    // hide the rocket for the whole of the climb through the worst of the air.
    let pressure = 0.5 * density * speed * speed;
    let mach = smoothstep(380.0, 800.0, speed)
        * smoothstep(1_500.0, 16_000.0, pressure)
        * if climbing > 0.0 { 0.3 } else { 0.6 };
    let strength = heat.max(mach);
    let white = if strength > 0.0 {
        (mach / (heat + mach)).clamp(0.0, 1.0) * (1.0 - heat).powi(2)
    } else {
        0.0
    };
    // From the air at about 85 km to that at about 48 km.
    let thin = (((1e-3 / density.max(1e-12)).log10()) / 2.0).clamp(0.0, 1.0);
    let fast = (speed / 7800.0).clamp(0.0, 1.4);
    let length = radius * (1.5 + 4.0 * fast) * (1.0 + 1.0 * thin) * (0.35 + 0.65 * strength);
    let spread = radius * (0.7 + 1.1 * fast.min(1.0)) * (0.5 + 0.5 * strength);
    EntryLook {
        strength,
        length,
        spread,
        thin,
        white,
    }
}

/// A triangle of the hull, with a normal at each corner.
#[derive(Debug, Clone, Copy)]
pub struct HullTriangle {
    pub positions: [Vec3; 3],
    pub normals: [Vec3; 3],
}

/// A point on the hull.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HullPoint {
    pub position: Vec3,
    pub normal: Vec3,
}

/// Points scattered evenly over a hull.
#[derive(Debug, Clone, Default)]
pub struct HullPoints {
    pub points: Vec<HullPoint>,
    /// The mean distance between neighbours, m.
    pub spacing: f32,
}

/// A number in 0..1 that goes with `n` and `k`.
fn scatter(n: u32, k: u32) -> f32 {
    let mut h = n.wrapping_mul(0x9E37_79B1) ^ k.wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 13;
    h = h.wrapping_mul(0x27D4_EB2F);
    h ^= h >> 16;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// About `most` points over these triangles, as many on each as its share of the area, the same
/// ones every time. A big hull gets them further apart; a small one up to `per_square_metre`.
pub fn scatter_points(
    triangles: &[HullTriangle],
    most: usize,
    per_square_metre: f32,
) -> HullPoints {
    let area = |t: &HullTriangle| {
        (t.positions[1] - t.positions[0])
            .cross(t.positions[2] - t.positions[0])
            .length()
            * 0.5
    };
    let total: f32 = triangles.iter().map(area).sum();
    if total <= 0.0 {
        return HullPoints::default();
    }
    let density = (most as f32 / total).min(per_square_metre);
    let mut points = Vec::with_capacity((total * density) as usize + 16);
    let mut owed = 0.0f32;
    for (index, triangle) in triangles.iter().enumerate() {
        owed += area(triangle) * density;
        let count = owed as u32;
        owed -= count as f32;
        for k in 0..count {
            // Uniform over the triangle.
            let (a, b) = (
                scatter(index as u32, 2 * k),
                scatter(index as u32, 2 * k + 1),
            );
            let root = a.sqrt();
            let (u, v, w) = (1.0 - root, root * (1.0 - b), root * b);
            let [p0, p1, p2] = triangle.positions;
            let [n0, n1, n2] = triangle.normals;
            points.push(HullPoint {
                position: p0 * u + p1 * v + p2 * w,
                normal: (n0 * u + n1 * v + n2 * w).normalize_or_zero(),
            });
        }
    }
    HullPoints {
        points,
        spacing: 1.0 / density.sqrt(),
    }
}

/// The hull as the air meets it: a depth map looking along the flow.
#[derive(Debug, Clone)]
pub struct Airstream {
    /// The way the air goes, the map's depth axis.
    pub flow: Vec3,
    /// The map's axes across the flow.
    pub across: [Vec3; 2],
    /// Where the map's middle is, at depth 0 and in the hull's axes.
    pub centre: Vec3,
    /// Half the map's width, m.
    pub half_width: f32,
    /// How far the hull reaches across the flow from the map's middle, m.
    pub reach: f32,
    /// How far the hull reaches, upstream and downstream, along the flow from `centre`, m.
    pub front: f32,
    pub back: f32,
    /// [`AIRSTREAM_SIZE`] squared, row by row: how far along the flow the first of the hull is, m,
    /// or [`NOTHING_THERE`].
    pub depth: Vec<f32>,
    /// The same way: how far inside the hull's outline as the air sees it, m. Streaks come off the
    /// outline, whichever way the surface there faces: a heat shield's rim faces the air like the
    /// rest of it.
    pub inside: Vec<f32>,
    /// How many of the hull's points streaks can come off: reached by the air, and on the outline
    /// or running along the flow. The shader keeps so many of them, whatever the hull's shape.
    pub on_edge: usize,
}

/// How far each texel is inside the outline, m, by a two-pass chamfer distance. The map's edge
/// counts as outside.
fn inside_outline(depth: &[f32], texel: f32) -> Vec<f32> {
    let n = AIRSTREAM_SIZE;
    let mut out: Vec<f32> = depth
        .iter()
        .map(|d| if *d >= NOTHING_THERE { 0.0 } else { f32::MAX })
        .collect();
    let (straight, diagonal) = (1.0, std::f32::consts::SQRT_2);
    let at = |out: &[f32], i: i32, j: i32| {
        if (0..n as i32).contains(&i) && (0..n as i32).contains(&j) {
            out[j as usize * n + i as usize]
        } else {
            0.0
        }
    };
    for j in 0..n as i32 {
        for i in 0..n as i32 {
            let best = (at(&out, i - 1, j) + straight)
                .min(at(&out, i, j - 1) + straight)
                .min(at(&out, i - 1, j - 1) + diagonal)
                .min(at(&out, i + 1, j - 1) + diagonal);
            let here = &mut out[j as usize * n + i as usize];
            *here = here.min(best);
        }
    }
    for j in (0..n as i32).rev() {
        for i in (0..n as i32).rev() {
            let best = (at(&out, i + 1, j) + straight)
                .min(at(&out, i, j + 1) + straight)
                .min(at(&out, i + 1, j + 1) + diagonal)
                .min(at(&out, i - 1, j + 1) + diagonal);
            let here = &mut out[j as usize * n + i as usize];
            *here = here.min(best);
        }
    }
    for value in &mut out {
        *value *= texel;
    }
    out
}

/// How depth changes across the map on the plane of a point with this normal, given its parts
/// across the map and along the flow; `None` if it is edge-on to the flow.
fn plane_slope(across_x: f32, across_y: f32, along: f32) -> Option<(f32, f32)> {
    (along.abs() >= EDGE_ON).then(|| (-across_x / along, -across_y / along))
}

/// Build the map, for air going along `flow`. Each point is a small disc of the surface it is on,
/// tilted as that is, so a slope doesn't hide itself from the air.
pub fn airstream(hull: &HullPoints, flow: Vec3) -> Airstream {
    let n = AIRSTREAM_SIZE;
    let flow = flow.normalize_or(Vec3::NEG_Y);
    let (x_axis, y_axis) = flow.any_orthonormal_pair();
    let (mut low_x, mut high_x) = (f32::MAX, f32::MIN);
    let (mut low_y, mut high_y) = (f32::MAX, f32::MIN);
    let (mut front, mut back) = (f32::MAX, f32::MIN);
    for point in &hull.points {
        let (x, y, z) = (
            point.position.dot(x_axis),
            point.position.dot(y_axis),
            point.position.dot(flow),
        );
        (low_x, high_x) = (low_x.min(x), high_x.max(x));
        (low_y, high_y) = (low_y.min(y), high_y.max(y));
        (front, back) = (front.min(z), back.max(z));
    }
    if hull.points.is_empty() {
        (low_x, high_x, low_y, high_y, front, back) = (-1.0, 1.0, -1.0, 1.0, 0.0, 0.0);
    }
    let (middle_x, middle_y) = ((low_x + high_x) * 0.5, (low_y + high_y) * 0.5);
    let centre = x_axis * middle_x + y_axis * middle_y;
    // A little room, so that what is at the very edge of the hull still has a texel of its own.
    let half_width = ((high_x - low_x).max(high_y - low_y) * 0.5).max(0.2) * 1.08;
    let texel = 2.0 * half_width / n as f32;
    let mut depth = vec![NOTHING_THERE; n * n];
    // Over the gaps between the points, whichever is the coarser.
    let reach = (hull.spacing * 0.8 / texel).ceil().clamp(1.0, 6.0) as i32;
    for point in &hull.points {
        let relative = point.position - centre;
        let (x, y, z) = (
            relative.dot(x_axis),
            relative.dot(y_axis),
            relative.dot(flow),
        );
        let cx = ((x + half_width) / texel - 0.5).round() as i32;
        let cy = ((y + half_width) / texel - 0.5).round() as i32;
        let slope = plane_slope(
            point.normal.dot(x_axis),
            point.normal.dot(y_axis),
            point.normal.dot(flow),
        );
        let Some((slope_x, slope_y)) = slope else {
            // Edge-on, it hides only what is right behind it.
            if (0..n as i32).contains(&cx) && (0..n as i32).contains(&cy) {
                let map = &mut depth[cy as usize * n + cx as usize];
                *map = map.min(z);
            }
            continue;
        };
        let upstream = -UPSTREAM_TEXELS * texel;
        for j in (cy - reach).max(0)..=(cy + reach).min(n as i32 - 1) {
            for i in (cx - reach).max(0)..=(cx + reach).min(n as i32 - 1) {
                let tx = (i as f32 + 0.5) * texel - half_width - x;
                let ty = (j as f32 + 0.5) * texel - half_width - y;
                if tx * tx + ty * ty > (reach as f32 + 0.5).powi(2) * texel * texel {
                    continue;
                }
                let there = z + (tx * slope_x + ty * slope_y).max(upstream);
                let map = &mut depth[j as usize * n + i as usize];
                *map = map.min(there);
            }
        }
    }
    let reach = ((high_x - low_x).max(high_y - low_y) * 0.5).max(0.2);
    let mut map = Airstream {
        flow,
        across: [x_axis, y_axis],
        centre,
        half_width,
        reach,
        front: front - centre.dot(flow),
        back: back - centre.dot(flow),
        inside: inside_outline(&depth, texel),
        depth,
        on_edge: 0,
    };
    map.on_edge = hull.points.iter().filter(|p| map.gives_streak(p)).count();
    map
}

impl Airstream {
    /// Whether a streak can come off this point of the hull: as the shader has it, `lean` above its
    /// least in the trail.
    fn gives_streak(&self, point: &HullPoint) -> bool {
        let windward = -point.normal.dot(self.flow);
        if windward < -0.15 || !self.reaches(point) {
            return false;
        }
        let edge = 1.0 - windward.clamp(0.0, 1.0);
        let relative = point.position - self.centre;
        let texel = 2.0 * self.half_width / AIRSTREAM_SIZE as f32;
        let cell = |x: f32| {
            (((x + self.half_width) / texel) as i32).clamp(0, AIRSTREAM_SIZE as i32 - 1) as usize
        };
        let inside = self.inside[cell(relative.dot(self.across[1])) * AIRSTREAM_SIZE
            + cell(relative.dot(self.across[0]))];
        edge * edge * edge >= LEAST_LEAN || inside < self.reach * RIM_BAND + texel
    }

    /// Whether the air reaches this point of the hull (`exposure` in the shader, without its soft
    /// edge).
    pub fn reaches(&self, point: &HullPoint) -> bool {
        let n = AIRSTREAM_SIZE as i32;
        let relative = point.position - self.centre;
        let texel = 2.0 * self.half_width / n as f32;
        let (x, y, z) = (
            relative.dot(self.across[0]),
            relative.dot(self.across[1]),
            relative.dot(self.flow),
        );
        let i = (((x + self.half_width) / texel) as i32).clamp(0, n - 1);
        let j = (((y + self.half_width) / texel) as i32).clamp(0, n - 1);
        // The point's own surface at the middle of the texel, as the map has it.
        let slope = plane_slope(
            point.normal.dot(self.across[0]),
            point.normal.dot(self.across[1]),
            point.normal.dot(self.flow),
        );
        let tx = (i as f32 + 0.5) * texel - self.half_width - x;
        let ty = (j as f32 + 0.5) * texel - self.half_width - y;
        let most = UPSTREAM_TEXELS * texel;
        let along_plane = slope.map_or(0.0, |(sx, sy)| (tx * sx + ty * sy).clamp(-most, most));
        z + along_plane <= self.depth[(j * n + i) as usize] + most
    }
}

/// The hull's size across a flow: what Sutton and Graves's nose radius comes to.
pub fn nose_radius(reference_area: f64) -> f64 {
    (reference_area / std::f64::consts::PI).sqrt().max(0.3)
}

/// Which way the air goes over a vessel, in its own axes, given its airflow and attitude: the
/// direction the wake runs.
pub fn flow_in_vessel(attitude: glam::DQuat, airflow: DVec3) -> DVec3 {
    attitude.inverse() * (-airflow).normalize_or(-DVec3::Z)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed cylinder along Z, as a fan of triangles: `radius` wide and `length` long, its
    /// middle at the origin.
    fn cylinder(radius: f32, length: f32) -> Vec<HullTriangle> {
        let mut out = Vec::new();
        let n = 48;
        let at = |k: i32| {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            Vec3::new(a.cos(), a.sin(), 0.0)
        };
        let (top, bottom) = (Vec3::Z * length * 0.5, Vec3::NEG_Z * length * 0.5);
        for k in 0..n {
            let (a, b) = (at(k), at(k + 1));
            let (pa, pb) = (a * radius, b * radius);
            // Side.
            for positions in [
                [pa + bottom, pb + bottom, pb + top],
                [pa + bottom, pb + top, pa + top],
            ] {
                let normals = positions.map(|p| (p * Vec3::new(1.0, 1.0, 0.0)).normalize());
                out.push(HullTriangle { positions, normals });
            }
            // Ends.
            out.push(HullTriangle {
                positions: [top, pa + top, pb + top],
                normals: [Vec3::Z; 3],
            });
            out.push(HullTriangle {
                positions: [bottom, pb + bottom, pa + bottom],
                normals: [Vec3::NEG_Z; 3],
            });
        }
        out
    }

    #[test]
    fn apollo_comes_in_hot_and_a_low_orbit_decay_gently() {
        // Peak heating of a lunar return, at about 60 km and 10 km/s, on a 4.7 m nose: about 3 to 5
        // MW/m² in the Apollo records.
        let q = stagnation_heating(3e-4, 10_000.0, 4.7);
        assert!(q > 1e6 && q < 6e6, "{q}");
        assert!(glow_level(q) > 0.8, "{}", glow_level(q));
        // A plane at Mach 3 in thin air: nothing to see.
        assert_eq!(glow_level(stagnation_heating(0.02, 900.0, 1.0)), 0.0);
        // Faint high up, on the way in from orbit.
        let high = glow_level(stagnation_heating(1e-6, 7800.0, 2.0));
        assert!(high < 0.2, "{high}");
    }

    #[test]
    fn thin_fast_air_makes_long_faint_streaks_and_thick_slow_air_short_hot_ones() {
        // High up on the way in from orbit, at about 85 km.
        let high = entry_look(1e-5, 7800.0, 2.0, -200.0);
        // At the worst of it, at about 55 km.
        let peak = entry_look(5e-4, 6500.0, 2.0, -200.0);
        // Slowing, at about 40 km.
        let late = entry_look(4e-3, 2500.0, 2.0, -200.0);
        assert!(high.thin > 0.9 && peak.thin < 0.4, "{high:?} {peak:?}");
        assert!(peak.strength > high.strength, "{high:?} {peak:?}");
        assert!(late.strength < peak.strength, "{late:?} {peak:?}");
        // Longest of all relative to how strong it is when the air is thinnest.
        assert!(
            high.length / (0.35 + 0.65 * high.strength)
                > peak.length / (0.35 + 0.65 * peak.strength),
            "{high:?} {peak:?}"
        );
        assert!(late.length < peak.length, "{late:?} {peak:?}");
    }

    #[test]
    fn a_shock_in_thick_air_is_white_and_nothing_shows_when_slow() {
        // Falling at Mach 2 at about 12 km: not hot enough to glow, but a shock shows.
        let cone = entry_look(0.3, 600.0, 2.0, -100.0);
        assert!(cone.strength > 0.1, "{cone:?}");
        assert!(cone.white > 0.9, "{cone:?}");
        // Less of it climbing.
        let climbing = entry_look(0.3, 600.0, 2.0, 100.0);
        assert!(climbing.strength < cone.strength, "{climbing:?} {cone:?}");
        // A plane on approach.
        assert_eq!(entry_look(1.2, 80.0, 2.0, -5.0).strength, 0.0);
    }

    #[test]
    fn points_are_as_dense_everywhere_whatever_the_triangles() {
        // Two squares of the same size, one in 2 triangles and the other in 800.
        let square = |x: f32, cuts: u32| {
            let mut out = Vec::new();
            for j in 0..cuts {
                for i in 0..cuts {
                    let (size, a, b) = (1.0 / cuts as f32, i as f32, j as f32);
                    let c = |u: f32, v: f32| Vec3::new(x + (a + u) * size, (b + v) * size, 0.0);
                    for positions in [
                        [c(0.0, 0.0), c(1.0, 0.0), c(1.0, 1.0)],
                        [c(0.0, 0.0), c(1.0, 1.0), c(0.0, 1.0)],
                    ] {
                        out.push(HullTriangle {
                            positions,
                            normals: [Vec3::Z; 3],
                        });
                    }
                }
            }
            out
        };
        let mut triangles = square(0.0, 1);
        triangles.extend(square(2.0, 20));
        let hull = scatter_points(&triangles, 2000, 1e6);
        let coarse = hull.points.iter().filter(|p| p.position.x < 1.5).count() as f32;
        let fine = hull.points.iter().filter(|p| p.position.x > 1.5).count() as f32;
        assert!((coarse / fine - 1.0).abs() < 0.1, "{coarse} {fine}");
        assert!(
            (hull.points.len() as i32 - 2000).abs() < 3,
            "{}",
            hull.points.len()
        );
        // Always the same ones.
        assert_eq!(scatter_points(&triangles, 2000, 1e6).points, hull.points);
        // A small hull isn't crowded to reach the number.
        assert!(scatter_points(&triangles, 2000, 100.0).points.len() < 201);
    }

    #[test]
    fn the_air_reaches_only_the_end_facing_it() {
        let hull = scatter_points(&cylinder(2.0, 10.0), 20000, 1e6);
        // Coming down on the top end.
        let map = airstream(&hull, Vec3::NEG_Z);
        assert!((map.reach - 2.0).abs() < 0.2, "{}", map.reach);
        assert!(
            (map.back - map.front - 10.0).abs() < 0.1,
            "{map:?}",
            map = map.front
        );
        let share = |test: fn(&HullPoint) -> bool| {
            let chosen: Vec<_> = hull.points.iter().filter(|p| test(p)).collect();
            assert!(!chosen.is_empty());
            chosen.iter().filter(|p| map.reaches(p)).count() as f32 / chosen.len() as f32
        };
        assert!(share(|p| p.normal.z > 0.9) > 0.99, "the top is hit");
        assert!(
            share(|p| p.normal.z < -0.9) < 0.01,
            "the bottom is behind it"
        );
        // The side is out of the air, but for a rim at the top.
        assert!(share(|p| p.normal.z.abs() < 0.1 && p.position.z < 3.0) < 0.01);
        assert!(share(|p| p.normal.z.abs() < 0.1 && p.position.z > 4.97) > 0.9);
    }

    #[test]
    fn streaks_come_off_the_rim_of_a_disc_and_the_side_of_a_slanted_cylinder() {
        let hull = scatter_points(&cylinder(2.0, 10.0), 20000, 1e6);
        let flying_end_on = airstream(&hull, Vec3::NEG_Z);
        // The rim of the top end, a small part of the hull.
        let share = flying_end_on.on_edge as f32 / hull.points.len() as f32;
        assert!(share > 0.002 && share < 0.1, "{share}");
        // At an angle, all that the air reaches of its side counts too.
        let slanted = airstream(&hull, Vec3::new(0.5, 0.0, -1.0));
        assert!(slanted.on_edge > 2 * flying_end_on.on_edge);
    }

    #[test]
    fn the_outline_is_measured_from_the_edge_of_what_the_air_sees() {
        let hull = scatter_points(&cylinder(2.0, 10.0), 20000, 1e6);
        let map = airstream(&hull, Vec3::NEG_Z);
        let n = AIRSTREAM_SIZE;
        let texel = 2.0 * map.half_width / n as f32;
        let at = |x: f32, y: f32| {
            let i = ((x + map.half_width) / texel) as usize;
            let j = ((y + map.half_width) / texel) as usize;
            map.inside[j * n + i]
        };
        // A disc 2 m across, seen end on: the map's axes don't matter.
        assert!((at(0.0, 0.0) - 2.0).abs() < 0.2, "{}", at(0.0, 0.0));
        assert!((at(1.0, 0.0) - 1.0).abs() < 0.2, "{}", at(1.0, 0.0));
        assert!((at(0.0, -1.5) - 0.5).abs() < 0.2, "{}", at(0.0, -1.5));
        assert_eq!(at(2.12, 0.0), 0.0);
    }

    #[test]
    fn the_air_reaches_the_near_side_of_a_vessel_flying_sideways() {
        let hull = scatter_points(&cylinder(2.0, 10.0), 20000, 1e6);
        let map = airstream(&hull, Vec3::NEG_X);
        let near = hull
            .points
            .iter()
            .filter(|p| p.normal.x > 0.5 && p.position.z.abs() < 4.0);
        let far = hull
            .points
            .iter()
            .filter(|p| p.normal.x < -0.5 && p.position.z.abs() < 4.0);
        let hits = |points: Vec<&HullPoint>| {
            points.iter().filter(|p| map.reaches(p)).count() as f32 / points.len() as f32
        };
        assert!(hits(near.collect()) > 0.99);
        assert!(hits(far.collect()) < 0.01);
    }

    #[test]
    fn a_slope_does_not_hide_itself_from_the_air() {
        // A cone with its point into the flow: every point of its side is hit.
        let (mut triangles, n) = (Vec::new(), 64);
        let tip = Vec3::new(0.0, 0.0, 8.0);
        let slope = Vec3::new(8.0, 0.0, 2.0).normalize();
        for k in 0..n {
            let at = |k: i32| k as f32 / n as f32 * std::f32::consts::TAU;
            let rim = |a: f32| Vec3::new(a.cos() * 2.0, a.sin() * 2.0, 0.0);
            let normal = |a: f32| Vec3::new(a.cos() * slope.x, a.sin() * slope.x, slope.z);
            let (a, b) = (at(k), at(k + 1));
            triangles.push(HullTriangle {
                positions: [rim(a), rim(b), tip],
                normals: [normal(a), normal(b), normal((a + b) * 0.5)],
            });
        }
        let hull = scatter_points(&triangles, 8000, 1e6);
        let map = airstream(&hull, Vec3::NEG_Z);
        let hit = hull.points.iter().filter(|p| map.reaches(p)).count();
        assert!(
            hit as f32 > hull.points.len() as f32 * 0.98,
            "{hit} of {}",
            hull.points.len()
        );
    }
}
