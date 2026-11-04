//! Targets: choosing one (Shift+click a body or vessel in map view, the Universe Manifest
//! window, or a part's own window for one part of a vessel), and what is shown about it: distance
//! and relative speed, the closest approaches along the planned trajectory, and where the orbit
//! crosses the target's plane.
//!
//! | Action | Effect |
//! | --- | --- |
//! | `target.body:<id>`, `target.vessel:<key>`, `target.clear` | Choose or drop the target |
//! | `target.part:<vessel>-<part>` | Target one part of another vessel: a docking port is then the one to dock with |
//! | `vessel.fly:<key>` | Fly another vessel (`key` as in `manifest.items`): the camera frames it and stays in the view it is in, and no burn stays selected |
//! | `plan.match_plane` | Plan a burn at the next node that matches the target's plane |
//! | `warp.to:approach` | Warp to the first closest approach |

use bevy::{input::mouse::MouseButton, math::DVec3, prelude::*, window::PrimaryWindow};
use osp_sim::{
    body::BodyId,
    orbit::Orbit,
    rendezvous::{Approach, PlaneNodes, Target, closest_approaches, plane_match, plane_nodes},
    time::SimTime,
    vessel::VesselId,
};
use osp_ui::{LayoutVisibility, UiAction, UiData};

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera},
    flight_plan::PlanUi,
    frame::RenderFrame,
    map_labels::{MapLabels, Marker},
    orbit_lines::{
        HoverPoint, OrbitLinesSet, Prediction, TARGET_COLOR, ViewPointer, countdown,
        distance_label, project,
    },
    part_info::{has_port, parse_key, part_title},
    sim::{Sim, TimeWarp},
};

/// Closest approaches shown.
const APPROACHES: usize = 2;
/// Range-rate samples per patch when searching for approaches, at most.
const MAX_SAMPLES: f64 = 8_192.0;
/// Pick distance for vessels and small bodies in map view, logical pixels.
const PICK_PX: f32 = 14.0;
const NODE_COLOR: Color = Color::srgb(0.55, 0.95, 0.55);

pub struct TargetingPlugin;

/// Target markers; the flight plan snaps clicks to them, so it runs after.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct TargetMarksSet;

impl Plugin for TargetingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TargetInfo>()
            .add_systems(Update, target_actions.in_set(FrameSet::Input))
            .add_systems(Update, open_docking_window.in_set(FrameSet::Render))
            .add_systems(
                Update,
                (pick_target, update_target_info, draw_target, publish_target)
                    .chain()
                    .in_set(FrameSet::Render)
                    .in_set(TargetMarksSet)
                    .after(OrbitLinesSet),
            );
    }
}

/// What is known about the active vessel's target, rebuilt with the prediction.
#[derive(Resource, Default)]
pub struct TargetInfo {
    key: Option<(u64, VesselId, Option<Target>, Option<SimTime>)>,
    pub approaches: Vec<ApproachMark>,
    /// Plane crossings on the current orbit, if the target orbits the same body.
    pub nodes: Option<NodeMarks>,
}

#[derive(Debug, Clone, Copy)]
pub struct ApproachMark {
    pub approach: Approach,
    /// Body both positions are relative to.
    pub frame: BodyId,
    pub own: DVec3,
    pub other: DVec3,
}

#[derive(Debug, Clone, Copy)]
pub struct NodeMarks {
    pub frame: BodyId,
    pub orbit: Orbit,
    pub nodes: PlaneNodes,
    /// The target's orbit normal (inertial).
    pub normal: DVec3,
}

