//! Curves in the (radius, height) half-plane: the outlines that [`super::Revolved`]
//! subparts turn about the axis. A curve is a chain of segments, each starting where the one
//! before it ended: straight lines, elliptical (and circular) arcs, nose-cone tapers along a
//! [`ShellProfile`] and cubic Béziers. Points are `DVec2`s, `x` the radius and `y` the
//! height.
//!
//! A curve faces the right of the way it runs: one running up the outside of a tank, or
//! counter-clockwise around a region, faces out.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

use glam::DVec2;

use super::quadrature::integrate;
use crate::data::ShellProfile;

/// Most a curve turns between neighbouring points of a tessellation, rad.
const MAX_TURN: f64 = 0.1;
/// Deepest a tessellation halves a stretch of curve.
const MAX_DEPTH: u32 = 20;

#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    Line {
        from: DVec2,
        to: DVec2,
    },
    /// An arc of the ellipse around `center` with semi-axes `radii` (radial, axial), from
    /// angle `from` to `to`, rad, measured from the radial direction towards +height.
    Ellipse {
        center: DVec2,
        radii: DVec2,
        from: f64,
        to: f64,
    },
    /// From a base `from` to a tip `to`, the radius changing with height as a nose cone of
    /// `profile` does: the curved profiles leave the base parallel to the axis.
    Taper {
        from: DVec2,
        to: DVec2,
        profile: ShellProfile,
    },
    /// A cubic Bézier through its four control points.
    Bezier {
        points: [DVec2; 4],
    },
}

impl Segment {
    /// The point `t` of the way along it, 0 to 1.
    pub fn point(&self, t: f64) -> DVec2 {
        match *self {
            Segment::Line { from, to } => from.lerp(to, t),
            Segment::Ellipse {
                center,
                radii,
                from,
                to,
            } => {
                let a = from + (to - from) * t;
                center + radii * DVec2::new(a.cos(), a.sin())
            }
            Segment::Taper { from, to, profile } => {
                let s = taper(profile, from, to, t);
                DVec2::new(
                    to.x + (from.x - to.x) * s.left,
                    from.y + (to.y - from.y) * s.u,
                )
            }
            Segment::Bezier {
                points: [p0, p1, p2, p3],
            } => {
                let m = 1.0 - t;
                p0 * (m * m * m)
                    + p1 * (3.0 * m * m * t)
                    + p2 * (3.0 * m * t * t)
                    + p3 * (t * t * t)
            }
        }
    }

    /// The rate of change of [`Self::point`] with `t`.
    pub fn derivative(&self, t: f64) -> DVec2 {
        match *self {
            Segment::Line { from, to } => to - from,
            Segment::Ellipse {
                radii, from, to, ..
            } => {
                let a = from + (to - from) * t;
                radii * DVec2::new(-a.sin(), a.cos()) * (to - from)
            }
            Segment::Taper { from, to, profile } => {
                let s = taper(profile, from, to, t);
                DVec2::new((from.x - to.x) * s.dleft, (to.y - from.y) * s.du)
            }
            Segment::Bezier {
                points: [p0, p1, p2, p3],
            } => {
                let m = 1.0 - t;
                (p1 - p0) * (3.0 * m * m) + (p2 - p1) * (6.0 * m * t) + (p3 - p2) * (3.0 * t * t)
            }
        }
    }

    /// The rate of change of [`Self::derivative`] with `t`.
    pub fn second_derivative(&self, t: f64) -> DVec2 {
        match *self {
            Segment::Line { .. } => DVec2::ZERO,
            Segment::Ellipse {
                radii, from, to, ..
            } => {
                let a = from + (to - from) * t;
                -radii * DVec2::new(a.cos(), a.sin()) * (to - from).powi(2)
            }
            // Numerically: the taper laws' own are long, and only the walls' sizing needs them.
            Segment::Taper { .. } => {
                let (a, b) = ((t - 1e-5).max(0.0), (t + 1e-5).min(1.0));
                (self.derivative(b) - self.derivative(a)) / (b - a)
            }
            Segment::Bezier {
                points: [p0, p1, p2, p3],
            } => ((p2 - p1 * 2.0 + p0) * (1.0 - t) + (p3 - p2 * 2.0 + p1) * t) * 6.0,
        }
    }

