//! Patched-conic trajectory prediction.
//!
//! Built from the same [`next_event`] / [`apply_event`] the simulation uses, so a predicted
//! encounter is exactly what will happen when time actually passes, at any warp. Planned
//! burns ([`crate::plan`]) are applied as impulses at their time, in the orbit frame of the
//! trajectory as it is then.

use glam::DVec3;

use crate::{
    body::BodyId,
    events::{EventKind, TrajectoryEvent, apply_event, next_event},
    orbit::{Orbit, StateVector},
    plan::Burn,
    system::SolarSystem,
    time::SimTime,
};

/// One conic segment of a predicted trajectory.
#[derive(Debug, Clone)]
pub struct Patch {
    pub frame: BodyId,
    pub orbit: Orbit,
    pub start: SimTime,
    pub end: SimTime,
    /// What ends this patch; `None` if it runs to the prediction horizon.
    pub ending: Option<PatchEnd>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PatchEnd {
    Event(TrajectoryEvent),
    /// Planned burn `index` (into the burns passed to [`predict`]), giving `delta_v`
    /// (inertial, m/s) to the vessel at `state` (relative to the patch's body).
    Burn {
        index: usize,
        state: StateVector,
        delta_v: DVec3,
    },
}

impl PatchEnd {
    pub fn time(&self, patch: &Patch) -> SimTime {
        match self {
            PatchEnd::Event(event) => event.time,
            PatchEnd::Burn { .. } => patch.end,
        }
    }

