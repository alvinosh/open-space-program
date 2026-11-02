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
//! | `staging.drop`, `staging.drop_at`, `staging.drop_before`, `staging.drop_new`, `staging.add`, `staging.add_at`, `staging.remove`, `staging.merge` | rearrange the sequences (see [`edit_sequences`]) |
//! | `staging.split:<vessel>:<part>` | give that part a sequence of its own |
//! | `staging.undo` | put the sequences back as they were before the last of those |
//! | `staging.menu:<sequence>` | open the menu of a row in the staging window |
//!
//! Throttle and SAS commands disengage the burn executor (see `flight_plan` for its actions).

use bevy::{math::DVec3, prelude::*};
use osp_sim::{
    attitude::{NavFrame, SasMode},
    parts::PartId,
    propulsion::Propulsion,
    staging::{Activation, Sequences, Target},
    vessel::{Vessel, VesselId},
};
use osp_ui::{ActionState, LayoutVisibility, UiAction};

use crate::{
    FrameSet,
    camera::CameraRig,
    menu::Menu,
    sim::{Sim, TimeWarp},
};

/// Throttle change per second while a throttle key is held.
const THROTTLE_RATE: f64 = 0.5;
/// The staging window's row menu (layout).
const SEQUENCE_MENU: &str = "staging_menu";

pub struct ControlsPlugin;

impl Plugin for ControlsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputLocked>()
            .init_resource::<SequenceEdits>()
            .add_systems(Update, apply_actions.in_set(FrameSet::Input));
    }
}

/// When set, player input (keyboard and camera) is ignored, e.g. during scripted tests.
#[derive(Resource, Default)]
pub struct InputLocked(pub bool);

/// What the staging window keeps between edits.
#[derive(Resource, Default)]
pub struct SequenceEdits {
    /// The sequences of a vessel as they were before the last edit.
    undo: Option<(VesselId, Sequences)>,
    /// The row whose menu is open.
    pub menu: Option<usize>,
}

impl SequenceEdits {
    /// The sequences to go back to, if the last edit was to `vessel` and nothing has fired or
    /// fallen away since.
    fn undo_of(&self, vessel: &Vessel) -> Option<&Sequences> {
        let (id, before) = self.undo.as_ref()?;
        (*id == vessel.id() && before.same_activations(&vessel.propulsion.sequences))
            .then_some(before)
    }

    pub fn can_undo(&self, vessel: &Vessel) -> bool {
        self.undo_of(vessel).is_some()
    }
}

