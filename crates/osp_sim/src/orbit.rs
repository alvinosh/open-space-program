//! Conic orbits anchored to an exact epoch state.
//!
//! An [`Orbit`] stores the state vector at an epoch and propagates it with universal
//! variables, so it is exact for every conic. Classical elements are derived for display
//! only: they are singular for circular, equatorial, parabolic or radial orbits, but the
//! propagation never touches them.

use std::f64::consts::TAU;
use std::ops::{Add, Sub};

use glam::{DQuat, DVec3};

use crate::{kepler, search::first_crossing, time::SimTime};

/// Position and velocity relative to some body, in inertial axes.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StateVector {
    pub position: DVec3,
    pub velocity: DVec3,
}

impl StateVector {
    pub const fn new(position: DVec3, velocity: DVec3) -> Self {
        Self { position, velocity }
    }

    /// Rate of change of the distance from the origin.
    pub fn radial_speed(&self) -> f64 {
        self.position.dot(self.velocity) / self.position.length()
    }
}

impl Add for StateVector {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.position + rhs.position, self.velocity + rhs.velocity)
    }
}

impl Sub for StateVector {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.position - rhs.position, self.velocity - rhs.velocity)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Radius decreasing through the threshold.
    Inbound,
    /// Radius increasing through the threshold.
    Outbound,
}

/// Classical Keplerian elements (radians, metres). Display and data-entry only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Elements {
    /// Infinite for parabolic, negative for hyperbolic orbits.
    pub semi_major_axis: f64,
    pub eccentricity: f64,
    pub inclination: f64,
    pub longitude_of_ascending_node: f64,
    pub argument_of_periapsis: f64,
    pub true_anomaly: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orbit {
    mu: f64,
    epoch: SimTime,
    state: StateVector,
    // Derived, cached.
    alpha: f64,
    period: Option<f64>,
    angular_momentum: DVec3,
    eccentricity: f64,
    semi_latus_rectum: f64,
    periapsis: f64,
    /// Perifocal basis: towards periapsis, 90° ahead in the plane, orbit normal.
    basis: [DVec3; 3],
}

impl Orbit {
    pub fn new(mu: f64, epoch: SimTime, state: StateVector) -> Self {
        let StateVector {
            position: r,
            velocity: v,
        } = state;
        let alpha = kepler::reciprocal_semi_major_axis(mu, r, v);
        let h = r.cross(v);
        let e_vec = ((v.length_squared() - mu / r.length()) * r - r.dot(v) * v) / mu;
        let e = e_vec.length();
        let p = h.length_squared() / mu;

        let normal = h.try_normalize().unwrap_or_else(|| any_perpendicular(r));
        let towards_periapsis = if e > 1e-10 {
            e_vec / e
        } else {
            // Circular: periapsis is undefined; measure anomalies from the epoch position.
            r.reject_from(normal).normalize()
        };
        let ahead = normal.cross(towards_periapsis);

        Self {
            mu,
            epoch,
            state,
            alpha,
            period: kepler::period(mu, alpha),
            angular_momentum: h,
            eccentricity: e,
            semi_latus_rectum: p,
            periapsis: p / (1.0 + e),
            basis: [towards_periapsis, ahead, normal],
        }
    }

    /// Orbit from classical elements, with `mean_anomaly` at `epoch`. Bound orbits only.
    pub fn from_elements(mu: f64, elements: &Elements, mean_anomaly: f64, epoch: SimTime) -> Self {
        let e = elements.eccentricity;
        let a = elements.semi_major_axis;
        assert!(
            e < 1.0 && a > 0.0,
            "from_elements supports bound orbits only"
        );
        let rotation = DQuat::from_rotation_z(elements.longitude_of_ascending_node)
            * DQuat::from_rotation_x(elements.inclination)
            * DQuat::from_rotation_z(elements.argument_of_periapsis);
        let rp = a * (1.0 - e);
        let at_periapsis = StateVector::new(
            rotation * DVec3::new(rp, 0.0, 0.0),
            rotation * DVec3::new(0.0, (mu * (1.0 + e) / rp).sqrt(), 0.0),
        );
        // Propagate from periapsis to the requested mean anomaly instead of solving
        // Kepler's equation in elements.
        let mean_motion = (mu / (a * a * a)).sqrt();
        let since_periapsis = kepler::wrap_pi(mean_anomaly) / mean_motion;
        let at_periapsis = Orbit::new(mu, epoch, at_periapsis);
        Orbit::new(mu, epoch, at_periapsis.state_at_offset(since_periapsis))
    }