fn target_actions(
    mut actions: MessageReader<UiAction>,
    mut sim: ResMut<Sim>,
    mut warp: ResMut<TimeWarp>,
    mut plan_ui: ResMut<PlanUi>,
    mut rig: ResMut<CameraRig>,
    info: Res<TargetInfo>,
    time: Res<Time>,
) {
    let real = time.elapsed_secs_f64();
    for action in actions.read() {
        let arg = action.arg().unwrap_or_default();
        match action.name() {
            "target.clear" => sim.active_vessel_mut().target = None,
            "target.body" => {
                let body = sim.universe.system.find(arg);
                let parent = sim.active_vessel().parent();
                if let Some(body) = body.filter(|b| *b != parent) {
                    sim.active_vessel_mut().target = Some(Target::Body(body));
                }
            }
            "target.vessel" => {
                let id = arg
                    .parse()
                    .ok()
                    .map(VesselId)
                    .filter(|&id| id != sim.active_id() && sim.universe.vessel(id).is_some());
                if let Some(id) = id {
                    sim.active_vessel_mut().target = Some(Target::Vessel(id));
                }
            }
            "target.part" => {
                let found = parse_key(arg).filter(|&(id, part)| {
                    id != sim.active_id()
                        && (sim.universe.vessel(id))
                            .is_some_and(|v| v.propulsion.parts().get(part).is_some())
                });
                if let Some((id, part)) = found {
                    sim.active_vessel_mut().target = Some(Target::Part(id, part));
                }
            }
            "vessel.fly" => {
                let id = arg.parse().ok().map(VesselId);
                let Some(id) = id.filter(|&id| sim.universe.vessel(id).is_some()) else {
                    warn!("no vessel to fly in `{}`", action.0);
                    continue;
                };
                sim.fly(id);
                plan_ui.selected = None;
                rig.follow = None;
                rig.frame(sim.active_vessel().propulsion.length());
            }
            "plan.match_plane" => {
                let Some(marks) = info.nodes else {
                    warp.notice = Some(("No target plane to match".into(), real));
                    continue;
                };
                let next = [marks.nodes.ascending, marks.nodes.descending]
                    .into_iter()
                    .flatten()
                    .min();
                let burn = next.and_then(|t| plane_match(&marks.orbit, marks.normal, t));
                if let Some(burn) = burn {
                    plan_ui.selected = Some(sim.active_vessel_mut().plan.add(burn));
                }
            }
            "warp.to" if arg == "approach" => match info.approaches.first() {
                Some(mark) => {
                    warp.warp_to(mark.approach.time, "closest approach", &sim, real);
                }
                None => warp.notice = Some(("No closest approach".into(), real)),
            },
            _ => {}
        }
    }
}

/// Shift+click in map view targets the body or vessel under the mouse.
#[allow(clippy::too_many_arguments)]
fn pick_target(
    mut sim: ResMut<Sim>,
    rig: Res<CameraRig>,
    frame: Res<RenderFrame>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    camera: Single<(&Camera, &Projection, &Transform), With<MainCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    pointer: ViewPointer,
) {
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    if !rig.map_view || !shift || !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(cursor) = pointer.position() else {
        return;
    };
    let (camera, projection, transform) = *camera;
    let fov = match projection {
        Projection::Perspective(p) => p.fov as f64,
        _ => 1.0,
    };
    let pixels_per_radian = window.height() as f64 / fov;
    let system = &sim.universe.system;
    let eye = rig.position;
    let parent = sim.active_vessel().parent();
    // (screen distance beyond the object's disc, target)
    let mut best: Option<(f32, Target)> = None;
    let mut consider = |at: DVec3, radius: f64, target: Target| {
        let Some(screen) = project(camera, transform, crate::frame::to_render(at)) else {
            return;
        };
        let disc = (radius / at.distance(eye).max(1.0) * pixels_per_radian) as f32;
        let d = (cursor.distance(screen) - disc).max(0.0);
        if d < PICK_PX && best.is_none_or(|(b, _)| d < b) {
            best = Some((d, target));
        }
    };
    for body in system.bodies() {
        if body.id != parent {
            consider(
                frame.body_offset(body.id),
                body.radius,
                Target::Body(body.id),
            );
        }
    }
    for (i, other) in sim.universe.vessels.iter().enumerate() {
        if i != sim.active_index() {
            let at = frame.relative(other.parent(), other.state_at(system, frame.time).position);
            consider(at, 0.0, Target::Vessel(other.id()));
        }
    }
    if let Some((_, target)) = best {
        sim.active_vessel_mut().target = Some(target);
    }
}