    /// How sharply it bends at `t`, 1/m: positive where it turns left, away from the side it
    /// faces.
    pub fn curvature(&self, t: f64) -> f64 {
        let (d, dd) = (self.derivative(t), self.second_derivative(t));
        let speed = d.length();
        if speed > 0.0 {
            d.perp_dot(dd) / speed.powi(3)
        } else {
            0.0
        }
    }

    fn check(&self) -> Result<(), String> {
        let finite = |v: DVec2| v.is_finite();
        match *self {
            Segment::Line { from, to } => {
                if !finite(from) || !finite(to) || from == to {
                    return Err(format!("a line from {from} to {to} has no length"));
                }
            }
            Segment::Ellipse {
                center,
                radii,
                from,
                to,
            } => {
                if !finite(center) || !(radii.x > 0.0 && radii.y > 0.0) || !radii.is_finite() {
                    return Err(format!(
                        "an elliptical arc needs positive radii, not {radii}"
                    ));
                }
                if !(from.is_finite() && to.is_finite() && from != to) {
                    return Err("an elliptical arc must turn through some angle".into());
                }
                if (to - from).abs() > std::f64::consts::TAU + 1e-9 {
                    return Err("an elliptical arc turns at most once around".into());
                }
            }
            Segment::Taper { from, to, profile } => {
                if !finite(from) || !finite(to) || from.y == to.y {
                    return Err("a taper must change height".into());
                }
                let drop = (from.x - to.x).abs();
                if profile == ShellProfile::Ogive && (to.y - from.y).abs() < drop * (1.0 - 1e-9) {
                    return Err(
                        "a tangent ogive is at least as long as its radius changes (an \
                         elliptical profile is blunter)"
                            .into(),
                    );
                }
            }
            Segment::Bezier { points } => {
                if !points.iter().all(|p| finite(*p)) || points.iter().all(|p| *p == points[0]) {
                    return Err("a Bézier needs distinct control points".into());
                }
            }
        }
        Ok(())
    }

    /// Stretches of `t` over which the height only rises or only falls.
    fn monotone(&self) -> Vec<(f64, f64)> {
        let mut cuts = match *self {
            Segment::Line { .. } | Segment::Taper { .. } => Vec::new(),
            Segment::Ellipse { from, to, .. } => {
                // Height turns where the angle passes π/2 + kπ.
                let (lo, hi) = (from.min(to), from.max(to));
                let first = ((lo - FRAC_PI_2) / PI).ceil() as i64;
                let last = ((hi - FRAC_PI_2) / PI).floor() as i64;
                (first..=last)
                    .map(|k| (FRAC_PI_2 + k as f64 * PI - from) / (to - from))
                    .collect()
            }
            Segment::Bezier {
                points: [p0, p1, p2, p3],
            } => {
                // The height's rate of change is a quadratic in t.
                let (a, b, c) = (p1.y - p0.y, p2.y - p1.y, p3.y - p2.y);
                quadratic_roots(a - 2.0 * b + c, 2.0 * (b - a), a)
            }
        };
        cuts.retain(|t| *t > 1e-12 && *t < 1.0 - 1e-12);
        cuts.sort_by(f64::total_cmp);
        let mut edges = vec![0.0];
        edges.extend(cuts);
        edges.push(1.0);
        edges.windows(2).map(|w| (w[0], w[1])).collect()
    }
}

/// Real roots of a t² + b t + c.
fn quadratic_roots(a: f64, b: f64, c: f64) -> Vec<f64> {
    if a.abs() < 1e-14 * (b.abs() + c.abs()) {
        return if b != 0.0 { vec![-c / b] } else { Vec::new() };
    }
    let discriminant = b * b - 4.0 * a * c;
    if discriminant < 0.0 {
        return Vec::new();
    }
    let root = discriminant.sqrt();
    vec![(-b - root) / (2.0 * a), (-b + root) / (2.0 * a)]
}

/// Where a taper is at parameter `s`: `u` of the way up (by height) with `left` of its change
/// in radius still to come, and their rates of change with `s`. The height runs as
/// sin(π s / 2), which bunches points towards the tip, where blunt profiles turn fastest,
/// and keeps the rates finite there.
struct TaperAt {
    u: f64,
    du: f64,
    left: f64,
    dleft: f64,
}

