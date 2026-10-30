//! Map-view markers on celestial bodies: a box around each body's disc with its
//! name under it. Click one to centre the map on that body; Shift+click to target it (again
//! to drop the target).
//!
//! A marker is left out while its body is big on screen, hidden behind another body, or
//! crowded by a more important marker (the target's, then the Sun's, the planets' and the
//! moons', bigger bodies first), so zoomed out a planet's moons tuck into its marker.

use bevy::{
    picking::{events::PointerClick, hover::Hovered, pointer::PointerButton},
    prelude::*,
};
use osp_sim::{
    body::{Body, BodyId},
    rendezvous::Target,
};
use osp_ui::{Tooltip, UiDocuments, ViewOverlay};

use crate::{
    FrameSet,
    camera::{CameraRig, Follow, MainCamera},
    frame::{RenderFrame, to_render},
    map_labels::{MapLabels, hidden},
    orbit_lines::{TARGET_COLOR, project},
    sim::Sim,
};

/// Half the side of the smallest box, logical pixels.
const MIN_HALF_PX: f32 = 9.0;
/// Gap between a body's disc and its box, px.
const PADDING_PX: f32 = 3.0;
/// Bodies bigger than this on screen (disc radius, px) are left unmarked.
const MAX_DISC_PX: f32 = 40.0;
/// Width the name is centred in, px.
const COLUMN_PX: f32 = 160.0;
const NAME_GAP_PX: f32 = 2.0;
/// Height of the name line and rough glyph width, px, for keeping markers apart.
const LINE_PX: f32 = 15.0;
const GLYPH_PX: f32 = 7.0;
const COLOR: Color = Color::srgba(0.85, 0.9, 1.0, 0.55);
const HOVER_COLOR: Color = Color::WHITE;
const HOVER_FILL: Color = Color::srgba(1.0, 1.0, 1.0, 0.08);

pub struct BodyMarkersPlugin;

impl Plugin for BodyMarkersPlugin {
    fn build(&self, app: &mut App) {
        // In `Render`, so the map labels keep off the markers.
        app.add_systems(Update, place.in_set(FrameSet::Render));
    }
}

/// A body's marker: a column centred on the body holding its box and name.
#[derive(Component)]
struct BodyMarker {
    body: BodyId,
    outline: Entity,
    name: Entity,
}

