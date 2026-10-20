//! Trajectory events for coasting (gravity-only) motion.
//!
//! A coasting vessel follows an exact conic until one of these happens:
//! - it leaves its body's sphere of influence (solved analytically),
//! - it enters a child body's sphere of influence (conservative-advancement search),
//! - it descends below the body's *coast floor*, i.e. into the atmosphere or near terrain,
//!   where other forces apply and it must be integrated (solved analytically).
//!
//! Every event is resolved to the first tick at which the boundary has been crossed, and the
//! result does not depend on how the time span is split up. That is what makes warp
//! invariant: 1x, 100 000x and the map-view prediction all agree to the tick.

use crate::{
    body::BodyId,
    orbit::{Direction, Orbit},
    search::first_crossing,
    system::SolarSystem,
    time::SimTime,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// Leaving the current body's SOI into its parent's.
    SoiExit,
    /// Entering a child body's SOI.
    SoiEntry(BodyId),
    /// Descending below the coast floor (atmosphere or terrain proximity).
    Descent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrajectoryEvent {
    pub time: SimTime,
    pub kind: EventKind,
}

/// Earliest event in `(after, until]` for `orbit` around `frame`.
pub fn next_event(
    system: &SolarSystem,
    frame: BodyId,
    orbit: &Orbit,
    after: SimTime,
    until: SimTime,
) -> Option<TrajectoryEvent> {
    let body = system.body(frame);
    let mut best: Option<TrajectoryEvent> = None;
    keep_earliest(
        &mut best,
        orbit.next_radius_crossing(
            after,
            until,
            body.coast_floor(),
            Direction::Inbound,
            body.radius,
        ),
        EventKind::Descent,
    );
    if body.parent.is_some() {
        keep_earliest(
            &mut best,
            orbit.next_radius_crossing(after, until, body.soi, Direction::Outbound, body.radius),
            EventKind::SoiExit,
        );
    }
    for &child in &body.children {
        // Only search up to the best event so far: anything later is irrelevant.
        let limit = best.map_or(until, |b| b.time);
        let entry = soi_entry(system, frame, child, orbit, after, limit);
        keep_earliest(&mut best, entry, EventKind::SoiEntry(child));
    }
    best
}

fn keep_earliest(best: &mut Option<TrajectoryEvent>, time: Option<SimTime>, kind: EventKind) {
    if let Some(time) = time
        && best.is_none_or(|b| time < b.time)
    {
        *best = Some(TrajectoryEvent { time, kind });
    }
}

/// First tick in `(after, until]` at which `orbit` is inside `child`'s SOI.
fn soi_entry(
    system: &SolarSystem,
    frame: BodyId,
    child: BodyId,
    orbit: &Orbit,
    after: SimTime,
    until: SimTime,
) -> Option<SimTime> {
    if until <= after {
        return None;
    }
    let child_body = system.body(child);
    let child_orbit = child_body.orbit.as_ref()?;
    let soi = child_body.soi;

    // Radial reachability: the two radius ranges must come within one SOI of each other.
    let child_min = child_orbit.periapsis();
    let child_max = child_orbit.apoapsis().unwrap_or(f64::INFINITY);
    let vessel_max = orbit.apoapsis().unwrap_or(f64::INFINITY);
    if vessel_max < child_min - soi || orbit.periapsis() > child_max + soi {
        return None;
    }

    // |d/dt |r_v − r_c|| ≤ |v_v| + |v_c|, each bounded by its periapsis speed.
    let rate_bound =
        orbit.max_speed(system.body(frame).radius) + child_orbit.max_speed(child_body.radius);
    let separation = |t: SimTime| {
        let v = orbit.state_at(t);
        let c = child_orbit.state_at(t);
        (v - c, (v.position - c.position).length() - soi)
    };

    let (relative, gap) = separation(after);
    let mut start = after;
    if gap <= 0.0 {
        if relative.position.dot(relative.velocity) < 0.0 {
            // Inside and approaching: we are entering right now.
            return Some(after.add_ticks(1));
        }
        // Inside but receding (we just left this SOI): wait until clearly outside.
        start = first_crossing(
            |t| -separation(t).1 + f64::MIN_POSITIVE,
            after,
            until,
            rate_bound,
        )?;
    }
    first_crossing(|t| separation(t).1, start, until, rate_bound)
}

/// Re-express `orbit` (around `frame`) after a SOI event. Descent leaves it unchanged.
pub fn apply_event(
    system: &SolarSystem,
    frame: BodyId,
    orbit: &Orbit,
    event: TrajectoryEvent,
) -> (BodyId, Orbit) {
    let state = orbit.state_at(event.time);
    let target = match event.kind {
        EventKind::SoiExit => system.body(frame).parent.expect("SoiExit from root"),
        EventKind::SoiEntry(child) => child,
        EventKind::Descent => return (frame, *orbit),
    };
    let state = system.transform(state, frame, target, event.time);
    (
        target,
        Orbit::new(system.body(target).gm, event.time, state),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{orbit::StateVector, system::tests::rss};

    /// LEO state with a prograde burn that reaches the Moon's distance. Returns the orbit
    /// and the time the Moon's SOI is entered.
    pub(crate) fn translunar(system: &SolarSystem) -> (BodyId, Orbit) {
        let earth = system.find("Earth").unwrap();
        let moon = system.find("Moon").unwrap();
        let mu = system.body(earth).gm;
        let t0 = SimTime::J2000;
        // Aim apoapsis where the Moon will be ~5 days later: put periapsis opposite that.
        let moon_later = system
            .local_state(moon, t0.add_seconds(5.0 * 86_400.0))
            .position;
        let moon_normal = system.body(moon).orbit.unwrap().normal();
        let rp = system.body(earth).radius + 300e3;
        let towards = -moon_later.reject_from(moon_normal).normalize();
        let ra = moon_later.length();
        let a = (rp + ra) / 2.0;
        let vp = (mu * (2.0 / rp - 1.0 / a)).sqrt();
        let ahead = moon_normal.cross(towards);
        // Time of flight to apoapsis is half a period; start so we arrive with the Moon.
        let half = std::f64::consts::PI * (a * a * a / mu).sqrt();
        let start = t0.add_seconds(5.0 * 86_400.0 - half);
        let orbit = Orbit::new(mu, start, StateVector::new(towards * rp, ahead * vp));
        (earth, orbit)
    }

    #[test]
    fn finds_moon_encounter_independent_of_window() {
        let system = rss();
        let (earth, orbit) = translunar(&system);
        let start = orbit.epoch();
        let horizon = start.add_seconds(10.0 * 86_400.0);

        let one_shot = next_event(&system, earth, &orbit, start, horizon).unwrap();
        assert!(matches!(one_shot.kind, EventKind::SoiEntry(_)));

        // Chop the same span into 1-minute windows, like 1x warp would.
        let mut t = start;
        let chunked = loop {
            let end = t.add_seconds(60.0).min(horizon);
            if let Some(event) = next_event(&system, earth, &orbit, t, end) {
                break event;
            }
            t = end;
        };
        assert_eq!(one_shot, chunked);

        // The entry tick is the first one inside the SOI.
        let (moon, local) = apply_event(&system, earth, &orbit, one_shot);
        let soi = system.body(moon).soi;
        assert!(local.epoch_state().position.length() <= soi);
        let before = orbit.state_at(one_shot.time.add_ticks(-1)).position
            - system
                .local_state(moon, one_shot.time.add_ticks(-1))
                .position;
        assert!(before.length() > soi);
    }

    #[test]
    fn lunar_flyby_returns_to_earth_soi() {
        let system = rss();
        let (earth, orbit) = translunar(&system);
        let start = orbit.epoch();
        let entry = next_event(
            &system,
            earth,
            &orbit,
            start,
            start.add_seconds(30.0 * 86_400.0),
        )
        .unwrap();
        let (moon, moon_orbit) = apply_event(&system, earth, &orbit, entry);
        let exit = next_event(
            &system,
            moon,
            &moon_orbit,
            entry.time,
            entry.time.add_seconds(30.0 * 86_400.0),
        );
        // Either we hit the Moon (descent below its terrain margin) or fly by and leave.
        let exit = exit.expect("something must happen inside the Moon's SOI");
        if exit.kind == EventKind::SoiExit {
            let (back, after) = apply_event(&system, moon, &moon_orbit, exit);
            assert_eq!(back, earth);
            // Patching is exact: positions agree across the boundary.
            let a = system.transform(moon_orbit.state_at(exit.time), moon, earth, exit.time);
            assert!((a.position - after.epoch_state().position).length() < 1e-6);
        }
    }

    #[test]
    fn no_spurious_reentry_right_after_exit() {
        let system = rss();
        let earth = system.find("Earth").unwrap();
        let moon = system.find("Moon").unwrap();
        let t = SimTime::J2000;
        // Sitting exactly on the Moon's SOI boundary, moving away from the Moon.
        let moon_state = system.local_state(moon, t);
        let soi = system.body(moon).soi;
        let out = moon_state.position.normalize();
        let state = StateVector::new(
            moon_state.position + out * (soi + 1e-6),
            moon_state.velocity + out * 300.0,
        );
        let orbit = Orbit::new(system.body(earth).gm, t, state);
        let event = next_event(&system, earth, &orbit, t, t.add_seconds(3_600.0));
        assert!(!matches!(
            event.map(|e| e.kind),
            Some(EventKind::SoiEntry(_))
        ));
    }
}