    pub fn mu(&self) -> f64 {
        self.mu
    }

    pub fn epoch(&self) -> SimTime {
        self.epoch
    }

    pub fn epoch_state(&self) -> StateVector {
        self.state
    }

    pub fn state_at(&self, time: SimTime) -> StateVector {
        self.state_at_offset(time.seconds_since(self.epoch))
    }

    /// State `dt` seconds after the epoch (sub-tick resolution).
    pub fn state_at_offset(&self, dt: f64) -> StateVector {
        let (r, v) = kepler::propagate(
            self.mu,
            self.state.position,
            self.state.velocity,
            self.alpha,
            self.period,
            dt,
        );
        StateVector::new(r, v)
    }

    /// Same orbit, re-anchored at `time`.
    pub fn rebased(&self, time: SimTime) -> Orbit {
        Orbit::new(self.mu, time, self.state_at(time))
    }

    pub fn is_bound(&self) -> bool {
        self.alpha > 0.0
    }

    pub fn eccentricity(&self) -> f64 {
        self.eccentricity
    }

    /// Infinite for parabolic, negative for hyperbolic orbits.
    pub fn semi_major_axis(&self) -> f64 {
        1.0 / self.alpha
    }

    pub fn semi_latus_rectum(&self) -> f64 {
        self.semi_latus_rectum
    }

    pub fn periapsis(&self) -> f64 {
        self.periapsis
    }

    /// `None` for unbound orbits.
    pub fn apoapsis(&self) -> Option<f64> {
        self.is_bound()
            .then(|| self.semi_latus_rectum / (1.0 - self.eccentricity))
    }

    pub fn period(&self) -> Option<f64> {
        self.period
    }

    pub fn angular_momentum(&self) -> DVec3 {
        self.angular_momentum
    }

    pub fn normal(&self) -> DVec3 {
        self.basis[2]
    }

    /// Specific orbital energy, J/kg.
    pub fn energy(&self) -> f64 {
        -self.mu * self.alpha / 2.0
    }

    /// Angle between the orbit normal and `pole` (e.g. a body's rotation axis).
    pub fn inclination_to(&self, pole: DVec3) -> f64 {
        self.normal().dot(pole).clamp(-1.0, 1.0).acos()
    }

    /// True anomaly of a position on this orbit.
    pub fn true_anomaly_of(&self, position: DVec3) -> f64 {
        let [p, q, _] = self.basis;
        position.dot(q).atan2(position.dot(p))
    }

    /// Seconds since the most recent periapsis passage (negative before it on open orbits;
    /// in (−P/2, P/2] on closed ones).
    pub fn time_since_periapsis(&self, time: SimTime) -> f64 {
        let nu = self.true_anomaly_of(self.state_at(time).position);
        kepler::time_from_periapsis(self.mu, self.periapsis, self.eccentricity, nu)
    }

    /// The first time after `after` that the orbit passes true anomaly `nu`; `None` if an
    /// open orbit has already passed it.
    pub fn next_time_at_true_anomaly(&self, nu: f64, after: SimTime) -> Option<SimTime> {
        let now = self.time_since_periapsis(after);
        let then = kepler::time_from_periapsis(
            self.mu,
            self.periapsis,
            self.eccentricity,
            kepler::wrap_pi(nu),
        );
        let dt = match self.period {
            Some(period) => (then - now).rem_euclid(period),
            None if then >= now => then - now,
            None => return None,
        };
        Some(after.add_seconds(dt))
    }

    /// The first time after `after` that the orbit passes closest to `position`.
    pub fn next_time_at(&self, position: DVec3, after: SimTime) -> Option<SimTime> {
        self.next_time_at_true_anomaly(self.true_anomaly_of(position), after)
    }

