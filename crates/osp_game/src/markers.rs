//! Markers on what can be picked out in the view: a box round each body (map view) and each vessel,
//! with the name under it.
//!
//! Which vessels get one is [`gets_box`]. Spent stages and other debris get none: in the map they
//! keep their dim dot (see `orbit_lines`), with a spot here that takes the click and names them under
//! the pointer. A vessel too big on screen for a box to help keeps only its name, so its parts can
//! still be right-clicked.
//!
//! A click on a vessel's box or a right-click on a body's opens the `marker_menu`, whose items fire
//! the actions the Universe Manifest fires. A left click on a body's box centres the map on it, and
//! Shift+click on either targets it (again to drop the target).
//!
//! A marker is left out while its body is big on screen, hidden behind another body, or crowded by a
//! more important marker: the target's, then the Sun's, the planets' and the moons', bigger bodies
//! first, then the vessels', nearest first. So zoomed out a planet's moons and what orbits it tuck
//! into its marker.

use bevy::{
    math::DVec3,
    picking::{events::PointerClick, hover::Hovered, pointer::PointerButton},
    prelude::*,
};
use osp_sim::{body::BodyId, rendezvous::Target, vessel::VesselId};
use osp_ui::{LayoutVisibility, Tooltip, UiAction, UiData, UiDocuments, ViewOverlay, format};

use crate::{
    FrameSet,
    camera::{CameraRig, Follow, MainCamera},
    editing::EditorStatus,
    frame::{RenderFrame, to_render},
    map_labels::{MapLabels, hidden},
    menu::Menu,
    orbit_lines::{TARGET_COLOR, project},
    sim::Sim,
};

/// How near a vessel gets a box in the flight view, m.
const BOX_RANGE: f64 = 100e3;
/// The menu a marker opens (layout).
const MENU: &str = "marker_menu";
/// Half the side of the smallest box, logical pixels.
const MIN_HALF_PX: f32 = 9.0;
/// Gap between what is marked and its box, px.
const PADDING_PX: f32 = 3.0;
/// Bodies bigger than this on screen (radius, px) get no marker.
const MAX_DISC_PX: f32 = 40.0;
/// Vessels bigger than this on screen (radius, px) get no box: their parts are big enough to
/// click, and a box over them would take the click instead.
const MAX_VESSEL_PX: f32 = 6.0;
/// How far under the middle of a vessel with no box its name goes at most, px.
const MAX_NAME_DROP_PX: f32 = 120.0;
/// Width the name is centred in, px.
const COLUMN_PX: f32 = 180.0;
const NAME_GAP_PX: f32 = 2.0;
/// Height of the name line and rough glyph width, px, for keeping markers apart.
const LINE_PX: f32 = 15.0;
const GLYPH_PX: f32 = 7.0;
const COLOR: Color = Color::srgba(0.85, 0.9, 1.0, 0.55);
const HOVER_COLOR: Color = Color::WHITE;
const HOVER_FILL: Color = Color::srgba(1.0, 1.0, 1.0, 0.08);

pub struct MarkersPlugin;

impl Plugin for MarkersPlugin {
    fn build(&self, app: &mut App) {
        // In `Render`, so the map labels keep off the markers.
        app.init_resource::<MarkerMenu>()
            .add_systems(Update, (place.in_set(FrameSet::Render), publish_menu));
    }
}

/// What a marker is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    Body(BodyId),
    Vessel(VesselId),
}

/// What the menu is open on.
#[derive(Resource, Default)]
struct MarkerMenu(Option<Subject>);

/// A marker: a column centred on what it marks, holding its box and name.
#[derive(Component)]
struct Marker {
    subject: Subject,
    outline: Entity,
    name: Entity,
}

/// On a box: the name of what it marks, for scripts to find it by.
#[derive(Component)]
pub struct MarkerBox(pub String);

/// Whether a vessel gets a box: `targeted`, or no debris and in the map or within [`BOX_RANGE`] of
/// the vessel being flown, which never gets one.
pub fn gets_box(map: bool, flown: bool, debris: bool, targeted: bool, distance: f64) -> bool {
    !flown && (targeted || (!debris && (map || distance <= BOX_RANGE)))
}

