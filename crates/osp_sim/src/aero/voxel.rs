//! The air's view of a vessel's body, after FAR's voxels: the outer surfaces of its parts
//! cut by planes across its axis, each cut filled in from outside, so that what is enclosed
//! (a fairing's payload, engines in an interstage, a service module's tanks) adds nothing of
//! its own and only the outer skin meets the air.
//!
//! A cut is sampled along rows, like a voxel grid run-length encoded along each row and so
//! exact along it: where a row crosses the cut's segments splits it into intervals, and an
//! interval is outside when a flood from the edge of the grid reaches it, passing between
//! neighbouring rows wherever no segment stands between them, however thin. The rest is the
//! body: an open skirt's mouth, a fairing's halves and the space between them all count,
//! the gaps in a truss do not. The same is done along columns, and each cut's figures are
//! the mean of the two: its area and the middle of it, its width along each axis, the
//! perimeter where the air touches it (Cauchy–Crofton: π/4 of the crossings of the rows and
//! the columns, times their spacing) and how far that reaches from the middle.
//!
//! Cuts are taken where the surfaces have corners along the axis, no further apart than
//! [`MAX_GAP`] of the length; where a surface lies flat across the axis (a base, a step,
//! the end of a skirt), both the cut just below it and the cut just above it.

use std::f64::consts::PI;

use glam::{DVec2, DVec3};

/// Rows (and columns) across the body's widest extent.
const RESOLUTION: usize = 160;
/// Cuts are at most this share of the body's length apart.
const MAX_GAP: f64 = 1.0 / 64.0;
/// Corners along the axis closer together than this share of the length are cut once.
const MIN_GAP: f64 = 1.0 / 512.0;
/// Heights closer than this are the same (the meshes are single precision), m.
const SAME_HEIGHT: f64 = 1e-5;
/// Gaps narrower than this share of a row let no air through: where surfaces meet, their
/// cuts can miss each other by rounding.
const GAP: f64 = 1e-3;

/// A cut across the body.
#[derive(Debug, Clone, PartialEq)]
pub struct Cut {
    /// Height along the axis, m.
    pub z: f64,
    /// m².
    pub area: f64,
    /// The middle of the area, m.
    pub center: DVec2,
    /// How far it reaches along x and along y, gaps left out, m.
    pub width: DVec2,
    /// Round it where the air touches it, m.
    pub perimeter: f64,
    /// Farthest its outside reaches from `center`, m.
    pub reach: f64,
    /// The body's intervals along each row.
    rows: Lines,
}

/// The rows the cuts are sampled along; the columns are spaced the same.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Grid {
    /// The corner the first row and the first column are half a step from.
    origin: DVec2,
    step: f64,
    /// Rows (along x, stacked up y) and columns.
    count: [usize; 2],
}

/// A body as the air sees it: cuts across its axis.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Body {
    /// Bottom up; two at the same height where a surface lies across the axis, the one
    /// below it first.
    pub cuts: Vec<Cut>,
    grid: Option<Grid>,
}

