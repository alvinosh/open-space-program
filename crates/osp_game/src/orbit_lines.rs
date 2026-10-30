//! Map-view orbit lines: body orbits plus the vessel's trajectory, as it is and through
//! every planned burn.
//!
//! The prediction uses the simulation's own event code, so what is drawn is exactly what
//! will happen if nothing changes. It is recomputed only when the trajectory or the plan
//! changes. The trajectory as it is (no burns) is drawn solid and whole, as in KSP; from the
//! first planned burn on, the planned legs are dashed beside it in the plan's colours, the
//! selected burn's brightest.
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
//! flight plan to place burns on: the solid line up to the first burn, then the planned
//! legs. Apsides, SOI changes and the impact are small markers whose figures show on hover
//! ([`crate::map_labels`]). Other vessels are drawn with their orbits in grey, the target's
//! brighter.

use bevy::{
    ecs::system::SystemParam, math::DVec3, picking::hover::HoverMap, prelude::*,
    window::PrimaryWindow,
};
use osp_sim::{
    body::BodyId,
    events::{EventKind, TrajectoryEvent},
    orbit::Orbit,
    plan::Burn,
    prediction::{Patch, PatchEnd, predict},
    rendezvous::Target,
    system::SolarSystem,
    time::SimTime,
    vessel::Motion,
};
use osp_ui::format::{distance, duration};

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera},
    controls::InputLocked,
    flight_plan::PlanUi,
    frame::{RenderFrame, to_render},
    map_labels::{MapLabels, Marker, body_spheres, hidden},
    sim::Sim,
};

/// How far a drawn line may stray from the true curve, in pixels.
const LINE_TOLERANCE_PX: f64 = 0.5;
/// Patches drawn besides one per planned burn.
const MAX_PATCHES: usize = 6;
/// How far ahead to predict, seconds.
pub const PREDICTION_HORIZON: f64 = 60.0 * 86_400.0;
/// How close the mouse must be to the trajectory to pick a point on it, logical pixels.
const HOVER_PX: f32 = 12.0;
/// Planned legs: dash and gap lengths along the screen, logical pixels.
const DASH_PX: f32 = 9.0;
const GAP_PX: f32 = 6.0;
/// Longest stretch of screen a single chord is dashed along; beyond it (a chord reaching
/// far off screen) it is drawn whole.
const MAX_DASHED_PX: f32 = 20_000.0;
const PATCH_COLORS: [Color; 3] = [
    Color::srgb(1.0, 0.85, 0.2),
    Color::srgb(0.3, 0.9, 1.0),
    Color::srgb(1.0, 0.4, 0.9),
];
/// After a planned burn.
const PLAN_COLORS: [Color; 2] = [Color::srgb(1.0, 0.55, 0.15), Color::srgb(1.0, 0.75, 0.45)];
/// Planned legs of the burns other than the selected one.
const UNSELECTED_ALPHA: f32 = 0.45;
const OTHER_COLOR: Color = Color::srgba(0.7, 0.7, 0.7, 0.45);
/// Debris, fainter still.
const DEBRIS_COLOR: Color = Color::srgba(0.6, 0.55, 0.5, 0.25);
const DANGER_COLOR: Color = Color::srgb(1.0, 0.25, 0.2);
pub const TARGET_COLOR: Color = Color::srgb(0.95, 0.45, 0.85);
const VESSEL_ICON: &str = "icons/vessel.svg";

pub struct OrbitLinesPlugin;

/// Prediction and drawing; systems using [`TrajectoryHover`] run after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct OrbitLinesSet;

impl Plugin for OrbitLinesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Prediction>()
            .init_resource::<TrajectoryHover>()
            .init_resource::<ScriptedPointer>()
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
    /// The trajectory through every planned burn.
    pub patches: Vec<Patch>,
    /// The trajectory with no burns: the same as `patches` up to the first burn.
    pub coast: Vec<Patch>,
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

    /// When the first planned burn happens, where the trajectory as it is and the planned
    /// one part.
    pub fn first_burn(&self) -> Option<SimTime> {
        self.patches
            .iter()
            .find(|p| matches!(p.ending, Some(PatchEnd::Burn { .. })))
            .map(|p| p.end)
    }
}

/// The point on the predicted trajectory under the mouse in map view, if any.
#[derive(Resource, Default)]
pub struct TrajectoryHover(pub Option<HoverPoint>);