fn update_target_info(sim: Res<Sim>, prediction: Res<Prediction>, mut info: ResMut<TargetInfo>) {
    let vessel = sim.active_vessel();
    let now = sim.universe.time();
    let target = vessel.target;
    let target_orbit = target.and_then(|t| sim.universe.target_orbit(t, now));
    let key = (
        prediction.revision,
        vessel.id(),
        target,
        target_orbit.map(|(_, o)| o.epoch()),
    );
    if info.key == Some(key) {
        return;
    }
    info.key = Some(key);
    info.approaches.clear();
    info.nodes = None;
    let (Some(target), Some((target_frame, target_orbit))) = (target, target_orbit) else {
        return;
    };
    let system = &sim.universe.system;
    // Where the search ended, for when the distance only ever shrinks.
    let mut last_end = None;
    // On course for the target body itself: the encounter says it all.
    let encounter = prediction.patches.iter().any(|p| match target {
        Target::Body(body) => p.frame == body,
        Target::Vessel(_) | Target::Part(..) => false,
    });
    for patch in prediction.patches.iter().filter(|_| !encounter) {
        // Inside a target body's sphere of influence its distance is the altitude.
        if target == Target::Body(patch.frame) {
            continue;
        }
        let from = patch.start.max(now);
        let periods: Vec<f64> = [patch.orbit.period(), target_orbit.period()]
            .into_iter()
            .flatten()
            .collect();
        let longest = periods.iter().copied().fold(0.0, f64::max);
        let shortest = periods.iter().copied().fold(f64::INFINITY, f64::min);
        let end = if longest > 0.0 {
            patch.end.min(from.add_seconds(3.0 * longest))
        } else {
            patch.end
        };
        let span = end.seconds_since(from);
        if span <= 0.0 {
            continue;
        }
        let fine = if shortest.is_finite() {
            shortest / 64.0
        } else {
            span / 512.0
        };
        let step = fine.max(span / MAX_SAMPLES);
        let relative = |t: SimTime| {
            system.transform(target_orbit.state_at(t), target_frame, patch.frame, t)
                - patch.orbit.state_at(t)
        };
        let wanted = APPROACHES - info.approaches.len();
        let found = closest_approaches(relative, from, end, step, wanted);
        let at_end = relative(end);
        let at_start = relative(from);
        if at_end.position.length() < at_start.position.length() {
            last_end = Some((patch.frame, patch.orbit, end, at_end));
        }
        for approach in found {
            let own = patch.orbit.state_at(approach.time).position;
            info.approaches.push(ApproachMark {
                approach,
                frame: patch.frame,
                own,
                other: own + relative(approach.time).position,
            });
        }
        if info.approaches.len() >= APPROACHES {
            break;
        }
    }
    // Closing all the way: the nearest point searched, as KSP shows within the coming orbits.
    if info.approaches.is_empty()
        && let Some((frame, orbit, time, relative)) = last_end
    {
        let own = orbit.state_at(time).position;
        info.approaches.push(ApproachMark {
            approach: Approach {
                time,
                distance: relative.position.length(),
                speed: relative.velocity.length(),
            },
            frame,
            own,
            other: own + relative.position,
        });
    }
    if let Some(patch) = prediction.patches.first()
        && patch.frame == target_frame
    {
        let normal = target_orbit.normal();
        info.nodes =
            plane_nodes(&patch.orbit, normal, patch.start.max(now)).map(|nodes| NodeMarks {
                frame: patch.frame,
                orbit: patch.orbit,
                nodes,
                normal,
            });
    }
}

fn draw_target(
    rig: Res<CameraRig>,
    frame: Res<RenderFrame>,
    info: Res<TargetInfo>,
    prediction: Res<Prediction>,
    mut labels: ResMut<MapLabels>,
    mut gizmos: Gizmos,
) {
    if !rig.map_view || frame.origin_body.is_none() {
        return;
    }
    let time_to = |time: SimTime| countdown(time.seconds_since(frame.time));
    for (k, mark) in info.approaches.iter().enumerate() {
        let (own, other) = (
            frame.to_render(mark.frame, mark.own),
            frame.to_render(mark.frame, mark.other),
        );
        gizmos.line(own, other, TARGET_COLOR.with_alpha(0.6));
        let approach = mark.approach;
        let detail = format!(
            "Closest approach {}, {}, {:.1} m/s",
            approach_distance(approach.distance),
            time_to(approach.time),
            approach.speed
        );
        // On the trajectory the vessel will fly, so a burn can go right there.
        let snap = HoverPoint {
            frame: mark.frame,
            local: mark.own,
            time: approach.time,
        };
        labels.add(
            Marker::dot(own, TARGET_COLOR)
                .tag(format!("CA{}", k + 1))
                .detail(detail)
                .snap(Some(snap)),
        );
        labels.add(Marker::dot(other, TARGET_COLOR).detail("Target at closest approach"));
    }
    if let Some(marks) = info.nodes {
        let inclination = marks.nodes.inclination.to_degrees();
        // Crossings are on the orbit as it is: past the first burn the vessel flies another.
        let first_burn = prediction.first_burn();
        for (time, tag, name) in [
            (marks.nodes.ascending, "AN", "Ascending node"),
            (marks.nodes.descending, "DN", "Descending node"),
        ] {
            let Some(time) = time else { continue };
            let at = marks.orbit.state_at(time).position;
            let snap = HoverPoint {
                frame: marks.frame,
                local: at,
                time,
            };
            labels.add(
                Marker::dot(frame.to_render(marks.frame, at), NODE_COLOR)
                    .tag(tag)
                    .detail(format!("{name} {inclination:.2} deg, {}", time_to(time)))
                    .snap(first_burn.is_none_or(|t| time <= t).then_some(snap)),
            );
        }
    }
}

/// Short distances in m, then km.
fn approach_distance(metres: f64) -> String {
    if metres < 10e3 {
        format!("{metres:.0} m")
    } else {
        distance_label(metres)
    }
}

