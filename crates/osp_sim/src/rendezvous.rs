//! Targets and rendezvous geometry: closest approaches between two trajectories, the nodes
//! where an orbit crosses a target's orbital plane, and the burn that matches planes.
//!
//! Closest approaches are the local minima of the distance between two motions, i.e. where
//! the range rate (r·v of the relative state) changes from closing to opening. The range
//! rate is sampled finely enough that two minima cannot share a sample interval, and each
//! sign change is then refined to the microsecond, so the reported time and distance do not
//! depend on the sampling.

use glam::DVec3;

use crate::{
    body::BodyId,
    orbit::{Orbit, StateVector},
    plan::Burn,
    time::SimTime,
    vessel::VesselId,
};

/// What a vessel is navigating towards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Body(BodyId),
    Vessel(VesselId),
}

/// A local minimum of the distance between two motions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Approach {
    pub time: SimTime,
    pub distance: f64,
    /// Relative speed there, m/s.
    pub speed: f64,
}

/// Closest approaches between two motions over `[from, to]`, at most `max` of them, in time
/// order. `relative(t)` is the second's state relative to the first. `step` (s) must be
/// shorter than the time between two minima, e.g. 1/64 of the shorter orbital period.
pub fn closest_approaches(
    relative: impl Fn(SimTime) -> StateVector,
    from: SimTime,
    to: SimTime,
    step: f64,
    max: usize,
) -> Vec<Approach> {
    let range_rate = |t: SimTime| {
        let s = relative(t);
        s.position.dot(s.velocity)
    };
    let span = to.seconds_since(from);
    if span <= 0.0 || step <= 0.0 || max == 0 {
        return Vec::new();
    }
    let steps = (span / step).ceil().max(1.0) as usize;
    let at = |k: usize| from.add_seconds(span * k as f64 / steps as f64);
    let mut found = Vec::new();
    let (mut t0, mut f0) = (from, range_rate(from));
    for k in 1..=steps {
        let t1 = at(k);
        let f1 = range_rate(t1);
        if f0 < 0.0 && f1 >= 0.0 {
            let time = refine(&range_rate, t0, f0, t1, f1);
            let s = relative(time);
            found.push(Approach {
                time,
                distance: s.position.length(),
                speed: s.velocity.length(),
            });
            if found.len() == max {
                break;
            }
        }
        (t0, f0) = (t1, f1);
    }
    found
}

/// Root of `f` in `[a, b]` where `f(a) < 0 ≤ f(b)`, to a microsecond (Illinois method,
/// which never leaves the bracket).
fn refine(
    f: &impl Fn(SimTime) -> f64,
    mut a: SimTime,
    mut fa: f64,
    mut b: SimTime,
    mut fb: f64,
) -> SimTime {
    let mut side = 0;
    for _ in 0..200 {
        let width = b.seconds_since(a);
        if width <= 1e-6 {
            break;
        }
        let share = (fa / (fa - fb)).clamp(0.01, 0.99);
        let t = a.add_seconds(width * share);
        let ft = f(t);
        if ft < 0.0 {
            (a, fa) = (t, ft);
            if side == -1 {
                fb /= 2.0;
            }
            side = -1;
        } else {
            (b, fb) = (t, ft);
            if side == 1 {
                fa /= 2.0;
            }
            side = 1;
        }
    }
    b
}

/// Where an orbit crosses a target's orbital plane, and how steeply.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaneNodes {
    /// Next crossing to the target normal's side (up through the plane), if it comes.
    pub ascending: Option<SimTime>,
    pub descending: Option<SimTime>,
    /// Angle between the two orbital planes, rad.
    pub inclination: f64,
}

/// Plane crossings of `orbit` after `after`, against a target plane with `normal`. `None`
/// when the planes are the same.
pub fn plane_nodes(orbit: &Orbit, normal: DVec3, after: SimTime) -> Option<PlaneNodes> {
    let own = orbit.normal();
    let inclination = own.angle_between(normal);
    let line = own.cross(normal).try_normalize()?;
    let nu = orbit.true_anomaly_of(line);
    let first = orbit.next_time_at_true_anomaly(nu, after);
    let second = orbit.next_time_at_true_anomaly(nu + std::f64::consts::PI, after);
    // At `line` the vessel moves up through the target plane if its velocity has a
    // component along the target's normal.
    let up_at_first = match (first, second) {
        (Some(t), _) => orbit.state_at(t).velocity.dot(normal) > 0.0,
        (None, Some(t)) => orbit.state_at(t).velocity.dot(normal) < 0.0,
        (None, None) => return None,
    };
    let (ascending, descending) = if up_at_first {
        (first, second)
    } else {
        (second, first)
    };
    Some(PlaneNodes {
        ascending,
        descending,
        inclination,
    })
}

