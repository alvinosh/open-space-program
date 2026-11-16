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
//!
//! In the editor the part under the pointer is a steady, faint blue, and the part picked to be
//! shaped wears the accent, pulsing; while one is picked the next engines are not marked, as they would
//! wear the same colour.

use std::collections::HashMap;

use bevy::{light::NotShadowCaster, prelude::*};
use osp_sim::{parts::PartId, staging::Activation, vessel::VesselId};
use osp_ui::{LayoutVisibility, UiAction, UiHover};

use crate::{
    FrameSet,
    part_info::{ShowsPart, parse_group},
    sim::Sim,
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
/// The colour of the part picked in the editor: the HUD's accent.
const PICKED_COLOR: &str = "#38e0c4";
/// Their places in the palette, after the window colours.
const HOVER: usize = MARK_COLORS.len();
const NEXT: usize = MARK_COLORS.len() + 1;
const PICKED: usize = MARK_COLORS.len() + 2;
/// The editor's part under the pointer: blue too, but steady and faint, as it changes whenever the
/// mouse moves.
const POINTED: usize = MARK_COLORS.len() + 3;
const POINTED_ALPHA: f32 = 0.22;
/// A part in hand in the editor, shown where it would go: the accent, steady, or red where it may
/// not go.
const GHOST: usize = MARK_COLORS.len() + 4;
const REFUSED: usize = MARK_COLORS.len() + 5;
const REFUSED_COLOR: &str = "#ff4d4d";
const GHOST_ALPHA: f32 = 0.45;

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
    /// In the editor: the vessel on show, the part under the pointer and the part picked.
    editor: Option<(VesselId, Option<PartId>, Option<PartId>)>,
    /// In the editor: the parts of a part in hand, shown where they would go, and whether they
    /// may not go there.
    ghost: Option<(VesselId, Vec<PartId>, bool)>,
}

impl PartMarks {
    /// The window `key` is about to open for a tile's parts, not just its own.
    pub fn open_for_tile(&mut self, key: &str, vessel: VesselId, parts: Vec<PartId>) {
        self.tiles.insert(key.to_owned(), (vessel, parts));
    }

    /// What the editor points at and has picked, on the vessel on show; none outside it.
    pub fn set_editor(&mut self, editor: Option<(VesselId, Option<PartId>, Option<PartId>)>) {
        if self.editor != editor {
            self.editor = editor;
        }
    }

    /// The parts of what is in hand in the editor, on the vessel on show, and whether they may not
    /// go where they are; none when nothing is.
    pub fn set_ghost(&mut self, ghost: Option<(VesselId, Vec<PartId>, bool)>) {
        if self.ghost != ghost {
            self.ghost = ghost;
        }
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

    /// The tint of `part` in the palette: hover, then its window's colour, then the editor's pick,
    /// then next.
    fn look(&self, vessel: VesselId, part: PartId) -> Option<usize> {
        let marks = |parts: &Option<Parts>| {
            parts
                .as_ref()
                .is_some_and(|(v, parts)| *v == vessel && parts.contains(&part))
        };
        let (pointed, picked) = match self.editor {
            Some((v, pointed, picked)) if v == vessel => (pointed, picked),
            _ => (None, None),
        };
        if let Some((v, parts, refused)) = &self.ghost
            && *v == vessel
            && parts.contains(&part)
        {
            return Some(if *refused { REFUSED } else { GHOST });
        }
        if marks(&self.hover) {
            return Some(HOVER);
        }
        if pointed == Some(part) {
            return Some(POINTED);
        }
        if let Some(color) = self.color_index(vessel, part) {
            return Some(color);
        }
        if picked.is_some() {
            return (picked == Some(part)).then_some(PICKED);
        }
        marks(&self.next).then_some(NEXT)
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
    // Every mesh drawn for a part, whatever the renderer draws it with.
    meshes: Query<&Mesh3d, Without<Tint>>,
    tints: Query<(Entity, &Tint, &ChildOf)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut palette: Local<Vec<Handle<StandardMaterial>>>,
) {
    let editing = marks
        .editor
        .is_some_and(|(_, a, b)| a.is_some() || b.is_some())
        || marks.ghost.is_some();
    let nothing =
        marks.marks.is_empty() && marks.hover.is_none() && marks.next.is_none() && !editing;
    if nothing && tints.is_empty() {
        return;
    }
    if palette.is_empty() {
        let colors = (MARK_COLORS.iter().copied()).chain([
            HOVER_COLOR,
            NEXT_COLOR,
            PICKED_COLOR,
            HOVER_COLOR,
            PICKED_COLOR,
            REFUSED_COLOR,
        ]);
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
            material.base_color.set_alpha(match i {
                NEXT => NEXT_ALPHA,
                POINTED => POINTED_ALPHA,
                GHOST | REFUSED => GHOST_ALPHA,
                _ => alpha,
            });
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
