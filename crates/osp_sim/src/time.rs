//! Simulation time.
//!
//! Time is an integer count of microseconds since the J2000 epoch (2000-01-01 12:00 TDB).
//! Integer time never drifts, orders totally, and makes every event land on an exact,
//! reproducible tick regardless of frame rate or warp. Differences between two instants
//! convert to `f64` seconds exactly for spans up to ~285 years, which is what the orbital
//! math consumes.

use std::ops::{Add, Sub};

use serde::{Deserialize, Serialize};

pub const TICKS_PER_SECOND: i64 = 1_000_000;
const SECONDS_PER_TICK: f64 = 1e-6;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct SimTime(i64);

impl SimTime {
    pub const J2000: SimTime = SimTime(0);

    pub const fn from_ticks(ticks: i64) -> Self {
        Self(ticks)
    }

    pub const fn ticks(self) -> i64 {
        self.0
    }

    /// Nearest tick to `seconds` after J2000.
    pub fn from_seconds(seconds: f64) -> Self {
        Self((seconds * TICKS_PER_SECOND as f64).round() as i64)
    }

    /// Seconds since J2000. Lossy far from the epoch; use [`Self::seconds_since`] for math.
    pub fn as_seconds(self) -> f64 {
        self.0 as f64 * SECONDS_PER_TICK
    }

    /// Exact (to f64 rounding) number of seconds from `earlier` to `self`.
    pub fn seconds_since(self, earlier: SimTime) -> f64 {
        (self.0 - earlier.0) as f64 * SECONDS_PER_TICK
    }

    /// `self + seconds`, rounded to the nearest tick.
    pub fn add_seconds(self, seconds: f64) -> Self {
        Self(self.0 + (seconds * TICKS_PER_SECOND as f64).round() as i64)
    }

    /// `self + seconds`, rounded up to the next tick: the first representable instant at or
    /// after the exact time. Events use this so the state at the event tick is always on
    /// the far side of the boundary that triggered it.
    pub fn add_seconds_ceil(self, seconds: f64) -> Self {
        Self(self.0 + (seconds * TICKS_PER_SECOND as f64).ceil() as i64)
    }

    pub const fn add_ticks(self, ticks: i64) -> Self {
        Self(self.0 + ticks)
    }

    pub const fn ticks_since(self, earlier: SimTime) -> i64 {
        self.0 - earlier.0
    }
}

/// A span of simulation time in ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Ticks(pub i64);

impl Ticks {
    pub const fn from_millis(ms: i64) -> Self {
        Self(ms * 1_000)
    }

    pub fn as_seconds(self) -> f64 {
        self.0 as f64 * SECONDS_PER_TICK
    }
}

impl Add<Ticks> for SimTime {
    type Output = SimTime;
    fn add(self, rhs: Ticks) -> SimTime {
        SimTime(self.0 + rhs.0)
    }
}

impl Sub<Ticks> for SimTime {
    type Output = SimTime;
    fn sub(self, rhs: Ticks) -> SimTime {
        SimTime(self.0 - rhs.0)
    }
}

impl Sub for SimTime {
    type Output = Ticks;
    fn sub(self, rhs: SimTime) -> Ticks {
        Ticks(self.0 - rhs.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn differences_are_exact_far_from_epoch() {
        // 250 years after J2000, a 20 ms step is still represented exactly.
        let far = SimTime::from_seconds(250.0 * 365.25 * 86_400.0);
        let next = far + Ticks::from_millis(20);
        assert_eq!(next.seconds_since(far), 0.02);
        assert_eq!(next - far, Ticks(20_000));
    }

    #[test]
    fn repeated_steps_do_not_drift() {
        let mut t = SimTime::J2000;
        for _ in 0..1_000_000 {
            t = t + Ticks::from_millis(20);
        }
        assert_eq!(t.seconds_since(SimTime::J2000), 20_000.0);
    }

    #[test]
    fn ceil_rounds_up() {
        let t = SimTime::J2000;
        assert_eq!(t.add_seconds_ceil(1e-7).ticks(), 1);
        assert_eq!(t.add_seconds_ceil(1e-6).ticks(), 1);
        assert_eq!(t.add_seconds(1.4e-6).ticks(), 1);
    }
}