    pub fn event(&self) -> Option<TrajectoryEvent> {
        match self {
            PatchEnd::Event(event) => Some(*event),
            PatchEnd::Burn { .. } => None,
        }
    }
}

/// The trajectory from `orbit` (relative to `frame`) at `from`, through `burns` (in time
/// order; those at or before `from` are skipped), up to `horizon` or `max_patches`. Stops
/// at a descent into an atmosphere or towards terrain, where it can no longer be predicted
/// exactly; later burns are then left out.
pub fn predict(
    system: &SolarSystem,
    frame: BodyId,
    orbit: Orbit,
    from: SimTime,
    horizon: SimTime,
    max_patches: usize,
    burns: &[Burn],
) -> Vec<Patch> {
    let mut patches = Vec::new();
    let (mut frame, mut orbit, mut start) = (frame, orbit, from);
    let mut next_burn = burns.partition_point(|b| b.time <= from);
    while patches.len() < max_patches && start < horizon {
        let burn = burns.get(next_burn).filter(|b| b.time < horizon);
        let until = burn.map_or(horizon, |b| b.time);
        match next_event(system, frame, &orbit, start, until) {
            Some(event) => {
                patches.push(Patch {
                    frame,
                    orbit,
                    start,
                    end: event.time,
                    ending: Some(PatchEnd::Event(event)),
                });
                if event.kind == EventKind::Descent {
                    break;
                }
                (frame, orbit) = apply_event(system, frame, &orbit, event);
                start = event.time;
            }
            None => {
                let Some(burn) = burn else {
                    patches.push(Patch {
                        frame,
                        orbit,
                        start,
                        end: horizon,
                        ending: None,
                    });
                    break;
                };
                let state = orbit.state_at(burn.time);
                let delta_v = burn.inertial(&state).unwrap_or(DVec3::ZERO);
                patches.push(Patch {
                    frame,
                    orbit,
                    start,
                    end: burn.time,
                    ending: Some(PatchEnd::Burn {
                        index: next_burn,
                        state,
                        delta_v,
                    }),
                });
                let after = StateVector::new(state.position, state.velocity + delta_v);
                orbit = Orbit::new(orbit.mu(), burn.time, after);
                start = burn.time;
                next_burn += 1;
            }
        }
    }
    patches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::tests::rss;

    /// A circular orbit 300 km above Earth, and Earth's id.
    fn leo(system: &SolarSystem) -> (BodyId, Orbit, f64) {
        let earth = system.find("Earth").unwrap();
        let body = system.body(earth);
        let r = body.radius + 300e3;
        let v = (body.gm / r).sqrt();
        let state = StateVector::new(DVec3::new(r, 0.0, 0.0), DVec3::new(0.0, v, 0.0));
        (earth, Orbit::new(body.gm, SimTime::J2000, state), r)
    }

    #[test]
    fn hohmann_transfer_through_two_burns() {
        let system = rss();
        let (earth, orbit, r1) = leo(&system);
        let mu = orbit.mu();
        let r2 = 42_164e3;
        let a = (r1 + r2) / 2.0;
        let dv1 = (mu / r1).sqrt() * ((2.0 * r2 / (r1 + r2)).sqrt() - 1.0);
        let dv2 = (mu / r2).sqrt() * (1.0 - (2.0 * r1 / (r1 + r2)).sqrt());
        let t1 = SimTime::J2000.add_seconds(600.0);
        let transfer = std::f64::consts::PI * (a * a * a / mu).sqrt();
        let t2 = t1.add_seconds(transfer);
        let burns = [
            Burn {
                time: t1,
                delta_v: DVec3::new(dv1, 0.0, 0.0),
            },
            Burn {
                time: t2,
                delta_v: DVec3::new(dv2, 0.0, 0.0),
            },
        ];
        let horizon = t2.add_seconds(86_400.0);
        let patches = predict(&system, earth, orbit, SimTime::J2000, horizon, 8, &burns);
        assert_eq!(patches.len(), 3);
        assert!(matches!(
            patches[0].ending,
            Some(PatchEnd::Burn { index: 0, .. })
        ));
        let apoapsis = patches[1].orbit.apoapsis().unwrap();
        assert!((apoapsis - r2).abs() < 1.0, "{}", apoapsis - r2);
        let geo = &patches[2].orbit;
        assert!(geo.eccentricity() < 1e-6, "{}", geo.eccentricity());
        assert!((geo.semi_major_axis() - r2).abs() < 10.0);
        assert!(patches[2].ending.is_none());
    }

    #[test]
    fn past_burns_are_skipped_and_order_is_kept() {
        let system = rss();
        let (earth, orbit, _) = leo(&system);
        let now = SimTime::J2000.add_seconds(1_000.0);
        let burns = [
            Burn {
                time: SimTime::J2000.add_seconds(500.0),
                delta_v: DVec3::new(500.0, 0.0, 0.0),
            },
            Burn {
                time: now.add_seconds(100.0),
                delta_v: DVec3::new(0.0, 10.0, 0.0),
            },
        ];
        let patches = predict(
            &system,
            earth,
            orbit,
            now,
            now.add_seconds(3_600.0),
            8,
            &burns,
        );
        assert_eq!(patches.len(), 2);
        let Some(PatchEnd::Burn { index, delta_v, .. }) = patches[0].ending else {
            panic!("ends at the burn")
        };
        assert_eq!(index, 1);
        // A normal burn tilts the plane without changing the speed much.
        assert!((delta_v.length() - 10.0).abs() < 1e-9);
        assert!(patches[1].orbit.normal().angle_between(orbit.normal()) > 1e-4);
    }

    #[test]
    fn a_deorbit_burn_ends_the_prediction_at_descent() {
        let system = rss();
        let (earth, orbit, _) = leo(&system);
        let burns = [
            Burn {
                time: SimTime::J2000.add_seconds(60.0),
                delta_v: DVec3::new(-150.0, 0.0, 0.0),
            },
            // Never reached: the vessel is in the atmosphere first.
            Burn {
                time: SimTime::J2000.add_seconds(20_000.0),
                delta_v: DVec3::new(100.0, 0.0, 0.0),
            },
        ];
        let horizon = SimTime::J2000.add_seconds(86_400.0);
        let patches = predict(&system, earth, orbit, SimTime::J2000, horizon, 8, &burns);
        let last = patches.last().unwrap();
        let Some(PatchEnd::Event(event)) = last.ending else {
            panic!("ends at an event")
        };
        assert_eq!(event.kind, EventKind::Descent);
        assert!(event.time < burns[1].time);
    }
}