/// A point of the trajectory the vessel will follow (with the burns planned before it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoverPoint {
    /// Body the point is relative to.
    pub frame: BodyId,
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
                prediction.coast.clear();
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
    let horizon = time.add_seconds(PREDICTION_HORIZON);
    prediction.patches = predict(
        system,
        vessel.parent(),
        orbit,
        time,
        horizon,
        MAX_PATCHES + prediction.burns.len(),
        &prediction.burns,
    );
    prediction.coast = if prediction.burns.is_empty() {
        prediction.patches.clone()
    } else {
        predict(
            system,
            vessel.parent(),
            orbit,
            time,
            horizon,
            MAX_PATCHES,
            &[],
        )
    };
}

/// Where a render-space point lands on screen (logical pixels), if in front of the camera.
pub fn project(camera: &Camera, transform: &Transform, point: Vec3) -> Option<Vec2> {
    camera
        .world_to_viewport(&GlobalTransform::from(*transform), point)
        .ok()
}

/// A pointer the capture scripts move about the map in place of the mouse (the window has
/// none while they render offscreen).
#[derive(Resource, Default)]
pub struct ScriptedPointer(pub Option<Vec2>);

/// The mouse over the 3D view.
#[derive(SystemParam)]
pub struct ViewPointer<'w, 's> {
    window: Single<'w, 's, &'static Window, With<PrimaryWindow>>,
    hover_map: Res<'w, HoverMap>,
    ui_nodes: Query<'w, 's, (), With<Node>>,
    locked: Res<'w, InputLocked>,
    scripted: Res<'w, ScriptedPointer>,
}

impl ViewPointer<'_, '_> {
    /// Where the mouse is (logical pixels), unless it is over a UI node or input is locked.
    pub fn position(&self) -> Option<Vec2> {
        if self.scripted.0.is_some() {
            return self.scripted.0;
        }
        let over_ui = self
            .hover_map
            .values()
            .any(|hits| hits.keys().any(|e| self.ui_nodes.contains(*e)));
        if over_ui || self.locked.0 {
            return None;
        }
        self.window.cursor_position()
    }
}

/// The point of the line through `points` (projected to `screen`) nearest `cursor` on
/// screen, if within `max_px`: (pixels, point).
pub fn nearest_on_line(
    points: &[DVec3],
    screen: &[Option<Vec2>],
    cursor: Vec2,
    max_px: f32,
) -> Option<(f32, DVec3)> {
    let mut nearest: Option<(f32, DVec3)> = None;
    for k in 1..points.len() {
        let (Some(a), Some(b)) = (screen[k - 1], screen[k]) else {
            continue;
        };
        let ab = b - a;
        let t = ((cursor - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
        let d = cursor.distance(a + ab * t);
        if d < max_px && nearest.is_none_or(|(best, _)| d < best) {
            nearest = Some((d, points[k - 1].lerp(points[k], t as f64)));
        }
    }
    nearest
}

/// Draw the line through `points` (render space, projected to `screen`) as dashes of even
/// length on screen, the pattern running on from chord to chord. (Gizmos' own dashes start
/// afresh on every segment, which breaks up on finely sampled curves.)
fn dashed(gizmos: &mut Gizmos, points: &[Vec3], screen: &[Option<Vec2>], color: Color) {
    const PERIOD: f32 = DASH_PX + GAP_PX;
    let mut phase = 0.0;
    for k in 1..points.len() {
        let (a, b) = (points[k - 1], points[k]);
        let (Some(sa), Some(sb)) = (screen[k - 1], screen[k]) else {
            continue;
        };
        let length = sa.distance(sb);
        if length > MAX_DASHED_PX {
            gizmos.line(a, b, color);
            continue;
        }
        let mut along = 0.0;
        while along < length {
            let dash = phase < DASH_PX;
            let step = if dash {
                DASH_PX - phase
            } else {
                PERIOD - phase
            }
            .min(length - along);
            if dash {
                gizmos.line(
                    a.lerp(b, along / length),
                    a.lerp(b, (along + step) / length),
                    color,
                );
            }
            along += step;
            phase = (phase + step) % PERIOD;
        }
    }
}

/// What drawing a leg of the trajectory needs to know about the view.
struct View<'a> {
    system: &'a SolarSystem,
    frame: &'a RenderFrame,
    eye: DVec3,
    /// Line tolerance as an angle from the camera, radians.
    angle: f64,
    camera: &'a Camera,
    transform: &'a Transform,
    cursor: Option<Vec2>,
    /// The bodies, for leaving points behind them alone.
    spheres: Vec<(Vec3, f32)>,
}

impl View<'_> {
    fn tolerance(&self, body: BodyId) -> impl Fn(DVec3) -> f64 + '_ {
        move |p| self.frame.relative(body, p).distance(self.eye) * self.angle
    }

    /// `T-<time>` until `time`.
    fn countdown(&self, time: SimTime) -> String {
        countdown(time.seconds_since(self.frame.time))
    }
}