    /// Upper bound on speed anywhere on the orbit with radius ≥ `r_floor` (vis-viva is
    /// largest at the smallest radius).
    pub fn max_speed(&self, r_floor: f64) -> f64 {
        let r = self.periapsis.max(r_floor).max(1.0);
        (self.mu * (2.0 / r - self.alpha)).max(0.0).sqrt()
    }

    /// Display elements at the epoch.
    pub fn elements(&self) -> Elements {
        let [p, _, n] = self.basis;
        let node = DVec3::Z.cross(n);
        let (lan, node_dir) = match node.try_normalize() {
            Some(dir) => (node.y.atan2(node.x).rem_euclid(TAU), dir),
            None => (0.0, DVec3::X),
        };
        let argp = n
            .dot(node_dir.cross(p))
            .atan2(node_dir.dot(p))
            .rem_euclid(TAU);
        Elements {
            semi_major_axis: self.semi_major_axis(),
            eccentricity: self.eccentricity,
            inclination: n.z.clamp(-1.0, 1.0).acos(),
            longitude_of_ascending_node: lan,
            argument_of_periapsis: argp,
            true_anomaly: self.true_anomaly_of(self.state.position),
        }
    }

    /// First tick in `(after, until]` at which the orbit has crossed `radius` in
    /// `direction`. `r_floor` is the body's surface radius (bounds speed for the
    /// near-radial fallback).
    ///
    /// Analytic for ordinary conics, so grazing passes are never missed and the result is
    /// independent of the search window. At the returned tick the state is already on the
    /// far side of the threshold.
    pub fn next_radius_crossing(
        &self,
        after: SimTime,
        until: SimTime,
        radius: f64,
        direction: Direction,
        r_floor: f64,
    ) -> Option<SimTime> {
        if until <= after || radius <= self.periapsis {
            return None;
        }
        if self.apoapsis().is_some_and(|ra| radius >= ra) {
            return None;
        }
        if self.periapsis < 1e-3 * r_floor {
            return self.radial_crossing(after, until, radius, direction, r_floor);
        }

        let now = self.state_at(after);
        let past = |r: f64| match direction {
            Direction::Inbound => r <= radius,
            Direction::Outbound => r >= radius,
        };
        let mut dt = self.time_to_radius(now.position, radius, direction)?;
        // Already on the far side (e.g. we just crossed): the next crossing in this
        // direction must come after one in the opposite direction.
        if past(now.position.length()) {
            let opposite = match direction {
                Direction::Inbound => Direction::Outbound,
                Direction::Outbound => Direction::Inbound,
            };
            let to_opposite = self.time_to_radius(now.position, radius, opposite);
            if to_opposite.is_none_or(|o| dt <= o) {
                dt += self.period?;
            }
        }

        // Polish with Newton on r(t) − R; the anomaly formulas lose a little precision
        // where the conic is nearly tangent to the sphere.
        let base = after.seconds_since(self.epoch);
        for _ in 0..3 {
            let s = self.state_at_offset(base + dt);
            let rate = s.radial_speed();
            if rate.abs() < 1e-9 {
                break;
            }
            let correction = (s.position.length() - radius) / rate;
            if correction.abs() > 1.0 || !correction.is_finite() {
                break;
            }
            dt -= correction;
        }

        let mut hit = after.add_seconds_ceil(dt).max(after.add_ticks(1));
        // Rounding can leave us a hair short of the boundary on near-tangent passes.
        for _ in 0..8 {
            if past(self.state_at(hit).position.length()) {
                break;
            }
            hit = hit.add_ticks(1);
        }
        (hit <= until).then_some(hit)
    }

    /// Seconds from `position` to the next crossing of `radius` in `direction`, assuming the
    /// radius is between periapsis and apoapsis.
    fn time_to_radius(&self, position: DVec3, radius: f64, direction: Direction) -> Option<f64> {
        let e = self.eccentricity;
        if e == 0.0 {
            return None;
        }
        let cos_nu = ((self.semi_latus_rectum / radius - 1.0) / e).clamp(-1.0, 1.0);
        let nu = match direction {
            Direction::Outbound => cos_nu.acos(),
            Direction::Inbound => -cos_nu.acos(),
        };
        let target = kepler::time_from_periapsis(self.mu, self.periapsis, e, nu);
        let now =
            kepler::time_from_periapsis(self.mu, self.periapsis, e, self.true_anomaly_of(position));
        let dt = target - now;
        match self.period {
            Some(period) => Some(dt.rem_euclid(period)),
            None => (dt > 0.0).then_some(dt),
        }
    }

