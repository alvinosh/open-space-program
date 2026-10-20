//! Root finding on the tick grid.

use crate::time::{SimTime, TICKS_PER_SECOND, Ticks};

/// Smallest step taken by [`first_crossing`]. A crossing shallower than
/// `rate_bound * MIN_STEP` could in principle be stepped over; at 1 ms and orbital speeds
/// that is metres against boundaries tens of thousands of kilometres wide.
pub const MIN_STEP: Ticks = Ticks::from_millis(1);

/// First tick in `(after, until]` where `g(t) <= 0`, given `g(after) > 0` and a bound on
/// how fast g can change (`|dg/dt| <= rate_bound`).
///
/// Uses conservative advancement: from a point where g > 0, g cannot reach zero sooner than
/// `g / rate_bound` seconds, so stepping by that amount can never jump over a crossing.
/// Steps are large far from the boundary and shrink near it. Once a sign change is
/// bracketed, bisection on whole ticks pins down the exact first tick.
///
/// The result depends only on `g`, not on where the search started, so callers that search
/// a long span in one go or in many small windows get the same tick.
pub fn first_crossing(
    g: impl Fn(SimTime) -> f64,
    after: SimTime,
    until: SimTime,
    rate_bound: f64,
) -> Option<SimTime> {
    let mut t = after;
    let mut value = g(t);
    debug_assert!(value > 0.0, "first_crossing must start outside the region");
    while t < until {
        let remaining = until.ticks_since(t);
        let safe = value / rate_bound * TICKS_PER_SECOND as f64;
        // `as` saturates, so huge safe steps are fine.
        let step = (safe as i64).max(MIN_STEP.0).min(remaining);
        let next = t.add_ticks(step);
        let next_value = g(next);
        if next_value <= 0.0 {
            return Some(bisect(&g, t, next));
        }
        t = next;
        value = next_value;
    }
    None
}

/// Given `g(lo) > 0` and `g(hi) <= 0`, the first tick in `(lo, hi]` with `g <= 0`.
fn bisect(g: &impl Fn(SimTime) -> f64, mut lo: SimTime, mut hi: SimTime) -> SimTime {
    while hi.ticks_since(lo) > 1 {
        let mid = lo.add_ticks(hi.ticks_since(lo) / 2);
        if g(mid) <= 0.0 {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_exact_tick_independent_of_window() {
        // Distance to a wall approached at 3 m/s, starting 1 km away.
        let start = SimTime::J2000;
        let g = |t: SimTime| 1_000.0 - 3.0 * t.seconds_since(start);
        let crossing = start.add_seconds_ceil(1_000.0 / 3.0);

        let one_window = first_crossing(g, start, start.add_seconds(10_000.0), 5.0).unwrap();
        assert_eq!(one_window, crossing);

        // Same search split into 7-second windows.
        let mut t = start;
        let found = loop {
            let end = t.add_seconds(7.0);
            if let Some(hit) = first_crossing(g, t, end, 5.0) {
                break hit;
            }
            t = end;
        };
        assert_eq!(found, crossing);
    }

    #[test]
    fn does_not_skip_a_brief_dip() {
        // g dips below zero for only ~2 ms around t = 500 s.
        let start = SimTime::J2000;
        let g = |t: SimTime| {
            let x = t.seconds_since(start) - 500.0;
            x * x * 1e6 - 1.0
        };
        let rate_bound = 2e6 * 600.0;
        let hit = first_crossing(g, start, start.add_seconds(1_000.0), rate_bound).unwrap();
        assert!((hit.seconds_since(start) - 499.999).abs() < 1e-5);
    }
}
