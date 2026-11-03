//! Which parts have a window open, and the colour that ties them together: each open part
//! window gets a colour of its own, the window's frame and the part's tile in the staging
//! window wear it, and the part is tinted with it on the vessel. Opening a window from a
//! staging tile marks every part of the tile (the three identical boosters it stands for).
//! A window that closes gives its colour back.
//!
//! Two marks belong to no window: a strong blue for the parts of the staging tile (or row
//! number) the pointer is over (white would vanish on a white rocket), and a steady teal on
//! the engines the next sequence lights, while the staging window is open. A window's colour
//! wins over teal, blue over both.

use std::collections::HashMap;

use bevy::{light::NotShadowCaster, prelude::*};
use osp_sim::{parts::PartId, staging::Activation, vessel::VesselId};
use osp_ui::{LayoutVisibility, UiAction, UiHover};

use crate::{
    FrameSet,
    part_info::{ShowsPart, parse_group},
    sim::Sim,
    vessel_material::VesselMaterial,
};

/// The layout group the part windows are in: hidden with the rest of the flight UI, the
/// marks go too.
const GROUP: &str = "flight";

/// Colours that tell open windows apart, as hex for the UI and the tint on the vessel.
/// None is a stage colour (those are the fuel bars) or the amber of a burning engine.
pub const MARK_COLORS: [&str; 6] = [
    "#ff4fa3", "#4dd2ff", "#ffd23f", "#7dff6b", "#b07cff", "#ff7a3d",
];

/// The colour of the parts under the pointer, and of the engines that light next (the HUD's
/// teal, as the next row of the staging window). Neither is a window colour.
const HOVER_COLOR: &str = "#2f6bff";
const NEXT_COLOR: &str = "#38e0c4";
/// Their places in the palette, after the window colours.
const HOVER: usize = MARK_COLORS.len();
const NEXT: usize = MARK_COLORS.len() + 1;

/// How opaque the tint is, at the dimmest and brightest of its slow pulse.
const TINT_ALPHA: (f32, f32) = (0.38, 0.62);
/// How opaque the next engines are: steady, so they do not compete with a pulsing mark.
const NEXT_ALPHA: f32 = 0.3;
/// Pulses per second.
const PULSE_RATE: f32 = 0.7;

pub struct PartHighlightPlugin;

impl Plugin for PartHighlightPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PartMarks>().add_systems(
            Update,
            (sync_marks, sync_hover, sync_next, tint_parts)
                .chain()
                .in_set(FrameSet::Render),
        );
    }
}

/// A part window's mark.
#[derive(Debug, Clone)]
struct Mark {
    /// The window's key, `<vessel>-<part>`.
    key: String,
    color: usize,
    vessel: VesselId,
    /// The parts it marks: its own, or those of the staging tile it was opened from.
    parts: Vec<PartId>,
}

/// Parts of one vessel.
type Parts = (VesselId, Vec<PartId>);

/// The marks of the open part windows, and of the parts under the pointer and the engines
/// that light next.
#[derive(Resource, Default, Debug)]
pub struct PartMarks {
    marks: Vec<Mark>,
    /// The parts a window is to mark, set when it is opened from a staging tile.
    tiles: HashMap<String, Parts>,
    /// The parts of the tile or row under the pointer.
    hover: Option<Parts>,
    /// What a capture script says is under the pointer (`part.hover`), when nothing is.
    scripted: Option<Parts>,
    /// The engines the next sequence lights.
    next: Option<Parts>,
}

impl PartMarks {
    /// The window `key` is about to open for a tile's parts, not just its own.
    pub fn open_for_tile(&mut self, key: &str, vessel: VesselId, parts: Vec<PartId>) {
        self.tiles.insert(key.to_owned(), (vessel, parts));
    }

