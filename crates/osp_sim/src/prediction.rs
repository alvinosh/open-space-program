//! Patched-conic trajectory prediction.
//!
//! Built from the same [`next_event`] / [`apply_event`] the simulation uses, so a predicted
//! encounter is exactly what will happen when time actually passes, at any warp.

use crate::{
    body::BodyId,
    events::{EventKind, TrajectoryEvent, apply_event, next_event},
    orbit::Orbit,
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
    pub ending: Option<TrajectoryEvent>,
}

pub fn predict(
    system: &SolarSystem,
    frame: BodyId,
    orbit: Orbit,
    from: SimTime,
    horizon: SimTime,
    max_patches: usize,
) -> Vec<Patch> {
    let mut patches = Vec::new();
    let (mut frame, mut orbit, mut start) = (frame, orbit, from);
    while patches.len() < max_patches && start < horizon {
        let event = next_event(system, frame, &orbit, start, horizon);
        let end = event.map_or(horizon, |e| e.time);
        patches.push(Patch {
            frame,
            orbit,
            start,
            end,
            ending: event,
        });
        match event {
            Some(e) if e.kind != EventKind::Descent => {
                (frame, orbit) = apply_event(system, frame, &orbit, e);
                start = e.time;
            }
            _ => break,
        }
    }
    patches
}