/// What is under a box's name line: the name alone in the map, and how far in the flight view.
fn line_of(name: &str, map: bool, distance: f64) -> String {
    if map {
        name.to_owned()
    } else {
        format!("{name}, {}", format::distance(distance))
    }
}

fn spawn_marker(commands: &mut Commands, subject: Subject, font: TextFont) {
    let hint = match subject {
        Subject::Body(_) => {
            "Click to centre the map here, right-click for more, Shift+click to target"
        }
        Subject::Vessel(_) => "Click for what you can do with it, Shift+click to target",
    };
    let outline = commands
        .spawn((
            MarkerBox(String::new()),
            Node {
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(2.0)),
                ..default()
            },
            BorderColor::all(COLOR),
            BackgroundColor(Color::NONE),
            Tooltip(hint.into()),
            Hovered::default(),
            ViewOverlay,
        ))
        .id();
    let name = commands
        .spawn((Text::new(""), font, TextColor(COLOR), ViewOverlay))
        .id();
    commands
        .spawn((
            Marker {
                subject,
                outline,
                name,
            },
            Node {
                position_type: PositionType::Absolute,
                width: Val::Px(COLUMN_PX),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(NAME_GAP_PX),
                ..default()
            },
            // Under every window, like the map labels.
            GlobalZIndex(-10),
            // Only the box and the name take the pointer, not the whole column.
            Pickable::IGNORE,
            Hovered::default(),
            Visibility::Hidden,
        ))
        .add_children(&[outline, name])
        .observe(click);
}

/// Target `subject`, or drop it as the target. The vessel being flown and the body it is around
/// cannot be targeted.
fn toggle_target(subject: Subject, sim: &mut Sim) {
    let parent = sim.active_vessel().parent();
    let flown = sim.active_id();
    let vessel = sim.active_vessel_mut();
    match subject {
        Subject::Body(body) if vessel.target == Some(Target::Body(body)) => vessel.target = None,
        Subject::Body(body) if body != parent => vessel.target = Some(Target::Body(body)),
        Subject::Vessel(id) if vessel.target.and_then(Target::vessel) == Some(id) => {
            vessel.target = None;
        }
        Subject::Vessel(id) if id != flown => vessel.target = Some(Target::Vessel(id)),
        _ => {}
    }
}

fn click(
    mut event: On<PointerClick>,
    markers: Query<&Marker>,
    keys: Res<ButtonInput<KeyCode>>,
    mut sim: ResMut<Sim>,
    mut rig: ResMut<CameraRig>,
    mut menu: ResMut<MarkerMenu>,
    mut ui: ResMut<LayoutVisibility>,
) {
    let primary = event.button == PointerButton::Primary;
    if !primary && event.button != PointerButton::Secondary {
        return;
    }
    let Ok(marker) = markers.get(event.entity) else {
        return;
    };
    event.propagate(false);
    let subject = marker.subject;
    if primary && keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        toggle_target(subject, &mut sim);
    } else if let (Subject::Body(body), true) = (subject, primary) {
        rig.set_follow(Some(Follow::Body(body)), &sim);
    } else {
        menu.0 = Some(subject);
        ui.layouts.insert(MENU.into(), true);
    }
}

