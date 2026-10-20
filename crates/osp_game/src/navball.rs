//! The navball: an attitude indicator, rendered by its own camera into a texture that the
//! `navball` UI widget displays.
//!
//! The ball is the sky around the vessel: blue above the local horizon, brown below, with
//! pitch circles every 10° and heading meridians every 30°. It turns so the direction the
//! nose points faces the viewer, with the pilot's head up, and shows the prograde,
//! retrograde, normal, anti-normal and radial directions of the current navigation frame
//! (surface or orbit). Markers pointing away are hidden behind the ball.
//!
//! As on a real flight director the markings read as seen from inside, so headings
//! increase to the right: the texture is laid out mirrored, which makes the turn from
//! vessel axes to the ball a proper rotation.

use bevy::{
    asset::RenderAssetUsages,
    camera::{RenderTarget, ScalingMode, visibility::RenderLayers},
    math::{DMat3, DQuat, DVec3},
    prelude::*,
    render::{
        render_resource::{Extent3d, TextureDimension, TextureFormat},
        view::{DebandDither, Tonemapping},
    },
    ui::ComputedNode,
};
use osp_ui::{WidgetContext, WidgetRegistry};

use crate::{FrameSet, sim::Sim};

/// Render layer of the navball scene, which no other camera sees.
const LAYER: usize = 7;
/// Size of the rendered ball, px.
const RESOLUTION: u32 = 512;
/// Ball texture, px (equirectangular: heading across, pitch down).
const TEXTURE: (u32, u32) = (2048, 1024);
const MARKER_SIZE: f32 = 0.34;

pub struct NavballPlugin;

impl Plugin for NavballPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut()
            .resource_mut::<WidgetRegistry>()
            .register("navball", navball_widget);
        app.add_systems(Startup, spawn_navball).add_systems(
            Update,
            (orient_navball, attach_images).in_set(FrameSet::Render),
        );
    }
}

/// The navball's rendered image.
#[derive(Resource)]
struct NavballImage(Handle<Image>);

#[derive(Component)]
struct NavballCamera;

#[derive(Component)]
struct Ball;

#[derive(Component, Clone, Copy)]
enum Marker {
    Prograde,
    Retrograde,
    Normal,
    AntiNormal,
    RadialOut,
    RadialIn,
}

impl Marker {
    const ALL: [Marker; 6] = [
        Marker::Prograde,
        Marker::Retrograde,
        Marker::Normal,
        Marker::AntiNormal,
        Marker::RadialOut,
        Marker::RadialIn,
    ];
}

/// UI node showing the navball.
#[derive(Component)]
struct NavballView;

/// `widget: "navball"`: shows the ball; size it with `style.width` / `height`.
fn navball_widget(ctx: &mut WidgetContext) -> Entity {
    ctx.commands.entity(ctx.entity).insert(NavballView);
    ctx.entity
}

fn attach_images(
    mut commands: Commands,
    image: Res<NavballImage>,
    new_views: Query<Entity, (With<NavballView>, Without<ImageNode>)>,
    views: Query<&ComputedNode, With<NavballView>>,
    mut camera: Single<&mut Camera, With<NavballCamera>>,
) {
    for view in &new_views {
        commands
            .entity(view)
            .insert(ImageNode::new(image.0.clone()));
    }
    // Only render while a navball is on screen.
    let shown = views.iter().any(|node| node.size().x > 0.0);
    if camera.is_active != shown {
        camera.is_active = shown;
    }
}

