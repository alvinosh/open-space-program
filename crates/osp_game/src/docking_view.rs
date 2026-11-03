//! What a pilot sees to dock by: a ring on each port and a line out along the target port's axis in
//! the world, a view out of the vessel's own port through a crosshair, and for checking them the
//! triangles vessels collide with.
//!
//! | Action | Does |
//! | --- | --- |
//! | `camera.port_view` | Look out of the vessel's port, or back at the vessel |
//! | `debug.colliders` | Draw what vessels collide with, or stop |

use bevy::{math::DVec3, prelude::*};
use osp_sim::{docking::PortApproach, vessel::Vessel};
use osp_ui::{UiAction, UiData};

use crate::{
    FrameSet,
    camera::{CameraRig, update_camera},
    frame::{RenderFrame, to_render},
    sim::Sim,
};

/// How far from the target's port the guides are drawn, m.
const GUIDE_RANGE: f64 = 500.0;
/// How far past the vessel the target port's axis is drawn, m, and at least how far.
const AXIS_PAST: f64 = 15.0;
const AXIS_LEAST: f64 = 30.0;
/// How far out of its own port the vessel's axis is drawn, m.
const OWN_AXIS: f64 = 6.0;
/// Segments round a ring.
const RING: usize = 48;
/// The most lines of colliders it draws in a frame.
const MOST_EDGES: usize = 400_000;
/// How far round the vessel it draws colliders, m.
const OVERLAY_RANGE: f64 = 200.0;
/// The eye is this far in front of the port, m: clear of what the port is on.
const EYE_AHEAD: f64 = 0.3;

const TARGET_COLOR: Color = Color::srgb(0.95, 0.45, 0.85);
const OWN_COLOR: Color = Color::srgb(0.3, 0.9, 0.85);
const READY_COLOR: Color = Color::srgb(0.35, 1.0, 0.45);
/// Bright enough to show on a white hull in full sun, which the exposure takes well past white.
const COLLIDER_COLOR: Color = Color::LinearRgba(LinearRgba::rgb(40.0, 1.5, 0.3));

pub struct DockingViewPlugin;

impl Plugin for DockingViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ShowColliders>()
            .add_systems(Update, view_actions.in_set(FrameSet::Input))
            .add_systems(
                Update,
                (
                    look_out_of_port.after(update_camera),
                    draw_guides,
                    draw_colliders,
                )
                    .in_set(FrameSet::Render),
            );
    }
}

/// Draw what vessels collide with.
#[derive(Resource, Default)]
struct ShowColliders(bool);

fn view_actions(
    mut actions: MessageReader<UiAction>,
    mut rig: ResMut<CameraRig>,
    mut show: ResMut<ShowColliders>,
) {
    for action in actions.read() {
        match action.name() {
            "camera.port_view" => rig.port_view = !rig.port_view,
            "debug.colliders" => show.0 = !show.0,
            _ => {}
        }
    }
}

/// Where the vessel's own port is and which way it faces, in its parent's frame: the one it would
/// dock by, or else its first free one.
fn own_port(sim: &Sim) -> Option<(DVec3, DVec3)> {
    let (system, now) = (&sim.universe.system, sim.render_time);
    if let Some(approach) = sim.universe.port_approach(sim.active_index(), now) {
        return Some((approach.near.at.position, approach.near.out));
    }
    let vessel = sim.active_vessel();
    let ports = vessel.propulsion.docking_ports();
    let port = ports.iter().find(|p| p.free)?;
    let state = vessel.port_state(system, port, now);
    Some((state.at.position, state.out))
}

/// The camera out of the vessel's port, the pilot's head up: the crosshair is its axis.
fn look_out_of_port(
    mut rig: ResMut<CameraRig>,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    mut data: ResMut<UiData>,
    mut camera: Single<&mut Transform, With<crate::camera::MainCamera>>,
) {
    let port = own_port(&sim).filter(|_| rig.port_view && !rig.map_view);
    data.set("dock.port_view", port.is_some());
    let Some((at, out)) = port else {
        return;
    };
    let vessel = sim.active_vessel();
    let turned = vessel
        .control_attitude(&sim.universe.system, frame.time)
        .orientation;
    let head = turned * DVec3::NEG_Y;
    let eye = frame.relative(vessel.parent(), at) + out * EYE_AHEAD;
    rig.position = eye;
    **camera =
        Transform::from_translation(to_render(eye)).looking_to(to_render(out), to_render(head));
}

