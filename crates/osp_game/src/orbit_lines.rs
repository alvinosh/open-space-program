//! Map-view orbit lines: body orbits plus the vessel's predicted patched-conic trajectory,
//! through every planned burn.
//!
//! The prediction uses the simulation's own event code, so what is drawn is exactly what
//! will happen if nothing changes. It is recomputed only when the trajectory or the plan
//! changes. Legs after a planned burn are drawn in the plan's colours.
//!
//! Where the trajectory descends into an atmosphere or towards terrain, prediction stops
//! (drag and the ground make the rest inexact); the conic is drawn on from there in red to
//! where it would meet the ground in vacuum, with the impact point marked on the surface as
//! it turns.
//!
//! Lines are sampled for the camera every frame: dense where they pass close by, sparse
//! across the far side of an orbit, so no chord strays more than half a pixel from the true
//! curve. A body's orbit starts and ends at the body, so its line always runs through it.
//!
//! The point of the trajectory under the mouse is kept in [`TrajectoryHover`], for the
//! flight plan to place burns on. Other vessels are drawn with their orbits in grey, the
//! target's brighter.

use bevy::{math::DVec3, picking::hover::HoverMap, prelude::*, window::PrimaryWindow};
use osp_sim::{
    body::BodyId,
    events::EventKind,
    orbit::Orbit,
    plan::Burn,
    prediction::{Patch, PatchEnd, predict},
    rendezvous::Target,
    time::SimTime,
    vessel::Motion,
};
use osp_ui::format::distance;

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera},
    frame::RenderFrame,
    map_labels::MapLabels,
    sim::Sim,
};

/// How far a drawn line may stray from the true curve, in pixels.
const LINE_TOLERANCE_PX: f64 = 0.5;
/// Patches drawn besides one per planned burn.
const MAX_PATCHES: usize = 6;
/// How far ahead to predict, seconds.
pub const PREDICTION_HORIZON: f64 = 60.0 * 86_400.0;
/// How close the mouse must be to the trajectory to pick a point on it, logical pixels.
const HOVER_PX: f32 = 10.0;
const PATCH_COLORS: [Color; 3] = [
    Color::srgb(1.0, 0.85, 0.2),
    Color::srgb(0.3, 0.9, 1.0),
    Color::srgb(1.0, 0.4, 0.9),
];
/// After a planned burn.
const PLAN_COLORS: [Color; 2] = [Color::srgb(1.0, 0.55, 0.15), Color::srgb(1.0, 0.75, 0.45)];
pub const BURN_COLOR: Color = Color::srgb(0.3, 0.65, 1.0);
const OTHER_COLOR: Color = Color::srgba(0.7, 0.7, 0.7, 0.45);
const DANGER_COLOR: Color = Color::srgb(1.0, 0.25, 0.2);
pub const TARGET_COLOR: Color = Color::srgb(0.95, 0.45, 0.85);

pub struct OrbitLinesPlugin;

/// Prediction and drawing; systems using [`TrajectoryHover`] run after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct OrbitLinesSet;

impl Plugin for OrbitLinesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Prediction>()
            .init_resource::<TrajectoryHover>()
            .add_systems(
                Update,
                (update_prediction, draw_orbits)
                    .chain()
                    .in_set(FrameSet::Render)
                    .in_set(OrbitLinesSet),
            );
    }
}

/// Cached prediction for the active vessel.
#[derive(Resource, Default)]
pub struct Prediction {
    pub patches: Vec<Patch>,
    /// The burns predicted through (`PatchEnd::Burn` indexes into these, which match the
    /// vessel's plan).
    pub burns: Vec<Burn>,
    /// Vessel, frame, orbit and plan the cache was built from; rebuilt when any changes.
    key: Option<(osp_sim::vessel::VesselId, usize, SimTime, u64)>,
    /// Bumped on every rebuild, for caches derived from the prediction.
    pub revision: u64,
}

