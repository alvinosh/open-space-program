//! Planning burns: the Flight Plan window's actions, placing and shaping burns in map view,
//! and warping to points of the trajectory.
//!
//! In map view, click the predicted trajectory to add a burn there. The selected burn shows
//! six handles (prograde, normal and radial, both ways): drag one outwards to add Δv that
//! way, faster the further you pull. Drag the burn's marker along the orbit to move it in
//! time. The plan itself lives on the vessel ([`osp_sim::plan`]); the executor flies it.
//!
//! | Action | Effect |
//! | --- | --- |
//! | `plan.add:ap`, `plan.add:pe` | New burn at the next apoapsis / periapsis |
//! | `plan.circularize:ap`, `plan.circularize:pe` | New burn there that circularises |
//! | `plan.select:<i>` | Select burn `i` |
//! | `plan.dv:<prograde|normal|radial>:<+1|-1>` | Change the selected burn by one step |
//! | `plan.dv_step:<m/s>`, `plan.time_step:<s>` | Step sizes |
//! | `plan.time:<+1|-1>`, `plan.orbit:<+1|-1>` | Move it by one time step / one orbit |
//! | `plan.zero`, `plan.delete`, `plan.clear` | Zero its Δv, delete it, delete all |
//! | `plan.aim:<m>` | Adjust the selected burn for a periapsis this high above the target body |
//! | `plan.execute` | Engage or disengage the burn executor |
//! | `warp.to:<burn|ap|pe|soi>` | Warp there, slowing on the way in |

use bevy::{
    input::mouse::MouseButton, math::DVec3, picking::hover::HoverMap, prelude::*,
    window::PrimaryWindow,
};
use osp_sim::{
    autopilot::{ULLAGE_LEAD, WARP_MARGIN},
    orbit::{Orbit, StateVector},
    plan::Burn,
    prediction::PatchEnd,
    rendezvous::Target,
    time::SimTime,
    transfer::{Course, aim_periapsis},
    vessel::Motion,
};
use osp_ui::{UiAction, UiData, record};

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera},
    controls::InputLocked,
    frame::{RenderFrame, to_render},
    orbit_lines::{PREDICTION_HORIZON, Prediction, TrajectoryHover, cursor_in_view, project},
    sim::{Sim, TimeWarp},
};

/// Grab distance for handles and markers, logical pixels.
const GRAB_PX: f32 = 12.0;
/// Handle length as a share of the distance to the camera.
const HANDLE_SCALE: f64 = 0.07;
/// Δv rate while dragging a handle `d` pixels out: (d / DRAG_PX)² m/s per second.
const DRAG_PX: f32 = 12.0;
/// Margin before a burn's start when warping to it, s.
const WARP_TO_BURN: f64 = WARP_MARGIN + ULLAGE_LEAD + 2.0;
const AXES: [&str; 3] = ["prograde", "normal", "radial"];
const AXIS_COLORS: [Color; 3] = [
    Color::srgb(0.84, 0.95, 0.24),
    Color::srgb(0.84, 0.36, 1.0),
    Color::srgb(0.24, 0.85, 1.0),
];

pub struct FlightPlanPlugin;

impl Plugin for FlightPlanPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlanUi>()
            .add_systems(Update, plan_actions.in_set(FrameSet::Input))
            .add_systems(
                Update,
                (map_interaction, publish_plan)
                    .chain()
                    .in_set(FrameSet::Render)
                    .after(crate::orbit_lines::OrbitLinesSet),
            );
    }
}

#[derive(Resource)]
pub struct PlanUi {
    pub selected: Option<usize>,
    pub dv_step: f64,
    pub time_step: f64,
    drag: Option<Drag>,
    /// The mouse went down on the trajectory: add a burn there if it comes up unmoved.
    press: Option<Vec2>,
}