fn spawn_navball(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let layer = RenderLayers::layer(LAYER);
    let target = images.add(Image::new_target_texture(
        RESOLUTION,
        RESOLUTION,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    commands.insert_resource(NavballImage(target.clone()));
    commands.spawn((
        NavballCamera,
        Camera3d::default(),
        Camera {
            // Before the main camera, so the UI shows this frame's ball.
            order: -1,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::from(target),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: 2.04,
                height: 2.04,
            },
            near: 0.1,
            far: 10.0,
            ..OrthographicProjection::default_3d()
        }),
        // The texture's colours exactly, without exposure or tonemapping.
        Tonemapping::None,
        DebandDither::Disabled,
        Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        layer.clone(),
    ));

    let mut unlit = |texture: Image, materials: &mut Assets<StandardMaterial>, blend: bool| {
        materials.add(StandardMaterial {
            base_color_texture: Some(images.add(texture)),
            unlit: true,
            alpha_mode: if blend {
                AlphaMode::Blend
            } else {
                AlphaMode::Opaque
            },
            ..default()
        })
    };
    let ball = unlit(ball_texture(), &mut materials, false);
    commands.spawn((
        Ball,
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(96, 48))),
        MeshMaterial3d(ball),
        Transform::default(),
        layer.clone(),
    ));
    let quad = meshes.add(Rectangle::new(MARKER_SIZE, MARKER_SIZE));
    for marker in Marker::ALL {
        let material = unlit(marker_texture(marker), &mut materials, true);
        commands.spawn((
            marker,
            Mesh3d(quad.clone()),
            MeshMaterial3d(material),
            Transform::default(),
            Visibility::Hidden,
            layer.clone(),
        ));
    }
    // The vessel symbol, fixed in front of the ball's centre.
    let reticle = unlit(reticle_texture(), &mut materials, true);
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(0.9, 0.45))),
        MeshMaterial3d(reticle),
        Transform::from_xyz(0.0, 0.0, 2.0),
        layer,
    ));
}

/// Turn the ball and place the markers for the active vessel.
fn orient_navball(
    sim: Res<Sim>,
    mut ball: Single<&mut Transform, With<Ball>>,
    mut markers: Query<(&Marker, &mut Transform, &mut Visibility), Without<Ball>>,
) {
    let system = &sim.universe.system;
    let now = sim.render_time;
    let vessel = sim.active_vessel();
    let body = system.body(vessel.parent());
    let attitude = vessel.attitude_state(system, now);
    let up = vessel.state_at(system, now).position.normalize();
    let north = (body.pole() - up * body.pole().dot(up)).normalize_or(DVec3::X);
    let east = north.cross(up);

    // On screen: the pilot's right (+X), head (−Y) and nose (+Z, facing the viewer).
    let q = attitude.orientation;
    let (right, head, nose) = (q * DVec3::X, -(q * DVec3::Y), q * DVec3::Z);
    // The ball's mirrored sky coordinates of a direction: (east, up, north).
    let sky = |d: DVec3| DVec3::new(d.dot(east), d.dot(up), d.dot(north));
    let to_screen = DMat3::from_cols(sky(right), sky(head), sky(nose)).transpose();
    // The sphere mesh has its pole on +Z and longitude 0 on +X: north at +X, east at +Y.
    let mesh_to_sky = DMat3::from_cols(DVec3::Z, DVec3::X, DVec3::Y);
    ball.rotation = DQuat::from_mat3(&(to_screen * mesh_to_sky)).as_quat();

    let basis = vessel.nav_basis(system, now);
    for (marker, mut transform, mut visibility) in &mut markers {
        let Some(basis) = basis else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let direction = match marker {
            Marker::Prograde => basis.prograde,
            Marker::Retrograde => -basis.prograde,
            Marker::Normal => basis.normal,
            Marker::AntiNormal => -basis.normal,
            Marker::RadialOut => basis.radial_out,
            Marker::RadialIn => -basis.radial_out,
        };
        let screen = DVec3::new(
            direction.dot(right),
            direction.dot(head),
            direction.dot(nose),
        );
        // Slightly above the surface; the ball hides those on the far side.
        transform.translation = (screen * 1.02).as_vec3();
        visibility.set_if_neq(if screen.z > -0.2 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

fn image(width: u32, height: u32, data: Vec<u8>) -> Image {
    Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t.clamp(0.0, 1.0))
}

fn rgb(hex: u32) -> [f32; 3] {
    [16, 8, 0].map(|s| ((hex >> s) & 0xff) as f32 / 255.0)
}

/// 3×5 glyphs for the ball's labels, top row first, bit 2 = left column.
fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [7, 5, 5, 5, 7],
        '1' => [2, 6, 2, 2, 7],
        '2' => [7, 1, 7, 4, 7],
        '3' => [7, 1, 7, 1, 7],
        '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 7, 1, 7],
        '6' => [7, 4, 7, 5, 7],
        '7' => [7, 1, 1, 1, 1],
        '8' => [7, 5, 7, 5, 7],
        '9' => [7, 5, 7, 1, 7],
        'N' => [5, 7, 7, 7, 5],
        'E' => [7, 4, 6, 4, 7],
        'S' => [7, 4, 7, 1, 7],
        'W' => [5, 5, 7, 7, 5],
        _ => [0; 5],
    }
}

