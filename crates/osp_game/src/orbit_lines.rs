//! Map-view orbit lines: body orbits plus the vessel's predicted patched-conic trajectory.
//!
//! The prediction uses the simulation's own event code, so what is drawn is exactly what
//! will happen if nothing changes. It is recomputed only when the trajectory changes.

use bevy::prelude::*;
use osp_sim::{
    events::EventKind,
    prediction::{Patch, predict},
    time::SimTime,
    vessel::Motion,
};

use crate::{FrameSet, camera::CameraRig, frame::RenderFrame, sim::Sim};

const ORBIT_SEGMENTS: usize = 256;
const MAX_PATCHES: usize = 6;
/// How far ahead to predict, seconds.
const PREDICTION_HORIZON: f64 = 60.0 * 86_400.0;
const PATCH_COLORS: [Color; 3] = [
    Color::srgb(1.0, 0.85, 0.2),
    Color::srgb(0.3, 0.9, 1.0),
    Color::srgb(1.0, 0.4, 0.9),
];

pub struct OrbitLinesPlugin;

impl Plugin for OrbitLinesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Prediction>().add_systems(
            Update,
            (update_prediction, draw_orbits)
                .chain()
                .in_set(FrameSet::Render),
        );
    }
}

/// Cached prediction for the active vessel.
#[derive(Resource, Default)]
pub struct Prediction {
    pub patches: Vec<Patch>,
    /// Orbit and frame the cache was built from; rebuilt when either changes.
    key: Option<(usize, SimTime)>,
}

fn update_prediction(sim: Res<Sim>, mut prediction: ResMut<Prediction>) {
    let system = &sim.universe.system;
    let vessel = sim.active_vessel();
    let time = sim.universe.time();
    let orbit = match vessel.motion() {
        Motion::Coasting(orbit) => *orbit,
        // Under power the trajectory changes every tick; predict from the osculating orbit.
        Motion::Integrated(_) => vessel.orbit(system, time),
        Motion::Landed { .. } => {
            prediction.patches.clear();
            prediction.key = None;
            return;
        }
    };
    let key = (vessel.parent().0, orbit.epoch());
    if prediction.key == Some(key) {
        return;
    }
    prediction.key = Some(key);
    prediction.patches = predict(
        system,
        vessel.parent(),
        orbit,
        time,
        time.add_seconds(PREDICTION_HORIZON),
        MAX_PATCHES,
    );
}

fn draw_orbits(
    sim: Res<Sim>,
    rig: Res<CameraRig>,
    frame: Res<RenderFrame>,
    prediction: Res<Prediction>,
    mut gizmos: Gizmos,
) {
    if !rig.map_view || frame.origin_body.is_none() {
        return;
    }
    let system = &sim.universe.system;

    for body in system.bodies() {
        let (Some(orbit), Some(parent)) = (&body.orbit, body.parent) else {
            continue;
        };
        let [r, g, b] = body.color;
        let points = orbit
            .sample_full(ORBIT_SEGMENTS, f64::INFINITY)
            .into_iter()
            .map(|p| frame.to_render(parent, p));
        gizmos.linestrip(points, Color::linear_rgb(r, g, b).with_alpha(0.5));
    }

    for (i, patch) in prediction.patches.iter().enumerate() {
        let color = PATCH_COLORS[i % PATCH_COLORS.len()];
        let start = patch.start.max(frame.time);
        let points = patch.orbit.sample_between(start, patch.end, ORBIT_SEGMENTS);
        // Drawn relative to the patch body's current position (KSP-style).
        gizmos.linestrip(
            points.into_iter().map(|p| frame.to_render(patch.frame, p)),
            color,
        );
        if let Some(event) = patch.ending
            && event.kind != EventKind::Descent
        {
            let at = patch.orbit.state_at(event.time).position;
            let radius = (system.body(patch.frame).radius * 0.02) as f32;
            gizmos.sphere(frame.to_render(patch.frame, at), radius, color);
        }
    }

    let vessel = sim.active_vessel();
    let body = system.body(vessel.parent());
    gizmos.sphere(
        frame.to_render(vessel.parent(), frame.origin_offset),
        (body.radius * 0.01) as f32,
        Color::WHITE,
    );
}
