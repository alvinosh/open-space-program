//! Turns named actions (from keybindings and UI buttons) into vessel and game commands.
//!
//! Keys are bound in data (`keybindings` collection), so mods can rebind them or add
//! buttons that fire the same actions. Actions handled here:
//!
//! | Action | Kind |
//! | --- | --- |
//! | `vessel.stage`, `vessel.throttle_full`, `vessel.throttle_cut` | press |
//! | `vessel.throttle_up`, `vessel.throttle_down` | hold |
//! | `vessel.pitch_down`/`_up`, `vessel.yaw_left`/`_right`, `vessel.roll_left`/`_right` | hold |
//! | `vessel.sas_toggle`, `vessel.sas_mode:<mode>` | press |
//! | `vessel.rcs_toggle` | press |
//! | `vessel.translate_forward`/`_back`, `_left`/`_right`, `_up`/`_down` | hold |
//! | `nav.toggle_frame` | press |
//! | `warp.increase`, `warp.decrease`, `warp.stop` | press |
//! | `view.toggle_map` | press |

use bevy::{math::DVec3, prelude::*};
use osp_sim::attitude::{NavFrame, SasMode};
use osp_ui::{ActionState, UiAction};

use crate::{
    FrameSet,
    camera::CameraRig,
    sim::{Sim, TimeWarp},
};

/// Throttle change per second while a throttle key is held.
const THROTTLE_RATE: f64 = 0.5;

pub struct ControlsPlugin;

impl Plugin for ControlsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputLocked>()
            .add_systems(Update, apply_actions.in_set(FrameSet::Input));
    }
}

/// When set, player input (keyboard and camera) is ignored, e.g. during scripted tests.
#[derive(Resource, Default)]
pub struct InputLocked(pub bool);

fn apply_actions(
    mut actions: MessageReader<UiAction>,
    held: Res<ActionState>,
    time: Res<Time>,
    mut sim: ResMut<Sim>,
    mut warp: ResMut<TimeWarp>,
    mut rig: ResMut<CameraRig>,
    locked: Res<InputLocked>,
) {
    if locked.0 {
        actions.read().for_each(drop);
        return;
    }
    let dt = time.delta_secs_f64();
    let now = time.elapsed_secs_f64();
    let ut = sim.universe.time();

    let vessel = sim.active_vessel_mut();
    let throttle = vessel.propulsion.throttle()
        + held.axis("vessel.throttle_up", "vessel.throttle_down") * THROTTLE_RATE * dt;
    vessel.set_throttle(throttle);
    vessel.rotation_input = DVec3::new(
        held.axis("vessel.pitch_up", "vessel.pitch_down"),
        held.axis("vessel.yaw_right", "vessel.yaw_left"),
        held.axis("vessel.roll_right", "vessel.roll_left"),
    );
    vessel.translation_input = DVec3::new(
        held.axis("vessel.translate_right", "vessel.translate_left"),
        held.axis("vessel.translate_down", "vessel.translate_up"),
        held.axis("vessel.translate_forward", "vessel.translate_back"),
    );

    for action in actions.read() {
        let (vessel, system) = sim.active_vessel_in_system();
        match action.name() {
            "vessel.stage" => vessel.activate_next_stage(system, ut),
            "vessel.throttle_full" => vessel.set_throttle(1.0),
            "vessel.throttle_cut" => vessel.set_throttle(0.0),
            "vessel.sas_toggle" => {
                let on = !vessel.sas().enabled();
                vessel.set_sas(system, ut, on);
            }
            "vessel.sas_mode" => match action.arg().and_then(SasMode::from_id) {
                Some(mode) => vessel.set_sas_mode(system, ut, mode),
                None => warn!("unknown SAS mode in `{}`", action.0),
            },
            "vessel.rcs_toggle" => {
                let on = !vessel.rcs();
                vessel.set_rcs(on);
            }
            "nav.toggle_frame" => {
                let frame = match vessel.nav_frame() {
                    NavFrame::Surface => NavFrame::Orbit,
                    NavFrame::Orbit => NavFrame::Surface,
                };
                vessel.set_nav_frame(system, ut, frame);
            }
            "warp.increase" => warp.increase(&sim, now),
            "warp.decrease" => warp.decrease(),
            "warp.stop" => warp.stop("Warp cancelled", now),
            "view.toggle_map" => rig.map_view = !rig.map_view,
            _ => {}
        }
    }
}