impl Default for PlanUi {
    fn default() -> Self {
        Self {
            selected: None,
            dv_step: 10.0,
            time_step: 60.0,
            drag: None,
            press: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Drag {
    /// A Δv handle: burn-frame axis (0..3), which way, and where the drag started.
    Axis { axis: usize, sign: f64, from: Vec2 },
    /// The burn's marker, along the orbit.
    Time,
}

/// The next apoapsis or periapsis of `orbit` after `time`.
fn apsis(orbit: &Orbit, time: SimTime, apo: bool) -> Option<SimTime> {
    if apo && !orbit.is_bound() {
        return None;
    }
    let nu = if apo { std::f64::consts::PI } else { 0.0 };
    orbit.next_time_at_true_anomaly(nu, time.add_ticks(1))
}

/// The coasting orbit the vessel is on (osculating under power).
fn current_orbit(sim: &Sim) -> Option<Orbit> {
    let vessel = sim.active_vessel();
    match vessel.motion() {
        Motion::Landed { .. } => None,
        _ => Some(vessel.orbit(&sim.universe.system, sim.universe.time())),
    }
}

/// The next apoapsis or periapsis after `after` on the predicted legs that start there or
/// later (so after the last planned burn, and in whichever SOI it falls), with the leg's
/// orbit.
fn next_apsis(prediction: &Prediction, after: SimTime, apo: bool) -> Option<(Orbit, SimTime)> {
    prediction
        .patches
        .iter()
        .filter(|p| p.end > after)
        .find_map(|p| {
            let t = apsis(&p.orbit, p.start.max(after), apo)?;
            (t <= p.end).then_some((p.orbit, t))
        })
}

#[allow(clippy::too_many_arguments)]
fn plan_actions(
    mut actions: MessageReader<UiAction>,
    mut sim: ResMut<Sim>,
    mut ui: ResMut<PlanUi>,
    mut warp: ResMut<TimeWarp>,
    prediction: Res<Prediction>,
    time: Res<Time>,
) {
    // Not gated by `InputLocked`: these come from buttons (and scripts), not raw input.
    let real = time.elapsed_secs_f64();
    for action in actions.read() {
        let arg = action.arg().unwrap_or_default();
        let now = sim.universe.time();
        match action.name() {
            "plan.add" | "plan.circularize" => {
                let Some(orbit) = current_orbit(&sim) else {
                    continue;
                };
                // Beyond any burn already planned, on the trajectory as it is there.
                let after = sim
                    .active_vessel()
                    .plan
                    .burns()
                    .last()
                    .map_or(now, |b| b.time);
                let apo = arg == "ap";
                let found = next_apsis(&prediction, after, apo)
                    .or_else(|| Some((orbit, apsis(&orbit, after, apo)?)));
                let Some((base, at)) = found else {
                    warp.notice = Some(("No apoapsis: escape trajectory".into(), real));
                    continue;
                };
                let state = base.state_at(at);
                let burn = if action.name() == "plan.circularize" {
                    let speed = (base.mu() / state.position.length()).sqrt();
                    let wanted = state.velocity.normalize() * speed;
                    Burn::from_inertial(at, &state, wanted - state.velocity)
                } else {
                    Some(Burn {
                        time: at,
                        delta_v: DVec3::ZERO,
                    })
                };
                if let Some(burn) = burn {
                    ui.selected = Some(sim.active_vessel_mut().plan.add(burn));
                }
            }
            "plan.select" => {
                ui.selected = arg
                    .parse()
                    .ok()
                    .filter(|&i| i < sim.active_vessel().plan.burns().len());
            }
            "plan.dv" => {
                let mut parts = arg.split(':');
                let axis = parts.next().and_then(|a| AXES.iter().position(|x| *x == a));
                let sign: f64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1.0);
                if let Some(axis) = axis {
                    let step = ui.dv_step;
                    edit_selected(&mut sim, &mut ui, |burn| burn.delta_v[axis] += sign * step);
                }
            }
            "plan.dv_step" => ui.dv_step = arg.parse().unwrap_or(ui.dv_step),
            "plan.time_step" => ui.time_step = arg.parse().unwrap_or(ui.time_step),
            "plan.time" | "plan.orbit" => {
                let sign: f64 = arg.parse().unwrap_or(1.0);
                let step = if action.name() == "plan.orbit" {
                    let period = ui
                        .selected
                        .and_then(|i| prediction.burn(i))
                        .and_then(|(patch, _)| patch.orbit.period());
                    let Some(period) = period else {
                        continue;
                    };
                    period
                } else {
                    ui.time_step
                };
                edit_selected(&mut sim, &mut ui, |burn| {
                    let moved = burn.time.add_seconds(sign * step);
                    burn.time = moved.max(now.add_seconds(1.0));
                });
            }
            "plan.zero" => edit_selected(&mut sim, &mut ui, |burn| burn.delta_v = DVec3::ZERO),
            "plan.aim" => {
                let (Some(index), Ok(altitude)) = (ui.selected, arg.parse::<f64>()) else {
                    continue;
                };
                let vessel = sim.active_vessel();
                let (Some(Target::Body(target)), Some(orbit)) =
                    (vessel.target, current_orbit(&sim))
                else {
                    warp.notice = Some(("Aim: target a body first".into(), real));
                    continue;
                };
                let system = &sim.universe.system;
                let burns = vessel.burns_ahead(system, now);
                let last = burns.last().map_or(now, |b| b.time);
                let course = Course {
                    system,
                    frame: vessel.parent(),
                    orbit,
                    from: now,
                    horizon: last.add_seconds(PREDICTION_HORIZON),
                };
                let goal = system.body(target).radius + altitude;
                match aim_periapsis(&course, &burns, index, target, goal) {
                    Some((burn, reached)) => {
                        let off = (reached - goal).abs();
                        ui.selected = Some(sim.active_vessel_mut().plan.set(index, burn));
                        if off > 1e3 {
                            let text = format!("Aim: closest reachable is {:.0} km off", off / 1e3);
                            warp.notice = Some((text, real));
                        }
                    }
                    None => {
                        warp.notice = Some(("Aim: no way to the target from here".into(), real))
                    }
                }
            }
            "plan.delete" => {
                if let Some(i) = ui.selected.take() {
                    let vessel = sim.active_vessel_mut();
                    if i == 0 {
                        vessel.disengage_burn(now);
                    }
                    vessel.plan.remove(i);
                }
            }
            "plan.clear" => {
                let vessel = sim.active_vessel_mut();
                vessel.disengage_burn(now);
                vessel.plan.clear();
                ui.selected = None;
            }
            "plan.execute" => {
                let (vessel, system) = sim.active_vessel_in_system();
                if vessel.executor().engaged() {
                    vessel.disengage_burn(now);
                } else {
                    vessel.engage_burn(system, now);
                }
            }
            "warp.to" => {
                let target = match arg {
                    "burn" => {
                        let vessel = sim.active_vessel();
                        vessel.plan.burns().first().map(|burn| {
                            let start =
                                vessel.burn_timing(burn, 0.0).map_or(burn.time, |t| t.start);
                            (start.add_seconds(-WARP_TO_BURN), "the next burn")
                        })
                    }
                    "ap" | "pe" => current_orbit(&sim)
                        .and_then(|o| apsis(&o, now, arg == "ap"))
                        .map(|t| {
                            let what = if arg == "ap" { "apoapsis" } else { "periapsis" };
                            (t, what)
                        }),
                    "soi" => prediction
                        .patches
                        .iter()
                        .filter_map(|p| p.ending?.event())
                        .find(|e| e.kind != osp_sim::events::EventKind::Descent)
                        .map(|e| (e.time, "the SOI change")),
                    // Other destinations (closest approach) are handled elsewhere.
                    _ => continue,
                };
                match target {
                    Some((time, what)) => warp.warp_to(time, what, &sim, real),
                    None => warp.notice = Some(("Nothing to warp to".into(), real)),
                }
            }
            _ => {}
        }
    }
}

/// Apply `change` to the selected burn, keeping it selected if it moves in the order.
fn edit_selected(sim: &mut Sim, ui: &mut PlanUi, change: impl FnOnce(&mut Burn)) {
    let Some(index) = ui.selected else {
        return;
    };
    let plan = &mut sim.active_vessel_mut().plan;
    let Some(mut burn) = plan.burns().get(index).copied() else {
        ui.selected = None;
        return;
    };
    change(&mut burn);
    ui.selected = Some(plan.set(index, burn));
}

#[allow(clippy::too_many_arguments)]
fn map_interaction(
    mut sim: ResMut<Sim>,
    mut ui: ResMut<PlanUi>,
    rig: Res<CameraRig>,
    frame: Res<RenderFrame>,
    prediction: Res<Prediction>,
    hover: Res<TrajectoryHover>,
    buttons: Res<ButtonInput<MouseButton>>,
    camera: Single<(&Camera, &Transform), With<MainCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    hover_map: Res<HoverMap>,
    ui_nodes: Query<(), With<Node>>,
    locked: Res<InputLocked>,
    (time, keys): (Res<Time>, Res<ButtonInput<KeyCode>>),
    mut gizmos: Gizmos,
) {
    let count = sim.active_vessel().plan.burns().len();
    if ui.selected.is_some_and(|i| i >= count) {
        ui.selected = None;
    }
    if !rig.map_view || locked.0 || frame.origin_body.is_none() {
        ui.drag = None;
        ui.press = None;
        return;
    }
    let (camera, transform) = *camera;
    let cursor = cursor_in_view(&window, &hover_map, &ui_nodes);
    let eye = rig.position;

    // The selected burn: where it is, its frame, and its handles on screen.
    let selected = ui.selected.and_then(|i| {
        let (patch, end) = prediction.burn(i)?;
        let PatchEnd::Burn { state, .. } = end else {
            return None;
        };
        let basis = Burn::frame(&state)?;
        Some((i, patch.frame, state, basis))
    });
    let mut handles = Vec::new();
    if let Some((_, body, state, basis)) = selected {
        let at = frame.relative(body, state.position);
        let length = at.distance(eye) * HANDLE_SCALE;
        for (axis, color) in AXIS_COLORS.into_iter().enumerate() {
            for sign in [1.0, -1.0] {
                let tip = at + basis.col(axis) * sign * length;
                gizmos.line(to_render(at), to_render(tip), color);
                gizmos.sphere(to_render(tip), (length * 0.08) as f32, color);
                handles.push((axis, sign, project(camera, transform, to_render(tip))));
            }
        }
        let marker = project(camera, transform, to_render(at));
        if let (Some(cursor), Some(marker)) = (cursor, marker)
            && buttons.just_pressed(MouseButton::Left)
        {
            let grabbed = handles
                .iter()
                .filter_map(|&(axis, sign, screen)| Some((axis, sign, screen?.distance(cursor))))
                .filter(|(_, _, d)| *d < GRAB_PX)
                .min_by(|a, b| a.2.total_cmp(&b.2));
            if let Some((axis, sign, _)) = grabbed {
                ui.drag = Some(Drag::Axis {
                    axis,
                    sign,
                    from: cursor,
                });
            } else if marker.distance(cursor) < GRAB_PX {
                ui.drag = Some(Drag::Time);
            }
        }
    }

    if !buttons.pressed(MouseButton::Left) {
        ui.drag = None;
    }
    match (ui.drag, selected, cursor) {
        (Some(Drag::Axis { axis, sign, from }), Some((i, body, state, basis)), Some(cursor)) => {
            // Pulling along the handle's direction on screen adds Δv that way.
            let at = frame.relative(body, state.position);
            let tip = at + basis.col(axis) * sign * at.distance(eye) * HANDLE_SCALE;
            let (Some(a), Some(b)) = (
                project(camera, transform, to_render(at)),
                project(camera, transform, to_render(tip)),
            ) else {
                return;
            };
            let pull = (cursor - from).dot((b - a).normalize_or_zero()) / DRAG_PX;
            let rate = (pull * pull.abs()) as f64 * sign;
            let dt = time.delta_secs_f64();
            ui.selected = Some(i);
            edit_selected(&mut sim, &mut ui, |burn| burn.delta_v[axis] += rate * dt);
        }
        (Some(Drag::Time), Some((i, _, _, _)), Some(_)) => {
            // Follow the mouse along the trajectory before the burn.
            if let Some(point) = hover.0
                && point.time > sim.universe.time()
            {
                ui.selected = Some(i);
                edit_selected(&mut sim, &mut ui, |burn| burn.time = point.time);
            }
        }
        _ => {}
    }

    // A click (press and release in place) on the trajectory adds a burn there; Shift+click
    // picks a target instead.
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    if ui.drag.is_none() && !shift && buttons.just_pressed(MouseButton::Left) && hover.0.is_some() {
        ui.press = cursor;
    }
    if buttons.just_released(MouseButton::Left) {
        if let (Some(from), Some(to), Some(point)) = (ui.press.take(), cursor, hover.0)
            && from.distance(to) < 4.0
            && point.time > sim.universe.time()
        {
            let burn = Burn {
                time: point.time,
                delta_v: DVec3::ZERO,
            };
            ui.selected = Some(sim.active_vessel_mut().plan.add(burn));
        }
        ui.press = None;
    }
}

fn publish_plan(
    sim: Res<Sim>,
    ui: Res<PlanUi>,
    hover: Res<TrajectoryHover>,
    prediction: Res<Prediction>,
    mut data: ResMut<UiData>,
) {
    let system = &sim.universe.system;
    let now = sim.render_time;
    let vessel = sim.active_vessel();
    let pressure = vessel.flight.static_pressure;
    let timing = |burn: &Burn| vessel.burn_timing(burn, pressure);
    let burns = vessel.plan.burns();
    let rows: Vec<_> = burns
        .iter()
        .enumerate()
        .map(|(i, burn)| {
            let t = timing(burn);
            record([
                ("index", i.into()),
                ("label", format!("B{}", i + 1).into()),
                ("time_to", burn.time.seconds_since(now).into()),
                ("delta_v", burn.delta_v.length().into()),
                ("duration", t.map(|t| t.duration).into()),
                ("start_in", t.map(|t| t.start.seconds_since(now)).into()),
                ("selected", (ui.selected == Some(i)).into()),
            ])
        })
        .collect();
    data.set("plan.count", rows.len());
    data.set("plan.burns", rows);
    data.set(
        "plan.total_dv",
        burns.iter().map(|b| b.delta_v.length()).sum::<f64>(),
    );
    data.set("plan.dv_step", ui.dv_step);
    data.set("plan.time_step", ui.time_step);
    let selected = ui.selected.and_then(|i| Some((i, burns.get(i)?)));
    data.set("plan.has_selection", selected.is_some());
    data.set("plan.selected", selected.map(|(i, _)| i));
    if let Some((i, burn)) = selected {
        let t = timing(burn);
        data.set("burn.label", format!("B{}", i + 1));
        data.set("burn.prograde", burn.delta_v.x);
        data.set("burn.normal", burn.delta_v.y);
        data.set("burn.radial", burn.delta_v.z);
        data.set("burn.delta_v", burn.delta_v.length());
        data.set("burn.time_to", burn.time.seconds_since(now));
        data.set("burn.duration", t.map(|t| t.duration));
        data.set("burn.start_in", t.map(|t| t.start.seconds_since(now)));
        // What the trajectory looks like after it.
        let after = prediction.burn(i).and_then(|(patch, end)| {
            let PatchEnd::Burn { state, delta_v, .. } = end else {
                return None;
            };
            let next = StateVector::new(state.position, state.velocity + delta_v);
            Some((patch.frame, Orbit::new(patch.orbit.mu(), patch.end, next)))
        });
        let radius = |frame| system.body(frame).radius;
        data.set(
            "burn.apoapsis",
            after.and_then(|(f, o)| o.apoapsis().map(|a| a - radius(f))),
        );
        data.set(
            "burn.periapsis",
            after.map(|(f, o)| o.periapsis() - radius(f)),
        );
        // Where it leads at the target body: the periapsis of the encounter.
        let arrival = match vessel.target {
            Some(Target::Body(body)) => prediction
                .patches
                .iter()
                .skip_while(|p| p.end <= burn.time)
                .find(|p| p.frame == body)
                .map(|p| p.orbit.periapsis() - system.body(body).radius),
            _ => None,
        };
        data.set("burn.arrival_periapsis", arrival);
        data.set(
            "burn.targets_body",
            matches!(vessel.target, Some(Target::Body(_))),
        );
    }

    let executor = vessel.executor();
    data.set("autoburn.engaged", executor.engaged());
    data.set(
        "autoburn.state",
        if executor.burning() {
            "Burning"
        } else if executor.engaged() {
            "Waiting"
        } else {
            "Off"
        },
    );
    data.set(
        "autoburn.remaining",
        vessel
            .burn_vector(system, sim.universe.time())
            .map(|v| v.length()),
    );

    data.set("map.hovering", hover.0.is_some());
    if let Some(point) = hover.0 {
        let frame = prediction.patches.get(point.patch).map(|p| p.frame);
        data.set("map.hover_time_to", point.time.seconds_since(now));
        data.set(
            "map.hover_altitude",
            frame.map(|f| point.local.length() - system.body(f).radius),
        );
    }
}