    /// The colour (hex) of window `key`, if it has one.
    pub fn color_of_window(&self, key: &str) -> Option<&'static str> {
        let mark = self.marks.iter().find(|m| m.key == key)?;
        Some(MARK_COLORS[mark.color])
    }

    /// The colour (hex) of the window that marks any of `parts` of `vessel`, if one does.
    pub fn color_of_parts(&self, vessel: VesselId, parts: &[PartId]) -> Option<&'static str> {
        let mark = self
            .marks
            .iter()
            .find(|m| m.vessel == vessel && m.parts.iter().any(|p| parts.contains(p)))?;
        Some(MARK_COLORS[mark.color])
    }

    fn color_index(&self, vessel: VesselId, part: PartId) -> Option<usize> {
        let mark = (self.marks.iter()).find(|m| m.vessel == vessel && m.parts.contains(&part))?;
        Some(mark.color)
    }

    /// The tint of `part` in the palette: hover, then its window's colour, then next.
    fn look(&self, vessel: VesselId, part: PartId) -> Option<usize> {
        let marks = |parts: &Option<Parts>| {
            parts
                .as_ref()
                .is_some_and(|(v, parts)| *v == vessel && parts.contains(&part))
        };
        if marks(&self.hover) {
            return Some(HOVER);
        }
        self.color_index(vessel, part)
            .or_else(|| marks(&self.next).then_some(NEXT))
    }

    /// Marks for the windows `open`, in the colours they had; a new window takes the
    /// first colour no open window wears (and the first again, past the last).
    fn sync(&mut self, open: &[(String, VesselId, PartId)]) {
        self.marks
            .retain(|m| open.iter().any(|(key, ..)| *key == m.key));
        self.tiles
            .retain(|key, _| open.iter().any(|(k, ..)| k == key));
        for (key, vessel, part) in open {
            if self.marks.iter().any(|m| m.key == *key) {
                continue;
            }
            let color = (0..MARK_COLORS.len())
                .find(|c| self.marks.iter().all(|m| m.color != *c))
                .unwrap_or(self.marks.len() % MARK_COLORS.len());
            let (vessel, parts) = match self.tiles.get(key) {
                Some((vessel, parts)) => (*vessel, parts.clone()),
                None => (*vessel, vec![*part]),
            };
            self.marks.push(Mark {
                key: key.clone(),
                color,
                vessel,
                parts,
            });
        }
    }
}

/// Keep a mark for every part window that is open and showing.
pub(crate) fn sync_marks(shown: Res<LayoutVisibility>, mut marks: ResMut<PartMarks>) {
    let open: Vec<_> = (shown.instances(crate::part_info::LAYOUT))
        .filter(|key| {
            shown.visible(
                &osp_ui::instance_id(crate::part_info::LAYOUT, key),
                Some(GROUP),
            )
        })
        .filter_map(|key| {
            let (vessel, part) = crate::part_info::parse_key(key)?;
            Some((key.to_owned(), vessel, part))
        })
        .collect();
    marks.sync(&open);
}

/// What the pointer is over: the parts of a staging tile or row (`hover: "part.group:..."`
/// in the layout), none while the flight UI is hidden.
fn sync_hover(
    hover: Res<UiHover>,
    shown: Res<LayoutVisibility>,
    mut actions: MessageReader<UiAction>,
    mut marks: ResMut<PartMarks>,
) {
    for action in actions.read().filter(|a| a.name() == "part.hover") {
        marks.scripted = action.arg().and_then(parse_group);
    }
    let over = hover.0.as_deref().and_then(group_of);
    let hover = if shown.hidden_groups.contains(GROUP) && !shown.visible("staging", Some(GROUP)) {
        None
    } else {
        over.or_else(|| marks.scripted.clone())
    };
    if marks.hover != hover {
        marks.hover = hover;
    }
}

/// The parts a `part.group:<vessel>:<part>,...` text names.
pub(crate) fn group_of(text: &str) -> Option<Parts> {
    parse_group(text.strip_prefix("part.group:")?)
}

/// The engines the next sequence lights, while the staging window shows.
fn sync_next(sim: Res<Sim>, shown: Res<LayoutVisibility>, mut marks: ResMut<PartMarks>) {
    let vessel = sim.active_vessel();
    let next = if shown.visible("staging", Some(GROUP)) {
        let sequences = vessel.propulsion.sequences.list();
        let engines = next_engines(sequences);
        (!engines.is_empty()).then(|| (vessel.id(), engines))
    } else {
        None
    };
    if marks.next != next {
        marks.next = next;
    }
}

/// The engines of the first sequence of `sequences`.
fn next_engines(sequences: &[Vec<Activation>]) -> Vec<PartId> {
    let Some(next) = sequences.first() else {
        return Vec::new();
    };
    (next.iter())
        .filter(|a| a.is_engine())
        .map(|a| a.part())
        .collect()
}

/// A copy of a marked part's mesh, drawn over it in the mark's colour.
#[derive(Component)]
struct Tint {
    color: usize,
}

