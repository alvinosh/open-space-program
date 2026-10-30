//! Turns named actions (from keybindings and UI buttons) into vessel and game commands.
//!
//! Keys are bound in data (`keybindings` collection), so mods can rebind them or add
//! buttons that fire the same actions. Actions handled here:
//!
//! | Action | Kind |
//! | --- | --- |
//! | `vessel.stage` | press: fire the next activation sequence |
//! | `part.do:<vessel>:<kind>:<part>` | do one activation by hand (see [`part_action`]) |
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
//! | `staging.drop`, `staging.drop_at`, `staging.drop_new`, `staging.add`, `staging.remove` | rearrange the sequences (see [`edit_sequences`]) |
//!
//! Throttle and SAS commands disengage the burn executor (see `flight_plan` for its actions).

use bevy::{math::DVec3, prelude::*};
use osp_sim::{
    attitude::{NavFrame, SasMode},
    parts::PartId,
    propulsion::Propulsion,
    staging::{Activation, Target},
    vessel::VesselId,
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
            "part.do" => {
                match part_action(action.arg().unwrap_or_default()) {
                    Some((vessel, activation)) => {
                        if !sim.perform(vessel, activation) {
                            warn!("the part does not offer `{}`", action.0);
                        }
                    }
                    None => warn!("cannot read `{}`", action.0),
                }
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
            "staging.drop" | "staging.drop_at" | "staging.drop_new" | "staging.add"
            | "staging.remove" => {
                let done = edit_sequences(&mut vessel.propulsion, action);
                if !done {
                    warn!("cannot do `{}` with the sequences", action.0);
                }
            }
            _ => {}
        }
    }
}

/// The vessel and activation `<vessel>:<kind>:<part>` names (`kind` as
/// [`Activation::kind`]): what a button in a part's window does by hand.
pub(crate) fn part_action(arg: &str) -> Option<(VesselId, Activation)> {
    let mut args = arg.split(':');
    let vessel = VesselId(args.next()?.parse().ok()?);
    let kind = args.next()?;
    let part = PartId(args.next()?.parse().ok()?);
    args.next().is_none().then_some(())?;
    Some((vessel, Activation::of_kind(kind, part)?))
}