impl Prediction {
    /// The patch that ends at planned burn `index`, and where the burn is.
    pub fn burn(&self, index: usize) -> Option<(&Patch, PatchEnd)> {
        self.patches.iter().find_map(|p| match p.ending {
            Some(end @ PatchEnd::Burn { index: i, .. }) if i == index => Some((p, end)),
            _ => None,
        })
    }
}

/// The point on the predicted trajectory under the mouse in map view, if any.
#[derive(Resource, Default)]
pub struct TrajectoryHover(pub Option<HoverPoint>);

#[derive(Debug, Clone, Copy)]
pub struct HoverPoint {
    /// Index into [`Prediction::patches`].
    pub patch: usize,
    /// Position relative to the patch's body.
    pub local: DVec3,
    pub time: SimTime,
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
            if prediction.key.is_some() {
                prediction.patches.clear();
                prediction.key = None;
                prediction.revision += 1;
            }
            return;
        }
    };
    let key = (
        vessel.id(),
        vessel.parent().0,
        orbit.epoch(),
        vessel.plan.revision(),
    );
    if prediction.key == Some(key) {
        return;
    }
    prediction.key = Some(key);
    prediction.revision += 1;
    prediction.burns = vessel.burns_ahead(system, time);
    prediction.patches = predict(
        system,
        vessel.parent(),
        orbit,
        time,
        time.add_seconds(PREDICTION_HORIZON),
        MAX_PATCHES + prediction.burns.len(),
        &prediction.burns,
    );
}

/// Where a render-space point lands on screen (logical pixels), if in front of the camera.
pub fn project(camera: &Camera, transform: &Transform, point: Vec3) -> Option<Vec2> {
    camera
        .world_to_viewport(&GlobalTransform::from(*transform), point)
        .ok()
}

/// The mouse position over the 3D view, unless it is over a UI node.
pub fn cursor_in_view(
    window: &Window,
    hover_map: &HoverMap,
    ui_nodes: &Query<(), With<Node>>,
) -> Option<Vec2> {
    let over_ui = hover_map
        .values()
        .any(|hits| hits.keys().any(|e| ui_nodes.contains(*e)));
    if over_ui {
        return None;
    }
    window.cursor_position()
}