impl Body {
    /// The body `triangles` bound, its axis +Z.
    pub fn new(triangles: &[[DVec3; 3]]) -> Self {
        let (mut lo, mut hi) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
        for v in triangles.iter().flatten() {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
        if triangles.is_empty() || hi.z - lo.z <= SAME_HEIGHT {
            return Self::default();
        }
        let size = (hi - lo).truncate().max_element().max(1e-3);
        let step = size / RESOLUTION as f64;
        let cells = |extent: f64| (extent / step).ceil() as usize + 4;
        let grid = Grid {
            origin: lo.truncate() - DVec2::splat(2.0 * step),
            step,
            count: [cells(hi.y - lo.y), cells(hi.x - lo.x)],
        };

        // Heights snapped to the corners they are meant to be at.
        let corners = corners(triangles);
        let snap = |z: f64| {
            let i = corners.partition_point(|&c| c < z - SAME_HEIGHT);
            corners
                .get(i)
                .copied()
                .filter(|c| (c - z).abs() <= SAME_HEIGHT)
                .unwrap_or(z)
        };
        let triangles: Vec<[DVec3; 3]> = triangles
            .iter()
            .map(|t| t.map(|v| DVec3::new(v.x, v.y, snap(v.z))))
            .collect();
        let low = |t: &[DVec3; 3]| t[0].z.min(t[1].z).min(t[2].z);
        let high = |t: &[DVec3; 3]| t[0].z.max(t[1].z).max(t[2].z);
        let mut order: Vec<usize> = (0..triangles.len()).collect();
        order.sort_by(|&a, &b| low(&triangles[a]).total_cmp(&low(&triangles[b])));

        // Each thread cuts a run of the planes, sweeping up through the triangles.
        let planes = planes(&triangles, &corners);
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
        let run = planes.len().div_ceil(threads).max(8);
        let cut_run = |planes: &[(f64, bool)]| {
            let (mut active, mut next) = (Vec::new(), 0);
            let mut cuts = Vec::with_capacity(planes.len());
            for &(z, above) in planes {
                while next < order.len() && low(&triangles[order[next]]) <= z {
                    active.push(order[next]);
                    next += 1;
                }
                active.retain(|&t| high(&triangles[t]) >= z);
                let segments: Vec<[DVec2; 2]> = active
                    .iter()
                    .filter_map(|&t| crossing(&triangles[t], z, above))
                    .collect();
                cuts.push(Cut::new(z, &segments, &grid));
            }
            cuts
        };
        let cuts = std::thread::scope(|scope| {
            let runs: Vec<_> = (planes.chunks(run))
                .map(|chunk| scope.spawn(|| cut_run(chunk)))
                .collect();
            let runs = runs
                .into_iter()
                .map(|r| r.join().expect("cutting the body"));
            runs.flatten().collect()
        });
        Self {
            cuts,
            grid: Some(grid),
        }
    }

    /// Whether `point` is inside the body: in it at the cuts on either side.
    pub fn encloses(&self, point: DVec3) -> bool {
        let Some(grid) = &self.grid else {
            return false;
        };
        let i = self.cuts.partition_point(|c| c.z < point.z);
        if i == 0 || i == self.cuts.len() {
            return false;
        }
        let row = ((point.y - grid.origin.y) / grid.step - 0.5).round();
        if row < 0.0 {
            return false;
        }
        let row = row as usize;
        self.cuts[i - 1..=i]
            .iter()
            .all(|cut| (cut.rows.line(row).iter()).any(|&[a, b]| a <= point.x && point.x <= b))
    }
}

impl Cut {
    fn new(z: f64, segments: &[[DVec2; 2]], grid: &Grid) -> Self {
        let rows = sweep(segments, grid, false);
        let columns = sweep(segments, grid, true);
        let area = (rows.area + columns.area) / 2.0;
        let center = if area > 0.0 {
            (rows.moment + columns.moment) / (2.0 * area)
        } else {
            DVec2::ZERO
        };
        let reach = (rows.exposed.iter().chain(&columns.exposed))
            .map(|p| p.distance(center))
            .fold(0.0, f64::max);
        let crossings = (rows.exposed.len() + columns.exposed.len()) as f64;
        Self {
            z,
            area,
            center,
            width: DVec2::new(rows.extent, columns.extent),
            perimeter: PI / 4.0 * grid.step * crossings,
            reach,
            rows: rows.inside,
        }
    }
}

/// What one sweep along rows (or columns) finds.
#[derive(Debug, Default)]
struct Sweep {
    area: f64,
    /// Of the area about the origin, m³.
    moment: DVec2,
    /// How far the body reaches along the lines, gaps left out, m.
    extent: f64,
    /// Where the lines cross from outside into the body or back.
    exposed: Vec<DVec2>,
    /// The body's intervals along each line.
    inside: Lines,
}

/// Intervals along each of a set of lines, one after another.
#[derive(Debug, Clone, Default, PartialEq)]
struct Lines {
    intervals: Vec<[f64; 2]>,
    /// Where each line's intervals start, and where the last one's end.
    starts: Vec<usize>,
}

impl Lines {
    fn line(&self, j: usize) -> &[[f64; 2]] {
        match (self.starts.get(j), self.starts.get(j + 1)) {
            (Some(&a), Some(&b)) => &self.intervals[a..b],
            _ => &[],
        }
    }
}

/// The cut `segments` make, sampled along rows, or along columns if `columns`.
fn sweep(segments: &[[DVec2; 2]], grid: &Grid, columns: bool) -> Sweep {
    // Lines run along x here; for columns, x and y swap.
    let swap = |p: DVec2| if columns { DVec2::new(p.y, p.x) } else { p };
    let origin = swap(grid.origin);
    let (h, n) = (grid.step, grid.count[usize::from(columns)]);
    let line = |j: usize| origin.y + (j as f64 + 0.5) * h;
    // Where each line is crossed, and what stands between line j and line j + 1: the
    // span of x each segment covers there.
    let mut crossings: Vec<(usize, f64)> = Vec::with_capacity(2 * segments.len());
    let mut blocked: Vec<(usize, [f64; 2])> = Vec::with_capacity(3 * segments.len());
    for s in segments {
        let (a, b) = (swap(s[0]), swap(s[1]));
        let (lo, hi) = (a.y.min(b.y), a.y.max(b.y));
        let x_at = |y: f64| {
            if b.y == a.y {
                a.x
            } else {
                a.x + (b.x - a.x) * ((y - a.y) / (b.y - a.y)).clamp(0.0, 1.0)
            }
        };
        let index = |y: f64| ((y - origin.y) / h - 0.5).max(0.0);
        let first = index(lo).floor() as usize;
        let last = (index(hi).ceil() as usize).min(n - 1);
        for j in first..=last {
            let y = line(j);
            if lo <= y && y < hi {
                crossings.push((j, x_at(y)));
            }
            if j < last {
                let (y0, y1) = (y.max(lo), line(j + 1).min(hi));
                if y0 <= y1 {
                    let (x0, x1) = if b.y == a.y {
                        (a.x, b.x)
                    } else {
                        (x_at(y0), x_at(y1))
                    };
                    blocked.push((j, [x0.min(x1), x0.max(x1)]));
                }
            }
        }
    }
    let crossings = by_line(n, crossings, |a, b| a.total_cmp(b));
    let at = &crossings.starts;
    let xs = &crossings.intervals;
    let mut bands = by_line(n - 1, blocked, |a, b| a[0].total_cmp(&b[0]));
    for j in 0..n - 1 {
        let (from, to) = (bands.starts[j], bands.starts[j + 1]);
        let mut end = from;
        for i in from..to {
            let [a, b] = bands.intervals[i];
            if end > from && a <= bands.intervals[end - 1][1] {
                bands.intervals[end - 1][1] = bands.intervals[end - 1][1].max(b);
            } else {
                bands.intervals[end] = [a, b];
                end += 1;
            }
        }
        bands.ends.push(end);
    }

    // Interval k of line j runs from its crossing k − 1 to its crossing k, the first and
    // the last without end; interval k of line j is number `at[j] + j + k`.
    let index = |j: usize, k: usize| at[j] + j + k;
    let count = |j: usize| at[j + 1] - at[j];
    let span = |j: usize, k: usize| {
        let c = &xs[at[j]..at[j + 1]];
        [
            if k == 0 { f64::NEG_INFINITY } else { c[k - 1] },
            c.get(k).copied().unwrap_or(f64::INFINITY),
        ]
    };
    let mut sets = Sets::new(xs.len() + n);
    for j in 0..n - 1 {
        let band = &bands.intervals[bands.starts[j]..bands.ends[j]];
        let (m0, m1) = (count(j) + 1, count(j + 1) + 1);
        let (mut p, mut q) = (0, 0);
        while p < m0 && q < m1 {
            let (a, b) = (span(j, p), span(j + 1, q));
            let (lo, hi) = (a[0].max(b[0]), a[1].min(b[1]));
            if free(band, lo, hi, h * GAP) {
                sets.join(index(j, p), index(j + 1, q));
            }
            if a[1] < b[1] {
                p += 1;
            } else {
                q += 1;
            }
        }
    }
    let mut outside = vec![false; xs.len() + n];
    for j in 0..n {
        for end in [index(j, 0), index(j, count(j))] {
            let root = sets.find(end);
            outside[root] = true;
        }
    }
    let mut out = |i: usize| outside[sets.find(i)];

    let mut sweep = Sweep {
        inside: Lines {
            intervals: Vec::new(),
            starts: vec![0],
        },
        ..Sweep::default()
    };
    let mut extent = Vec::new();
    for j in 0..n {
        let y = line(j);
        let mut was_out = true;
        let from = sweep.inside.intervals.len();
        for k in 0..=count(j) {
            let is_out = out(index(j, k));
            let [x0, x1] = span(j, k);
            if k > 0 && is_out != was_out {
                sweep.exposed.push(swap(DVec2::new(x0, y)));
            }
            if !is_out {
                let length = x1 - x0;
                sweep.area += h * length;
                sweep.moment += h * length * swap(DVec2::new((x0 + x1) / 2.0, y));
                extent.push([x0, x1]);
                match sweep.inside.intervals[from..].last_mut() {
                    Some(last) if x0 <= last[1] => last[1] = x1,
                    _ => sweep.inside.intervals.push([x0, x1]),
                }
            }
            was_out = is_out;
        }
        sweep.inside.starts.push(sweep.inside.intervals.len());
    }
    merge(&mut extent);
    sweep.extent = extent.iter().map(|[a, b]| b - a).sum();
    sweep
}

/// Items gathered by line, sorted along each line by `order`.
struct ByLine<T> {
    intervals: Vec<T>,
    /// Where each line's items start, and where the last one's end.
    starts: Vec<usize>,
    /// Where each line's items end, once merged.
    ends: Vec<usize>,
}

fn by_line<T: Copy + Default>(
    n: usize,
    items: Vec<(usize, T)>,
    order: impl Fn(&T, &T) -> std::cmp::Ordering,
) -> ByLine<T> {
    let mut starts = vec![0; n + 1];
    for &(j, _) in &items {
        starts[j + 1] += 1;
    }
    for j in 0..n {
        starts[j + 1] += starts[j];
    }
    let mut next = starts.clone();
    let mut sorted = vec![T::default(); items.len()];
    for (j, item) in items {
        sorted[next[j]] = item;
        next[j] += 1;
    }
    for j in 0..n {
        sorted[starts[j]..starts[j + 1]].sort_unstable_by(&order);
    }
    ByLine {
        intervals: sorted,
        starts,
        ends: Vec::with_capacity(n),
    }
}

/// Whether a stretch of `lo..hi` longer than `gap` is clear of `blocked` (sorted, merged).
fn free(blocked: &[[f64; 2]], lo: f64, hi: f64, gap: f64) -> bool {
    if hi - lo <= gap {
        return false;
    }
    // Everything from `lo` up to `clear` is blocked.
    let mut clear = lo;
    for b in &blocked[blocked.partition_point(|b| b[1] <= lo)..] {
        if b[0] >= hi {
            break;
        }
        if b[0] - clear > gap {
            return true;
        }
        clear = clear.max(b[1]);
        if clear >= hi {
            return false;
        }
    }
    hi - clear > gap
}

/// `intervals` sorted and merged where they overlap or touch.
fn merge(intervals: &mut Vec<[f64; 2]>) {
    intervals.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let mut merged: Vec<[f64; 2]> = Vec::with_capacity(intervals.len());
    for &[a, b] in intervals.iter() {
        match merged.last_mut() {
            Some(last) if a <= last[1] => last[1] = last[1].max(b),
            _ => merged.push([a, b]),
        }
    }
    *intervals = merged;
}

/// Where `triangle` crosses the plane at height `z`, just above it if `above`, else just
/// below.
fn crossing(triangle: &[DVec3; 3], z: f64, above: bool) -> Option<[DVec2; 2]> {
    let up = |v: &DVec3| if above { v.z > z } else { v.z >= z };
    let mut points = [DVec2::ZERO; 2];
    let mut found = 0;
    for k in 0..3 {
        let (a, b) = (triangle[k], triangle[(k + 1) % 3]);
        if up(&a) != up(&b) {
            let t = (z - a.z) / (b.z - a.z);
            points[found] = (a + (b - a) * t).truncate();
            found += 1;
        }
    }
    (found == 2).then_some(points)
}

/// The heights of the corners of `triangles`, bottom up, those within [`SAME_HEIGHT`] of
/// each other as one.
fn corners(triangles: &[[DVec3; 3]]) -> Vec<f64> {
    let mut corners: Vec<f64> = triangles.iter().flatten().map(|v| v.z).collect();
    corners.sort_by(f64::total_cmp);
    corners.dedup_by(|z, kept| *z - *kept <= SAME_HEIGHT);
    corners
}

/// Where to cut `triangles` (snapped to `corners`), bottom up: each height, and whether
/// just above it (else just below).
fn planes(triangles: &[[DVec3; 3]], corners: &[f64]) -> Vec<(f64, bool)> {
    let (bottom, top) = (corners[0], corners[corners.len() - 1]);
    let length = top - bottom;
    let mut flat: Vec<f64> = triangles
        .iter()
        .filter(|t| {
            let across = (t[1] - t[0]).truncate().perp_dot((t[2] - t[0]).truncate());
            t[0].z == t[1].z && t[1].z == t[2].z && across.abs() > 1e-12
        })
        .map(|t| t[0].z)
        .collect();
    flat.sort_by(f64::total_cmp);
    flat.dedup();
    let is_flat = |z: f64| flat.binary_search_by(|f| f.total_cmp(&z)).is_ok();

    let mut kept: Vec<(f64, bool)> = Vec::new();
    for &z in corners {
        let here = is_flat(z);
        if let Some(&(last, was_flat)) = kept.last()
            && z - last < MIN_GAP * length
            && z < top
        {
            if !here || was_flat {
                continue;
            }
            if kept.len() > 1 {
                kept.pop();
            }
        }
        kept.push((z, here));
    }
    let mut out = Vec::new();
    for (k, &(z, flat)) in kept.iter().enumerate() {
        if k > 0 {
            let last = kept[k - 1].0;
            let pieces = ((z - last) / (MAX_GAP * length)).ceil().max(1.0) as usize;
            out.extend((1..pieces).map(|i| (last + (z - last) * i as f64 / pieces as f64, true)));
        }
        if flat {
            out.push((z, false));
        }
        out.push((z, true));
    }
    out
}

/// Disjoint sets of intervals.
struct Sets(Vec<usize>);

impl Sets {
    fn new(n: usize) -> Self {
        Self((0..n).collect())
    }

