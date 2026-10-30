//! Planning burns (KSP's manoeuvre nodes): the Flight Plan window's actions, placing and
//! shaping burns in map view, and warping to points of the trajectory.
//!
//! In map view, as in KSP:
//!
//! - The pointer on the trajectory shows a ghost node where a burn would go, snapping onto
//!   apsides, plane crossings and closest approaches. Click to add a burn there, or
//!   right-click for a menu (add a burn, warp there).
//! - Every planned burn is a node on the trajectory, and the trajectory after it is dashed.
//!   Click a node to select it: six handles (prograde, normal and radial, both ways) appear
//!   round it, and a panel beside the navball gives its figures. Drag a handle outwards to
//!   add Δv that way, faster the further you pull; drag the node to slide it along its orbit.
//!   Right-click a node for its menu; click it again, or empty space, to put it away.
//!
//! The plan itself lives on the vessel ([`osp_sim::plan`]); the executor flies it.
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
//! | `map.add_burn`, `map.warp_here` | From the map's menu: a burn at / warp to its point |
//! | `warp.to:<burn|ap|pe|soi>` | Warp there, slowing on the way in |

use bevy::{input::mouse::MouseButton, math::DVec3, prelude::*};
use osp_sim::{
    autopilot::{ULLAGE_LEAD, WARP_MARGIN},
    body::BodyId,
    orbit::{Orbit, StateVector},
    plan::Burn,
    prediction::PatchEnd,
    rendezvous::Target,
    time::SimTime,
    transfer::{Course, aim_periapsis},
    vessel::Motion,
};
use osp_ui::{CLICK_SLOP, LayoutVisibility, UiAction, UiData, record};

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera},
    frame::{RenderFrame, to_render},
    map_labels::{MapLabels, Marker, body_spheres, hidden},
    orbit_lines::{
        HoverPoint, PREDICTION_HORIZON, Prediction, TrajectoryHover, ViewPointer, countdown,
        distance_label, nearest_on_line, project,
    },
    sim::{Sim, SimRestarted, TimeWarp},
    targeting::TargetMarksSet,
};

/// Grab distance for handles and nodes, logical pixels.
const GRAB_PX: f32 = 14.0;
/// How near a marker (apsis, plane crossing, closest approach) the pointer snaps onto it.
const SNAP_PX: f32 = 12.0;
/// Handle length as a share of the distance to the camera (some 60 px on screen).
const HANDLE_SCALE: f64 = 0.055;
/// Δv rate while dragging a handle `d` pixels out: (d / DRAG_PX)² m/s per second.
const DRAG_PX: f32 = 12.0;
/// Margin before a burn's start when warping to it, s.
const WARP_TO_BURN: f64 = WARP_MARGIN + ULLAGE_LEAD + 2.0;
const AXES: [&str; 3] = ["prograde", "normal", "radial"];
const AXIS_NAMES: [&str; 3] = ["Prograde", "Normal", "Radial"];
/// Each axis's handles, the positive way first: the navball's own markers.
const HANDLE_ICONS: [[&str; 2]; 3] = [
    ["icons/sas_prograde.svg", "icons/sas_retrograde.svg"],
    ["icons/sas_normal.svg", "icons/sas_anti_normal.svg"],
    ["icons/sas_radial_out.svg", "icons/sas_radial_in.svg"],
];
const HANDLE_PX: f32 = 22.0;
const AXIS_COLORS: [Color; 3] = [
    Color::srgb(0.84, 0.95, 0.24),
    Color::srgb(0.84, 0.36, 1.0),
    Color::srgb(0.24, 0.85, 1.0),
];
const NODE_ICON: &str = "icons/node.svg";
const NODE_PX: f32 = 18.0;
const NODE_COLOR: Color = Color::srgb(0.35, 0.7, 1.0);
/// The selected node, in the HUD's accent.
const SELECTED_COLOR: Color = Color::srgb(0.22, 0.88, 0.77);
/// Where a click would put a burn.
const GHOST_COLOR: Color = Color::srgba(1.0, 1.0, 1.0, 0.75);
/// The map's context menu (layout).
const MENU: &str = "map_menu";

