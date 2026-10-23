//! Turns named actions (from keybindings and UI buttons) into vessel and game commands.
//!
//! Keys are bound in data (`keybindings` collection), so mods can rebind them or add
//! buttons that fire the same actions. Actions handled here:
//!
//! | Action | Kind |
//! | --- | --- |
//! | `vessel.stage` | press: fire the next activation sequence |
//! | `vessel.throttle_full`, `vessel.throttle_cut` | press |
//! | `vessel.throttle_up`, `vessel.throttle_down` | hold |
//! | `vessel.pitch_down`/`_up`, `vessel.yaw_left`/`_right`, `vessel.roll_left`/`_right` | hold |
//! | `vessel.sas_toggle`, `vessel.sas_mode:<mode>` | press |
//! | `vessel.rcs_toggle` | press |
//! | `vessel.next`, `vessel.previous` | press: fly the next / previous vessel |
//! | `vessel.translate_forward`/`_back`, `_left`/`_right`, `_up`/`_down` | hold |
//! | `nav.toggle_frame` | press |
//! | `warp.increase`, `warp.decrease`, `warp.stop` | press |
//! | `view.toggle_map` | press |
//! | `staging.drop`, `staging.drop_new`, `staging.add`, `staging.remove` | rearrange the sequences (see [`edit_sequences`]) |
//!
//! Throttle and SAS commands disengage the burn executor (see `flight_plan` for its actions).

use bevy::{math::DVec3, prelude::*};
use osp_sim::{
    attitude::{NavFrame, SasMode},
    staging::{Sequences, Target},
};
use osp_ui::{ActionState, UiAction};

use crate::{
    FrameSet,
    camera::CameraRig,
    menu::Menu,
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
    menu: Res<Menu>,
) {
    if locked.0 || menu.open {
        actions.read().for_each(drop);
        return;
    }
    let dt = time.delta_secs_f64();
    let now = time.elapsed_secs_f64();
    let ut = sim.universe.time();

    let vessel = sim.active_vessel_mut();
    let throttle_input = held.axis("vessel.throttle_up", "vessel.throttle_down");
    if throttle_input != 0.0 {
        // The pilot takes the throttle back from the burn executor.
        vessel.disengage_burn(ut);
    }
    let throttle = vessel.propulsion.throttle() + throttle_input * THROTTLE_RATE * dt;
    vessel.set_throttle(throttle);
    vessel.rotation_input = DVec3::new(
        held.axis("vessel.pitch_up", "vessel.pitch_down"),
        held.axis("vessel.yaw_right", "vessel.yaw_left"),
        held.axis("vessel.roll_right", "vessel.roll_left"),
    );
    vessel.brakes = held.held("vessel.brakes");
    vessel.translation_input = DVec3::new(
        held.axis("vessel.translate_right", "vessel.translate_left"),
        held.axis("vessel.translate_down", "vessel.translate_up"),
        held.axis("vessel.translate_forward", "vessel.translate_back"),
    );

    for action in actions.read() {
        match action.name() {
            "vessel.next" => {
                sim.switch_vessel(1);
                continue;
            }
            "vessel.previous" => {
                sim.switch_vessel(-1);
                continue;
            }
            "vessel.stage" => {
                sim.stage_active();
                continue;
            }
            _ => {}
        }
        let (vessel, system) = sim.active_vessel_in_system();
        match action.name() {
            "vessel.throttle_full" => {
                vessel.disengage_burn(ut);
                vessel.set_throttle(1.0);
            }
            "vessel.throttle_cut" => {
                vessel.disengage_burn(ut);
                vessel.set_throttle(0.0);
            }
            "vessel.sas_toggle" => {
                vessel.disengage_burn(ut);
                let on = !vessel.sas().enabled();
                vessel.set_sas(system, ut, on);
            }
            "vessel.sas_mode" => match action.arg().and_then(SasMode::from_id) {
                Some(mode) => {
                    // Selecting the current mode again goes back to holding attitude.
                    let sas = vessel.sas();
                    let mode = if sas.enabled() && sas.mode() == mode {
                        SasMode::Stability
                    } else {
                        mode
                    };
                    if mode != SasMode::Burn {
                        vessel.disengage_burn(ut);
                    }
                    vessel.set_sas_mode(system, ut, mode);
                }
                None => warn!("unknown SAS mode in `{}`", action.0),
            },
            "vessel.rcs_toggle" => {
                let on = !vessel.rcs();
                vessel.set_rcs(on);
            }
            "nav.toggle_frame" => {
                let frame = match vessel.nav_frame() {
                    NavFrame::Surface => NavFrame::Orbit,
                    NavFrame::Orbit if vessel.target.is_some() => NavFrame::Target,
                    _ => NavFrame::Surface,
                };
                vessel.set_nav_frame(system, ut, frame);
            }
            "warp.increase" => warp.increase(&sim, now),
            "warp.decrease" => warp.decrease(),
            "warp.stop" => warp.stop("Warp cancelled", now),
            "view.toggle_map" => rig.map_view = !rig.map_view,
            "staging.drop" | "staging.drop_new" | "staging.add" | "staging.remove" => {
                let done = edit_sequences(&mut vessel.propulsion.sequences, action);
                if !done {
                    warn!("cannot do `{}` with the sequences", action.0);
                }
            }
            _ => {}
        }
    }
}