/// A label on the ball: text centred at (heading, pitch), degrees.
struct Label {
    text: &'static str,
    heading: f32,
    pitch: f32,
}

/// Glyph height on the ball, degrees of arc.
const LABEL_HEIGHT: f32 = 3.2;

impl Label {
    /// Whether the label covers the point (degrees from its centre, `dx` across as arc).
    fn covers(&self, dx: f32, dy: f32) -> bool {
        let unit = LABEL_HEIGHT / 5.0;
        let width = (self.text.len() * 4 - 1) as f32 * unit;
        let (x, y) = (dx + width / 2.0, LABEL_HEIGHT / 2.0 - dy);
        if x < 0.0 || y < 0.0 || x >= width || y >= LABEL_HEIGHT {
            return false;
        }
        let (cell, row) = ((x / unit) as usize, (y / unit) as usize);
        let (index, column) = (cell / 4, cell % 4);
        column < 3
            && self
                .text
                .chars()
                .nth(index)
                .is_some_and(|c| glyph(c)[row.min(4)] & (4 >> column) != 0)
    }
}

fn labels() -> Vec<Label> {
    let mut labels = Vec::new();
    let headings = [
        "N", "3", "6", "E", "12", "15", "S", "21", "24", "W", "30", "33",
    ];
    for (i, text) in headings.into_iter().enumerate() {
        labels.push(Label {
            text,
            heading: i as f32 * 30.0,
            pitch: 4.5,
        });
    }
    for heading in [45.0, 135.0, 225.0, 315.0] {
        for (pitch, text) in [(30.0, "30"), (60.0, "60"), (-30.0, "30"), (-60.0, "60")] {
            labels.push(Label {
                text,
                heading,
                pitch: pitch + if pitch > 0.0 { 3.0 } else { -3.0 },
            });
        }
    }
    labels
}

/// The ball's surface: u is heading 0..360°, v pitch +90..−90°.
fn ball_texture() -> Image {
    let (w, h) = TEXTURE;
    let degrees_per_px = 180.0 / h as f32;
    let (sky_high, sky_low) = (rgb(0x1d4f8f), rgb(0x4d8fd6));
    let (ground_high, ground_low) = (rgb(0x9a6a3c), rgb(0x4a2e18));
    let white = [0.95, 0.95, 0.92];
    let labels = labels();
    // Anti-aliased line coverage, from the distance to the line in degrees of arc.
    let line = |distance: f32, width: f32| {
        (0.5 + (width / 2.0 - distance.abs()) / degrees_per_px).clamp(0.0, 1.0)
    };
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let pitch = 90.0 - (y as f32 + 0.5) * degrees_per_px;
        let cos_pitch = pitch.to_radians().cos();
        let near: Vec<&Label> = labels
            .iter()
            .filter(|l| (l.pitch - pitch).abs() < LABEL_HEIGHT)
            .collect();
        for x in 0..w {
            let heading = (x as f32 + 0.5) / w as f32 * 360.0;
            let mut color = if pitch >= 0.0 {
                mix(sky_low, sky_high, pitch / 90.0)
            } else {
                mix(ground_high, ground_low, -pitch / 90.0)
            };
            let mut ink = line(pitch, 0.9);
            let ten = pitch - (pitch / 10.0).round() * 10.0;
            let major = (pitch / 10.0).round() as i32 % 3 == 0;
            ink = ink.max(0.8 * line(ten, if major { 0.45 } else { 0.25 }));
            if pitch.abs() < 80.0 {
                let thirty = heading - (heading / 30.0).round() * 30.0;
                let cardinal = (heading / 30.0).round() as i32 % 3 == 0;
                ink = ink.max(0.8 * line(thirty * cos_pitch, if cardinal { 0.5 } else { 0.3 }));
            }
            if pitch.abs() > 88.5 {
                ink = ink.max(line(90.0 - pitch.abs() - 1.0, 0.3));
            }
            for label in &near {
                let mut dx = heading - label.heading;
                dx -= (dx / 360.0).round() * 360.0;
                if label.covers(dx * cos_pitch, pitch - label.pitch) {
                    ink = 1.0;
                }
            }
            color = mix(color, white, ink);
            data.extend(color.map(|c| (c * 255.0).round() as u8));
            data.push(255);
        }
    }
    image(w, h, data)
}