    /// Crossing search for (near-)radial trajectories, where anomalies are degenerate.
    fn radial_crossing(
        &self,
        after: SimTime,
        until: SimTime,
        radius: f64,
        direction: Direction,
        r_floor: f64,
    ) -> Option<SimTime> {
        let sign = match direction {
            Direction::Inbound => 1.0,
            Direction::Outbound => -1.0,
        };
        let g = |t: SimTime| sign * (self.state_at(t).position.length() - radius);
        let bound = self.max_speed(r_floor);
        let mut start = after;
        if g(start) <= 0.0 {
            // On the far side already: wait until we come back across, then search.
            start = first_crossing(|t| -g(t) + f64::MIN_POSITIVE, after, until, bound)?;
        }
        first_crossing(g, start, until, bound)
    }

    /// Points along the orbit between two times, relative to the orbited body, for drawing
    /// it as a line: the first and last are exactly where the orbit is at those times, and no
    /// chord between neighbours strays from the curve by more than `tolerance` at that point
    /// (see [`Self::sample_true_anomaly`]).
    pub fn sample_between(
        &self,
        from: SimTime,
        to: SimTime,
        tolerance: impl Fn(DVec3) -> f64,
    ) -> Vec<DVec3> {
        if self.semi_latus_rectum < 1e-6 * self.state.position.length() {
            // Radial: a straight line; sample in time instead.
            return (0..=MIN_SEGMENTS)
                .map(|i| {
                    let f = i as f64 / MIN_SEGMENTS as f64;
                    let dt = to.seconds_since(from) * f;
                    self.state_at(from.add_seconds(dt)).position
                })
                .collect();
        }
        let nu0 = self.true_anomaly_of(self.state_at(from).position);
        let mut nu1 = self.true_anomaly_of(self.state_at(to).position);
        let full = self.period.is_some_and(|p| to.seconds_since(from) >= p);
        if full {
            nu1 = nu0 + TAU;
        } else if nu1 < nu0 {
            nu1 += TAU;
        }
        self.sample_true_anomaly(nu0, nu1, &tolerance)
    }

    /// The whole closed orbit, starting and ending where it is at `through`, so that a line
    /// drawn through the points passes exactly through the orbiting object. An open orbit is
    /// clipped to `max_radius`, with a point at `through` if that lies within the clip.
    /// `tolerance` as for [`Self::sample_between`].
    pub fn sample_full(
        &self,
        through: SimTime,
        max_radius: f64,
        tolerance: impl Fn(DVec3) -> f64,
    ) -> Vec<DVec3> {
        let now = self.true_anomaly_of(self.state_at(through).position);
        if self.is_bound() {
            return self.sample_true_anomaly(now, now + TAU, &tolerance);
        }
        let e = self.eccentricity;
        let asymptote = (-1.0 / e).clamp(-1.0, 1.0).acos() - 1e-3;
        let clip = ((self.semi_latus_rectum / max_radius - 1.0) / e)
            .clamp(-1.0, 1.0)
            .acos();
        let limit = asymptote.min(clip);
        let now = now.clamp(-limit, limit);
        let mut points = self.sample_true_anomaly(-limit, now, &tolerance);
        points.pop();
        points.extend(self.sample_true_anomaly(now, limit, &tolerance));
        points
    }

    /// Position on the orbit at true anomaly `nu`, relative to the orbited body.
    fn point_at_true_anomaly(&self, nu: f64) -> DVec3 {
        let [p, q, _] = self.basis;
        let r = self.semi_latus_rectum / (1.0 + self.eccentricity * nu.cos());
        r * (nu.cos() * p + nu.sin() * q)
    }

