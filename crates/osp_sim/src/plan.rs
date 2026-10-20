//! Flight plans: burns the pilot schedules ahead (KSP's maneuver nodes).
//!
//! A burn is a change of velocity at an instant, given in the orbit frame there (prograde,
//! normal, radial out), so it keeps its meaning when an earlier burn moves the trajectory.
//! Prediction ([`crate::prediction`]) applies burns as impulses; the burn executor
//! ([`crate::autopilot`]) flies them as finite burns centred on their time.

use glam::{DMat3, DVec3};

use crate::{attitude::NavBasis, orbit::StateVector, time::SimTime};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Burn {
    pub time: SimTime,
    /// Δv in the orbit frame at `time`: prograde, normal, radial out, m/s.
    pub delta_v: DVec3,
}

impl Burn {
    /// Orbit frame at `state` (relative to the body orbited) as a matrix whose columns are
    /// prograde, normal and radial out; `None` if the state is at rest or radial.
    pub fn frame(state: &StateVector) -> Option<DMat3> {
        NavBasis::orbital(state).map(|b| b.matrix())
    }

    /// The burn's Δv in inertial axes for a vessel at `state`.
    pub fn inertial(&self, state: &StateVector) -> Option<DVec3> {
        Self::frame(state).map(|m| m * self.delta_v)
    }

    /// A burn at `time` giving `inertial` Δv to a vessel at `state`.
    pub fn from_inertial(time: SimTime, state: &StateVector, inertial: DVec3) -> Option<Self> {
        Self::frame(state).map(|m| Self {
            time,
            delta_v: m.transpose() * inertial,
        })
    }
}

/// The burns a vessel has planned, in time order.
#[derive(Debug, Clone, Default)]
pub struct FlightPlan {
    burns: Vec<Burn>,
    /// Bumped on every change, so caches of the predicted trajectory know to rebuild.
    revision: u64,
}

impl FlightPlan {
    pub fn burns(&self) -> &[Burn] {
        &self.burns
    }

    pub fn is_empty(&self) -> bool {
        self.burns.is_empty()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Add a burn; returns its index in time order.
    pub fn add(&mut self, burn: Burn) -> usize {
        let index = self.burns.partition_point(|b| b.time <= burn.time);
        self.burns.insert(index, burn);
        self.revision += 1;
        index
    }

    /// Replace burn `index`; returns its new index, as a changed time can reorder it.
    pub fn set(&mut self, index: usize, burn: Burn) -> usize {
        if index >= self.burns.len() {
            return self.add(burn);
        }
        self.burns.remove(index);
        self.add(burn)
    }

    pub fn remove(&mut self, index: usize) -> Option<Burn> {
        (index < self.burns.len()).then(|| {
            self.revision += 1;
            self.burns.remove(index)
        })
    }

    pub fn clear(&mut self) {
        if !self.burns.is_empty() {
            self.burns.clear();
            self.revision += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inertial_round_trip() {
        let state = StateVector::new(DVec3::new(7e6, 0.0, 0.0), DVec3::new(0.0, 7.5e3, 0.5e3));
        let burn = Burn {
            time: SimTime::J2000,
            delta_v: DVec3::new(100.0, -20.0, 5.0),
        };
        let inertial = burn.inertial(&state).unwrap();
        assert!((inertial.length() - burn.delta_v.length()).abs() < 1e-9);
        // Prograde is along the velocity.
        let prograde = Burn {
            delta_v: DVec3::X,
            ..burn
        };
        let along = prograde.inertial(&state).unwrap();
        assert!(along.angle_between(state.velocity) < 1e-12);
        let back = Burn::from_inertial(burn.time, &state, inertial).unwrap();
        assert!((back.delta_v - burn.delta_v).length() < 1e-9);
    }

    #[test]
    fn plan_keeps_time_order() {
        let mut plan = FlightPlan::default();
        let at = |s: f64| Burn {
            time: SimTime::from_seconds(s),
            delta_v: DVec3::X,
        };
        assert_eq!(plan.add(at(100.0)), 0);
        assert_eq!(plan.add(at(50.0)), 0);
        assert_eq!(plan.add(at(200.0)), 2);
        let moved = plan.set(0, at(300.0));
        assert_eq!(moved, 2);
        let times: Vec<f64> = plan.burns().iter().map(|b| b.time.as_seconds()).collect();
        assert_eq!(times, [100.0, 200.0, 300.0]);
        let revision = plan.revision();
        plan.remove(1);
        assert!(plan.revision() > revision);
    }
}
