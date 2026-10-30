//! Map markers: small marks pinned to points in the 3D view (apsides, plane crossings,
//! closest approaches, planned burns and their handles). Systems push them into
//! [`MapLabels`] during `FrameSet::Render`; at the end of the frame they are projected to the
//! screen and drawn as UI under the windows.
//!
//! A marker is an icon (a dot unless it says otherwise) at a fixed size on screen, with a
//! short tag beside it ("Ap"). With the pointer on it the tag gives way to its detail
//! ("Apoapsis 300 km, T-00:45:03"), so the map stays readable however much is marked.
//! Markers on points hidden behind a planet are left out, and tags keep off areas reserved
//! for body markers and off each other.

use bevy::{platform::collections::HashMap, prelude::*};
use osp_ui::UiDocuments;

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera},
    content::{GameContent, mod_asset_path},
    frame::{RenderFrame, to_render},
    orbit_lines::{HoverPoint, ViewPointer, project},
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

/// The default icon.
pub const DOT: &str = "icons/map_dot.svg";
const DOT_PX: f32 = 9.0;
/// How near the pointer must be to a marker's point to show its detail, logical pixels.
pub const HOVER_PX: f32 = 10.0;
/// Height of a tag line and rough glyph width, logical pixels, for keeping tags apart.
const LINE: f32 = 15.0;
const GLYPH_PX: f32 = 6.5;
/// Gap between an icon and its tag, px.
const TAG_GAP: f32 = 4.0;
/// Behind a shown detail, so it reads over whatever it covers.
const DETAIL_BACKGROUND: Color = Color::srgba(0.04, 0.08, 0.12, 0.9);

/// A mark on the map.
#[derive(Debug, Clone)]
pub struct Marker {
    /// Render-space point.
    pub at: Vec3,
    pub color: Color,
    /// Picture (relative to the mod root), drawn `size` px square and centred on the point.
    pub icon: &'static str,
    pub size: f32,
    /// Shown beside it all the time; may be empty.
    pub tag: String,
    /// Shown instead of the tag while the pointer is on it; empty to keep the tag.
    pub detail: String,
    /// The trajectory point it marks, so a click there lands exactly on it.
    pub snap: Option<HoverPoint>,
}

impl Marker {
    pub fn dot(at: Vec3, color: Color) -> Self {
        Self::icon(at, DOT, DOT_PX, color)
    }

    pub fn icon(at: Vec3, icon: &'static str, size: f32, color: Color) -> Self {
        Self {
            at,
            color,
            icon,
            size,
            tag: String::new(),
            detail: String::new(),
            snap: None,
        }
    }

    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tag = tag.into();
        self
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn snap(mut self, snap: Option<HoverPoint>) -> Self {
        self.snap = snap;
        self
    }
}

/// Markers to draw this frame.
#[derive(Resource, Default)]
pub struct MapLabels {
    markers: Vec<Marker>,
    /// Screen areas tags keep off (logical pixels).
    reserved: Vec<Rect>,
}

impl MapLabels {
    pub fn add(&mut self, marker: Marker) {
        self.markers.push(marker);
    }

    /// Keep tags off `area` (logical pixels), such as a body's marker. Reserve during
    /// `FrameSet::Render`.
    pub fn reserve(&mut self, area: Rect) {
        self.reserved.push(area);
    }

    pub fn markers(&self) -> &[Marker] {
        &self.markers
    }
}

/// One pooled icon node.
#[derive(Component)]
struct IconNode;

/// One pooled tag node.
#[derive(Component)]
struct TagNode;