    /// Points from true anomaly `from` to `to`, both included. Starts from even steps and
    /// halves each one while the curve at its middle is further from the chord than the
    /// smallest `tolerance(point)` at its ends and middle, so the line is only as dense as it
    /// needs to be where it is seen: a tolerance proportional to the distance from the camera
    /// keeps every chord within a fixed angle, whether the camera is next to a planet or
    /// across its orbit.
    fn sample_true_anomaly(
        &self,
        from: f64,
        to: f64,
        tolerance: &impl Fn(DVec3) -> f64,
    ) -> Vec<DVec3> {
        let sample = |nu: f64| {
            let position = self.point_at_true_anomaly(nu);
            Sample {
                nu,
                position,
                tolerance: tolerance(position),
            }
        };
        let mut a = sample(from);
        let mut points = vec![a.position];
        for i in 1..=MIN_SEGMENTS {
            let b = sample(from + (to - from) * i as f64 / MIN_SEGMENTS as f64);
            refine(&sample, a, b, MAX_SUBDIVISIONS, &mut points);
            a = b;
        }
        points
    }
}

#[derive(Clone, Copy)]
struct Sample {
    nu: f64,
    position: DVec3,
    tolerance: f64,
}

/// Append the points after `a` up to and including `b`.
fn refine(sample: &impl Fn(f64) -> Sample, a: Sample, b: Sample, depth: u32, out: &mut Vec<DVec3>) {
    let mid = sample(0.5 * (a.nu + b.nu));
    let tolerance = mid.tolerance.min(a.tolerance).min(b.tolerance);
    if depth > 0 && mid.position.distance(0.5 * (a.position + b.position)) > tolerance {
        refine(sample, a, mid, depth - 1, out);
        refine(sample, mid, b, depth - 1, out);
    } else {
        out.push(b.position);
    }
}

/// Even steps in true anomaly that sampling starts from. Each covers at most 1/64 of a turn,
/// over which a conic bends one way, so its middle shows how far the chord strays.
const MIN_SEGMENTS: usize = 64;
/// Halvings allowed per starting step, which bounds the work where the camera sits on a line:
/// down to 1/4 194 304 of a turn, chords that stray 4 cm from Earth's orbit.
const MAX_SUBDIVISIONS: u32 = 16;