fn publish_target(sim: Res<Sim>, info: Res<TargetInfo>, mut data: ResMut<UiData>) {
    let system = &sim.universe.system;
    let now = sim.render_time;
    let vessel = sim.active_vessel();
    let active = sim.active_index();
    let target = vessel.target;
    data.set("target.has", target.is_some());
    let name = match target {
        Some(Target::Body(b)) => Some(system.body(b).name.clone()),
        Some(Target::Vessel(id)) => sim
            .universe
            .find_vessel(id)
            .map(|i| sim.universe.vessels[i].name.clone()),
        Some(Target::Part(id, part)) => sim.universe.vessel(id).map(|v| {
            let part = v.propulsion.parts().get(part);
            match part {
                Some(part) if has_port(v, part) => format!("{}, {} port", v.name, part_title(part)),
                Some(part) => format!("{}, {}", v.name, part_title(part)),
                None => v.name.clone(),
            }
        }),
        None => None,
    };
    data.set("target.name", name.unwrap_or_default());
    let own = vessel.state_at(system, now);
    let relative = sim
        .universe
        .target_state(active, now)
        .filter(|_| target.is_some())
        .map(|t| t - own);
    set_docking_data(&mut data, &sim, active, now);
    data.set("target.distance", relative.map(|r| r.position.length()));
    data.set("target.speed", relative.map(|r| r.velocity.length()));
    data.set(
        "target.closing_speed",
        relative.map(|r| -r.velocity.dot(r.position.normalize_or_zero())),
    );
    for (k, prefix) in ["target.approach", "target.approach2"].iter().enumerate() {
        let mark = info.approaches.get(k);
        data.set(
            format!("{prefix}_distance"),
            mark.map(|m| m.approach.distance),
        );
        data.set(
            format!("{prefix}_time_to"),
            mark.map(|m| m.approach.time.seconds_since(now)),
        );
        data.set(format!("{prefix}_speed"), mark.map(|m| m.approach.speed));
    }
    data.set("target.has_approach", !info.approaches.is_empty());
    let nodes = info.nodes.map(|m| m.nodes);
    data.set("target.has_plane", nodes.is_some());
    data.set(
        "target.inclination",
        nodes.map(|n| n.inclination.to_degrees()),
    );
    data.set(
        "target.an_time_to",
        nodes
            .and_then(|n| n.ascending)
            .map(|t| t.seconds_since(now)),
    );
    data.set(
        "target.dn_time_to",
        nodes
            .and_then(|n| n.descending)
            .map(|t| t.seconds_since(now)),
    );
}

/// How near the target's port the docking window opens by itself, m.
const DOCKING_WINDOW: f64 = 500.0;

/// The docking window's readout: how the port it would dock by stands towards the one on the
/// target. Off axis and sliding are in the pilot's axes, the ones the translation keys push along.
fn set_docking_data(data: &mut UiData, sim: &Sim, active: usize, now: SimTime) {
    let approach = sim.universe.port_approach(active, now);
    data.set("dock.has", approach.is_some());
    let Some(approach) = approach else {
        return;
    };
    let a = approach.alignment;
    let port = (sim.universe.vessel(approach.target))
        .and_then(|v| v.propulsion.parts().get(approach.target_port.part))
        .map_or(String::new(), |part| part.name.clone());
    let vessel = &sim.universe.vessels[active];
    let turned = vessel
        .control_attitude(&sim.universe.system, now)
        .orientation;
    let (right, up) = (turned * DVec3::X, turned * DVec3::NEG_Y);
    data.set("dock.port", port);
    data.set("dock.near", a.distance() < DOCKING_WINDOW);
    data.set("dock.gap", a.gap);
    data.set("dock.closing", a.closing);
    data.set("dock.off_right", a.off_axis.dot(right));
    data.set("dock.off_up", a.off_axis.dot(up));
    data.set("dock.slide_right", a.drift.dot(right));
    data.set("dock.slide_up", a.drift.dot(up));
    data.set("dock.angle", a.angle.to_degrees());
    data.set("dock.turning", a.turning.to_degrees());
    data.set("dock.rcs_off", !vessel.rcs());
    let problem = a.problem();
    let catching = sim.universe.in_soft_capture(vessel.id());
    data.set("dock.ready", catching || problem.is_none());
    let status = if catching {
        "Soft capture: closing the latches"
    } else {
        problem.map_or("Will capture", |p| p.label())
    };
    data.set("dock.status", status);
}

/// Open the docking window when a target's port comes within reach, once each time it does.
fn open_docking_window(sim: Res<Sim>, mut ui: ResMut<LayoutVisibility>, mut open: Local<bool>) {
    let near = sim
        .universe
        .port_approach(sim.active_index(), sim.render_time)
        .is_some_and(|a| a.alignment.distance() < DOCKING_WINDOW);
    if near != *open {
        *open = near;
        ui.layouts.insert("docking_info".into(), near);
    }
}
