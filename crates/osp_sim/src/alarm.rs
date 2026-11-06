//! The alarm clock: times the player wants the game to stop at and say so, whatever the warp.
//!
//! An alarm rings at a fixed time, or a little ahead of a planned burn, wherever the burn is moved
//! to. The game never warps past one: rails stop on it, and its ringing drops time warp to normal
//! speed. A rung alarm comes off the clock, and an alarm for a burn that is no longer planned goes
//! with the burn. Alarms are saved with the rest of the universe.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{plan::BurnId, time::SimTime, universe::Universe, vessel::VesselId};

/// Fixed times this close together are the same alarm, s.
const SAME_TIME: f64 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AlarmId(pub u64);

/// When an alarm rings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AlarmTime {
    /// At this instant.
    At(SimTime),
    /// `before` seconds ahead of the start of a vessel's planned burn, wherever it is moved.
    Burn {
        vessel: VesselId,
        burn: BurnId,
        before: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alarm {
    pub id: AlarmId,
    /// What it is for, as it is listed and as it is said when it rings.
    pub title: String,
    /// The vessel it is about, if any.
    pub vessel: Option<VesselId>,
    pub when: AlarmTime,
}

/// The alarms set, in the order they were set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlarmClock {
    alarms: Vec<Alarm>,
    next_id: u64,
}

impl AlarmClock {
    pub fn alarms(&self) -> &[Alarm] {
        &self.alarms
    }

    pub fn is_empty(&self) -> bool {
        self.alarms.is_empty()
    }

    pub fn get(&self, id: AlarmId) -> Option<&Alarm> {
        self.alarms.iter().find(|a| a.id == id)
    }

    pub fn remove(&mut self, id: AlarmId) -> Option<Alarm> {
        let index = self.alarms.iter().position(|a| a.id == id)?;
        Some(self.alarms.remove(index))
    }

    /// One already set for `when`.
    pub fn find(&self, when: AlarmTime) -> Option<&Alarm> {
        self.alarms.iter().find(|a| match (a.when, when) {
            (AlarmTime::At(a), AlarmTime::At(b)) => a.seconds_since(b).abs() < SAME_TIME,
            (
                AlarmTime::Burn { vessel, burn, .. },
                AlarmTime::Burn {
                    vessel: v, burn: b, ..
                },
            ) => (vessel, burn) == (v, b),
            _ => false,
        })
    }