/// A ring round `middle`, across `out`.
fn ring(gizmos: &mut Gizmos, middle: DVec3, out: DVec3, radius: f64, color: Color) {
    let side = out.any_orthonormal_vector();
    let other = out.cross(side);
    let point = |k: usize| {
        let angle = std::f64::consts::TAU * k as f64 / RING as f64;
        to_render(middle + (side * angle.cos() + other * angle.sin()) * radius)
    };
    for k in 0..RING {
        gizmos.line(point(k), point(k + 1), color);
    }
}

fn draw_guides(sim: Res<Sim>, rig: Res<CameraRig>, frame: Res<RenderFrame>, mut gizmos: Gizmos) {
    if rig.map_view {
        return;
    }
    let vessel = sim.active_vessel();
    let Some(approach): Option<PortApproach> = sim
        .universe
        .port_approach(sim.active_index(), sim.render_time)
    else {
        return;
    };
    let a = approach.alignment;
    if a.distance() > GUIDE_RANGE {
        return;
    }
    let parent = vessel.parent();
    let far = frame.relative(parent, approach.far.at.position);
    let near = frame.relative(parent, approach.near.at.position);
    let ready = a.problem().is_none();
    let target = if ready { READY_COLOR } else { TARGET_COLOR };
    let radius = approach.target_port.diameter / 2.0;
    ring(&mut gizmos, far, a.out, radius, target);
    // Rings down the axis every so often, to fly through, and the line itself.
    let reach = (a.gap + AXIS_PAST).max(AXIS_LEAST);
    gizmos.line(to_render(far), to_render(far + a.out * reach), target);
    let mut along = 5.0;
    while along < reach {
        ring(&mut gizmos, far + a.out * along, a.out, radius, target);
        along += 5.0;
    }
    let own = if ready { READY_COLOR } else { OWN_COLOR };
    let out = approach.near.out;
    ring(&mut gizmos, near, out, approach.port.diameter / 2.0, own);
    gizmos.line(to_render(near), to_render(near + out * OWN_AXIS), own);
}

/// The edges of the convex pieces vessels collide with, for vessels near the active one.
fn draw_colliders(
    mut sim: ResMut<Sim>,
    show: Res<ShowColliders>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    mut gizmos: Gizmos,
) {
    if !show.0 || rig.map_view {
        return;
    }
    let now = sim.render_time;
    let sim = &mut *sim;
    let own = sim.universe.focus.and_then(|id| sim.universe.vessel(id));
    let Some(here) = own.map(|v| (v.parent(), v.state_at(&sim.universe.system, now).position))
    else {
        return;
    };
    let near: Vec<&Vessel> = (sim.universe.vessels.iter())
        .filter(|v| {
            v.parent() == here.0
                && v.state_at(&sim.universe.system, now)
                    .position
                    .distance(here.1)
                    < OVERLAY_RANGE
        })
        .collect();
    let share = MOST_EDGES / near.len().max(1);
    for vessel in near {
        let collider = sim.universe.colliders.of(vessel.propulsion.parts());
        // What has not been cut yet appears when it has.
        sim.universe.colliders.prepare(&collider);
        let at = vessel.assembly_frame(&sim.universe.system, now);
        let body = vessel.parent();
        let mut left = share;
        for part in &collider.parts {
            let place = at * part.pose;
            let edges = part.mesh.piece_edges();
            for edge in edges.iter().take(left) {
                let [a, b] = edge.map(|p| to_render(frame.relative(body, place.point(p))));
                gizmos.line(a, b, COLLIDER_COLOR);
            }
            left = left.saturating_sub(edges.len());
        }
    }
}