fn taper(profile: ShellProfile, from: DVec2, to: DVec2, s: f64) -> TaperAt {
    let (length, drop) = ((to.y - from.y).abs(), (from.x - to.x).abs());
    let (sin, cos) = (FRAC_PI_2 * s).sin_cos();
    let (u, du) = (sin, FRAC_PI_2 * cos);
    let left = remaining(profile, u, length, drop);
    let dleft = match profile {
        _ if drop < 1e-9 => -du,
        ShellProfile::Conic => -du,
        ShellProfile::Ogive => {
            let rho = (drop * drop + length * length) / (2.0 * drop);
            let x = u * length;
            let root = (rho * rho - x * x).max(0.0).sqrt();
            if root > 1e-9 * rho {
                -x * length * du / (root * drop)
            } else {
                // Only at the tip of an ogive exactly as long as its radius changes, where
                // the root and du shrink together.
                -x * length * FRAC_PI_2 / (drop * drop)
            }
        }
        ShellProfile::Elliptical => -FRAC_PI_2 * sin,
        ShellProfile::Parabolic => -2.0 * u * du,
        ShellProfile::Haack => {
            let theta = (2.0 * u - 1.0).clamp(-1.0, 1.0).acos();
            if left > 0.0 {
                -2.0 * theta.sin() / (PI * left) * du
            } else {
                0.0
            }
        }
    };
    TaperAt { u, du, left, dleft }
}

/// How much of a nose cone's change in radius is still to come `u` of the way up it (0 at
/// its base, 1 at its tip), for one `length` long whose radius changes by `drop`.
pub(crate) fn remaining(profile: ShellProfile, u: f64, length: f64, drop: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    match profile {
        _ if drop < 1e-9 => 1.0 - u,
        ShellProfile::Conic => 1.0 - u,
        ShellProfile::Ogive => {
            // An arc of radius ρ from the tip, tangent to the side at the base.
            let x = u * length;
            let rho = (drop * drop + length * length) / (2.0 * drop);
            ((rho * rho - x * x).max(0.0).sqrt() + drop - rho).max(0.0) / drop
        }
        ShellProfile::Elliptical => (1.0 - u * u).max(0.0).sqrt(),
        ShellProfile::Parabolic => 1.0 - u * u,
        ShellProfile::Haack => {
            // θ runs from π at the base to 0 at the tip.
            let theta = (2.0 * u - 1.0).clamp(-1.0, 1.0).acos();
            ((theta - (2.0 * theta).sin() / 2.0) / PI).max(0.0).sqrt()
        }
    }
}

/// A point of a tessellated curve, and where on the curve it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvePoint {
    pub point: DVec2,
    pub segment: usize,
    pub t: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    segments: Vec<Segment>,
    /// (segment, t from, t to): stretches along which the height only rises or only falls.
    pieces: Vec<(usize, f64, f64)>,
    /// How big it is, m: the diagonal of its bounds with the axis.
    size: f64,
}