/// Distance from `p` to the segment `a`–`b`.
fn segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// An icon from strokes: `distance(p)` to the nearest stroke centre, in −1..1 units.
fn paint(width: u32, height: u32, color: [f32; 3], distance: impl Fn(Vec2) -> f32) -> Image {
    let scale = 2.0 / width as f32;
    let stroke = 0.11;
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let p = Vec2::new(
                (x as f32 + 0.5) * scale - 1.0,
                (height as f32 / 2.0 - y as f32 - 0.5) * scale,
            );
            let d = distance(p);
            let coverage = (0.5 + (stroke / 2.0 - d) / scale).clamp(0.0, 1.0);
            // A dark outline under the stroke keeps markers readable on any background.
            let outline = (0.5 + (stroke / 2.0 + 0.07 - d) / scale).clamp(0.0, 1.0);
            let c = mix([0.05, 0.05, 0.05], color, coverage / outline.max(1e-3));
            data.extend(c.map(|v| (v * 255.0).round() as u8));
            data.push((outline * 255.0).round() as u8);
        }
    }
    image(width, height, data)
}

fn ring(p: Vec2, r: f32) -> f32 {
    (p.length() - r).abs()
}

fn spokes(p: Vec2, directions: &[Vec2], from: f32, to: f32) -> f32 {
    directions
        .iter()
        .map(|d| segment(p, *d * from, *d * to))
        .fold(f32::INFINITY, f32::min)
}

fn triangle(p: Vec2, r: f32, apex_up: bool) -> (f32, [Vec2; 3]) {
    let sign = if apex_up { 1.0 } else { -1.0 };
    let corners = [90.0f32, 210.0, 330.0].map(|a| Vec2::from_angle(a.to_radians()) * r * sign);
    let edges = (0..3)
        .map(|i| segment(p, corners[i], corners[(i + 1) % 3]))
        .fold(f32::INFINITY, f32::min);
    (edges, corners)
}

fn marker_texture(marker: Marker) -> Image {
    let diagonal = [
        Vec2::new(1.0, 1.0),
        Vec2::new(-1.0, 1.0),
        Vec2::new(-1.0, -1.0),
        Vec2::new(1.0, -1.0),
    ]
    .map(Vec2::normalize);
    let (prograde, normal, radial) = (rgb(0xd7f23c), rgb(0xd65cff), rgb(0x3cd8ff));
    let size = 96;
    match marker {
        Marker::Prograde => paint(size, size, prograde, |p| {
            ring(p, 0.42)
                .min(spokes(p, &[Vec2::X, Vec2::NEG_X, Vec2::Y], 0.42, 0.9))
                .min(p.length() - 0.02)
        }),
        Marker::Retrograde => paint(size, size, prograde, |p| {
            let cross = spokes(p, &diagonal, 0.0, 0.3);
            ring(p, 0.42)
                .min(spokes(p, &[Vec2::X, Vec2::NEG_X, Vec2::NEG_Y], 0.42, 0.9))
                .min(cross)
        }),
        Marker::Normal => paint(size, size, normal, |p| {
            triangle(p, 0.6, true).0.min(p.length() - 0.02)
        }),
        Marker::AntiNormal => paint(size, size, normal, |p| {
            let (edges, corners) = triangle(p, 0.45, false);
            let out = corners
                .map(|c| segment(p, c, c * 1.9))
                .into_iter()
                .fold(f32::INFINITY, f32::min);
            edges.min(out)
        }),
        Marker::RadialOut => paint(size, size, radial, |p| {
            ring(p, 0.38)
                .min(spokes(p, &diagonal, 0.38, 0.85))
                .min(p.length() - 0.02)
        }),
        Marker::RadialIn => paint(size, size, radial, |p| {
            ring(p, 0.6).min(spokes(p, &diagonal, 0.18, 0.6))
        }),
    }
}

/// The fixed vessel symbol: wings and a chevron with a dot at the centre.
fn reticle_texture() -> Image {
    let points = [
        Vec2::new(-0.95, 0.0),
        Vec2::new(-0.4, 0.0),
        Vec2::new(0.0, -0.32),
        Vec2::new(0.4, 0.0),
        Vec2::new(0.95, 0.0),
    ];
    paint(192, 96, rgb(0xffa21f), |p| {
        let wings = points
            .windows(2)
            .map(|w| segment(p, w[0], w[1]))
            .fold(f32::INFINITY, f32::min);
        wings.min(p.length() - 0.03)
    })
}