fn spawn_marker(commands: &mut Commands, body: &Body, font: TextFont) {
    let outline = commands
        .spawn((
            Node {
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(2.0)),
                ..default()
            },
            BorderColor::all(COLOR),
            BackgroundColor(Color::NONE),
            Tooltip("Click to centre the map here, Shift+click to target".into()),
            Hovered::default(),
            ViewOverlay,
        ))
        .id();
    let name = commands
        .spawn((
            Text::new(body.name.clone()),
            font,
            TextColor(COLOR),
            ViewOverlay,
        ))
        .id();
    commands
        .spawn((
            BodyMarker {
                body: body.id,
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

/// Click: centre the map on the body. Shift+click: target it, or drop it as the target.
fn click(
    mut event: On<PointerClick>,
    markers: Query<&BodyMarker>,
    keys: Res<ButtonInput<KeyCode>>,
    mut sim: ResMut<Sim>,
    mut rig: ResMut<CameraRig>,
) {
    if event.button != PointerButton::Primary {
        return;
    }
    let Ok(marker) = markers.get(event.entity) else {
        return;
    };
    event.propagate(false);
    if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        let parent = sim.active_vessel().parent();
        let target = Target::Body(marker.body);
        let vessel = sim.active_vessel_mut();
        if vessel.target == Some(target) {
            vessel.target = None;
        } else if marker.body != parent {
            vessel.target = Some(target);
        }
    } else {
        rig.set_follow(Some(Follow::Body(marker.body)), &sim);
    }
}

#[allow(clippy::too_many_arguments)]
fn place(
    mut commands: Commands,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    docs: Res<UiDocuments>,
    mut labels: ResMut<MapLabels>,
    camera: Single<(&Camera, &Projection, &Transform), With<MainCamera>>,
    mut markers: Query<(Entity, &BodyMarker, &mut Node, &mut Visibility, &Hovered)>,
    mut outlines: Query<(&mut Node, &mut BorderColor, &mut BackgroundColor), Without<BodyMarker>>,
    mut names: Query<(&mut Text, &mut TextColor)>,
) {
    let bodies = sim.universe.system.bodies();
    // One marker per body, rebuilt with the system (content reload); they show next frame.
    if markers.iter().count() != bodies.len() {
        for (entity, ..) in &markers {
            commands.entity(entity).despawn();
        }
        for body in bodies {
            spawn_marker(&mut commands, body, docs.text_font(12.0));
        }
        return;
    }

    let (camera, projection, transform) = *camera;
    let target = sim.active_vessel().target;
    // Centre and half size of each body's box, indexed by `BodyId`.
    let mut spots: Vec<Option<(Vec2, f32)>> = vec![None; bodies.len()];
    if rig.map_view && frame.origin_body.is_some() {
        let fov = match projection {
            Projection::Perspective(p) => p.fov,
            _ => 1.0,
        };
        let half_height = camera.logical_viewport_size().map_or(1.0, |s| s.y / 2.0);
        let eye = rig.position;
        let mut order: Vec<&Body> = bodies.iter().collect();
        order.sort_by(|a, b| {
            let targeted = |body: &Body| target == Some(Target::Body(body.id));
            targeted(b)
                .cmp(&targeted(a))
                .then(a.depth.cmp(&b.depth))
                .then(b.radius.total_cmp(&a.radius))
        });
        let mut placed: Vec<Rect> = Vec::new();
        for body in order {
            let at = frame.body_offset(body.id);
            let angle = (body.radius / at.distance(eye)).min(1.0).asin() as f32;
            let disc = half_height * angle.tan() / (fov / 2.0).tan();
            if disc > MAX_DISC_PX {
                continue;
            }
            let others: Vec<(Vec3, f32)> = bodies
                .iter()
                .filter(|b| b.id != body.id)
                .map(|b| (to_render(frame.body_offset(b.id)), b.radius as f32))
                .collect();
            if hidden(to_render(eye), to_render(at), &others) {
                continue;
            }
            let Some(centre) = project(camera, transform, to_render(at)) else {
                continue;
            };
            let half = (disc + PADDING_PX).max(MIN_HALF_PX);
            let outline = Rect::from_center_half_size(centre, Vec2::splat(half));
            let name = Rect::from_center_size(
                centre + Vec2::Y * (half + NAME_GAP_PX + LINE_PX / 2.0),
                Vec2::new(GLYPH_PX * body.name.chars().count() as f32, LINE_PX),
            );
            let crowded = placed
                .iter()
                .any(|r| !r.intersect(outline).is_empty() || !r.intersect(name).is_empty());
            if !crowded {
                placed.extend([outline, name]);
                labels.reserve(outline);
                labels.reserve(name);
                spots[body.id.0] = Some((centre, half));
            }
        }
    }

    for (_, marker, mut node, mut visibility, hovered) in &mut markers {
        let (Some(body), Some(Some((centre, half)))) =
            (bodies.get(marker.body.0), spots.get(marker.body.0))
        else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        node.left = Val::Px(centre.x - COLUMN_PX / 2.0);
        node.top = Val::Px(centre.y - half);
        visibility.set_if_neq(Visibility::Inherited);
        let color = if target == Some(Target::Body(body.id)) {
            TARGET_COLOR
        } else if hovered.get() {
            HOVER_COLOR
        } else {
            COLOR
        };
        let fill = if hovered.get() {
            HOVER_FILL
        } else {
            Color::NONE
        };
        if let Ok((mut outline, mut border, mut background)) = outlines.get_mut(marker.outline) {
            outline.width = Val::Px(2.0 * half);
            outline.height = Val::Px(2.0 * half);
            border.set_if_neq(BorderColor::all(color));
            background.set_if_neq(BackgroundColor(fill));
        }
        if let Ok((mut text, mut text_color)) = names.get_mut(marker.name) {
            if text.0 != body.name {
                text.0.clone_from(&body.name);
            }
            text_color.set_if_neq(TextColor(color));
        }
    }
}