impl Curve {
    pub fn new(segments: Vec<Segment>) -> Result<Self, String> {
        if segments.is_empty() {
            return Err("a curve needs at least one segment".into());
        }
        for segment in &segments {
            segment.check()?;
        }
        let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
        for segment in &segments {
            for k in 0..=16 {
                let p = segment.point(k as f64 / 16.0);
                lo = lo.min(p).min(DVec2::new(0.0, p.y));
                hi = hi.max(p).max(DVec2::new(0.0, p.y));
            }
        }
        let size = (hi - lo).length().max(1e-6);
        for w in segments.windows(2) {
            let (end, start) = (w[0].point(1.0), w[1].point(0.0));
            if (end - start).length() > 1e-9 * size {
                return Err(format!(
                    "segments must join: one ends at {end}, the next starts at {start}"
                ));
            }
        }
        let pieces = segments
            .iter()
            .enumerate()
            .flat_map(|(i, s)| s.monotone().into_iter().map(move |(a, b)| (i, a, b)))
            .collect();
        let curve = Self {
            segments,
            pieces,
            size,
        };
        if curve
            .tessellate(1e-3 * size)
            .iter()
            .any(|p| p.point.x < -1e-9 * size)
        {
            return Err("a curve cannot cross the axis to negative radii".into());
        }
        Ok(curve)
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn start(&self) -> DVec2 {
        self.segments[0].point(0.0)
    }

    pub fn end(&self) -> DVec2 {
        self.segments[self.segments.len() - 1].point(1.0)
    }

    pub fn size(&self) -> f64 {
        self.size
    }

    /// Whether it encloses a region: it ends where it starts, or starts and ends on the
    /// axis (which closes it).
    pub fn is_closed(&self) -> bool {
        let (start, end, near) = (self.start(), self.end(), 1e-9 * self.size);
        (end - start).length() <= near || (start.x.abs() <= near && end.x.abs() <= near)
    }

    /// Its lowest and highest heights.
    pub fn heights(&self) -> (f64, f64) {
        self.pieces
            .iter()
            .flat_map(|&(s, a, b)| [self.segments[s].point(a).y, self.segments[s].point(b).y])
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), z| {
                (lo.min(z), hi.max(z))
            })
    }

    /// How far from the axis the curve reaches at `height`, if it touches it at the end of a
    /// rising or falling stretch: what it is at its lowest or highest.
    pub fn widest_at(&self, height: f64) -> Option<f64> {
        let near = 1e-9 * self.size;
        self.pieces
            .iter()
            .flat_map(|&(s, a, b)| [self.segments[s].point(a), self.segments[s].point(b)])
            .filter(|p| (p.y - height).abs() <= near)
            .map(|p| p.x.max(0.0))
            .reduce(f64::max)
    }

    /// ∫ f(point, rate of change) along the whole curve, over each segment's parameter.
    pub fn integrate<const N: usize>(&self, f: impl Fn(DVec2, DVec2) -> [f64; N]) -> [f64; N] {
        self.integrate_below(f64::INFINITY, f)
    }

    /// Like [`Self::integrate`], over the parts of the curve no higher than `height`.
    pub fn integrate_below<const N: usize>(
        &self,
        height: f64,
        f: impl Fn(DVec2, DVec2) -> [f64; N],
    ) -> [f64; N] {
        let mut sum = [0.0; N];
        for &(s, t0, t1) in &self.pieces {
            let segment = &self.segments[s];
            let (z0, z1) = (segment.point(t0).y, segment.point(t1).y);
            let (a, b) = if z0.max(z1) <= height {
                (t0, t1)
            } else if z0.min(z1) >= height {
                continue;
            } else {
                let t = crossing(segment, t0, t1, height);
                if z0 < z1 { (t0, t) } else { (t, t1) }
            };
            let part = integrate(|t| f(segment.point(t), segment.derivative(t)), a, b);
            for i in 0..N {
                sum[i] += part[i];
            }
        }
        sum
    }

    /// Where it stops rising and starts falling, or the other way, running level there: a
    /// torus's crowns, a dome's pole. Corners and level stretches are not among them.
    pub fn crowns(&self) -> Vec<DVec2> {
        let level = |d: DVec2| d.y.abs() <= 1e-9 * d.length();
        // Each stretch's way in height, and its ends' points and directions.
        let stretches: Vec<_> = self
            .pieces
            .iter()
            .map(|&(s, a, b)| {
                let segment = &self.segments[s];
                let (p, q) = (segment.point(a), segment.point(b));
                let rise = if (q.y - p.y).abs() <= 1e-12 * self.size {
                    0.0
                } else {
                    (q.y - p.y).signum()
                };
                (rise, q, segment.derivative(a), segment.derivative(b))
            })
            .collect();
        let mut pairs: Vec<_> = stretches.windows(2).map(|w| (w[0], w[1])).collect();
        if (self.end() - self.start()).length() <= 1e-9 * self.size {
            pairs.push((stretches[stretches.len() - 1], stretches[0]));
        }
        pairs
            .into_iter()
            .filter(|(a, b)| a.0 * b.0 < 0.0 && level(a.3) && level(b.2))
            .map(|(a, _)| a.1)
            .collect()
    }

    /// Where it crosses `height`: the radius, and +1 where it rises through it or −1 where
    /// it falls.
    pub fn crossings(&self, height: f64) -> Vec<(f64, f64)> {
        self.pieces
            .iter()
            .filter_map(|&(s, t0, t1)| {
                let segment = &self.segments[s];
                let (z0, z1) = (segment.point(t0).y, segment.point(t1).y);
                (z0.min(z1) < height && height < z0.max(z1)).then(|| {
                    let t = crossing(segment, t0, t1, height);
                    (segment.point(t).x, (z1 - z0).signum())
                })
            })
            .collect()
    }

    /// Points along it, from its start to its end, no further from the curve between them
    /// than `tolerance` and turning at most [`MAX_TURN`] from one to the next.
    pub fn tessellate(&self, tolerance: f64) -> Vec<CurvePoint> {
        let mut out = vec![CurvePoint {
            point: self.start(),
            segment: 0,
            t: 0.0,
        }];
        for (i, segment) in self.segments.iter().enumerate() {
            let pieces = match segment {
                Segment::Line { .. } => 1,
                Segment::Ellipse { from, to, .. } => {
                    ((to - from).abs() / FRAC_PI_4).ceil() as usize
                }
                _ => 4,
            }
            .max(1);
            for k in 0..pieces {
                let (t0, t1) = (k as f64 / pieces as f64, (k + 1) as f64 / pieces as f64);
                subdivide(segment, i, t0, t1, tolerance, MAX_DEPTH, &mut out);
            }
        }
        out
    }

    /// Its length, m.
    pub fn length(&self) -> f64 {
        self.integrate(|_, d| [d.length()])[0]
    }

    /// The point `share` of the way along it by length, and the unit direction it runs
    /// there.
    pub fn at_share(&self, share: f64) -> (DVec2, DVec2) {
        let points = self.tessellate(1e-5 * self.size);
        let mut lengths = vec![0.0];
        for w in points.windows(2) {
            lengths.push(lengths[lengths.len() - 1] + (w[1].point - w[0].point).length());
        }
        let target = share.clamp(0.0, 1.0) * lengths[lengths.len() - 1];
        let k = lengths
            .windows(2)
            .position(|w| target <= w[1])
            .unwrap_or(points.len() - 2);
        let (p, q) = (points[k], points[k + 1]);
        let f = ((target - lengths[k]) / (lengths[k + 1] - lengths[k])).clamp(0.0, 1.0);
        let (s, t) = if p.segment == q.segment {
            (q.segment, p.t + (q.t - p.t) * f)
        } else {
            (q.segment, q.t * f)
        };
        let segment = &self.segments[s];
        let direction = segment.derivative(t);
        let direction = if direction.length() > 1e-12 * self.size {
            direction.normalize()
        } else {
            (q.point - p.point).normalize()
        };
        (segment.point(t), direction)
    }
}