    fn find(&mut self, mut i: usize) -> usize {
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }

    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        self.0[a.max(b)] = a.min(b);
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use super::*;

    /// A tube from `z0` to `z1`, `radius` round the line through `at` (x, y) along the
    /// axis, 72 sides round, as triangles; closed at the ends with flat caps if `capped`.
    fn tube(radius: f64, z0: f64, z1: f64, capped: bool, at: DVec2) -> Vec<[DVec3; 3]> {
        let sides = 72;
        let p = |k: usize, z: f64| {
            let a = TAU * k as f64 / sides as f64;
            (at + DVec2::from_angle(a) * radius).extend(z)
        };
        let mut out = Vec::new();
        for k in 0..sides {
            let (a0, a1) = (p(k, z0), p(k + 1, z0));
            let (b0, b1) = (p(k, z1), p(k + 1, z1));
            out.extend([[a0, a1, b1], [a0, b1, b0]]);
            if capped {
                out.push([at.extend(z0), a1, a0]);
                out.push([at.extend(z1), b0, b1]);
            }
        }
        out
    }

    /// The share of a round section's area its polygon of 72 sides has.
    fn polygon(radius: f64) -> f64 {
        let n = 72.0;
        n / 2.0 * (TAU / n).sin() * radius * radius
    }