#[allow(clippy::too_many_arguments)]
fn draw_orbits(
    sim: Res<Sim>,
    rig: Res<CameraRig>,
    frame: Res<RenderFrame>,
    prediction: Res<Prediction>,
    camera: Single<(&Camera, &Projection, &Transform), With<MainCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    hover_map: Res<HoverMap>,
    ui_nodes: Query<(), With<Node>>,
    mut hover: ResMut<TrajectoryHover>,
    mut labels: ResMut<MapLabels>,
    mut gizmos: Gizmos,
) {
    hover.0 = None;
    if !rig.map_view || frame.origin_body.is_none() {
        return;
    }
    let system = &sim.universe.system;

    // Radians per pixel, so a tolerance at a point is its distance from the camera times this.
    let (camera, projection, camera_transform) = *camera;
    let fov = match projection {
        Projection::Perspective(p) => p.fov,
        _ => 60f32.to_radians(),
    };
    let height = camera.physical_viewport_size().map_or(1080, |s| s.y.max(1));
    let angle = LINE_TOLERANCE_PX * fov as f64 / height as f64;
    let (frame, eye) = (&*frame, rig.position);
    let tolerance = |body: BodyId| move |p: DVec3| frame.relative(body, p).distance(eye) * angle;
    let cursor = cursor_in_view(&window, &hover_map, &ui_nodes);

    for body in system.bodies() {
        let (Some(orbit), Some(parent)) = (&body.orbit, body.parent) else {
            continue;
        };
        let [r, g, b] = body.color;
        let points = orbit
            .sample_full(frame.time, f64::INFINITY, tolerance(parent))
            .into_iter()
            .map(|p| frame.to_render(parent, p));
        gizmos.linestrip(points, Color::linear_rgb(r, g, b).with_alpha(0.5));
    }

    // (distance in pixels, patch, local point) of the nearest trajectory point to the mouse.
    let mut nearest: Option<(f32, usize, DVec3)> = None;
    let mut planned = false;
    let mut leg = 0;
    for (i, patch) in prediction.patches.iter().enumerate() {
        let color = if planned {
            PLAN_COLORS[leg % PLAN_COLORS.len()]
        } else {
            PATCH_COLORS[leg % PATCH_COLORS.len()]
        };
        let start = patch.start.max(frame.time);
        let points = patch
            .orbit
            .sample_between(start, patch.end, tolerance(patch.frame));
        // Drawn relative to the patch body's current position (KSP-style).
        let render: Vec<Vec3> = points
            .iter()
            .map(|p| frame.to_render(patch.frame, *p))
            .collect();
        gizmos.linestrip(render.iter().copied(), color);
        if let Some(cursor) = cursor {
            let screen: Vec<_> = render
                .iter()
                .map(|p| project(camera, camera_transform, *p))
                .collect();
            for k in 1..points.len() {
                let (Some(a), Some(b)) = (screen[k - 1], screen[k]) else {
                    continue;
                };
                let ab = b - a;
                let t = ((cursor - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                let d = cursor.distance(a + ab * t);
                if d < HOVER_PX && nearest.is_none_or(|(best, _, _)| d < best) {
                    let local = points[k - 1].lerp(points[k], t as f64);
                    nearest = Some((d, i, local));
                }
            }
        }
        let distance = |at: DVec3| frame.relative(patch.frame, at).distance(eye);
        // Apsides on this leg, labelled with their altitude.
        let radius = system.body(patch.frame).radius;
        for (apo, name) in [(true, "Ap"), (false, "Pe")] {
            let Some(t) = apsis_in(&patch.orbit, start, patch.end, apo) else {
                continue;
            };
            let at = patch.orbit.state_at(t).position;
            let render = frame.to_render(patch.frame, at);
            gizmos.sphere(render, (distance(at) * 0.004) as f32, color);
            labels.add(
                render,
                format!("{name} {}", distance_label(at.length() - radius)),
                color,
            );
        }
        match patch.ending {
            Some(PatchEnd::Event(event)) if event.kind != EventKind::Descent => {
                let at = patch.orbit.state_at(event.time).position;
                let radius = (system.body(patch.frame).radius * 0.02) as f32;
                gizmos.sphere(frame.to_render(patch.frame, at), radius, color);
                leg += 1;
            }
            Some(PatchEnd::Event(event)) => {
                let body = system.body(patch.frame);
                let Some(impact) = impact_after(&patch.orbit, body, event.time) else {
                    continue;
                };
                let arc = patch
                    .orbit
                    .sample_between(event.time, impact, tolerance(patch.frame));
                gizmos.linestrip(
                    arc.iter().map(|p| frame.to_render(patch.frame, *p)),
                    DANGER_COLOR,
                );
                // Where it lands, carried round with the ground to now.
                let hit = patch.orbit.state_at(impact).position;
                let fixed = body.orientation(impact).inverse() * hit;
                let now = body.orientation(frame.time) * fixed;
                let render = frame.to_render(patch.frame, now);
                let size = (distance(now) * 0.006) as f32;
                let up = crate::frame::to_render(now.normalize());
                let side = up.any_orthonormal_vector();
                let other = up.cross(side);
                for axis in [side + other, side - other] {
                    gizmos.line(render - axis * size, render + axis * size, DANGER_COLOR);
                }
                labels.add(
                    render,
                    format!(
                        "Impact {}",
                        osp_ui::format::duration(impact.seconds_since(frame.time))
                    ),
                    DANGER_COLOR,
                );
            }
            Some(PatchEnd::Burn {
                index,
                state,
                delta_v,
            }) => {
                let at = frame.to_render(patch.frame, state.position);
                labels.add(
                    at,
                    format!("B{} {:.1} m/s", index + 1, delta_v.length()),
                    BURN_COLOR,
                );
                let scale = distance(state.position);
                gizmos.sphere(at, (scale * 0.008) as f32, BURN_COLOR);
                if let Some(direction) = delta_v.try_normalize() {
                    let tip =
                        frame.relative(patch.frame, state.position) + direction * scale * 0.08;
                    gizmos.arrow(at, crate::frame::to_render(tip), BURN_COLOR);
                }
                if !planned {
                    planned = true;
                    leg = 0;
                } else {
                    leg += 1;
                }
            }
            _ => {}
        }
    }
    if let Some((_, i, local)) = nearest {
        let patch = &prediction.patches[i];
        let time = patch
            .orbit
            .next_time_at(local, patch.start.max(frame.time))
            .filter(|t| *t <= patch.end);
        if let Some(time) = time {
            let at = patch.orbit.state_at(time).position;
            let scale = frame.relative(patch.frame, at).distance(eye);
            gizmos.sphere(
                frame.to_render(patch.frame, at),
                (scale * 0.006) as f32,
                Color::WHITE,
            );
            hover.0 = Some(HoverPoint {
                patch: i,
                local: at,
                time,
            });
        }
    }

    let vessel = sim.active_vessel();
    let body = system.body(vessel.parent());
    gizmos.sphere(
        frame.to_render(vessel.parent(), frame.origin_offset),
        (body.radius * 0.01) as f32,
        Color::WHITE,
    );

    // Every other vessel, its orbit through it.
    let target = vessel.target;
    for (i, other) in sim.universe.vessels.iter().enumerate() {
        if i == sim.active_index() {
            continue;
        }
        let targeted = target == Some(Target::Vessel(other.id()));
        let color = if targeted { TARGET_COLOR } else { OTHER_COLOR };
        let parent = other.parent();
        let at = other.state_at(system, frame.time).position;
        let render = frame.to_render(parent, at);
        let scale = frame.relative(parent, at).distance(eye);
        gizmos.sphere(render, (scale * 0.005) as f32, color);
        labels.add(render, other.name.clone(), color);
        if !other.is_landed() {
            let orbit = other.orbit(system, frame.time);
            let radius = system.body(parent).soi.min(1e13);
            let points = orbit
                .sample_full(frame.time, radius, tolerance(parent))
                .into_iter()
                .map(|p| frame.to_render(parent, p));
            gizmos.linestrip(points, color);
        }
    }
    // A targeted body's orbit stands out too.
    if let Some(Target::Body(id)) = target
        && let (Some(orbit), Some(parent)) = (&system.body(id).orbit, system.body(id).parent)
    {
        let points = orbit
            .sample_full(frame.time, f64::INFINITY, tolerance(parent))
            .into_iter()
            .map(|p| frame.to_render(parent, p));
        gizmos.linestrip(points, TARGET_COLOR);
    }
}

/// When `orbit` meets the ground of `body` after `after`, ignoring the air: first at the
/// mean radius, then again at the terrain height found there.
fn impact_after(orbit: &Orbit, body: &osp_sim::body::Body, after: SimTime) -> Option<SimTime> {
    let until = after.add_seconds(orbit.period().unwrap_or(86_400.0));
    let inbound = osp_sim::orbit::Direction::Inbound;
    let mut radius = body.radius;
    let mut impact = None;
    for _ in 0..2 {
        let t = orbit.next_radius_crossing(after, until, radius, inbound, 0.0)?;
        let fixed = body.orientation(t).inverse() * orbit.state_at(t).position;
        radius = body.radius + body.terrain_height(fixed.normalize());
        impact = Some(t);
    }
    impact
}

/// The next apoapsis or periapsis of `orbit` in `[from, to]`, if there is one.
fn apsis_in(orbit: &Orbit, from: SimTime, to: SimTime, apo: bool) -> Option<SimTime> {
    if apo && !orbit.is_bound() {
        return None;
    }
    let nu = if apo { std::f64::consts::PI } else { 0.0 };
    orbit
        .next_time_at_true_anomaly(nu, from)
        .filter(|t| *t <= to)
}

/// Altitudes for labels: whole km below 100 000 km, then the usual distance format.
pub fn distance_label(metres: f64) -> String {
    if metres.abs() < 1e8 {
        format!("{:.0} km", metres / 1e3)
    } else {
        distance(metres)
    }
}