/// Rearrange the activation sequences from the staging panel, where identical parts in a
/// sequence share an icon ([`Propulsion::groups`]). What is dragged is
/// `item:<sequence>:<group>` (an icon and all its parts), `one:<sequence>:<group>` (its
/// count, for one of its parts) or `seq:<sequence>` (a whole sequence):
///
/// - `staging.drop:<dragged>:<sequence>`: the parts move into that sequence; a sequence
///   moves to its place.
/// - `staging.drop_at:<dragged>:<place>`: the parts move into a new sequence before the one
///   at `<place>` (the strip between two rows); a sequence moves there.
/// - `staging.drop_new:<dragged>`: the parts move into a new last sequence; a sequence
///   moves to the end.
/// - `staging.add`: a new empty last sequence. `staging.remove:<sequence>`: remove an empty
///   one.
///
/// Returns false if the action does not fit the sequences as they are.
pub(crate) fn edit_sequences(propulsion: &mut Propulsion, action: &UiAction) -> bool {
    let args: Vec<&str> = action.arg().map_or(Vec::new(), |a| a.split(':').collect());
    let num = |i: usize| args.get(i).and_then(|a| a.parse::<usize>().ok());
    // The places in sequence `s` of group `g`'s parts, or of the last of them.
    let members = |propulsion: &Propulsion, s: usize, g: usize, one: bool| {
        let seq = propulsion.sequences.list().get(s)?;
        let mut members = propulsion.groups(seq).get(g)?.members.clone();
        if one {
            members.drain(..members.len() - 1);
        }
        Some(members)
    };
    let last = propulsion.sequences.list().len();
    match (action.name(), args.first().copied()) {
        ("staging.add", _) => {
            propulsion.sequences.insert_empty(last);
            true
        }
        ("staging.remove", _) => num(0).is_some_and(|s| propulsion.sequences.remove_empty(s)),
        ("staging.drop", Some(what @ ("item" | "one"))) => match (num(1), num(2), num(3)) {
            // Back onto its own sequence: nothing to do.
            (Some(s), Some(_), Some(to)) if s == to => true,
            (Some(s), Some(g), Some(to)) => {
                members(propulsion, s, g, what == "one").is_some_and(|m| {
                    propulsion
                        .sequences
                        .move_activations(s, &m, Target::Into(to))
                })
            }
            _ => false,
        },
        ("staging.drop", Some("seq")) => match (num(1), num(2)) {
            (Some(s), Some(to)) => s == to || propulsion.sequences.move_sequence(s, to),
            _ => false,
        },
        ("staging.drop_at", Some(what @ ("item" | "one"))) => match (num(1), num(2), num(3)) {
            (Some(s), Some(g), Some(at)) if at <= last => members(propulsion, s, g, what == "one")
                .is_some_and(|m| {
                    propulsion
                        .sequences
                        .move_activations(s, &m, Target::New(at))
                }),
            _ => false,
        },
        ("staging.drop_at", Some("seq")) => match (num(1), num(2)) {
            // The others shift when it leaves, so a place after it is one less.
            (Some(s), Some(at)) if at <= last => {
                let to = if s < at { at - 1 } else { at };
                s == to || propulsion.sequences.move_sequence(s, to)
            }
            _ => false,
        },
        ("staging.drop_new", Some(what @ ("item" | "one"))) => match (num(1), num(2)) {
            (Some(s), Some(g)) => members(propulsion, s, g, what == "one").is_some_and(|m| {
                propulsion
                    .sequences
                    .move_activations(s, &m, Target::New(last))
            }),
            _ => false,
        },
        ("staging.drop_new", Some("seq")) => {
            num(1).is_some_and(|s| s + 1 == last || propulsion.sequences.move_sequence(s, last - 1))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use osp_sim::data::CraftDef;

    use super::*;

    fn act(s: &str) -> UiAction {
        UiAction(s.into())
    }

    #[test]
    fn a_part_button_names_the_vessel_the_kind_and_the_part() {
        assert_eq!(
            part_action("3:decoupler:12"),
            Some((VesselId(3), Activation::Decoupler(PartId(12))))
        );
        assert_eq!(
            part_action("0:engines:5"),
            Some((VesselId(0), Activation::Engine(PartId(5))))
        );
        assert_eq!(part_action("3:wings:12"), None);
        assert_eq!(part_action("3:engines"), None);
        assert_eq!(part_action("3:engines:1:2"), None);
        assert_eq!(part_action("x:engines:1"), None);
    }

    /// The Pathfinder as it stands on the pad.
    fn pathfinder() -> Propulsion {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/core");
        let registry = osp_mods::load(&root, None).registry;
        let catalog = crate::content::catalog(&registry, &mut Vec::new());
        let def: CraftDef = registry.get("craft", "pathfinder").unwrap();
        Propulsion::from_craft(&def, &catalog).unwrap()
    }

    /// How many parts each icon of each sequence stands for, by kind.
    fn icons(p: &Propulsion) -> Vec<Vec<(&'static str, usize)>> {
        (p.sequences.list().iter())
            .map(|seq| {
                (p.groups(seq).iter())
                    .map(|g| (g.kind, g.members.len()))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_strips_between_rows_place_sequences_and_icons_exactly() {
        let mut p = pathfinder();
        let count = p.sequences.list().len();
        let names = |p: &Propulsion| p.sequences.list().to_vec();
        let start = names(&p);
        assert!(count >= 3, "{count}");
        // The first sequence to the strip above the last row: it lands second to last.
        assert!(edit_sequences(
            &mut p,
            &act(&format!("staging.drop_at:seq:0:{}", count - 1))
        ));
        let moved = names(&p);
        assert_eq!(moved[count - 2], start[0]);
        assert_eq!(moved[0], start[1]);
        // The strip right above or below its own row changes nothing.
        let before = names(&p);
        assert!(edit_sequences(&mut p, &act("staging.drop_at:seq:1:1")));
        assert!(edit_sequences(&mut p, &act("staging.drop_at:seq:1:2")));
        assert_eq!(names(&p), before);
        // The strip past the last row puts it last.
        assert!(edit_sequences(
            &mut p,
            &act(&format!("staging.drop_at:seq:0:{count}"))
        ));
        assert_eq!(names(&p).last().unwrap(), &before[0]);
        // An icon dropped on a strip starts a sequence there.
        let mut q = pathfinder();
        let n = icons(&q)[0][0].1;
        assert!(edit_sequences(&mut q, &act("staging.drop_at:one:0:0:0")));
        assert_eq!(q.sequences.list().len(), count + 1);
        assert_eq!(icons(&q)[0], [("engines", 1)]);
        assert_eq!(icons(&q)[1], [("engines", n - 1)]);
        // Nowhere past the end.
        assert!(!edit_sequences(&mut p, &act("staging.drop_at:seq:0:99")));
    }

    #[test]
    fn the_staging_panel_rearranges_sequences() {
        let mut p = pathfinder();
        let start = icons(&p);
        let n = start[0][0].1;
        assert!(start[0] == [("engines", n)] && n > 1, "{start:?}");
        // The count pulls one engine of the cluster out; the icon takes the rest.
        assert!(edit_sequences(&mut p, &act("staging.drop:one:0:0:1")));
        assert_eq!(icons(&p)[0], [("engines", n - 1)]);
        assert!(icons(&p)[1].contains(&("engines", 1)));
        assert!(edit_sequences(&mut p, &act("staging.drop:item:0:0:2")));
        assert!(p.sequences.list()[0].is_empty());
        assert!(icons(&p)[2].contains(&("engines", n - 1)));
        // Whole sequences reorder; an icon can start a new last one.
        assert!(edit_sequences(&mut p, &act("staging.drop:seq:2:0")));
        assert!(icons(&p)[0].contains(&("engines", n - 1)));
        let last = p.sequences.list().len();
        assert!(edit_sequences(&mut p, &act("staging.drop_new:one:0:0")));
        assert_eq!(p.sequences.list().len(), last + 1);
        assert!(edit_sequences(&mut p, &act("staging.add")));
        assert!(p.sequences.list().last().unwrap().is_empty());
        let n = p.sequences.list().len();
        assert!(edit_sequences(
            &mut p,
            &act(&format!("staging.remove:{}", n - 1))
        ));
        // Dropping a sequence on itself is fine; nonsense is not.
        assert!(edit_sequences(&mut p, &act("staging.drop:seq:1:1")));
        assert!(!edit_sequences(&mut p, &act("staging.drop:item:9:0:1")));
        assert!(!edit_sequences(&mut p, &act("staging.drop:item:0:7:1")));
        assert!(!edit_sequences(&mut p, &act("staging.remove:0")));
    }
}