fn clear(mut labels: ResMut<MapLabels>) {
    labels.markers.clear();
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

/// Every body as a render-space sphere, for [`hidden`]; none outside map view.
pub fn body_spheres(sim: &Sim, frame: &RenderFrame) -> Vec<(Vec3, f32)> {
    if frame.origin_body.is_none() {
        return Vec::new();
    }
    sim.universe
        .system
        .bodies()
        .iter()
        .map(|b| (to_render(frame.body_offset(b.id)), b.radius as f32))
        .collect()
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn place(
    mut commands: Commands,
    labels: Res<MapLabels>,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    docs: Res<UiDocuments>,
    content: Res<GameContent>,
    assets: Res<AssetServer>,
    pointer: ViewPointer,
    camera: Single<(&Camera, &Transform), With<MainCamera>>,
    mut icons: Query<(&mut Node, &mut ImageNode, &mut Visibility), With<IconNode>>,
    mut tags: Query<
        (
            &mut Node,
            &mut Text,
            &mut TextColor,
            &mut BackgroundColor,
            &mut GlobalZIndex,
            &mut Visibility,
        ),
        (With<TagNode>, Without<IconNode>),
    >,
    mut pictures: Local<HashMap<&'static str, Handle<Image>>>,
) {
    let (camera, transform) = *camera;
    let spheres = body_spheres(&sim, &frame);
    let eye = to_render(rig.position);
    let shown: Vec<(Vec2, &Marker)> = labels
        .markers
        .iter()
        .filter(|m| !hidden(eye, m.at, &spheres))
        .filter_map(|m| Some((project(camera, transform, m.at)?, m)))
        .collect();
    // The marker under the pointer, nearest first; later markers win ties (handles are
    // pushed after the burn they belong to).
    let cursor = pointer.position();
    let hovered = cursor.and_then(|cursor| {
        shown
            .iter()
            .enumerate()
            .map(|(i, (at, m))| (i, at.distance(cursor) - (m.size / 2.0 - HOVER_PX).max(0.0)))
            .filter(|(_, d)| *d < HOVER_PX)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    });

    // Icons, centred on their points.
    let mut icon_nodes = icons.iter_mut();
    let mut missing_icons = 0;
    for (i, (at, marker)) in shown.iter().enumerate() {
        let size = if hovered == Some(i) {
            marker.size * 1.2
        } else {
            marker.size
        };
        let picture = pictures.entry(marker.icon).or_insert_with(|| {
            match mod_asset_path(&content.0, marker.icon) {
                Some(path) => assets.load(path),
                None => {
                    warn!("map icon `{}` is not in any mod", marker.icon);
                    Handle::default()
                }
            }
        });
        let Some((mut node, mut image, mut visibility)) = icon_nodes.next() else {
            missing_icons += 1;
            continue;
        };
        node.left = Val::Px(at.x - size / 2.0);
        node.top = Val::Px(at.y - size / 2.0);
        node.width = Val::Px(size);
        node.height = Val::Px(size);
        if image.image != *picture {
            image.image = picture.clone();
        }
        image.color = marker.color;
        visibility.set_if_neq(Visibility::Inherited);
    }
    for (_, _, mut visibility) in icon_nodes {
        visibility.set_if_neq(Visibility::Hidden);
    }

    // Tags just right of their icons, nudged right past reserved areas and down past the
    // tags placed before them; the same tag at the same spot (two legs' periapsides at one
    // burn) once. The hovered marker's detail goes over everything.
    let mut placed: Vec<Rect> = labels.reserved.clone();
    let mut texts: Vec<(Vec2, &str, Color, bool)> = Vec::new();
    let mut tagged: Vec<(Vec2, &str)> = Vec::new();
    for (i, (at, marker)) in shown.iter().enumerate() {
        if hovered == Some(i)
            || marker.tag.is_empty()
            || tagged
                .iter()
                .any(|(p, tag)| *tag == marker.tag && p.distance(*at) < HOVER_PX)
        {
            continue;
        }
        tagged.push((*at, &marker.tag));
        let width = GLYPH_PX * marker.tag.chars().count() as f32;
        let mut corner = Vec2::new(at.x + marker.size / 2.0 + TAG_GAP, at.y - LINE / 2.0);
        for _ in 0..8 {
            let rect = Rect::from_corners(corner, corner + Vec2::new(width, LINE));
            match placed.iter().position(|r| !r.intersect(rect).is_empty()) {
                None => break,
                Some(k) if k < labels.reserved.len() => corner.x = placed[k].max.x + 4.0,
                Some(_) => corner.y += LINE,
            }
        }
        placed.push(Rect::from_corners(corner, corner + Vec2::new(width, LINE)));
        texts.push((corner, &marker.tag, marker.color, false));
    }
    if let Some((at, marker)) = hovered.map(|i| shown[i]) {
        let text = if marker.detail.is_empty() {
            &marker.tag
        } else {
            &marker.detail
        };
        if !text.is_empty() {
            let corner = Vec2::new(at.x + marker.size * 0.6 + TAG_GAP, at.y - LINE / 2.0 - 3.0);
            texts.push((corner, text, marker.color, true));
        }
    }
    let mut tag_nodes = tags.iter_mut();
    let mut missing_tags = Vec::new();
    for (corner, label, tint, detail) in texts {
        let Some((mut node, mut text, mut color, mut background, mut z, mut visibility)) =
            tag_nodes.next()
        else {
            missing_tags.push(label.to_owned());
            continue;
        };
        node.left = Val::Px(corner.x);
        node.top = Val::Px(corner.y);
        let padding = if detail {
            UiRect::axes(Val::Px(6.0), Val::Px(3.0))
        } else {
            UiRect::ZERO
        };
        if node.padding != padding {
            node.padding = padding;
        }
        if text.0 != label {
            text.0 = label.to_owned();
        }
        color.0 = tint;
        background.set_if_neq(BackgroundColor(if detail {
            DETAIL_BACKGROUND
        } else {
            Color::NONE
        }));
        z.set_if_neq(GlobalZIndex(if detail { -8 } else { -10 }));
        visibility.set_if_neq(Visibility::Inherited);
    }
    for (.., mut visibility) in tag_nodes {
        visibility.set_if_neq(Visibility::Hidden);
    }

    // More markers than nodes: grow the pools (they show from next frame). All sit under
    // every window and leave the pointer to the map.
    for _ in 0..missing_icons {
        commands.spawn((
            IconNode,
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            ImageNode::default(),
            GlobalZIndex(-10),
            Pickable::IGNORE,
            Visibility::Hidden,
        ));
    }
    for label in missing_tags {
        commands.spawn((
            TagNode,
            Node {
                position_type: PositionType::Absolute,
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..default()
            },
            Text::new(label),
            docs.text_font(12.0),
            TextColor(Color::WHITE),
            TextShadow {
                offset: Vec2::splat(1.0),
                color: Color::srgba(0.0, 0.0, 0.0, 0.8),
            },
            BackgroundColor(Color::NONE),
            GlobalZIndex(-10),
            Pickable::IGNORE,
            Visibility::Hidden,
        ));
    }
}