/// `marker.*` while the menu is open, and closing it once something has been chosen from it or what
/// it was open on has gone.
fn publish_menu(
    mut actions: MessageReader<UiAction>,
    mut menu: ResMut<MarkerMenu>,
    sim: Res<Sim>,
    rig: Res<CameraRig>,
    frame: Res<RenderFrame>,
    mut ui: ResMut<LayoutVisibility>,
    mut data: ResMut<UiData>,
) {
    let chosen = actions.read().any(|action| {
        matches!(
            action.name(),
            "target.vessel"
                | "target.body"
                | "target.clear"
                | "vessel.fly"
                | "manifest.follow"
                | "manifest.follow_body"
        )
    });
    if !ui.shown(MENU) {
        return;
    }
    let subject = menu.0.filter(|subject| match subject {
        Subject::Body(_) => true,
        Subject::Vessel(id) => *id != sim.active_id() && sim.universe.vessel(*id).is_some(),
    });
    let Some(subject) = subject.filter(|_| !chosen) else {
        menu.0 = None;
        ui.layouts.insert(MENU.into(), false);
        return;
    };
    let flown = sim.active_vessel();
    let system = &sim.universe.system;
    let here = frame.relative(flown.parent(), flown.state_at(system, frame.time).position);
    let (name, key, distance, targeted, can_target) = match subject {
        Subject::Body(id) => {
            let body = system.body(id);
            let distance = frame.body_offset(id).distance(here) - body.radius;
            let targeted = flown.target == Some(Target::Body(id));
            let can_target = !targeted && id != flown.parent();
            (
                body.name.clone(),
                body.name.clone(),
                distance,
                targeted,
                can_target,
            )
        }
        Subject::Vessel(id) => {
            let Some(vessel) = sim.universe.vessel(id) else {
                return;
            };
            let there = frame.relative(
                vessel.parent(),
                vessel.state_at(system, frame.time).position,
            );
            let targeted = flown.target.and_then(Target::vessel) == Some(id);
            (
                vessel.name.clone(),
                id.0.to_string(),
                there.distance(here),
                targeted,
                !targeted,
            )
        }
    };
    let is_vessel = matches!(subject, Subject::Vessel(_));
    data.set("marker.name", name);
    data.set("marker.key", key);
    data.set("marker.distance", distance);
    data.set("marker.is_vessel", is_vessel);
    data.set("marker.targeted", targeted);
    data.set("marker.target_vessel", is_vessel && can_target);
    data.set("marker.target_body", !is_vessel && can_target);
    data.set("marker.centre_vessel", is_vessel && rig.map_view);
    data.set("marker.centre_body", !is_vessel && rig.map_view);
}

/// A marker where it stands this frame.
struct Spot {
    subject: Subject,
    /// On screen, physical px.
    centre: Vec2,
    /// Half the side of its box; 0 when it has none.
    half: f32,
    /// How far under `centre` the marker starts, px: above it for a box.
    top: f32,
    /// The whole name, for scripts.
    name: String,
    /// What is written under it.
    line: String,
    /// Only a place to click: nothing shows until the pointer is on it.
    spot_only: bool,
    targeted: bool,
}

/// One thing that may get a marker.
struct Candidate {
    subject: Subject,
    /// From the origin, sim axes, m.
    at: DVec3,
    radius: f64,
    name: String,
    line: String,
    targeted: bool,
    /// Bodies first, in order of importance; then vessels, nearest first.
    rank: (u8, usize, f64),
}

/// Everything that may get a marker now, the most important first.
fn candidates(sim: &Sim, frame: &RenderFrame, map: bool) -> Vec<Candidate> {
    let system = &sim.universe.system;
    let flown = sim.active_vessel();
    let target = flown.target;
    let here = frame.relative(flown.parent(), flown.state_at(system, frame.time).position);
    let mut found = Vec::new();
    if map {
        found.extend(system.bodies().iter().map(|body| Candidate {
            subject: Subject::Body(body.id),
            at: frame.body_offset(body.id),
            radius: body.radius,
            name: body.name.clone(),
            line: body.name.clone(),
            targeted: target == Some(Target::Body(body.id)),
            rank: (0, body.depth, -body.radius),
        }));
    }
    for vessel in &sim.universe.vessels {
        let at = frame.relative(
            vessel.parent(),
            vessel.state_at(system, frame.time).position,
        );
        let distance = at.distance(here);
        let targeted = target.and_then(Target::vessel) == Some(vessel.id());
        let is_flown = vessel.id() == flown.id();
        if gets_box(map, is_flown, vessel.is_debris(), targeted, distance) {
            found.push(Candidate {
                subject: Subject::Vessel(vessel.id()),
                at,
                radius: vessel.propulsion.length() / 2.0,
                name: vessel.name.clone(),
                line: line_of(&vessel.name, map, distance),
                targeted,
                rank: (1, 0, distance),
            });
        }
    }
    found.sort_by(|a, b| {
        (b.targeted.cmp(&a.targeted))
            .then(a.rank.0.cmp(&b.rank.0))
            .then(a.rank.1.cmp(&b.rank.1))
            .then(a.rank.2.total_cmp(&b.rank.2))
    });
    found
}