/// Rearrange the activation sequences from the staging panel. What is dragged is
/// `item:<sequence>:<index>` (an activation) or `seq:<sequence>` (a whole sequence):
///
/// - `staging.drop:<dragged>:<sequence>`: an activation moves into that sequence; a
///   sequence moves to its place.
/// - `staging.drop_new:<dragged>`: an activation moves into a new last sequence; a sequence
///   moves to the end.
/// - `staging.add`: a new empty last sequence. `staging.remove:<sequence>`: remove an empty
///   one.
///
/// Returns false if the action does not fit the sequences as they are.
pub(crate) fn edit_sequences(sequences: &mut Sequences, action: &UiAction) -> bool {
    let args: Vec<&str> = action.arg().map_or(Vec::new(), |a| a.split(':').collect());
    let num = |i: usize| args.get(i).and_then(|a| a.parse::<usize>().ok());
    let last = sequences.list().len();
    match (action.name(), args.first().copied()) {
        ("staging.add", _) => {
            sequences.insert_empty(last);
            true
        }
        ("staging.remove", _) => num(0).is_some_and(|s| sequences.remove_empty(s)),
        ("staging.drop", Some("item")) => match (num(1), num(2), num(3)) {
            // Back onto its own sequence: nothing to do.
            (Some(s), Some(_), Some(to)) if s == to => true,
            (Some(s), Some(i), Some(to)) => sequences.move_activation(s, i, Target::Into(to)),
            _ => false,
        },
        ("staging.drop", Some("seq")) => match (num(1), num(2)) {
            (Some(s), Some(to)) => s == to || sequences.move_sequence(s, to),
            _ => false,
        },
        ("staging.drop_new", Some("item")) => match (num(1), num(2)) {
            (Some(s), Some(i)) => sequences.move_activation(s, i, Target::New(last)),
            _ => false,
        },
        ("staging.drop_new", Some("seq")) => {
            num(1).is_some_and(|s| s + 1 == last || sequences.move_sequence(s, last - 1))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use osp_sim::staging::Activation::*;

    use super::*;

    fn act(s: &str) -> UiAction {
        UiAction(s.into())
    }

    #[test]
    fn the_staging_panel_rearranges_sequences() {
        let mut s = Sequences::new(vec![
            vec![Engines(0)],
            vec![Decoupler(0), Engines(1)],
            vec![Fairing(2)],
        ]);
        assert!(edit_sequences(&mut s, &act("staging.drop:item:1:1:2")));
        assert_eq!(s.list()[2], [Fairing(2), Engines(1)]);
        assert!(edit_sequences(&mut s, &act("staging.drop:seq:2:0")));
        assert_eq!(s.list()[0], [Fairing(2), Engines(1)]);
        assert!(edit_sequences(&mut s, &act("staging.drop_new:item:0:1")));
        assert_eq!(s.list().last().unwrap(), &[Engines(1)]);
        assert!(edit_sequences(&mut s, &act("staging.add")));
        assert!(s.list().last().unwrap().is_empty());
        let n = s.list().len();
        assert!(edit_sequences(
            &mut s,
            &act(&format!("staging.remove:{}", n - 1))
        ));
        // Dropping a sequence on itself is fine; nonsense is not.
        assert!(edit_sequences(&mut s, &act("staging.drop:seq:1:1")));
        assert!(!edit_sequences(&mut s, &act("staging.drop:item:9:0:1")));
        assert!(!edit_sequences(&mut s, &act("staging.remove:0")));
    }
}