    fn add(&mut self, title: String, vessel: Option<VesselId>, when: AlarmTime) -> AlarmId {
        let id = AlarmId(self.next_id);
        self.next_id += 1;
        self.alarms.push(Alarm {
            id,
            title,
            vessel,
            when,
        });
        id
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum AlarmError {
    #[error("That time has passed")]
    Passed,
    #[error("That alarm is already set")]
    AlreadySet,
    #[error("That burn is not planned")]
    NoBurn,
}

impl Universe {
    /// Set an alarm. It has to ring after now, and not be one already set.
    pub fn set_alarm(
        &mut self,
        title: impl Into<String>,
        vessel: Option<VesselId>,
        when: AlarmTime,
    ) -> Result<AlarmId, AlarmError> {
        let time = self.ring_time(when).ok_or(AlarmError::NoBurn)?;
        if time <= self.time() {
            return Err(AlarmError::Passed);
        }
        if self.alarms.find(when).is_some() {
            return Err(AlarmError::AlreadySet);
        }
        Ok(self.alarms.add(title.into(), vessel, when))
    }

    /// When `alarm` rings; `None` if what it was for is gone.
    pub fn alarm_time(&self, alarm: &Alarm) -> Option<SimTime> {
        self.ring_time(alarm.when)
    }

    fn ring_time(&self, when: AlarmTime) -> Option<SimTime> {
        match when {
            AlarmTime::At(time) => Some(time),
            AlarmTime::Burn {
                vessel,
                burn,
                before,
            } => {
                let vessel = self.vessel(vessel)?;
                let burn = vessel.plan.burn(burn)?;
                let start = vessel.burn_timing(burn, 0.0).map_or(burn.time, |t| t.start);
                Some(start.add_seconds(-before))
            }
        }
    }

    /// The alarms that will ring, soonest first, with when.
    pub fn alarms_by_time(&self) -> Vec<(SimTime, &Alarm)> {
        let mut alarms: Vec<_> = (self.alarms.alarms().iter())
            .filter_map(|alarm| Some((self.alarm_time(alarm)?, alarm)))
            .collect();
        alarms.sort_by_key(|(time, alarm)| (*time, alarm.id));
        alarms
    }

    /// When the next alarm rings.
    pub fn next_alarm(&self) -> Option<SimTime> {
        (self.alarms.alarms().iter())
            .filter_map(|alarm| self.alarm_time(alarm))
            .min()
    }

    /// Take off the clock what is due by now, soonest first, and say so in the flown vessel's
    /// log; alarms for burns no longer planned go quietly.
    pub fn ring_alarms(&mut self) -> Vec<Alarm> {
        let now = self.time();
        let mut rung = Vec::new();
        let mut keep = Vec::new();
        for alarm in std::mem::take(&mut self.alarms.alarms) {
            match self.alarm_time(&alarm) {
                Some(time) if time <= now => rung.push((time, alarm)),
                Some(_) => keep.push(alarm),
                None => {}
            }
        }
        self.alarms.alarms = keep;
        rung.sort_by_key(|(time, alarm)| (*time, alarm.id));
        let index = self.focus.and_then(|id| self.find_vessel(id));
        for (_, alarm) in &rung {
            let about = (alarm.vessel)
                .filter(|&id| Some(id) != self.focus)
                .and_then(|id| self.vessel(id))
                .map(|v| format!(" ({})", v.name))
                .unwrap_or_default();
            if let Some(index) = index {
                self.vessels[index].record(now, format!("Alarm: {}{about}", alarm.title));
            }
        }
        rung.into_iter().map(|(_, alarm)| alarm).collect()
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::*;
    use crate::{plan::Burn, system::tests::rss, vessel::tests::leo_vessel};

    fn universe() -> Universe {
        let system = rss();
        let (vessel, t0) = leo_vessel(&system);
        let mut u = Universe::new(system, vec![vessel], t0);
        u.focus = Some(u.vessels[0].id());
        u
    }

    #[test]
    fn rails_stop_on_an_alarm_and_it_rings_once() {
        let mut u = universe();
        let t0 = u.time();
        let at = t0.add_seconds(3_600.0);
        u.set_alarm("Wake up", None, AlarmTime::At(at)).unwrap();
        assert_eq!(u.next_alarm(), Some(at));
        // Not yet.
        u.advance_rails(t0.add_seconds(1_800.0));
        assert!(u.ring_alarms().is_empty());
        // The game advances no further than the next alarm.
        let target = t0.add_seconds(86_400.0).min(u.next_alarm().unwrap());
        u.advance_rails(target);
        assert_eq!(u.time(), at);
        let rung = u.ring_alarms();
        assert_eq!(rung.len(), 1);
        assert_eq!(rung[0].title, "Wake up");
        assert!(u.alarms.is_empty() && u.next_alarm().is_none());
        let log = &u.vessels[0].log;
        assert_eq!(log.last().unwrap().message, "Alarm: Wake up");
        assert!(u.ring_alarms().is_empty());
    }

    #[test]
    fn what_has_passed_or_is_set_cannot_be_set() {
        let mut u = universe();
        let t0 = u.time();
        assert_eq!(
            u.set_alarm("Late", None, AlarmTime::At(t0)),
            Err(AlarmError::Passed)
        );
        let at = t0.add_seconds(600.0);
        u.set_alarm("A", None, AlarmTime::At(at)).unwrap();
        assert_eq!(
            u.set_alarm("B", None, AlarmTime::At(at.add_seconds(0.5))),
            Err(AlarmError::AlreadySet)
        );
        u.set_alarm("C", None, AlarmTime::At(at.add_seconds(60.0)))
            .unwrap();
        let titles: Vec<_> = u
            .alarms_by_time()
            .iter()
            .map(|(_, a)| a.title.clone())
            .collect();
        assert_eq!(titles, ["A", "C"]);
    }

    #[test]
    fn a_burns_alarm_follows_the_burn_and_goes_with_it() {
        let mut u = universe();
        let t0 = u.time();
        let vessel = u.vessels[0].id();
        let burn = Burn {
            time: t0.add_seconds(2_000.0),
            delta_v: DVec3::new(100.0, 0.0, 0.0),
        };
        let index = u.vessels[0].plan.add(burn);
        let id = u.vessels[0].plan.id(index).unwrap();
        let when = AlarmTime::Burn {
            vessel,
            burn: id,
            before: 30.0,
        };
        u.set_alarm("Manoeuvre", Some(vessel), when).unwrap();
        let first = u.next_alarm().unwrap();
        // Ahead of the burn's start, which is ahead of its middle.
        assert!(first < burn.time.add_seconds(-30.0));
        // Moved an hour later, the alarm moves with it.
        let moved = Burn {
            time: burn.time.add_seconds(3_600.0),
            ..burn
        };
        u.vessels[0].plan.set(index, moved);
        let later = u.next_alarm().unwrap();
        assert!((later.seconds_since(first) - 3_600.0).abs() < 1.0);
        // Deleted, it goes, without ringing.
        u.vessels[0].plan.clear();
        assert_eq!(u.next_alarm(), None);
        assert!(u.ring_alarms().is_empty());
        assert!(u.alarms.is_empty());
        let none = AlarmTime::Burn {
            vessel,
            burn: id,
            before: 30.0,
        };
        assert_eq!(u.set_alarm("Gone", None, none), Err(AlarmError::NoBurn));
    }

    #[test]
    fn alarms_are_saved() {
        let mut u = universe();
        let at = u.time().add_seconds(600.0);
        u.set_alarm("Saved", None, AlarmTime::At(at)).unwrap();
        let snapshot = u.snapshot();
        let bytes = rmp_serde::to_vec_named(&snapshot).unwrap();
        let mut other = universe();
        other
            .restore(rmp_serde::from_slice(&bytes).unwrap())
            .unwrap();
        assert_eq!(other.next_alarm(), Some(at));
        // A new one does not take a saved one's id.
        let id = (other.set_alarm("New", None, AlarmTime::At(at.add_seconds(60.0)))).unwrap();
        assert_eq!(other.alarms.alarms().len(), 2);
        assert_ne!(other.alarms.alarms()[0].id, id);
    }
}