    fn near(a: f64, b: f64, share: f64) -> bool {
        (a - b).abs() <= share * b.abs()
    }

    #[test]
    fn a_tube_is_cut_round_and_filled() {
        let body = Body::new(&tube(1.5, 0.0, 10.0, false, DVec2::ZERO));
        let middle = &body.cuts[body.cuts.len() / 2];
        assert!(near(middle.area, polygon(1.5), 2e-3), "{}", middle.area);
        assert!(
            near(middle.perimeter, TAU * 1.5, 5e-3),
            "{}",
            middle.perimeter
        );
        assert!(near(middle.width.x, 3.0, 1e-3) && near(middle.width.y, 3.0, 2e-3));
        assert!(middle.center.length() < 1e-3, "{}", middle.center);
        assert!(near(middle.reach, 1.5, 2e-3), "{}", middle.reach);
        // Every cut of a straight tube is the same.
        let areas: Vec<f64> = body.cuts.iter().map(|c| c.area).collect();
        let (top, rest) = areas.split_last().unwrap();
        assert!(
            rest.iter().all(|&a| (a - areas[0]).abs() < 1e-12),
            "{areas:?}"
        );
        assert_eq!(*top, 0.0);
        assert!(body.encloses(DVec3::new(0.3, -0.4, 5.0)));
        assert!(!body.encloses(DVec3::new(1.6, 0.0, 5.0)));
        assert!(!body.encloses(DVec3::new(0.0, 0.0, 10.5)));
    }

