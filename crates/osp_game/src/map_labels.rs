//! Text labels pinned to points in the 3D view (map markers: apsides, nodes, closest
//! approaches). Systems push labels into [`MapLabels`] during `FrameSet::Render`; at the end
//! of the frame they are projected to the screen and drawn as UI text under the windows.
//! Labels on points hidden behind a planet are left out, and labels keep off areas reserved
//! for body markers.

use bevy::prelude::*;

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera},
    frame::{RenderFrame, to_render},
    orbit_lines::project,
    sim::Sim,
};

pub struct MapLabelsPlugin;

impl Plugin for MapLabelsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapLabels>()
            .add_systems(Update, clear.in_set(FrameSet::Camera))
            .add_systems(Update, place.after(FrameSet::Render));
    }
}

/// Labels to draw this frame.
#[derive(Resource, Default)]
pub struct MapLabels {
    labels: Vec<(Vec3, String, Color)>,
    /// Screen areas labels keep off (logical pixels).
    reserved: Vec<Rect>,
}

impl MapLabels {
    /// Label the render-space point `at`.
    pub fn add(&mut self, at: Vec3, text: impl Into<String>, color: Color) {
        self.labels.push((at, text.into(), color));
    }

    /// Keep labels off `area` (logical pixels), such as a body's marker. Reserve during
    /// `FrameSet::Render`.
    pub fn reserve(&mut self, area: Rect) {
        self.reserved.push(area);
    }
}

/// Height of a label line, logical pixels.
const LINE: f32 = 15.0;

/// One pooled label node.
#[derive(Component)]
struct LabelNode;

fn clear(mut labels: ResMut<MapLabels>) {
    labels.labels.clear();
    labels.reserved.clear();
}

/// Whether a body's sphere hides `point` from `eye` (render space, render-space spheres).
pub fn hidden(eye: Vec3, point: Vec3, spheres: &[(Vec3, f32)]) -> bool {
    let to = point - eye;
    let length = to.length();
    let direction = to / length.max(1e-6);
    spheres.iter().any(|&(center, radius)| {
        // Nearest approach of the sight line to the centre, before the point.
        let along = (center - eye).dot(direction);
        along > 0.0 && along < length && (eye + direction * along).distance(center) < radius * 0.999
    })
}

fn place(
    mut commands: Commands,
    labels: Res<MapLabels>,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    camera: Single<(&Camera, &Transform), With<MainCamera>>,
    mut nodes: Query<(&mut Node, &mut Text, &mut TextColor, &mut Visibility), With<LabelNode>>,
) {
    let (camera, transform) = *camera;
    let spheres: Vec<(Vec3, f32)> = if frame.origin_body.is_some() {
        sim.universe
            .system
            .bodies()
            .iter()
            .map(|b| (to_render(frame.body_offset(b.id)), b.radius as f32))
            .collect()
    } else {
        Vec::new()
    };
    let eye = to_render(rig.position);
    // Just right of and above each point, nudged right past reserved areas and down past
    // the labels placed before it.
    let mut placed: Vec<Rect> = labels.reserved.clone();
    let positioned: Vec<_> = labels
        .labels
        .iter()
        .filter(|(at, _, _)| !hidden(eye, *at, &spheres))
        .filter_map(|(at, text, color)| {
            let screen = project(camera, transform, *at)?;
            let width = 7.5 * text.chars().count() as f32;
            let mut corner = Vec2::new(screen.x + 8.0, screen.y - 18.0);
            for _ in 0..8 {
                let rect = Rect::from_corners(corner, corner + Vec2::new(width, LINE));
                match placed.iter().position(|r| !r.intersect(rect).is_empty()) {
                    None => break,
                    Some(i) if i < labels.reserved.len() => corner.x = placed[i].max.x + 4.0,
                    Some(_) => corner.y += LINE,
                }
            }
            placed.push(Rect::from_corners(corner, corner + Vec2::new(width, LINE)));
            Some((corner, text, color))
        })
        .collect();
    let mut shown = positioned.into_iter();
    for (mut node, mut text, mut color, mut visibility) in &mut nodes {
        match shown.next() {
            Some((corner, label, tint)) => {
                node.left = Val::Px(corner.x);
                node.top = Val::Px(corner.y);
                if text.0 != *label {
                    text.0.clone_from(label);
                }
                color.0 = *tint;
                visibility.set_if_neq(Visibility::Inherited);
            }
            None => {
                visibility.set_if_neq(Visibility::Hidden);
            }
        }
    }
    // More labels than nodes: grow the pool (they show from next frame).
    for (_, label, tint) in shown {
        commands.spawn((
            LabelNode,
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Text::new(label.clone()),
            TextFont {
                font_size: FontSize::Px(12.0),
                ..default()
            },
            TextColor(*tint),
            // Under every window.
            GlobalZIndex(-10),
            Pickable::IGNORE,
            Visibility::Hidden,
        ));
    }
}