fn any_perpendicular(v: DVec3) -> DVec3 {
    v.any_orthonormal_vector()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MU: f64 = 3.986_004_418e14;
    const R_EARTH: f64 = 6.371e6;

    fn leo_ellipse() -> Orbit {
        // 200 x 1000 km, starting at periapsis.
        let rp = R_EARTH + 200e3;
        let ra = R_EARTH + 1_000e3;
        let a = (rp + ra) / 2.0;
        let vp = (MU * (2.0 / rp - 1.0 / a)).sqrt();
        Orbit::new(
            MU,
            SimTime::J2000,
            StateVector::new(
                DVec3::new(rp, 0.0, 0.0),
                DVec3::new(0.0, vp * 0.8, vp * 0.6),
            ),
        )
    }

    #[test]
    fn apsides_and_period() {
        let o = leo_ellipse();
        assert!((o.periapsis() - (R_EARTH + 200e3)).abs() < 1e-3);
        assert!((o.apoapsis().unwrap() - (R_EARTH + 1_000e3)).abs() < 1e-3);
        let p = o.period().unwrap();
        let back = o.state_at(SimTime::J2000.add_seconds(p * 1_000.0));
        assert!((back.position - o.epoch_state().position).length() < 1e-2);
    }

    #[test]
    fn elements_round_trip() {
        let elements = Elements {
            semi_major_axis: 2.4e7,
            eccentricity: 0.73,
            inclination: 0.5,
            longitude_of_ascending_node: 1.1,
            argument_of_periapsis: 4.0,
            true_anomaly: 0.0,
        };
        let o = Orbit::from_elements(MU, &elements, 0.3, SimTime::J2000);
        let got = o.elements();
        assert!((got.semi_major_axis - 2.4e7).abs() < 1e-3);
        assert!((got.eccentricity - 0.73).abs() < 1e-12);
        assert!((got.inclination - 0.5).abs() < 1e-12);
        assert!((got.longitude_of_ascending_node - 1.1).abs() < 1e-12);
        assert!((got.argument_of_periapsis - 4.0).abs() < 1e-12);
    }

    #[test]
    fn radius_crossings_are_exact() {
        let o = leo_ellipse();
        let t0 = SimTime::J2000;
        let horizon = t0.add_seconds(1e6);
        let r = R_EARTH + 600e3;
        let out = o
            .next_radius_crossing(t0, horizon, r, Direction::Outbound, R_EARTH)
            .unwrap();
        let inn = o
            .next_radius_crossing(t0, horizon, r, Direction::Inbound, R_EARTH)
            .unwrap();
        assert!(out < inn);
        for (t, dir) in [(out, Direction::Outbound), (inn, Direction::Inbound)] {
            let s = o.state_at(t);
            let before = o.state_at(t.add_ticks(-1)).position.length();
            let after = s.position.length();
            // The boundary lies between the previous tick and this one.
            match dir {
                Direction::Outbound => assert!(before < r && after >= r),
                Direction::Inbound => assert!(before > r && after <= r),
            }
        }
        // Starting exactly at a crossing finds the *next* one, a full period later.
        let again = o
            .next_radius_crossing(out, horizon, r, Direction::Outbound, R_EARTH)
            .unwrap();
        let p = o.period().unwrap();
        assert!((again.seconds_since(out) - p).abs() < 1e-5);
        // Unreachable radii.
        assert!(
            o.next_radius_crossing(t0, horizon, R_EARTH, Direction::Inbound, R_EARTH)
                .is_none()
        );
        assert!(
            o.next_radius_crossing(t0, horizon, 1e8, Direction::Outbound, R_EARTH)
                .is_none()
        );
    }

    #[test]
    fn grazing_crossing_is_found() {
        // Periapsis 1 m below a 140 km atmosphere boundary.
        let boundary = R_EARTH + 140e3;
        let rp = boundary - 1.0;
        let ra = R_EARTH + 400e3;
        let a = (rp + ra) / 2.0;
        let vp = (MU * (2.0 / rp - 1.0 / a)).sqrt();
        let o = Orbit::new(
            MU,
            SimTime::J2000,
            StateVector::new(DVec3::new(-rp, 0.0, 0.0), DVec3::new(0.0, -vp, 0.0)),
        );
        let start = SimTime::J2000.add_seconds(100.0);
        let hit = o
            .next_radius_crossing(
                start,
                start.add_seconds(1e5),
                boundary,
                Direction::Inbound,
                R_EARTH,
            )
            .unwrap();
        let p = o.period().unwrap();
        let expected = p - 100.0; // next periapsis, minus the brief dip before it
        assert!((hit.seconds_since(start) - expected).abs() < 30.0);
        assert!(o.state_at(hit).position.length() <= boundary);
    }

    #[test]
    fn hyperbolic_soi_exit() {
        let r = R_EARTH + 300e3;
        let v = (2.0 * MU / r).sqrt() + 500.0;
        let o = Orbit::new(
            MU,
            SimTime::J2000,
            StateVector::new(DVec3::new(r, 0.0, 0.0), DVec3::new(0.0, v, 0.0)),
        );
        let soi = 9.24e8;
        let t = o
            .next_radius_crossing(
                SimTime::J2000,
                SimTime::J2000.add_seconds(1e8),
                soi,
                Direction::Outbound,
                R_EARTH,
            )
            .unwrap();
        let r_at = o.state_at(t).position.length();
        assert!(r_at >= soi && r_at - soi < 0.01, "{}", r_at - soi);
        // Never comes back.
        assert!(
            o.next_radius_crossing(
                t,
                SimTime::J2000.add_seconds(1e9),
                soi,
                Direction::Inbound,
                R_EARTH
            )
            .is_none()
        );
    }

    #[test]
    fn radial_fall_hits_surface() {
        let r0 = R_EARTH + 1_000e3;
        let o = Orbit::new(
            MU,
            SimTime::J2000,
            StateVector::new(DVec3::new(0.0, r0, 0.0), DVec3::ZERO),
        );
        let t = o
            .next_radius_crossing(
                SimTime::J2000,
                SimTime::J2000.add_seconds(1e4),
                R_EARTH,
                Direction::Inbound,
                R_EARTH,
            )
            .unwrap();
        let r = o.state_at(t).position.length();
        assert!(r <= R_EARTH && R_EARTH - r < 0.01, "{}", R_EARTH - r);
    }

    /// Earth's orbit around the Sun, Earth at its position at J2000 + 6 h.
    fn heliocentric_earth() -> (Orbit, SimTime) {
        let elements = Elements {
            semi_major_axis: 1.495_98e11,
            eccentricity: 0.0167,
            inclination: 0.0,
            longitude_of_ascending_node: 0.0,
            argument_of_periapsis: 1.79,
            true_anomaly: 0.0,
        };
        let orbit = Orbit::from_elements(1.327_124_4e20, &elements, -0.04, SimTime::J2000);
        (orbit, SimTime::from_seconds(21_600.0))
    }

    fn distance_to_line(points: &[DVec3], p: DVec3) -> f64 {
        points
            .windows(2)
            .map(|w| {
                let ab = w[1] - w[0];
                let s = ((p - w[0]).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
                (w[0] + ab * s).distance(p)
            })
            .fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn drawn_orbit_passes_through_the_body() {
        let (orbit, now) = heliocentric_earth();
        // Seen from a camera 25 000 km from Earth, half a pixel is ~12 km there.
        let here = orbit.state_at(now).position;
        let camera = here + DVec3::new(0.0, 0.0, 2.5e7);
        let points = orbit.sample_full(now, f64::INFINITY, |p| p.distance(camera) * 4.8e-4);
        assert!(
            points[0].distance(here) < 1e-3,
            "{}",
            points[0].distance(here)
        );
        assert!(points.last().unwrap().distance(here) < 1e-3);
        // Smooth near the camera: the curve a quarter of a degree either side of Earth
        // is on the line to within the tolerance there (the chord's middle is tested, and
        // over so short an arc the curve bends evenly, so that is where it strays most).
        let nu = orbit.true_anomaly_of(here);
        for k in -10..=10 {
            let p = orbit.point_at_true_anomaly(nu + k as f64 * 4e-4);
            let miss = distance_to_line(&points, p);
            assert!(miss < 1.01 * p.distance(camera) * 4.8e-4, "{k}: {miss}");
        }
        // Dense only where it is seen: a few hundred points, where 256 even ones missed
        // Earth by ~6000 km.
        assert!(points.len() < 600, "{}", points.len());
    }

    #[test]
    fn time_at_a_point_on_the_orbit() {
        let o = leo_ellipse();
        let t0 = SimTime::J2000.add_seconds(1_234.5);
        let period = o.period().unwrap();
        for f in [0.1, 0.37, 0.5, 0.93] {
            let t = t0.add_seconds(period * f);
            let found = o.next_time_at(o.state_at(t).position, t0).unwrap();
            assert!(
                found.seconds_since(t).abs() < 1e-3,
                "{f}: {}",
                found.seconds_since(t)
            );
        }
        // Open orbits do not come back to points already passed.
        let r = R_EARTH + 300e3;
        let v = (2.0 * MU / r).sqrt() + 500.0;
        let h = Orbit::new(
            MU,
            SimTime::J2000,
            StateVector::new(DVec3::new(r, 0.0, 0.0), DVec3::new(0.0, v, 0.0)),
        );
        let later = SimTime::J2000.add_seconds(3_000.0);
        let ahead = h.state_at(later.add_seconds(500.0)).position;
        let found = h.next_time_at(ahead, later).unwrap();
        assert!(found.seconds_since(later.add_seconds(500.0)).abs() < 1e-3);
        assert!(
            h.next_time_at(h.state_at(SimTime::J2000).position, later)
                .is_none()
        );
    }

    #[test]
    fn sampling_follows_the_tolerance() {
        let o = leo_ellipse();
        let t0 = SimTime::J2000;
        let t1 = t0.add_seconds(o.period().unwrap() * 0.7);
        let points = o.sample_between(t0, t1, |_| 10.0);
        assert!(points[0].distance(o.state_at(t0).position) < 1e-6);
        assert!(points.last().unwrap().distance(o.state_at(t1).position) < 1e-3);
        for i in 0..=1_000 {
            let t = t0.add_seconds(t1.seconds_since(t0) * i as f64 / 1_000.0);
            let miss = distance_to_line(&points, o.state_at(t).position);
            assert!(miss < 10.1, "{miss}");
        }
        let coarse = o.sample_between(t0, t1, |_| 1e3).len();
        assert!(coarse < points.len(), "{coarse} {}", points.len());
    }
}
