//! Flight plans: burns the pilot schedules ahead (KSP's maneuver nodes).
//!
//! A burn is a change of velocity at an instant, given in the orbit frame there (prograde,
//! normal, radial out), so it keeps its meaning when an earlier burn moves the trajectory.
//! Prediction ([`crate::prediction`]) applies burns as impulses; the burn executor
//! ([`crate::autopilot`]) flies them as finite burns centred on their time.

use glam::{DMat3, DVec3};
use serde::{Deserialize, Serialize};

use crate::{attitude::NavBasis, orbit::StateVector, time::SimTime};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

/// What a planned burn is known by for as long as it is planned: it keeps it when it is moved or
/// reshaped, so what refers to it (an alarm) follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BurnId(pub u64);

/// The burns a vessel has planned, in time order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(from = "SavedPlan")]
pub struct FlightPlan {
    burns: Vec<Burn>,
    /// Bumped on every change, so caches of the predicted trajectory know to rebuild.
    revision: u64,
    /// Each burn's id, in the same order as `burns`.
    ids: Vec<BurnId>,
    next_id: u64,
}

/// A plan as saved. Saves from before burns had ids have none, and get new ones.
#[derive(Deserialize)]
struct SavedPlan {
    burns: Vec<Burn>,
    revision: u64,
    #[serde(default)]
    ids: Vec<BurnId>,
    #[serde(default)]
    next_id: u64,
}

impl From<SavedPlan> for FlightPlan {
    fn from(saved: SavedPlan) -> Self {
        let mut plan = Self {
            burns: saved.burns,
            revision: saved.revision,
            ids: saved.ids,
            next_id: saved.next_id,
        };
        if plan.ids.len() != plan.burns.len() {
            plan.ids = (0..plan.burns.len() as u64).map(BurnId).collect();
            plan.next_id = plan.burns.len() as u64;
        }
        plan
    }
}

impl FlightPlan {
    pub fn burns(&self) -> &[Burn] {
        &self.burns
    }

    /// The id of burn `index`.
    pub fn id(&self, index: usize) -> Option<BurnId> {
        self.ids.get(index).copied()
    }

    /// Where burn `id` is in the plan, if it is still planned.
    pub fn index_of(&self, id: BurnId) -> Option<usize> {
        self.ids.iter().position(|&i| i == id)
    }

    /// Burn `id`, if it is still planned.
    pub fn burn(&self, id: BurnId) -> Option<&Burn> {
        self.index_of(id).map(|index| &self.burns[index])
    }

    pub fn is_empty(&self) -> bool {
        self.burns.is_empty()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Add a burn; returns its index in time order.
    pub fn add(&mut self, burn: Burn) -> usize {
        let id = BurnId(self.next_id);
        self.next_id += 1;
        self.insert(id, burn)
    }

    fn insert(&mut self, id: BurnId, burn: Burn) -> usize {
        let index = self.burns.partition_point(|b| b.time <= burn.time);
        self.burns.insert(index, burn);
        self.ids.insert(index, id);
        self.revision += 1;
        index
    }

    /// Replace burn `index`, which keeps its id; returns its new index, as a changed time can
    /// reorder it.
    pub fn set(&mut self, index: usize, burn: Burn) -> usize {
        if index >= self.burns.len() {
            return self.add(burn);
        }
        self.burns.remove(index);
        let id = self.ids.remove(index);
        self.insert(id, burn)
    }

    /// Change burn `index`'s Δv, leaving it where it is in the plan.
    pub fn set_delta_v(&mut self, index: usize, delta_v: DVec3) {
        if let Some(burn) = self.burns.get_mut(index)
            && burn.delta_v != delta_v
        {
            burn.delta_v = delta_v;
            self.revision += 1;
        }
    }

    pub fn remove(&mut self, index: usize) -> Option<Burn> {
        (index < self.burns.len()).then(|| {
            self.revision += 1;
            self.ids.remove(index);
            self.burns.remove(index)
        })
    }

    pub fn clear(&mut self) {
        if !self.burns.is_empty() {
            self.burns.clear();
            self.ids.clear();
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

    #[test]
    fn a_burn_keeps_its_id_when_moved() {
        let mut plan = FlightPlan::default();
        let at = |s: f64| Burn {
            time: SimTime::from_seconds(s),
            delta_v: DVec3::X,
        };
        plan.add(at(100.0));
        plan.add(at(200.0));
        let first = plan.id(0).unwrap();
        let second = plan.id(1).unwrap();
        assert_ne!(first, second);
        // Moved past the other one, it is still itself.
        let moved = plan.set(0, at(300.0));
        assert_eq!(plan.id(moved), Some(first));
        assert_eq!(plan.index_of(second), Some(0));
        assert_eq!(plan.burn(first).unwrap().time, SimTime::from_seconds(300.0));
        plan.remove(plan.index_of(first).unwrap());
        assert_eq!(plan.index_of(first), None);
        // A new burn never takes a gone one's id.
        plan.add(at(400.0));
        assert_ne!(plan.id(1), Some(first));
    }

    #[test]
    fn a_plan_saved_without_ids_gets_them() {
        #[derive(Serialize)]
        struct Old {
            burns: Vec<Burn>,
            revision: u64,
        }
        let burn = Burn {
            time: SimTime::from_seconds(100.0),
            delta_v: DVec3::X,
        };
        let old = Old {
            burns: vec![burn, burn],
            revision: 3,
        };
        let text = serde_json::to_string(&old).unwrap();
        let mut plan: FlightPlan = serde_json::from_str(&text).unwrap();
        let (a, b) = (plan.id(0).unwrap(), plan.id(1).unwrap());
        assert_ne!(a, b);
        plan.add(burn);
        assert!(plan.id(2).is_some_and(|c| c != a && c != b));
    }
}