pub struct FlightPlanPlugin;

impl Plugin for FlightPlanPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlanUi>()
            .add_systems(
                Update,
                (forget_on_restart, plan_actions)
                    .chain()
                    .in_set(FrameSet::Input),
            )
            .add_systems(
                Update,
                (map_interaction, publish_plan)
                    .chain()
                    .in_set(FrameSet::Render)
                    .after(crate::orbit_lines::OrbitLinesSet)
                    .after(TargetMarksSet),
            );
    }
}

/// A scenario started afresh: the burn that was selected belonged to another vessel.
fn forget_on_restart(mut restarted: MessageReader<SimRestarted>, mut ui: ResMut<PlanUi>) {
    if restarted.read().count() > 0 {
        *ui = PlanUi::default();
    }
}

#[derive(Resource)]
pub struct PlanUi {
    pub selected: Option<usize>,
    pub dv_step: f64,
    pub time_step: f64,
    drag: Option<Drag>,
    /// Where a left click began and what was under it: released in place, it acts on that.
    press: Option<(Vec2, Under)>,
    /// Where a right click began.
    right_press: Option<Vec2>,
    /// What the map's menu was opened on. `menu_next` opens it next frame, once the click
    /// asking for it has closed any menu already open.
    menu: Option<MenuOn>,
    menu_next: Option<MenuOn>,
}