    #[test]
    fn a_flat_step_is_cut_on_both_sides() {
        // A wide capped drum under a narrow one.
        let mut triangles = tube(2.0, 0.0, 4.0, true, DVec2::ZERO);
        triangles.extend(tube(1.0, 4.0, 8.0, true, DVec2::ZERO));
        let body = Body::new(&triangles);
        let at = |z: f64| -> Vec<f64> {
            body.cuts
                .iter()
                .filter(|c| (c.z - z).abs() < 1e-9)
                .map(|c| c.area)
                .collect()
        };
        let (wide, narrow) = (polygon(2.0), polygon(1.0));
        let [below, above] = at(4.0)[..] else {
            panic!("{:?}", at(4.0))
        };
        assert!(near(below, wide, 2e-3) && near(above, narrow, 3e-3));
        // The base and the top are flat too: nothing below or above them.
        assert_eq!(at(0.0)[0], 0.0);
        assert!(near(at(0.0)[1], wide, 2e-3));
        assert_eq!(at(8.0)[1], 0.0);
    }

    #[test]
    fn what_a_shell_encloses_is_not_seen() {
        // An open tube with a narrower one inside, and the same tube cut in two halves
        // round a sphere-ish lump: the outside alone counts.
        let mut triangles = tube(2.0, 0.0, 6.0, false, DVec2::ZERO);
        let alone = Body::new(&triangles);
        triangles.extend(tube(0.5, 1.0, 5.0, true, DVec2::new(0.8, 0.0)));
        let with = Body::new(&triangles);
        let middle = |b: &Body| {
            b.cuts
                .iter()
                .find(|c| (c.z - 3.0).abs() < 0.1)
                .unwrap()
                .clone()
        };
        let (a, b) = (middle(&alone), middle(&with));
        assert!((a.area - b.area).abs() < 1e-9 * a.area);
        assert!((a.perimeter - b.perimeter).abs() < 1e-9 * a.perimeter);
        assert!(with.encloses(DVec3::new(0.8, 0.0, 3.0)));
    }