fn apply_actions(
    mut actions: MessageReader<UiAction>,
    held: Res<ActionState>,
    time: Res<Time>,
    mut sim: ResMut<Sim>,
    mut warp: ResMut<TimeWarp>,
    mut rig: ResMut<CameraRig>,
    locked: Res<InputLocked>,
    menu: Res<Menu>,
    mut edits: ResMut<SequenceEdits>,
    mut layouts: ResMut<LayoutVisibility>,
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
            "staging.menu" => {
                edits.menu = action.arg().and_then(|a| a.parse().ok());
                layouts.layouts.insert(SEQUENCE_MENU.into(), true);
                continue;
            }
            "staging.split" => {
                layouts.layouts.insert(SEQUENCE_MENU.into(), false);
                let (vessel, part) = split_action(action.arg().unwrap_or_default());
                let vessels = sim.universe.vessels.iter_mut();
                let Some(vessel) = vessels.into_iter().find(|v| Some(v.id()) == vessel) else {
                    continue;
                };
                let before = vessel.propulsion.sequences.clone();
                if part.is_some_and(|p| vessel.propulsion.sequences.split_off(p)) {
                    edits.undo = Some((vessel.id(), before));
                } else {
                    warn!("cannot do `{}` with the sequences", action.0);
                }
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
                    vessel.press_sas_mode(system, ut, mode);
                    if vessel.sas().mode() != SasMode::Burn || !vessel.sas().enabled() {
                        vessel.disengage_burn(ut);
                    }
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
            "staging.drop"
            | "staging.drop_at"
            | "staging.drop_before"
            | "staging.drop_new"
            | "staging.add"
            | "staging.add_at"
            | "staging.remove"
            | "staging.merge" => {
                layouts.layouts.insert(SEQUENCE_MENU.into(), false);
                let before = vessel.propulsion.sequences.clone();
                if !edit_sequences(&mut vessel.propulsion, action) {
                    warn!("cannot do `{}` with the sequences", action.0);
                } else if vessel.propulsion.sequences != before {
                    edits.undo = Some((vessel.id(), before));
                }
            }
            "staging.undo" => {
                if let Some(before) = edits.undo_of(vessel).cloned() {
                    vessel.propulsion.sequences = before;
                    edits.undo = None;
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
    Some((vessel, Activation::of_kind(kind, part, None)?))
}

/// The vessel and part `<vessel>:<part>` names.
fn split_action(arg: &str) -> (Option<VesselId>, Option<PartId>) {
    let mut args = arg.split(':');
    let vessel = args.next().and_then(|a| Some(VesselId(a.parse().ok()?)));
    let part = args.next().and_then(|a| Some(PartId(a.parse().ok()?)));
    (vessel, part)
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
/// - `staging.drop_before:<dragged>:<sequence>:<group>`: the parts move into that sequence just
///   before that icon, so they fire before it; a sequence goes into it there.
/// - `staging.drop_new:<dragged>`: the parts move into a new last sequence; a sequence
///   moves to the end.
/// - `staging.add`: a new empty last sequence. `staging.add_at:<place>`: one before the one at
///   `<place>`. `staging.remove:<sequence>`: remove an empty one.
/// - `staging.merge:<sequence>`: the sequence goes into the one before it.
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
        ("staging.add_at", _) => num(0).is_some_and(|at| {
            at <= last && {
                propulsion.sequences.insert_empty(at);
                true
            }
        }),
        ("staging.remove", _) => num(0).is_some_and(|s| propulsion.sequences.remove_empty(s)),
        ("staging.merge", _) => num(0).is_some_and(|s| propulsion.sequences.merge_down(s)),
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
        ("staging.drop_before", Some(what @ ("item" | "one"))) => {
            match (num(1), num(2), num(3), num(4)) {
                (Some(s), Some(g), Some(to), Some(before)) => {
                    let place = (propulsion.sequences.list().get(to)).and_then(|seq| {
                        Some(*propulsion.groups(seq).get(before)?.members.first()?)
                    });
                    let moved = members(propulsion, s, g, what == "one");
                    match (moved, place) {
                        // Before itself: nothing to do.
                        (Some(m), Some(at)) if s == to && m.contains(&at) => true,
                        (Some(m), Some(at)) => {
                            propulsion
                                .sequences
                                .move_activations(s, &m, Target::Before(to, at))
                        }
                        _ => false,
                    }
                }
                _ => false,
            }
        }
        // A whole row dropped on a tile goes by what the tile's row does with it.
        ("staging.drop_before", Some("seq")) => match (num(1), num(2)) {
            (Some(s), Some(to)) => s == to || propulsion.sequences.move_sequence(s, to),
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
            Some((VesselId(3), Activation::decoupler(PartId(12))))
        );
        assert_eq!(
            part_action("0:engines:5"),
            Some((VesselId(0), Activation::engine(PartId(5))))
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
    fn a_row_menu_adds_either_side_of_a_row_and_merges_it_with_the_one_below() {
        let mut p = pathfinder();
        let count = p.sequences.list().len();
        // Below the first row, and above it.
        assert!(edit_sequences(&mut p, &act("staging.add_at:0")));
        assert!(p.sequences.list()[0].is_empty());
        assert!(edit_sequences(&mut p, &act("staging.add_at:2")));
        assert!(p.sequences.list()[2].is_empty());
        assert_eq!(p.sequences.list().len(), count + 2);
        assert!(!edit_sequences(&mut p, &act("staging.add_at:99")));
        // Both empty ones merge away into what is below them; the first row has nothing below.
        assert!(edit_sequences(&mut p, &act("staging.merge:2")));
        assert!(edit_sequences(&mut p, &act("staging.merge:1")));
        assert!(p.sequences.list()[0].len() > 1 || count == 1);
        assert_eq!(p.sequences.list().len(), count);
        assert!(!edit_sequences(&mut p, &act("staging.merge:0")));
    }

    #[test]
    fn an_icon_dropped_on_another_goes_just_before_it() {
        let mut p = pathfinder();
        // A second row's icon in front of the first row's.
        let (second, first) = (icons(&p)[1].clone(), icons(&p)[0].clone());
        assert!(edit_sequences(
            &mut p,
            &act("staging.drop_before:item:1:0:0:0")
        ));
        assert_eq!(icons(&p)[0][0], second[0]);
        assert_eq!(icons(&p)[0][1], first[0]);
        // And behind again, inside the row, by putting the other one in front of it.
        assert!(edit_sequences(
            &mut p,
            &act("staging.drop_before:item:0:1:0:0")
        ));
        assert_eq!(icons(&p)[0][0], first[0]);
        assert_eq!(icons(&p)[0][1], second[0]);
        // Onto itself changes nothing, and a row dropped on a tile goes to that row's place.
        let before = icons(&p);
        assert!(edit_sequences(
            &mut p,
            &act("staging.drop_before:item:0:1:0:1")
        ));
        assert_eq!(icons(&p), before);
        assert!(edit_sequences(
            &mut p,
            &act("staging.drop_before:seq:1:0:0")
        ));
        assert_eq!(icons(&p)[0], before[1]);
    }

    #[test]
    fn a_split_names_the_vessel_and_the_part() {
        assert_eq!(split_action("3:12"), (Some(VesselId(3)), Some(PartId(12))));
        assert_eq!(split_action("x:12"), (None, Some(PartId(12))));
        assert_eq!(split_action("3"), (Some(VesselId(3)), None));
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