impl Default for PlanUi {
    fn default() -> Self {
        Self {
            selected: None,
            dv_step: 10.0,
            time_step: 60.0,
            drag: None,
            press: None,
            right_press: None,
            menu: None,
            menu_next: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Drag {
    /// A Δv handle: burn-frame axis (0..3), which way, and where the drag started.
    Axis { axis: usize, sign: f64, from: Vec2 },
    /// The selected burn's node, along its orbit.
    Time,
}

#[derive(Debug, Clone, Copy)]
enum Under {
    Burn(usize),
    Trajectory(HoverPoint),
    Nothing,
}

#[derive(Debug, Clone, Copy)]
enum MenuOn {
    Point(HoverPoint),
    /// The selected burn.
    Burn,
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
    mut layouts: ResMut<LayoutVisibility>,
    prediction: Res<Prediction>,
    time: Res<Time>,
) {
    // Not gated by `InputLocked`: these come from buttons (and scripts), not raw input.
    let real = time.elapsed_secs_f64();
    for action in actions.read() {
        let arg = action.arg().unwrap_or_default();
        let now = sim.universe.time();
        // Whatever the map's menu offered has been chosen (or something else has).
        if action.name().starts_with("plan.")
            || action.name().starts_with("map.")
            || action.name() == "warp.to"
        {
            layouts.layouts.insert(MENU.into(), false);
        }
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
            "map.add_burn" => {
                if let Some(MenuOn::Point(point)) = ui.menu
                    && point.time > now
                {
                    let burn = Burn {
                        time: point.time,
                        delta_v: DVec3::ZERO,
                    };
                    ui.selected = Some(sim.active_vessel_mut().plan.add(burn));
                }
            }
            "map.warp_here" => {
                if let Some(MenuOn::Point(point)) = ui.menu {
                    warp.warp_to(point.time, "the chosen point", &sim, real);
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

/// A planned burn's node on the trajectory.
struct BurnNode {
    index: usize,
    /// Body the state is relative to.
    body: BodyId,
    state: StateVector,
    at: Vec3,
    screen: Option<Vec2>,
}

/// One of the selected burn's handles.
struct Handle {
    axis: usize,
    sign: f64,
    icon: &'static str,
    tip: Vec3,
    screen: Option<Vec2>,
    /// Its way on screen (unit), before any pull.
    direction: Vec2,
}

#[allow(clippy::too_many_arguments)]
fn map_interaction(
    mut sim: ResMut<Sim>,
    mut ui: ResMut<PlanUi>,
    rig: Res<CameraRig>,
    frame: Res<RenderFrame>,
    prediction: Res<Prediction>,
    hover: Res<TrajectoryHover>,
    pointer: ViewPointer,
    (buttons, keys, time): (
        Res<ButtonInput<MouseButton>>,
        Res<ButtonInput<KeyCode>>,
        Res<Time>,
    ),
    camera: Single<(&Camera, &Transform), With<MainCamera>>,
    mut layouts: ResMut<LayoutVisibility>,
    mut labels: ResMut<MapLabels>,
    mut gizmos: Gizmos,
) {
    let count = sim.active_vessel().plan.burns().len();
    if ui.selected.is_some_and(|i| i >= count) {
        ui.selected = None;
    }
    if let Some(on) = ui.menu_next.take() {
        ui.menu = Some(on);
        layouts.layouts.insert(MENU.into(), true);
    }
    if !rig.map_view || frame.origin_body.is_none() {
        ui.drag = None;
        ui.press = None;
        ui.right_press = None;
        if layouts.shown(MENU) {
            layouts.layouts.insert(MENU.into(), false);
        }
        return;
    }
    let (camera, transform) = *camera;
    // Where a point is on screen, unless a body hides it.
    let spheres = body_spheres(&sim, &frame);
    let eye = rig.position;
    let project =
        |p: Vec3| project(camera, transform, p).filter(|_| !hidden(to_render(eye), p, &spheres));
    let cursor = pointer.position();
    let now = sim.universe.time();

    let nodes: Vec<BurnNode> = prediction
        .patches
        .iter()
        .filter_map(|patch| {
            let Some(PatchEnd::Burn { index, state, .. }) = patch.ending else {
                return None;
            };
            let at = frame.to_render(patch.frame, state.position);
            Some(BurnNode {
                index,
                body: patch.frame,
                state,
                at,
                screen: project(at),
            })
        })
        .collect();
    let selected = ui
        .selected
        .and_then(|i| nodes.iter().find(|n| n.index == i));

    // The selected burn's handles; one being dragged follows the pointer out along its way.
    let mut handles = Vec::new();
    if let Some(node) = selected
        && let Some(basis) = Burn::frame(&node.state)
    {
        let at = frame.relative(node.body, node.state.position);
        let length = at.distance(eye) * HANDLE_SCALE;
        for (axis, icons) in HANDLE_ICONS.iter().enumerate() {
            for (icon, sign) in icons.iter().zip([1.0, -1.0]) {
                let way = basis.col(axis) * sign * length;
                let (Some(a), Some(b)) = (node.screen, project(to_render(at + way))) else {
                    continue;
                };
                let (direction, screen_length) = ((b - a).normalize_or_zero(), a.distance(b));
                let mut reach = 1.0;
                if let (
                    Some(Drag::Axis {
                        axis: x,
                        sign: s,
                        from,
                    }),
                    Some(cursor),
                ) = (ui.drag, cursor)
                    && (x, s) == (axis, sign)
                    && screen_length > 1.0
                {
                    let pull = (cursor - from).dot(direction) / screen_length;
                    reach += pull.clamp(-0.7, 3.0) as f64;
                }
                let tip = to_render(at + way * reach);
                handles.push(Handle {
                    axis,
                    sign,
                    icon,
                    tip,
                    screen: project(tip),
                    direction,
                });
            }
        }
    }

    // What is under the pointer: a handle, else a node, else a point of the trajectory,
    // caught by a marker on it if one is close.
    let handle_under = nearest_within(cursor, handles.iter().map(|h| (h.screen, GRAB_PX)));
    let node_under =
        nearest_within(cursor, nodes.iter().map(|n| (n.screen, GRAB_PX))).map(|k| nodes[k].index);
    let snaps: Vec<(HoverPoint, Option<Vec2>)> = labels
        .markers()
        .iter()
        .filter_map(|m| Some((m.snap?, project(m.at))))
        .collect();
    let snapped =
        nearest_within(cursor, snaps.iter().map(|(_, at)| (*at, SNAP_PX))).map(|k| snaps[k].0);
    let point = snapped.or(hover.0).filter(|p| p.time > now);

    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if let Some(cursor) = cursor {
        // Shift+click picks a target instead (see `targeting`).
        if buttons.just_pressed(MouseButton::Left) && !shift {
            if let Some(k) = handle_under {
                let (axis, sign) = (handles[k].axis, handles[k].sign);
                ui.drag = Some(Drag::Axis {
                    axis,
                    sign,
                    from: cursor,
                });
            } else {
                let under = match (node_under, point) {
                    (Some(i), _) => Under::Burn(i),
                    (None, Some(point)) => Under::Trajectory(point),
                    (None, None) => Under::Nothing,
                };
                ui.press = Some((cursor, under));
            }
        }
        // A press on a node that moves off drags the node.
        if let Some((from, Under::Burn(i))) = ui.press
            && buttons.pressed(MouseButton::Left)
            && from.distance(cursor) >= CLICK_SLOP
        {
            ui.press = None;
            ui.selected = Some(i);
            ui.drag = Some(Drag::Time);
        }
        if buttons.just_pressed(MouseButton::Right) {
            ui.right_press = Some(cursor);
        }
    }
    if !buttons.pressed(MouseButton::Left) {
        ui.drag = None;
    }

    match (ui.drag, cursor) {
        (Some(Drag::Axis { axis, sign, from }), Some(cursor)) => {
            // Pulling along the handle's way on screen adds Δv that way.
            let way = handles
                .iter()
                .find(|h| (h.axis, h.sign) == (axis, sign))
                .map(|h| h.direction);
            if let Some(way) = way {
                let pull = (cursor - from).dot(way) / DRAG_PX;
                let rate = (pull * pull.abs()) as f64 * sign;
                let dt = time.delta_secs_f64();
                edit_selected(&mut sim, &mut ui, |burn| burn.delta_v[axis] += rate * dt);
            }
        }
        (Some(Drag::Time), Some(cursor)) => {
            // Round the orbit the burn is on, from where that leg starts.
            let moved = ui.selected.and_then(|i| {
                let (patch, _) = prediction.burn(i)?;
                let from = patch.start.max(now);
                let span = patch.end.seconds_since(from).max(60.0);
                let to = from.add_seconds(patch.orbit.period().unwrap_or(2.0 * span));
                let tolerance = |p: DVec3| frame.relative(patch.frame, p).distance(eye) * 1e-3;
                let points = patch.orbit.sample_between(from, to, tolerance);
                let screen: Vec<_> = points
                    .iter()
                    .map(|p| project(frame.to_render(patch.frame, *p)))
                    .collect();
                let (_, local) = nearest_on_line(&points, &screen, cursor, f32::INFINITY)?;
                patch.orbit.next_time_at(local, from).filter(|t| *t > now)
            });
            if let Some(moved) = moved {
                edit_selected(&mut sim, &mut ui, |burn| burn.time = moved);
            }
        }
        _ => {}
    }

    // Clicks: on a node to select it (or put it away), on the trajectory to add a burn, on
    // empty space to put the selection away. A right-click opens the menu.
    if buttons.just_released(MouseButton::Left) {
        if let (Some((from, under)), Some(cursor)) = (ui.press.take(), cursor)
            && from.distance(cursor) < CLICK_SLOP
        {
            match under {
                Under::Burn(i) => ui.selected = (ui.selected != Some(i)).then_some(i),
                Under::Trajectory(point) => {
                    let burn = Burn {
                        time: point.time,
                        delta_v: DVec3::ZERO,
                    };
                    ui.selected = Some(sim.active_vessel_mut().plan.add(burn));
                }
                Under::Nothing => ui.selected = None,
            }
        }
        ui.press = None;
    }
    if buttons.just_released(MouseButton::Right) {
        if let (Some(from), Some(cursor)) = (ui.right_press.take(), cursor)
            && from.distance(cursor) < CLICK_SLOP
        {
            if let Some(i) = node_under {
                ui.selected = Some(i);
                ui.menu_next = Some(MenuOn::Burn);
            } else if let Some(point) = point {
                ui.menu_next = Some(MenuOn::Point(point));
            }
        }
        ui.right_press = None;
    }

    // The nodes, the selected one's handles, and a ghost node where a click would add one.
    let system = &sim.universe.system;
    let burns = sim.active_vessel().plan.burns();
    for node in &nodes {
        let Some(burn) = burns.get(node.index) else {
            continue;
        };
        let chosen = ui.selected == Some(node.index);
        let (size, color) = if chosen {
            (NODE_PX * 1.25, SELECTED_COLOR)
        } else {
            (NODE_PX, NODE_COLOR)
        };
        let marker = Marker::icon(node.at, NODE_ICON, size, color);
        // The selected one's figures are in its panel. Numbered when there are several, so
        // they match the flight plan's list.
        if chosen {
            labels.add(marker);
            continue;
        }
        let label = format!("B{}", node.index + 1);
        let detail = format!(
            "{label} {:.1} m/s, {}",
            burn.delta_v.length(),
            countdown(burn.time.seconds_since(frame.time))
        );
        let marker = marker.detail(detail);
        labels.add(if burns.len() > 1 {
            marker.tag(label)
        } else {
            marker
        });
    }
    if let Some(node) = selected
        && let Some(burn) = burns.get(node.index)
    {
        for handle in &handles {
            let color = AXIS_COLORS[handle.axis].with_alpha(0.5);
            gizmos.line(node.at, handle.tip, color);
            let detail = format!(
                "{} {:.1} m/s",
                AXIS_NAMES[handle.axis], burn.delta_v[handle.axis]
            );
            labels
                .add(Marker::icon(handle.tip, handle.icon, HANDLE_PX, Color::WHITE).detail(detail));
        }
    }
    let busy = ui.drag.is_some() || handle_under.is_some() || node_under.is_some();
    if let Some(point) = point.filter(|_| !busy) {
        let altitude = point.local.length() - system.body(point.frame).radius;
        let tag = format!(
            "{} · {}",
            countdown(point.time.seconds_since(frame.time)),
            distance_label(altitude)
        );
        let at = frame.to_render(point.frame, point.local);
        labels.add(Marker::icon(at, NODE_ICON, NODE_PX * 0.9, GHOST_COLOR).tag(tag));
    }
}

/// The index of the point nearest `cursor` among `points` (screen point, reach), if within
/// its reach.
fn nearest_within(
    cursor: Option<Vec2>,
    points: impl Iterator<Item = (Option<Vec2>, f32)>,
) -> Option<usize> {
    let cursor = cursor?;
    points
        .enumerate()
        .filter_map(|(k, (at, reach))| Some((k, at?.distance(cursor))).filter(|(_, d)| *d < reach))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
}

fn publish_plan(
    sim: Res<Sim>,
    ui: Res<PlanUi>,
    rig: Res<CameraRig>,
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
    data.set("map.node_panel", rig.map_view && selected.is_some());
    if let Some((i, burn)) = selected {
        let t = timing(burn);
        data.set("burn.label", format!("B{}", i + 1));
        data.set("burn.is_next", i == 0);
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

    let altitude = |point: &HoverPoint| point.local.length() - system.body(point.frame).radius;
    data.set("map.hovering", hover.0.is_some());
    if let Some(point) = hover.0 {
        data.set("map.hover_time_to", point.time.seconds_since(now));
        data.set("map.hover_altitude", altitude(&point));
    }
    data.set("map.menu_node", matches!(ui.menu, Some(MenuOn::Burn)));
    if let Some(MenuOn::Point(point)) = ui.menu {
        data.set("map.menu_time_to", point.time.seconds_since(now));
        data.set("map.menu_altitude", altitude(&point));
    }
}