    #[test]
    fn bodies_side_by_side_add_up() {
        // A core and two boosters clear of it along x.
        let mut triangles = tube(1.5, 0.0, 30.0, true, DVec2::ZERO);
        triangles.extend(tube(1.0, 2.0, 22.0, true, DVec2::new(3.0, 0.0)));
        triangles.extend(tube(1.0, 2.0, 22.0, true, DVec2::new(-3.0, 0.0)));
        let body = Body::new(&triangles);
        let cut = body.cuts.iter().find(|c| (c.z - 10.0).abs() < 0.3).unwrap();
        let area = polygon(1.5) + 2.0 * polygon(1.0);
        assert!(near(cut.area, area, 3e-3), "{} {area}", cut.area);
        let perimeter = TAU * (1.5 + 2.0);
        assert!(near(cut.perimeter, perimeter, 1e-2), "{}", cut.perimeter);
        // Across x the gaps between them are left out: 3 + 2 + 2 m; across y the core.
        assert!(
            near(cut.width.x, 7.0, 3e-3) && near(cut.width.y, 3.0, 3e-3),
            "{}",
            cut.width
        );
        assert!(cut.center.length() < 1e-3);
        assert!(near(cut.reach, 4.0, 3e-3), "{}", cut.reach);
        // One booster on its own puts the middle off the axis, towards it.
        let mut one = tube(1.5, 0.0, 30.0, true, DVec2::ZERO);
        one.extend(tube(1.0, 2.0, 22.0, true, DVec2::new(3.0, 0.0)));
        let body = Body::new(&one);
        let cut = body.cuts.iter().find(|c| (c.z - 10.0).abs() < 0.3).unwrap();
        let x = 3.0 * polygon(1.0) / (polygon(1.5) + polygon(1.0));
        assert!((cut.center.x - x).abs() < 5e-3 && cut.center.y.abs() < 1e-3);
    }