/// Time until something, as the UI's `countdown` format shows it.
pub fn countdown(seconds: f64) -> String {
    let sign = if seconds < 0.0 { "T+" } else { "T-" };
    format!("{sign}{}", duration(seconds.abs()))
}

#[allow(clippy::too_many_arguments)]
fn draw_orbits(
    sim: Res<Sim>,
    rig: Res<CameraRig>,
    frame: Res<RenderFrame>,
    prediction: Res<Prediction>,
    plan_ui: Res<PlanUi>,
    camera: Single<(&Camera, &Projection, &Transform), With<MainCamera>>,
    pointer: ViewPointer,
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
    let view = View {
        system,
        frame: &frame,
        eye: rig.position,
        angle: LINE_TOLERANCE_PX * fov as f64 / height as f64,
        camera,
        transform: camera_transform,
        cursor: pointer.position(),
        spheres: body_spheres(&sim, &frame),
    };
    let frame = &*frame;

    for body in system.bodies() {
        let (Some(orbit), Some(parent)) = (&body.orbit, body.parent) else {
            continue;
        };
        let [r, g, b] = body.color;
        let points = orbit
            .sample_full(frame.time, f64::INFINITY, view.tolerance(parent))
            .into_iter()
            .map(|p| frame.to_render(parent, p));
        gizmos.linestrip(points, Color::linear_rgb(r, g, b).with_alpha(0.5));
    }

    let first_burn = prediction.first_burn();
    let mut nearest = None;
    // The trajectory as it is, solid and whole. Up to the first burn it is the planned one
    // too, so points on it can be picked.
    let mut leg = 0;
    for patch in &prediction.coast {
        let until = first_burn.map_or(patch.end, |t| t.min(patch.end));
        let pick = (until > patch.start.max(frame.time)).then_some(until);
        let color = PATCH_COLORS[leg % PATCH_COLORS.len()];
        draw_leg(
            &view,
            patch,
            color,
            false,
            pick,
            &mut gizmos,
            &mut labels,
            &mut nearest,
        );
        if continues(patch) {
            leg += 1;
        }
    }
    // The planned legs, dashed from the first burn on; the selected burn's stand out.
    let mut after = None;
    for patch in &prediction.patches {
        if let Some(burn) = after {
            let mut color = PLAN_COLORS[leg % PLAN_COLORS.len()];
            if plan_ui.selected.is_some_and(|s| s != burn) {
                color = color.with_alpha(UNSELECTED_ALPHA);
            }
            let pick = Some(patch.end);
            draw_leg(
                &view,
                patch,
                color,
                true,
                pick,
                &mut gizmos,
                &mut labels,
                &mut nearest,
            );
        }
        if let Some(PatchEnd::Burn { index, .. }) = patch.ending {
            after = Some(index);
            leg = 0;
        } else if continues(patch) {
            leg += 1;
        }
    }
    hover.0 = nearest.map(|(_, point)| point);

    let vessel = sim.active_vessel();
    let at = vessel.state_at(system, frame.time).position;
    labels.add(
        Marker::icon(
            frame.to_render(vessel.parent(), at),
            VESSEL_ICON,
            16.0,
            Color::WHITE,
        )
        .detail(vessel.name.clone()),
    );

    // Every other vessel, its orbit through it; debris unnamed until hovered.
    let target = vessel.target;
    for (i, other) in sim.universe.vessels.iter().enumerate() {
        if i == sim.active_index() {
            continue;
        }
        let targeted = target == Some(Target::Vessel(other.id()));
        let color = if targeted {
            TARGET_COLOR
        } else if other.is_debris() {
            DEBRIS_COLOR
        } else {
            OTHER_COLOR
        };
        let parent = other.parent();
        let at = other.state_at(system, frame.time).position;
        let marker = Marker::dot(frame.to_render(parent, at), color).detail(other.name.clone());
        labels.add(if other.is_debris() && !targeted {
            marker
        } else {
            marker.tag(other.name.clone())
        });
        if !other.is_landed() {
            let orbit = other.orbit(system, frame.time);
            let radius = system.body(parent).soi.min(1e13);
            let points = orbit
                .sample_full(frame.time, radius, view.tolerance(parent))
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
            .sample_full(frame.time, f64::INFINITY, view.tolerance(parent))
            .into_iter()
            .map(|p| frame.to_render(parent, p));
        gizmos.linestrip(points, TARGET_COLOR);
    }
}

/// Whether the trajectory goes on past this patch into another body's sphere of influence.
fn continues(patch: &Patch) -> bool {
    patch
        .ending
        .and_then(|end| end.event())
        .is_some_and(|event| event.kind != EventKind::Descent)
}

/// Draw one leg of the trajectory, its apsides and how it ends. Points on it up to `pick`
/// can be picked with the mouse; the nearest so far is kept in `nearest`.
#[allow(clippy::too_many_arguments)]
fn draw_leg(
    view: &View,
    patch: &Patch,
    color: Color,
    dashes: bool,
    pick: Option<SimTime>,
    gizmos: &mut Gizmos,
    labels: &mut MapLabels,
    nearest: &mut Option<(f32, HoverPoint)>,
) {
    let frame = view.frame;
    let start = patch.start.max(frame.time);
    if patch.end <= start {
        return;
    }
    let points = patch
        .orbit
        .sample_between(start, patch.end, view.tolerance(patch.frame));
    // Drawn relative to the patch body's current position (KSP-style).
    let render: Vec<Vec3> = points
        .iter()
        .map(|p| frame.to_render(patch.frame, *p))
        .collect();
    let screen: Vec<Option<Vec2>> = render
        .iter()
        .map(|p| project(view.camera, view.transform, *p))
        .collect();
    if dashes {
        dashed(gizmos, &render, &screen, color);
    } else {
        gizmos.linestrip(render.iter().copied(), color);
    }
    let point_at = |time: SimTime| HoverPoint {
        frame: patch.frame,
        local: patch.orbit.state_at(time).position,
        time,
    };
    let eye = to_render(view.eye);
    if let (Some(cursor), Some(until)) = (view.cursor, pick)
        && let Some((d, local)) = nearest_on_line(&points, &screen, cursor, HOVER_PX)
        && nearest.is_none_or(|(best, _)| d < best)
        && !hidden(eye, frame.to_render(patch.frame, local), &view.spheres)
        && let Some(time) = patch
            .orbit
            .next_time_at(local, start)
            .filter(|t| *t <= until)
    {
        *nearest = Some((d, point_at(time)));
    }

    let radius = view.system.body(patch.frame).radius;
    for (apo, tag, name) in [(true, "Ap", "Apoapsis"), (false, "Pe", "Periapsis")] {
        let Some(t) = apsis_in(&patch.orbit, start, patch.end, apo) else {
            continue;
        };
        let point = point_at(t);
        let detail = format!(
            "{name} {}, {}",
            distance_label(point.local.length() - radius),
            view.countdown(t)
        );
        labels.add(
            Marker::dot(frame.to_render(patch.frame, point.local), color)
                .tag(tag)
                .detail(detail)
                .snap(pick.filter(|until| t <= *until).map(|_| point)),
        );
    }
    let Some(PatchEnd::Event(event)) = patch.ending else {
        return;
    };
    if event.kind == EventKind::Descent {
        draw_impact(view, patch, event, gizmos, labels);
        return;
    }
    let (tag, detail) = match event.kind {
        EventKind::SoiEntry(body) => {
            let name = &view.system.body(body).name;
            (name.clone(), format!("Encounter {name}"))
        }
        _ => {
            let name = &view.system.body(patch.frame).name;
            ("Escape".into(), format!("Escape {name}"))
        }
    };
    let at = patch.orbit.state_at(event.time).position;
    labels.add(
        Marker::dot(frame.to_render(patch.frame, at), color)
            .tag(tag)
            .detail(format!("{detail}, {}", view.countdown(event.time))),
    );
}

/// Where a descent leads: on in red to where the conic meets the ground in vacuum, and a
/// cross there, carried round with the ground to now.
fn draw_impact(
    view: &View,
    patch: &Patch,
    event: TrajectoryEvent,
    gizmos: &mut Gizmos,
    labels: &mut MapLabels,
) {
    let frame = view.frame;
    let body = view.system.body(patch.frame);
    let Some(impact) = impact_after(&patch.orbit, body, event.time) else {
        return;
    };
    let arc = patch
        .orbit
        .sample_between(event.time, impact, view.tolerance(patch.frame));
    gizmos.linestrip(
        arc.iter().map(|p| frame.to_render(patch.frame, *p)),
        DANGER_COLOR,
    );
    let hit = patch.orbit.state_at(impact).position;
    let fixed = body.orientation(impact).inverse() * hit;
    let now = body.orientation(frame.time) * fixed;
    let render = frame.to_render(patch.frame, now);
    let size = (frame.relative(patch.frame, now).distance(view.eye) * 0.006) as f32;
    let up = to_render(now.normalize());
    let side = up.any_orthonormal_vector();
    let other = up.cross(side);
    for axis in [side + other, side - other] {
        gizmos.line(render - axis * size, render + axis * size, DANGER_COLOR);
    }
    labels.add(
        Marker::dot(render, DANGER_COLOR)
            .tag("Impact")
            .detail(format!("Impact {}", view.countdown(impact))),
    );
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