/// Tint the meshes of the marked parts, and take the tint off those no longer marked.
/// Models arrive a few frames after their vessel is built, so it looks every frame.
#[allow(clippy::type_complexity)]
fn tint_parts(
    mut commands: Commands,
    marks: Res<PartMarks>,
    time: Res<Time>,
    parts: Query<(Entity, &ShowsPart)>,
    children: Query<&Children>,
    meshes: Query<
        &Mesh3d,
        (
            Without<Tint>,
            Or<(
                With<MeshMaterial3d<VesselMaterial>>,
                With<MeshMaterial3d<StandardMaterial>>,
            )>,
        ),
    >,
    tints: Query<(Entity, &Tint, &ChildOf)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut palette: Local<Vec<Handle<StandardMaterial>>>,
) {
    let nothing = marks.marks.is_empty() && marks.hover.is_none() && marks.next.is_none();
    if nothing && tints.is_empty() {
        return;
    }
    if palette.is_empty() {
        let colors = MARK_COLORS.iter().copied().chain([HOVER_COLOR, NEXT_COLOR]);
        palette.extend(colors.map(|hex| {
            let color = Srgba::hex(hex).unwrap_or(Srgba::WHITE);
            materials.add(StandardMaterial {
                base_color: color.into(),
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                fog_enabled: false,
                ..default()
            })
        }));
    }
    // Breathe, so a marked part stands out from a painted one of the same colour.
    let (low, high) = TINT_ALPHA;
    let pulse = 0.5 + 0.5 * (time.elapsed_secs() * PULSE_RATE * std::f32::consts::TAU).sin();
    let alpha = low + (high - low) * pulse;
    for (i, handle) in palette.iter().enumerate() {
        if let Some(mut material) = materials.get_mut(handle) {
            material
                .base_color
                .set_alpha(if i == NEXT { NEXT_ALPHA } else { alpha });
        }
    }

    // The mesh to tint, and in which colour: those of a marked part, wherever in its
    // model they are.
    let mut wanted: HashMap<Entity, usize> = HashMap::new();
    for (entity, shows) in &parts {
        let Some(color) = marks.look(shows.vessel, shows.part) else {
            continue;
        };
        for mesh in std::iter::once(entity).chain(children.iter_descendants(entity)) {
            if meshes.contains(mesh) {
                wanted.insert(mesh, color);
            }
        }
    }
    for (tint, kind, parent) in &tints {
        if wanted.get(&parent.parent()) == Some(&kind.color) {
            wanted.remove(&parent.parent());
        } else {
            // It may have gone already, with the vessel it was on.
            commands.entity(tint).try_despawn();
        }
    }
    for (mesh, color) in wanted {
        let Ok(handle) = meshes.get(mesh) else {
            continue;
        };
        commands.spawn((
            Tint { color },
            handle.clone(),
            MeshMaterial3d(palette[color].clone()),
            Transform::default(),
            NotShadowCaster,
            ChildOf(mesh),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(part: u32) -> (String, VesselId, PartId) {
        (format!("1-{part}"), VesselId(1), PartId(part))
    }

    #[test]
    fn windows_get_different_colours_and_give_them_back() {
        let mut marks = PartMarks::default();
        marks.sync(&[window(3), window(4)]);
        let (a, b) = (marks.color_of_window("1-3"), marks.color_of_window("1-4"));
        assert!(a.is_some() && b.is_some() && a != b);
        // One closes: the other keeps its colour, and a new one takes the free one.
        marks.sync(&[window(4)]);
        assert_eq!(marks.color_of_window("1-4"), b);
        assert_eq!(marks.color_of_window("1-3"), None);
        marks.sync(&[window(4), window(5)]);
        assert_eq!(marks.color_of_window("1-5"), a);
    }

    #[test]
    fn a_window_from_a_tile_marks_all_its_parts() {
        let mut marks = PartMarks::default();
        let boosters = vec![PartId(7), PartId(8), PartId(9)];
        marks.open_for_tile("1-7", VesselId(1), boosters.clone());
        marks.sync(&[window(7)]);
        let color = marks.color_of_window("1-7");
        for p in &boosters {
            assert_eq!(marks.color_index(VesselId(1), *p), Some(0));
        }
        assert_eq!(marks.color_of_parts(VesselId(1), &[PartId(9)]), color);
        assert_eq!(marks.color_of_parts(VesselId(1), &[PartId(2)]), None);
        assert_eq!(marks.color_of_parts(VesselId(2), &[PartId(9)]), None);
        // Closing the window forgets the tile.
        marks.sync(&[]);
        assert!(marks.tiles.is_empty() && marks.marks.is_empty());
    }

    fn hovering_app() -> App {
        let mut app = App::new();
        app.init_resource::<PartMarks>()
            .init_resource::<UiHover>()
            .init_resource::<LayoutVisibility>()
            .add_message::<UiAction>()
            .add_systems(Update, sync_hover);
        app
    }

    fn hover(app: &mut App, text: Option<&str>) -> Option<Parts> {
        app.world_mut().resource_mut::<UiHover>().0 = text.map(str::to_owned);
        app.update();
        app.world().resource::<PartMarks>().hover.clone()
    }

    #[test]
    fn the_parts_of_a_tile_under_the_pointer_are_hovered() {
        let mut app = hovering_app();
        let boosters = (VesselId(1), vec![PartId(7), PartId(8), PartId(9)]);
        assert_eq!(hover(&mut app, Some("part.group:1:7,8,9")), Some(boosters));
        // One split off a count is a tile with just that part.
        assert_eq!(
            hover(&mut app, Some("part.group:1:8")),
            Some((VesselId(1), vec![PartId(8)]))
        );
        // Not a tile, a tile with nothing in it, or nothing at all.
        assert_eq!(hover(&mut app, Some("staging.add")), None);
        assert_eq!(hover(&mut app, Some("part.group:")), None);
        assert_eq!(hover(&mut app, Some("part.group:-")), None);
        assert_eq!(hover(&mut app, None), None);
    }

    #[test]
    fn hover_is_gone_with_the_flight_ui_and_a_script_can_set_it() {
        let mut app = hovering_app();
        app.world_mut()
            .write_message(UiAction("part.hover:1:4,5".into()));
        let scripted = Some((VesselId(1), vec![PartId(4), PartId(5)]));
        assert_eq!(hover(&mut app, None), scripted);
        // The pointer wins over the script.
        assert_eq!(
            hover(&mut app, Some("part.group:1:9")),
            Some((VesselId(1), vec![PartId(9)]))
        );
        app.world_mut()
            .resource_mut::<LayoutVisibility>()
            .hidden_groups
            .insert(GROUP.to_owned());
        assert_eq!(hover(&mut app, Some("part.group:1:9")), None);
        app.world_mut()
            .resource_mut::<LayoutVisibility>()
            .hidden_groups
            .clear();
        app.world_mut()
            .write_message(UiAction("part.hover:".into()));
        assert_eq!(hover(&mut app, None), None);
    }

    #[test]
    fn the_pointer_beats_a_window_which_beats_the_next_engines() {
        let mut marks = PartMarks::default();
        let v = VesselId(1);
        marks.next = Some((v, vec![PartId(3), PartId(4), PartId(5)]));
        marks.sync(&[window(4)]);
        marks.hover = Some((v, vec![PartId(5)]));
        assert_eq!(marks.look(v, PartId(3)), Some(NEXT));
        assert_eq!(marks.look(v, PartId(4)), Some(0));
        assert_eq!(marks.look(v, PartId(5)), Some(HOVER));
        assert_eq!(marks.look(v, PartId(6)), None);
        assert_eq!(marks.look(VesselId(2), PartId(3)), None);
    }

    #[test]
    fn the_next_engines_are_those_of_the_first_sequence_only() {
        let (engine, decoupler) = (Activation::engine, Activation::decoupler);
        let sequences = vec![
            vec![engine(PartId(2)), decoupler(PartId(9)), engine(PartId(3))],
            vec![engine(PartId(4))],
        ];
        assert_eq!(next_engines(&sequences), [PartId(2), PartId(3)]);
        // A sequence that only lets go lights nothing.
        assert!(next_engines(&[vec![decoupler(PartId(9))]]).is_empty());
        assert!(next_engines(&[]).is_empty());
    }

    #[test]
    fn a_part_window_by_itself_marks_its_part() {
        let mut marks = PartMarks::default();
        marks.sync(&[window(3)]);
        assert_eq!(marks.color_index(VesselId(1), PartId(3)), Some(0));
        assert_eq!(marks.color_index(VesselId(1), PartId(4)), None);
    }
}