    #[test]
    fn halves_meeting_close_what_they_hold() {
        // Two half tubes, each its own surface, meeting along x: the space between them is
        // the body's, as a whole tube's would be.
        let half = |from: f64| {
            let sides = 36;
            let p = |k: usize, z: f64| {
                let a = from + PI * k as f64 / sides as f64;
                (DVec2::from_angle(a) * 2.0).extend(z)
            };
            let mut out = Vec::new();
            for k in 0..sides {
                let (a0, a1, b0, b1) = (p(k, 0.0), p(k + 1, 0.0), p(k, 5.0), p(k + 1, 5.0));
                out.extend([[a0, a1, b1], [a0, b1, b0]]);
            }
            out
        };
        let mut triangles = half(0.0);
        triangles.extend(half(PI));
        let body = Body::new(&triangles);
        let cut = &body.cuts[body.cuts.len() / 2];
        assert!(near(cut.area, polygon(2.0), 2e-3), "{}", cut.area);
        // One half alone holds nothing: a sliver.
        let body = Body::new(&half(0.0));
        assert!(body.cuts[body.cuts.len() / 2].area < 1e-2);
    }

    #[test]
    fn the_gaps_in_a_truss_are_outside() {
        // Eight thin struts on a circle.
        let mut triangles = Vec::new();
        for k in 0..8 {
            let at = DVec2::from_angle(TAU * k as f64 / 8.0) * 1.5;
            triangles.extend(tube(0.05, 0.0, 3.0, true, at));
        }
        let body = Body::new(&triangles);
        let cut = &body.cuts[body.cuts.len() / 2];
        assert!(near(cut.area, 8.0 * polygon(0.05), 0.1), "{}", cut.area);
        assert!(!body.encloses(DVec3::new(0.0, 0.0, 1.5)));
    }
}