/// Where the markers go this frame: the candidates that fit, then a spot on each piece of debris
/// in the map.
fn spots(
    sim: &Sim,
    frame: &RenderFrame,
    rig: &CameraRig,
    camera: (&Camera, &Projection, &Transform),
    labels: &mut MapLabels,
) -> Vec<Spot> {
    let (camera, projection, transform) = camera;
    let fov = match projection {
        Projection::Perspective(p) => p.fov,
        _ => 1.0,
    };
    let half_height = camera.logical_viewport_size().map_or(1.0, |s| s.y / 2.0);
    let eye = rig.position;
    // Whether no body but `own` stands in the way of `at`.
    let in_sight = |at: DVec3, own: Option<BodyId>| {
        let others: Vec<(Vec3, f32)> = (sim.universe.system.bodies().iter())
            .filter(|body| Some(body.id) != own)
            .map(|body| (to_render(frame.body_offset(body.id)), body.radius as f32))
            .collect();
        !hidden(to_render(eye), to_render(at), &others)
    };

    let mut found = Vec::new();
    let mut placed: Vec<Rect> = Vec::new();
    for candidate in candidates(sim, frame, rig.map_view) {
        let angle = (candidate.radius / candidate.at.distance(eye))
            .min(1.0)
            .asin() as f32;
        let disc = half_height * angle.tan() / (fov / 2.0).tan();
        let body = match candidate.subject {
            Subject::Body(id) => Some(id),
            Subject::Vessel(_) => None,
        };
        if (body.is_some() && disc > MAX_DISC_PX) || !in_sight(candidate.at, body) {
            continue;
        }
        let Some(centre) = project(camera, transform, to_render(candidate.at)) else {
            continue;
        };
        let boxed = body.is_some() || disc <= MAX_VESSEL_PX;
        let half = if boxed {
            (disc + PADDING_PX).max(MIN_HALF_PX)
        } else {
            0.0
        };
        let top = if boxed {
            -half
        } else {
            disc.min(MAX_NAME_DROP_PX)
        };
        let outline = Rect::from_center_half_size(centre, Vec2::splat(half));
        let name = Rect::from_center_size(
            centre + Vec2::Y * (top.max(half) + NAME_GAP_PX + LINE_PX / 2.0),
            Vec2::new(GLYPH_PX * candidate.line.chars().count() as f32, LINE_PX),
        );
        let crowded = placed
            .iter()
            .any(|r| !r.intersect(outline).is_empty() || !r.intersect(name).is_empty());
        if crowded {
            continue;
        }
        placed.extend([outline, name]);
        labels.reserve(outline);
        labels.reserve(name);
        found.push(Spot {
            subject: candidate.subject,
            centre,
            half,
            top,
            name: candidate.name,
            line: candidate.line,
            spot_only: false,
            targeted: candidate.targeted,
        });
    }

    if rig.map_view {
        let system = &sim.universe.system;
        let target = sim.active_vessel().target;
        let debris = (sim.universe.vessels.iter())
            .filter(|vessel| vessel.is_debris())
            .filter(|vessel| target.and_then(Target::vessel) != Some(vessel.id()));
        for vessel in debris {
            let at = frame.relative(
                vessel.parent(),
                vessel.state_at(system, frame.time).position,
            );
            if !in_sight(at, None) {
                continue;
            }
            if let Some(centre) = project(camera, transform, to_render(at)) {
                found.push(Spot {
                    subject: Subject::Vessel(vessel.id()),
                    centre,
                    half: MIN_HALF_PX,
                    top: -MIN_HALF_PX,
                    name: vessel.name.clone(),
                    line: vessel.name.clone(),
                    spot_only: true,
                    targeted: false,
                });
            }
        }
    }
    found
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn place(
    mut commands: Commands,
    ui_scale: Res<UiScale>,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    docs: Res<UiDocuments>,
    (menu, editor, layouts): (Res<Menu>, Res<EditorStatus>, Res<LayoutVisibility>),
    mut labels: ResMut<MapLabels>,
    camera: Single<(&Camera, &Projection, &Transform), With<MainCamera>>,
    mut markers: Query<(Entity, &Marker, &mut Node, &mut Visibility)>,
    mut outlines: Query<
        (
            &mut Node,
            &mut BorderColor,
            &mut BackgroundColor,
            &mut MarkerBox,
            &Hovered,
        ),
        Without<Marker>,
    >,
    mut names: Query<(&mut Text, &mut TextColor)>,
) {
    let showing = !menu.open && !editor.is_open() && !layouts.hidden_groups.contains("flight");
    let spots = if showing {
        spots(&sim, &frame, &rig, *camera, &mut labels)
    } else {
        Vec::new()
    };

    // A marker for everything that needs one; they show next frame.
    for spot in &spots {
        if !markers.iter().any(|(_, m, ..)| m.subject == spot.subject) {
            spawn_marker(&mut commands, spot.subject, docs.text_font(12.0));
        }
    }

    for (entity, marker, mut node, mut visibility) in &mut markers {
        let gone = match marker.subject {
            Subject::Body(id) => sim.universe.system.bodies().get(id.0).is_none(),
            Subject::Vessel(id) => sim.universe.vessel(id).is_none(),
        };
        if gone {
            commands.entity(entity).despawn();
            continue;
        }
        let Some(spot) = spots.iter().find(|spot| spot.subject == marker.subject) else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        node.left = Val::Px(spot.centre.x / ui_scale.0 - COLUMN_PX / 2.0);
        node.top = Val::Px(spot.centre.y / ui_scale.0 + spot.top);
        visibility.set_if_neq(Visibility::Inherited);
        let Ok((mut outline, mut border, mut background, mut named, hovered)) =
            outlines.get_mut(marker.outline)
        else {
            continue;
        };
        let hovered = hovered.get();
        let color = if spot.targeted {
            TARGET_COLOR
        } else if hovered {
            HOVER_COLOR
        } else {
            COLOR
        };
        let fill = if hovered { HOVER_FILL } else { Color::NONE };
        let display = if spot.half > 0.0 {
            Display::Flex
        } else {
            Display::None
        };
        if outline.display != display {
            outline.display = display;
        }
        outline.width = Val::Px(2.0 * spot.half);
        outline.height = Val::Px(2.0 * spot.half);
        let edge = if spot.spot_only { Color::NONE } else { color };
        border.set_if_neq(BorderColor::all(edge));
        background.set_if_neq(BackgroundColor(fill));
        if named.0 != spot.name {
            named.0.clone_from(&spot.name);
        }
        if let Ok((mut text, mut text_color)) = names.get_mut(marker.name) {
            let line = if spot.spot_only && !hovered {
                ""
            } else {
                spot.line.as_str()
            };
            if text.0 != line {
                text.0 = line.to_owned();
            }
            text_color.set_if_neq(TextColor(color));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vessel_flown_never_gets_a_box() {
        assert!(!gets_box(true, true, false, false, 0.0));
        assert!(!gets_box(false, true, false, true, 0.0));
    }

    #[test]
    fn in_the_flight_view_only_what_is_near_gets_a_box() {
        assert!(gets_box(false, false, false, false, BOX_RANGE));
        assert!(!gets_box(false, false, false, false, BOX_RANGE + 1.0));
    }

    #[test]
    fn in_the_map_every_vessel_gets_a_box_at_any_distance() {
        assert!(gets_box(true, false, false, false, 1e12));
    }

    #[test]
    fn the_target_gets_a_box_at_any_distance() {
        assert!(gets_box(false, false, false, true, 1e12));
        assert!(gets_box(false, false, true, true, 1e12));
        assert!(gets_box(true, false, true, true, 1e12));
    }

    #[test]
    fn debris_gets_no_box_however_near() {
        assert!(!gets_box(false, false, true, false, 1.0));
        assert!(!gets_box(true, false, true, false, 1.0));
    }

    #[test]
    fn the_flight_view_says_how_far() {
        assert_eq!(line_of("Kestrel", true, 2300.0), "Kestrel");
        let line = line_of("Kestrel", false, 2300.0);
        assert!(line.starts_with("Kestrel, 2"), "{line}");
    }
}