/// A burn at `time` (on `orbit`, ideally at a plane node) that turns the velocity into the
/// plane with `normal`, keeping the speed.
pub fn plane_match(orbit: &Orbit, normal: DVec3, time: SimTime) -> Option<Burn> {
    let state = orbit.state_at(time);
    let n = normal.try_normalize()?;
    let along = state.velocity - n * state.velocity.dot(n);
    let wanted = along.try_normalize()? * state.velocity.length();
    Burn::from_inertial(time, &state, wanted - state.velocity)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MU: f64 = 3.986_004_418e14;

    fn circular(radius: f64, phase: f64, normal_tilt: f64) -> Orbit {
        let speed = (MU / radius).sqrt();
        let position = DVec3::new(phase.cos(), phase.sin(), 0.0) * radius;
        let along = DVec3::new(-phase.sin(), phase.cos(), 0.0);
        let tilt = glam::DQuat::from_axis_angle(position.normalize(), normal_tilt);
        Orbit::new(
            MU,
            SimTime::J2000,
            StateVector::new(position, tilt * along * speed),
        )
    }

    #[test]
    fn coplanar_circles_meet_at_conjunction() {
        // The target 20 km higher and 2° ahead: the chaser, faster below, catches up.
        let (r1, r2) = (6_771e3, 6_791e3);
        let chaser = circular(r1, 0.0, 0.0);
        let target = circular(r2, 2f64.to_radians(), 0.0);
        let rate = (MU / (r1 * r1 * r1)).sqrt() - (MU / (r2 * r2 * r2)).sqrt();
        let expected = 2f64.to_radians() / rate;
        let relative = |t: SimTime| target.state_at(t) - chaser.state_at(t);
        let period = chaser.period().unwrap();
        let found = closest_approaches(
            relative,
            SimTime::J2000,
            SimTime::J2000.add_seconds(2.0 * expected),
            period / 64.0,
            2,
        );
        assert_eq!(found.len(), 1, "{found:?}");
        let a = found[0];
        assert!((a.time.seconds_since(SimTime::J2000) - expected).abs() < 1e-3);
        assert!((a.distance - 20e3).abs() < 1e-3, "{}", a.distance);
    }

    #[test]
    fn refined_minimum_beats_the_samples() {
        // A crossing orbit: sampled distances miss the true minimum, the refined one doesn't.
        let target = circular(7_000e3, 0.3, 0.0);
        let speed = (MU / 6_900e3).sqrt() * 1.01;
        let chaser = Orbit::new(
            MU,
            SimTime::J2000,
            StateVector::new(DVec3::new(6_900e3, 0.0, 0.0), DVec3::new(0.0, speed, 50.0)),
        );
        let relative = |t: SimTime| target.state_at(t) - chaser.state_at(t);
        let period = chaser.period().unwrap();
        let end = SimTime::J2000.add_seconds(3.0 * period);
        let found = closest_approaches(relative, SimTime::J2000, end, period / 64.0, 8);
        assert!(!found.is_empty());
        for a in &found {
            // Distance is a minimum: slightly earlier and later are both further.
            let d = |dt: f64| relative(a.time.add_seconds(dt)).position.length();
            assert!(d(-0.01) >= a.distance && d(0.01) >= a.distance, "{a:?}");
        }
    }

    #[test]
    fn nodes_and_plane_match() {
        let target = circular(7_000e3, 0.0, 0.0);
        let chaser = circular(6_900e3, 0.5, 5f64.to_radians());
        let nodes = plane_nodes(&chaser, target.normal(), SimTime::J2000).unwrap();
        assert!((nodes.inclination - 5f64.to_radians()).abs() < 1e-9);
        let ascending = nodes.ascending.unwrap();
        let descending = nodes.descending.unwrap();
        // At a node the chaser is in the target's plane, moving up through it at the AN.
        for t in [ascending, descending] {
            let s = chaser.state_at(t);
            assert!(
                s.position.dot(target.normal()).abs() < 1e-3,
                "{}",
                s.position.z
            );
        }
        assert!(chaser.state_at(ascending).velocity.dot(target.normal()) > 0.0);
        let half = chaser.period().unwrap() / 2.0;
        assert!((descending.seconds_since(ascending).abs() - half).abs() < 1e-3);

        let burn = plane_match(&chaser, target.normal(), ascending).unwrap();
        let state = chaser.state_at(ascending);
        let after = Orbit::new(
            MU,
            ascending,
            StateVector::new(
                state.position,
                state.velocity + burn.inertial(&state).unwrap(),
            ),
        );
        assert!(after.normal().angle_between(target.normal()) < 1e-9);
        assert!((after.semi_major_axis() - chaser.semi_major_axis()).abs() < 1e-3);
        // Changing planes by 5° at 7.6 km/s costs 2 v sin(i/2).
        let speed = state.velocity.length();
        let cost = 2.0 * speed * (2.5f64.to_radians()).sin();
        assert!((burn.delta_v.length() - cost).abs() < 1e-6);
    }
}