/// The parameter where `segment`, rising or falling throughout `t0..t1`, is at `height`.
fn crossing(segment: &Segment, mut t0: f64, mut t1: f64, height: f64) -> f64 {
    let rising = segment.point(t1).y > segment.point(t0).y;
    for _ in 0..80 {
        let mid = (t0 + t1) / 2.0;
        if (segment.point(mid).y < height) == rising {
            t0 = mid;
        } else {
            t1 = mid;
        }
        if t1 - t0 < 1e-16 {
            break;
        }
    }
    (t0 + t1) / 2.0
}

/// Push the points of `segment` after `t0` up to `t1`, halving the stretch while it strays
/// from its chord or turns too much.
fn subdivide(
    segment: &Segment,
    index: usize,
    t0: f64,
    t1: f64,
    tolerance: f64,
    depth: u32,
    out: &mut Vec<CurvePoint>,
) {
    if depth > 0 && !matches!(segment, Segment::Line { .. }) {
        let (a, b, mid) = (segment.point(t0), segment.point(t1), (t0 + t1) / 2.0);
        let m = segment.point(mid);
        let chord = b - a;
        let off = if chord.length() > 0.0 {
            (m - a).perp_dot(chord).abs() / chord.length()
        } else {
            (m - a).length()
        };
        let (da, db) = (segment.derivative(t0), segment.derivative(t1));
        let turn = if da.length() > 0.0 && db.length() > 0.0 {
            da.perp_dot(db).atan2(da.dot(db)).abs()
        } else {
            0.0
        };
        if off > tolerance || turn > MAX_TURN {
            subdivide(segment, index, t0, mid, tolerance, depth - 1, out);
            subdivide(segment, index, mid, t1, tolerance, depth - 1, out);
            return;
        }
    }
    out.push(CurvePoint {
        point: segment.point(t1),
        segment: index,
        t: t1,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quarter_circle(radius: f64) -> Curve {
        Curve::new(vec![Segment::Ellipse {
            center: DVec2::ZERO,
            radii: DVec2::splat(radius),
            from: -FRAC_PI_2,
            to: 0.0,
        }])
        .unwrap()
    }

    #[test]
    fn lengths_follow_the_curve() {
        let arc = quarter_circle(2.0);
        assert!((arc.length() - PI).abs() < 1e-12);
        let (mid, direction) = arc.at_share(0.5);
        let expected = DVec2::from_angle(-FRAC_PI_2 / 2.0) * 2.0;
        assert!((mid - expected).length() < 1e-6, "{mid}");
        assert!((direction - DVec2::from_angle(FRAC_PI_4)).length() < 1e-6);
    }

    #[test]
    fn tapers_run_from_base_to_tip_along_their_profile() {
        for profile in [
            ShellProfile::Conic,
            ShellProfile::Ogive,
            ShellProfile::Elliptical,
            ShellProfile::Parabolic,
            ShellProfile::Haack,
        ] {
            let (from, to) = (DVec2::new(1.5, 2.0), DVec2::new(0.0, 5.0));
            let segment = Segment::Taper { from, to, profile };
            assert!((segment.point(0.0) - from).length() < 1e-12);
            assert!((segment.point(1.0) - to).length() < 1e-12);
            // The rate of change matches the points, however blunt the profile.
            for t in [0.1, 0.5, 0.9, 0.999] {
                let h = 1e-7;
                let numeric = (segment.point(t + h) - segment.point(t - h)) / (2.0 * h);
                let d = segment.derivative(t);
                assert!(
                    (numeric - d).length() < 1e-5 * d.length().max(1.0),
                    "{profile:?} at {t}: {numeric} vs {d}"
                );
            }
            // The curved ones leave the base parallel to the axis.
            if profile != ShellProfile::Conic {
                assert!(segment.derivative(0.0).x.abs() < 1e-9, "{profile:?}");
            }
        }
    }

    #[test]
    fn crossings_and_integrals_below_a_height() {
        // A semicircle from the bottom of the axis round to the top: a sphere's outline.
        let sphere = Curve::new(vec![Segment::Ellipse {
            center: DVec2::new(0.0, 1.0),
            radii: DVec2::splat(1.0),
            from: -FRAC_PI_2,
            to: FRAC_PI_2,
        }])
        .unwrap();
        assert!(sphere.is_closed());
        assert_eq!(sphere.heights(), (0.0, 2.0));
        let crossings = sphere.crossings(1.0);
        assert_eq!(crossings.len(), 1);
        assert!((crossings[0].0 - 1.0).abs() < 1e-12 && crossings[0].1 == 1.0);
        // Green's theorem: the area enclosed with the axis, ∮ r dz, below the middle.
        let [half] = sphere.integrate_below(1.0, |p, d| [p.x * d.y]);
        assert!((half - FRAC_PI_4).abs() < 1e-12);
    }

    #[test]
    fn bad_curves_are_refused() {
        let gap = Curve::new(vec![
            Segment::Line {
                from: DVec2::ZERO,
                to: DVec2::X,
            },
            Segment::Line {
                from: DVec2::new(1.0, 0.5),
                to: DVec2::Y,
            },
        ]);
        assert!(gap.is_err());
        let past_the_axis = Curve::new(vec![Segment::Line {
            from: DVec2::X,
            to: DVec2::new(-1.0, 1.0),
        }]);
        assert!(past_the_axis.is_err());
        let short_ogive = Curve::new(vec![Segment::Taper {
            from: DVec2::new(2.0, 0.0),
            to: DVec2::new(0.0, 1.0),
            profile: ShellProfile::Ogive,
        }]);
        assert!(short_ogive.is_err());
    }
}
