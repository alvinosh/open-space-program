//! The editor: where a craft is looked at before it flies. For now it only shows it. The player can
//! walk round it and change its staging with the same window as in flight, then launch it from
//! the pad, the runway or orbit (`editor.launch`); placing parts is still to come.
//!
//! The craft on show is a real vessel (`Vessel::on_display`), the one the player flies for now, so
//! the vessel renderer draws it and the staging window edits it as they do in flight. It stands far
//! over the star's pole, with time stopped, inside a hangar drawn around it that hides everything
//! else in the game, and goes when the editor closes. What launches is a new vessel built from
//! the same craft, with the sequences the player left it with.
//!
//! The camera here is its own: the flight camera lets go while the editor is open
//! ([`CameraRig::held`]). Right or left drag looks around, middle drag or WASD, Q and E moves,
//! the wheel zooms, all of it inside the hangar.

use bevy::{
    asset::RenderAssetUsages,
    camera::Exposure,
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    light::{NotShadowCaster, light_consts::lux},
    math::{Affine2, DVec2, DVec3},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, TextureFormat},
};
use osp_sim::{
    data::{CraftDef, LaunchSiteDef, LaunchSiteStartDef, OrbitStartDef, StartDef},
    time::SimTime,
    vessel::{Vessel, VesselId},
};
use osp_ui::{LayoutVisibility, PointerOverUi, UiAction, UiData};

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera, update_camera},
    content::{GameContent, catalog},
    controls::InputLocked,
    frame::{RenderFrame, Sunlight, to_render},
    menu::{Menu, Play, menu_actions},
    saves::Saves,
    sim::{Sim, SimRestarted, TimeWarp},
};

/// The layout with the launch button.
const LAYOUT: &str = "editor";
/// What stays on the screen of what flight shows, as the editor hides the rest: the staging
/// window, its menu, a part's window (to split a part off) and the dock that holds them.
const KEPT_LAYOUTS: [&str; 4] = ["dock_left", "staging", "staging_menu", "part_info"];
/// A new game's clock: around midday at the Cape, like the stock scenarios. Seconds since J2000.
const NEW_GAME_TIME: f64 = 21_600.0;
/// What a craft weighs by when there is no launch site to ask, m/s².
const STANDARD_GRAVITY: f64 = 9.80665;
/// How high an orbit a launch into orbit goes to, m.
const ORBIT_ALTITUDE: f64 = 200e3;
/// The light from the sky that the hangar stands in for, without a sky to see: `brightness` of
/// the ambient light, against the sun's lux at full exposure.
const HANGAR_AMBIENT: f32 = 6_000.0;
/// The fill light, as a share of the sun.
const FILL_LIGHT: f32 = 0.25;
/// The hangar is this many times the craft's size across, and the ends of this.
const HANGAR_RADIUS: f64 = 2.5;
const HANGAR_HEIGHT: f64 = 2.0;
const HANGAR_LEAST_RADIUS: f64 = 60.0;
const HANGAR_LEAST_HEIGHT: f64 = 40.0;
/// Between the lines on the floor, m, how wide they are, and how far between the heavier ones.
const GRID_STEP: f32 = 5.0;
const GRID_WIDTH: f32 = 0.06;
const GRID_TILE: f32 = 25.0;
/// How wide the floor's tile is, texels: a power of two.
const FLOOR_TEXELS: usize = 1024;
/// The floor's colours, linear.
const FLOOR_COLOR: [f32; 3] = [0.040, 0.047, 0.056];
const FLOOR_LINE: [f32; 3] = [0.090, 0.110, 0.130];
const FLOOR_MAJOR_LINE: [f32; 3] = [0.150, 0.190, 0.220];
/// Steepest the camera looks down or up, rad.
const MAX_PITCH: f64 = 1.5;
/// How close the camera comes to what it looks at, m.
const NEAREST: f64 = 2.0;
/// The camera stays this high above the floor, m.
const OVER_FLOOR: f64 = 0.5;
/// How far towards the walls and the roof it goes, as shares of the way there.
const WALL_REACH: f64 = 0.92;
const ROOF_REACH: f64 = 0.9;

pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Editor>().add_systems(
            Update,
            (
                // After the menu, which leaves Esc to the editor while the editor is open.
                editor_actions.in_set(FrameSet::Input).after(menu_actions),
                drive_camera.in_set(FrameSet::Camera).after(update_camera),
                place_hangar.in_set(FrameSet::Render),
                // After what lights the world, which would take this for a night in deep space.
                light_hangar.after(FrameSet::Render),
                publish_editor,
            ),
        );
    }
}

/// Where a launch goes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Pad,
    Runway,
    Orbit,
}

impl Place {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "pad" => Some(Self::Pad),
            "runway" => Some(Self::Runway),
            "orbit" => Some(Self::Orbit),
            _ => None,
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Pad => "pad",
            Self::Runway => "runway",
            Self::Orbit => "orbit",
        }
    }
}

/// The first launch site that has a pad, and the first that has a runway.
fn sites(all: &[LaunchSiteDef]) -> (Option<&LaunchSiteDef>, Option<&LaunchSiteDef>) {
    (
        all.iter().find(|site| site.pad.is_some()),
        all.iter().find(|site| site.runway.is_some()),
    )
}

/// What a launch from `place` starts as, if there is a site for it. An orbit goes round the body
/// of the first pad, inclined as a launch from there would be, starting at the point `sun` gives
/// for a body and inclination, degrees.
fn start_of(
    place: Place,
    all: &[LaunchSiteDef],
    sun: impl Fn(&str, f64) -> f64,
) -> Option<StartDef> {
    let (pad, runway) = sites(all);
    let at = |site: &LaunchSiteDef| LaunchSiteStartDef {
        site: site.id.clone(),
    };
    match place {
        Place::Pad => Some(StartDef::LaunchSite(at(pad?))),
        Place::Runway => Some(StartDef::Runway(at(runway?))),
        Place::Orbit => {
            let site = pad.or(runway)?;
            let inclination = site.latitude.abs();
            Some(StartDef::Orbit(OrbitStartDef {
                body: site.body.clone(),
                periapsis_altitude: ORBIT_ALTITUDE,
                apoapsis_altitude: ORBIT_ALTITUDE,
                inclination,
                longitude_of_ascending_node: 0.0,
                plane: None,
                argument_of_periapsis: 0.0,
                mean_anomaly: sun(&site.body, inclination),
                spin: [0.0; 3],
            }))
        }
    }
}

/// The craft on show.
struct Showing {
    craft: CraftDef,
    vessel: VesselId,
    /// The vessel that was being flown.
    flown: VesselId,
    /// The box round the craft as it stands, from the base of its stack, sim axes.
    low: DVec3,
    high: DVec3,
    /// The hangar, and the ambient light to go back to when it goes.
    hangar: Entity,
    ambient: f32,
    /// What it will weigh by, m/s²: gravity on the ground where it would launch.
    gravity: f64,
    place: Place,
    notice: String,
    yaw: f64,
    pitch: f64,
    distance: f64,
    /// What the camera looks at, from the middle of the craft.
    pan: DVec3,
    /// The left button went down off the UI, so it drags the view.
    dragging: bool,
}

impl Showing {
    fn size(&self) -> f64 {
        (self.high - self.low).max_element()
    }

    fn radius(&self) -> f64 {
        (self.size() * HANGAR_RADIUS).max(HANGAR_LEAST_RADIUS)
    }

    fn height(&self) -> f64 {
        (self.size() * HANGAR_HEIGHT).max(HANGAR_LEAST_HEIGHT)
    }

    /// The middle of the floor under the craft, from the base of its stack.
    fn floor(&self) -> DVec3 {
        let middle = (self.low + self.high) / 2.0;
        DVec3::new(middle.x, middle.y, self.low.z)
    }
}

/// What the player is doing to the view this frame.
#[derive(Debug, Default)]
struct ViewInput {
    /// Mouse movement, px: looking around, and dragging the view along.
    turn: DVec2,
    drag: DVec2,
    /// Lines of the wheel, away from the player.
    zoom: f64,
    /// Forward, right and up, each -1..1.
    walk: DVec3,
    /// How long `walk` lasted, s.
    dt: f64,
}

impl Showing {
    /// Which way along the floor the view is from what it looks at.
    fn away(&self) -> DVec3 {
        DVec3::new(self.yaw.cos(), self.yaw.sin(), 0.0)
    }

    /// Move the view as the player asks, keeping it in the hangar.
    fn steer(&mut self, input: &ViewInput) {
        self.yaw += input.turn.x * 0.005;
        self.pitch = (self.pitch + input.turn.y * 0.005).clamp(-MAX_PITCH, MAX_PITCH);
        self.distance *= 0.88f64.powf(input.zoom);

        let away = self.away();
        let right = DVec3::new(-away.y, away.x, 0.0);
        // Dragged, what is on the screen stays under the pointer.
        let per_pixel = self.distance * 0.0012;
        self.pan += (DVec3::Z * input.drag.y + right * input.drag.x) * per_pixel;
        let step = self.distance.max(10.0) * 0.6 * input.dt;
        self.pan += (-away * input.walk.x + right * input.walk.y + DVec3::Z * input.walk.z) * step;

        let radius = self.radius();
        let middle = (self.low + self.high) / 2.0;
        let floor = self.floor();
        let reach = (radius * 0.6).max(self.size());
        let pan = self.pan.truncate().clamp_length_max(reach);
        let height = (middle.z + self.pan.z)
            .clamp(floor.z + OVER_FLOOR, floor.z + self.height() * ROOF_REACH);
        self.pan = pan.extend(height - middle.z);
        let walls = radius * WALL_REACH - pan.length();
        self.distance = self.distance.clamp(NEAREST, walls.max(NEAREST));
    }

    /// Where the camera is and what it looks at, sim axes, given where the base of the stack is.
    fn view(&self, base: DVec3) -> (DVec3, DVec3) {
        let focus = base + (self.low + self.high) / 2.0 + self.pan;
        let around = self.away() * self.pitch.cos() + DVec3::Z * self.pitch.sin();
        let mut eye = focus + around * self.distance;
        eye.z = eye.z.max(base.z + self.floor().z + OVER_FLOOR);
        (eye, focus)
    }
}

/// Whether the editor is open, and with what.
#[derive(Resource, Default)]
pub struct Editor(Option<Showing>);

impl Editor {
    pub fn is_open(&self) -> bool {
        self.0.is_some()
    }

    /// What the craft on show will weigh by, m/s², if there is one: its figures are for the launch,
    /// not for where it stands on show.
    pub fn gravity(&self) -> Option<f64> {
        self.0.as_ref().map(|showing| showing.gravity)
    }
}

/// Gravity and the air's pressure on the ground at the site launches go from by default, m/s² and
/// Pa: what the craft's figures are for.
fn launch_ground(sim: &Sim, all: &[LaunchSiteDef]) -> (f64, f64) {
    let (pad, runway) = sites(all);
    let system = &sim.universe.system;
    let body = pad.or(runway).and_then(|site| system.find(&site.body));
    body.map_or((STANDARD_GRAVITY, 0.0), |id| {
        let body = system.body(id);
        let air = body
            .atmosphere
            .as_ref()
            .map_or(0.0, |air| air.sample(0.0).pressure);
        (body.gm / (body.radius * body.radius), air)
    })
}

/// The box round `vessel`'s parts as it stands, from the base of its stack, sim axes.
fn extent(sim: &mut Sim, index: usize) -> (DVec3, DVec3) {
    let universe = &mut sim.universe;
    let vessel = &universe.vessels[index];
    let tree = vessel.propulsion.parts();
    let attitude = vessel.attitude_at(&universe.system, universe.time());
    let base = DVec3::Z * tree.base();
    let Some(bounds) = universe.colliders.of(tree).bounds() else {
        return (DVec3::splat(-1.0), DVec3::splat(1.0));
    };
    let (mut low, mut high) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
    for corner in 0..8 {
        let at = DVec3::new(
            if corner & 1 == 0 {
                bounds.min.x
            } else {
                bounds.max.x
            },
            if corner & 2 == 0 {
                bounds.min.y
            } else {
                bounds.max.y
            },
            if corner & 4 == 0 {
                bounds.min.z
            } else {
                bounds.max.z
            },
        );
        let at = attitude * (at - base);
        low = low.min(at);
        high = high.max(at);
    }
    (low, high)
}

/// The floor and walls of the hall around the craft on show.
#[derive(Component)]
struct Hangar;

fn spawn_hangar(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    radius: f32,
    height: f32,
) -> Entity {
    let floor = materials.add(StandardMaterial {
        base_color_texture: Some(images.add(floor_image())),
        uv_transform: Affine2::from_scale(Vec2::splat(2.0 * radius / GRID_TILE)),
        // Matte: polished, it throws the key light back at the first view in a white streak.
        perceptual_roughness: 0.9,
        reflectance: 0.2,
        ..default()
    });
    let walls = materials.add(StandardMaterial {
        unlit: true,
        cull_mode: None,
        ..default()
    });
    commands
        .spawn((Hangar, Transform::default(), Visibility::default()))
        .with_children(|hangar| {
            hangar.spawn((
                Mesh3d(meshes.add(Circle::new(radius).mesh().resolution(96))),
                MeshMaterial3d(floor),
                Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                NotShadowCaster,
            ));
            hangar.spawn((
                Mesh3d(meshes.add(walls_mesh(radius, height))),
                MeshMaterial3d(walls),
                NotShadowCaster,
            ));
            // The sun is the key light (see `light_hangar`); this lifts the side it does not reach.
            hangar.spawn((
                DirectionalLight {
                    illuminance: lux::RAW_SUNLIGHT * FILL_LIGHT,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::default().looking_to(Vec3::new(-0.7, -0.2, 0.6), Vec3::Y),
            ));
        })
        .id()
}

/// One tile of the floor: lines every [`GRID_STEP`], heavier at its edge, with a full chain of mip
/// levels, so that the lines fade with distance instead of breaking up. Averaged in linear light.
fn floor_image() -> Image {
    let line = |at: f32, step: f32, width: f32| {
        let off = at.rem_euclid(step);
        off.min(step - off) < width / 2.0
    };
    let mut size = FLOOR_TEXELS;
    let mut level: Vec<[f32; 3]> = (0..size * size)
        .map(|index| {
            let per_texel = GRID_TILE / size as f32;
            let x = ((index % size) as f32 + 0.5) * per_texel;
            let y = ((index / size) as f32 + 0.5) * per_texel;
            if line(x, GRID_TILE, GRID_WIDTH * 2.5) || line(y, GRID_TILE, GRID_WIDTH * 2.5) {
                FLOOR_MAJOR_LINE
            } else if line(x, GRID_STEP, GRID_WIDTH) || line(y, GRID_STEP, GRID_WIDTH) {
                FLOOR_LINE
            } else {
                FLOOR_COLOR
            }
        })
        .collect();
    let mut data = Vec::new();
    let mut levels = 0;
    loop {
        levels += 1;
        for &[r, g, b] in &level {
            data.extend(Srgba::from(LinearRgba::rgb(r, g, b)).to_u8_array());
        }
        if size == 1 {
            break;
        }
        let half = size / 2;
        level = (0..half * half)
            .map(|index| {
                let (x, y) = (index % half * 2, index / half * 2);
                let corners = [(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)];
                let mut sum = [0.0; 3];
                for (cx, cy) in corners {
                    let texel = level[cy * size + cx];
                    (0..3).for_each(|c| sum[c] += texel[c] / 4.0);
                }
                sum
            })
            .collect();
        size = half;
    }
    let mut image = Image::default();
    image.texture_descriptor.size = Extent3d {
        width: FLOOR_TEXELS as u32,
        height: FLOOR_TEXELS as u32,
        depth_or_array_layers: 1,
    };
    image.texture_descriptor.format = TextureFormat::Rgba8UnormSrgb;
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    });
    image
}

/// A round hall: walls curving into a roof, in a colour that darkens going up.
fn walls_mesh(radius: f32, height: f32) -> Mesh {
    const AROUND: u32 = 64;
    const UP: u32 = 16;
    let (mut positions, mut normals, mut colors, mut uvs) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for ring in 0..=UP {
        let share = ring as f32 / UP as f32;
        let angle = share * std::f32::consts::FRAC_PI_2;
        let (r, y) = (radius * angle.cos(), height * angle.sin());
        let shade = (1.0 - share).powi(2);
        let color = [
            0.012 + 0.05 * shade,
            0.018 + 0.07 * shade,
            0.026 + 0.085 * shade,
            1.0,
        ];
        for step in 0..=AROUND {
            let turn = step as f32 / AROUND as f32 * std::f32::consts::TAU;
            let (sin, cos) = turn.sin_cos();
            positions.push([r * cos, y, r * sin]);
            normals.push([-angle.cos() * cos, -angle.sin(), -angle.cos() * sin]);
            colors.push(color);
            uvs.push([0.0, 0.0]);
        }
    }
    let row = AROUND + 1;
    let mut indices = Vec::new();
    for ring in 0..UP {
        for step in 0..AROUND {
            let (a, b) = (ring * row + step, (ring + 1) * row + step);
            indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

#[allow(clippy::too_many_arguments)]
fn editor_actions(
    mut commands: Commands,
    mut actions: MessageReader<UiAction>,
    mut editor: ResMut<Editor>,
    mut menu: ResMut<Menu>,
    mut sim: ResMut<Sim>,
    mut warp: ResMut<TimeWarp>,
    mut rig: ResMut<CameraRig>,
    mut ui: ResMut<LayoutVisibility>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut saves: ResMut<Saves>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut restarted: MessageWriter<SimRestarted>,
    content: Res<GameContent>,
) {
    for action in actions.read() {
        match action.name() {
            "editor.open" if editor.0.is_none() => {
                let Some(id) = action.arg() else { continue };
                let registry = &content.0.registry;
                let craft = registry.get::<CraftDef>("craft", id);
                let catalog = catalog(registry, &mut Vec::new());
                let time = sim.universe.time();
                let vessel = craft.map_err(|e| e.to_string()).and_then(|craft| {
                    let system = &sim.universe.system;
                    let vessel = Vessel::on_display(&craft, &catalog, system, time);
                    Ok((craft, vessel.map_err(|e| e.to_string())?))
                });
                let (craft, vessel) = match vessel {
                    Ok(built) => built,
                    Err(e) => {
                        error!("cannot show `{id}`: {e}");
                        menu.notice = format!("Cannot show it: {e}");
                        continue;
                    }
                };
                let flown = sim.active_id();
                let shown = sim.universe.add_vessel(vessel);
                sim.universe.focus = Some(shown);
                let index = sim.active_index();
                let (low, high) = extent(&mut sim, index);
                let all = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
                let (gravity, pressure) = launch_ground(&sim, &all);
                sim.active_vessel_mut().flight.static_pressure = pressure;
                let mut showing = Showing {
                    craft,
                    vessel: shown,
                    flown,
                    low,
                    high,
                    hangar: Entity::PLACEHOLDER,
                    ambient: ambient.brightness,
                    gravity,
                    place: Place::Pad,
                    notice: String::new(),
                    yaw: std::f64::consts::PI * 1.25,
                    pitch: 0.12,
                    distance: 0.0,
                    pan: DVec3::ZERO,
                    dragging: false,
                };
                showing.distance = showing.size() * 1.15 + 6.0;
                showing.hangar = spawn_hangar(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    &mut images,
                    showing.radius() as f32,
                    showing.height() as f32,
                );
                editor.0 = Some(showing);
                ambient.brightness = HANGAR_AMBIENT;
                rig.held = true;
                rig.map_view = false;
                rig.follow = None;
                menu.show(false, &mut sim, &mut ui);
                sim.paused = true;
                ui.hidden_groups.insert("flight".into());
                ui.kept.extend(KEPT_LAYOUTS.map(str::to_owned));
                ui.layouts.insert(LAYOUT.into(), true);
                ui.layouts.insert("staging".into(), true);
            }
            "editor.place" => {
                let place = action.arg().and_then(Place::parse);
                if let (Some(showing), Some(place)) = (&mut editor.0, place) {
                    showing.place = place;
                    showing.notice.clear();
                }
            }
            // Esc in the editor goes back to the list of craft, as the Back button does.
            "editor.back" | "game.menu" => {
                let Some(showing) = editor.0.take() else {
                    continue;
                };
                sim.universe.focus = Some(showing.flown);
                sim.universe.remove_vessel(showing.vessel);
                close(&showing, &mut commands, &mut rig, &mut ui, &mut ambient);
                menu.play = Play::Craft;
                menu.show(true, &mut sim, &mut ui);
            }
            "editor.launch" => {
                let Some(showing) = editor.0.as_mut() else {
                    continue;
                };
                let registry = &content.0.registry;
                let all = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
                let new_game = menu.new_game.then(|| SimTime::from_seconds(NEW_GAME_TIME));
                let time = new_game.unwrap_or(sim.universe.time());
                let sun = |body: &str, inclination: f64| {
                    let system = &sim.universe.system;
                    Vessel::sunlit_anomaly(system, body, inclination, time).unwrap_or(0.0)
                };
                let Some(start) = start_of(showing.place, &all, sun) else {
                    showing.notice = "No launch site has one".into();
                    continue;
                };
                let vessel = sim.universe.vessel(showing.vessel);
                let sequences = vessel.map(|v| v.propulsion.sequences.clone());
                let catalog = catalog(registry, &mut Vec::new());
                let universe = &mut sim.universe;
                let launched =
                    universe.launch(&all, &catalog, &showing.craft, &start, sequences, new_game);
                let id = match launched {
                    Ok(id) => id,
                    Err(e) => {
                        error!("cannot launch `{}`: {e}", showing.craft.id);
                        showing.notice = format!("Cannot launch: {e}");
                        continue;
                    }
                };
                universe.remove_vessel(showing.vessel);
                universe.focus = Some(id);
                let Some(showing) = editor.0.take() else {
                    continue;
                };
                close(&showing, &mut commands, &mut rig, &mut ui, &mut ambient);
                info!(
                    "launched `{}` from {}",
                    showing.craft.id,
                    showing.place.id()
                );
                rig.frame(showing.size());
                sim.restarted();
                *warp = TimeWarp::default();
                restarted.write(SimRestarted);
                if menu.new_game {
                    saves.forget_current();
                }
                menu.new_game = false;
                menu.playing = true;
                menu.play = Play::Start;
                menu.show(false, &mut sim, &mut ui);
                // The game is in the list of saves from its first launch on.
                saves.save(&sim);
            }
            _ => {}
        }
    }
}

/// What the editor changed for the time it was open goes back.
fn close(
    showing: &Showing,
    commands: &mut Commands,
    rig: &mut CameraRig,
    ui: &mut LayoutVisibility,
    ambient: &mut GlobalAmbientLight,
) {
    commands.entity(showing.hangar).despawn();
    ambient.brightness = showing.ambient;
    rig.held = false;
    ui.kept.clear();
    ui.layouts.insert(LAYOUT.into(), false);
}

/// Look around the craft and move about, as the flight camera does around a vessel.
#[allow(clippy::too_many_arguments)]
fn drive_camera(
    mut editor: ResMut<Editor>,
    mut rig: ResMut<CameraRig>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    locked: Res<InputLocked>,
    over_ui: Res<PointerOverUi>,
    mut camera: Single<&mut Transform, With<MainCamera>>,
) {
    let Some(showing) = editor.0.as_mut() else {
        return;
    };
    let Some(vessel) = sim.universe.vessel(showing.vessel) else {
        return;
    };
    if buttons.just_pressed(MouseButton::Left) {
        showing.dragging = !over_ui.0;
    }
    if !buttons.pressed(MouseButton::Left) {
        showing.dragging = false;
    }
    let mut input = ViewInput::default();
    if !locked.0 {
        if buttons.pressed(MouseButton::Right) || showing.dragging {
            input.turn = motion.delta.as_dvec2();
        }
        if buttons.pressed(MouseButton::Middle) {
            input.drag = motion.delta.as_dvec2();
        }
        if !over_ui.0 {
            input.zoom = match scroll.unit {
                MouseScrollUnit::Line => scroll.delta.y as f64,
                MouseScrollUnit::Pixel => scroll.delta.y as f64 / 40.0,
            };
        }
        let axis = |plus: KeyCode, minus: KeyCode| {
            f64::from(keys.pressed(plus)) - f64::from(keys.pressed(minus))
        };
        input.walk = DVec3::new(
            axis(KeyCode::KeyW, KeyCode::KeyS),
            axis(KeyCode::KeyD, KeyCode::KeyA),
            axis(KeyCode::KeyE, KeyCode::KeyQ),
        );
        input.dt = time.delta_secs_f64();
    }
    showing.steer(&input);
    let base = frame.relative(
        vessel.parent(),
        vessel.base_at(&sim.universe.system, frame.time),
    );
    let (eye, focus) = showing.view(base);
    rig.position = eye;
    **camera = Transform::from_translation(to_render(eye)).looking_at(to_render(focus), Vec3::Y);
}

/// Keep the floor under the craft.
fn place_hangar(
    editor: Res<Editor>,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    mut hangars: Query<&mut Transform, With<Hangar>>,
) {
    let Some(showing) = &editor.0 else { return };
    let Some(vessel) = sim.universe.vessel(showing.vessel) else {
        return;
    };
    let base = frame.relative(
        vessel.parent(),
        vessel.base_at(&sim.universe.system, frame.time),
    );
    for mut transform in &mut hangars {
        transform.translation = to_render(base + showing.floor());
    }
}

/// The sun comes down from one side of the hangar, wherever the real one is, with the eye set for
/// daylight.
fn light_hangar(
    editor: Res<Editor>,
    mut sun: Single<&mut Transform, With<Sunlight>>,
    mut exposure: Single<&mut Exposure, With<MainCamera>>,
) {
    if editor.0.is_some() {
        **sun = Transform::default().looking_to(Vec3::new(0.35, -0.8, -0.6), Vec3::Y);
        exposure.ev100 = Exposure::SUNLIGHT.ev100;
    }
}

/// `editor.*`, see `docs/modding.md`.
fn publish_editor(
    editor: Res<Editor>,
    sim: Res<Sim>,
    content: Res<GameContent>,
    mut published: Local<Option<(VesselId, Place, String)>>,
    mut data: ResMut<UiData>,
) {
    let Some(showing) = &editor.0 else {
        *published = None;
        return;
    };
    let now = (showing.vessel, showing.place, showing.notice.clone());
    if published.as_ref() == Some(&now) {
        return;
    }
    *published = Some(now);
    if let Some(vessel) = sim.universe.vessel(showing.vessel) {
        let propulsion = &vessel.propulsion;
        data.set("editor.name", vessel.name.as_str());
        data.set("editor.mass", propulsion.mass());
        data.set("editor.height", propulsion.length());
        data.set("editor.parts", propulsion.parts().parts().len());
    }
    data.set("editor.place", showing.place.id());
    data.set("editor.notice", showing.notice.as_str());
    let all = content
        .0
        .registry
        .all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
    let (pad, runway) = sites(&all);
    data.set("editor.pad", pad.map_or("", LaunchSiteDef::display_name));
    data.set(
        "editor.runway",
        runway.map_or("", LaunchSiteDef::display_name),
    );
    let body = pad.or(runway).map_or("", |site| site.body.as_str());
    data.set(
        "editor.orbit",
        format!("{:.0} km around {body}", ORBIT_ALTITUDE / 1e3),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(id: &str, pad: bool, runway: bool) -> LaunchSiteDef {
        let pad = pad.then(|| serde_json::json!({ "size": [10, 10] }));
        let runway = runway.then(|| serde_json::json!({ "size": [100, 10] }));
        serde_json::from_value(serde_json::json!({
            "id": id, "body": "Earth", "latitude": -28.0, "longitude": 0.0,
            "pad": pad, "runway": runway,
        }))
        .unwrap()
    }

    #[test]
    fn a_launch_goes_from_the_first_site_that_has_what_it_needs() {
        let all = [
            site("strip", false, true),
            site("a", true, false),
            site("b", true, true),
        ];
        let sun = |_: &str, _: f64| 40.0;
        let on = |place| match start_of(place, &all, sun) {
            Some(StartDef::LaunchSite(s)) => format!("pad {}", s.site),
            Some(StartDef::Runway(s)) => format!("runway {}", s.site),
            Some(StartDef::Orbit(o)) => {
                format!(
                    "orbit {} at {} from {}",
                    o.body, o.inclination, o.mean_anomaly
                )
            }
            _ => "nothing".to_owned(),
        };
        assert_eq!(on(Place::Pad), "pad a");
        assert_eq!(on(Place::Runway), "runway strip");
        assert_eq!(on(Place::Orbit), "orbit Earth at 28 from 40");
        assert!(start_of(Place::Runway, &[site("a", true, false)], sun).is_none());
        // Without a pad, an orbit goes round the body of the runway.
        assert!(start_of(Place::Orbit, &[site("strip", false, true)], sun).is_some());
    }

    #[test]
    fn places_read_back_as_they_are_written() {
        for place in [Place::Pad, Place::Runway, Place::Orbit] {
            assert_eq!(Place::parse(place.id()), Some(place));
        }
        assert_eq!(Place::parse("moon"), None);
    }

    /// The points of a mesh's positions.
    fn points(mesh: &Mesh) -> Vec<Vec3> {
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap();
        positions
            .as_float3()
            .unwrap()
            .iter()
            .map(|p| Vec3::from_array(*p))
            .collect()
    }

    #[test]
    fn the_hall_rises_from_the_edge_of_the_floor_to_its_roof() {
        let walls = points(&walls_mesh(100.0, 50.0));
        let top = walls.iter().map(|p| p.y).fold(0.0, f32::max);
        assert!((top - 50.0).abs() < 1e-3);
        let widest = walls.iter().map(|p| p.xz().length()).fold(0.0, f32::max);
        assert!((widest - 100.0).abs() < 1e-3);
        assert!(walls.iter().all(|p| p.y >= 0.0));
    }

    #[test]
    fn the_floor_has_every_mip_level_and_its_lines_show_up_close_and_fade_far_away() {
        let image = floor_image();
        let levels = image.texture_descriptor.mip_level_count as usize;
        assert_eq!(1 << (levels - 1), FLOOR_TEXELS);
        let data = image.data.as_ref().unwrap();
        let texels: usize = (0..levels)
            .map(|level| (FLOOR_TEXELS >> level).pow(2))
            .sum();
        assert_eq!(data.len(), texels * 4);
        // At the top a texel is on a line or it is not; the last level is the average of them all.
        let texel = |offset: usize| data[offset * 4];
        let on_line = texel(0);
        let between = texel(FLOOR_TEXELS * (FLOOR_TEXELS / 10) + FLOOR_TEXELS / 10);
        let average = texel(texels - 1);
        assert!(on_line > between);
        assert!(
            between < average && average < on_line,
            "{between} {average} {on_line}"
        );
    }

    /// A craft 100 m tall on show.
    fn showing() -> Showing {
        Showing {
            craft: CraftDef {
                id: "test".into(),
                name: String::new(),
                parts: Vec::new(),
                sequences: None,
                triggers: Vec::new(),
            },
            vessel: VesselId(0),
            flown: VesselId(0),
            low: DVec3::new(-5.0, -5.0, -2.0),
            high: DVec3::new(5.0, 5.0, 98.0),
            hangar: Entity::PLACEHOLDER,
            ambient: 0.0,
            gravity: STANDARD_GRAVITY,
            place: Place::Pad,
            notice: String::new(),
            yaw: 0.0,
            pitch: 0.1,
            distance: 120.0,
            pan: DVec3::ZERO,
            dragging: false,
        }
    }

    #[test]
    fn looking_around_keeps_the_view_on_what_it_looks_at() {
        let mut view = showing();
        let base = DVec3::new(10.0, -20.0, 30.0);
        let (_, focus) = view.view(base);
        let turn = DVec2::new(300.0, 80.0);
        view.steer(&ViewInput { turn, ..default() });
        let (eye, turned) = view.view(base);
        assert_eq!(focus, turned);
        assert!((eye.distance(focus) - 120.0).abs() < 1e-6);
    }

    #[test]
    fn dragging_and_walking_move_the_view_and_the_wheel_brings_it_closer() {
        let mut view = showing();
        let base = DVec3::ZERO;
        let (_, focus) = view.view(base);
        view.steer(&ViewInput {
            drag: DVec2::new(0.0, 100.0),
            ..default()
        });
        assert!(
            view.view(base).1.z > focus.z,
            "dragging down looks higher up"
        );

        // Facing the craft from +X, forward is towards -X and right is towards +Y.
        let mut view = showing();
        let walk = DVec3::new(1.0, 1.0, 0.0);
        view.steer(&ViewInput {
            walk,
            dt: 0.1,
            ..default()
        });
        let moved = view.view(base).1 - focus;
        assert!(moved.x < 0.0 && moved.y > 0.0, "{moved}");

        let mut view = showing();
        view.steer(&ViewInput {
            zoom: 3.0,
            ..default()
        });
        assert!(view.distance < 120.0);
        view.steer(&ViewInput {
            zoom: -3.0,
            ..default()
        });
        assert!((view.distance - 120.0).abs() < 1e-6);
    }

    #[test]
    fn however_far_it_goes_the_view_stays_inside_the_hangar() {
        let mut view = showing();
        let base = DVec3::ZERO;
        let hard = [
            ViewInput {
                zoom: -500.0,
                ..default()
            },
            ViewInput {
                zoom: 500.0,
                ..default()
            },
            ViewInput {
                drag: DVec2::splat(1e6),
                ..default()
            },
            ViewInput {
                drag: DVec2::splat(-1e6),
                ..default()
            },
            ViewInput {
                turn: DVec2::new(0.0, 1e5),
                ..default()
            },
            ViewInput {
                turn: DVec2::new(1e4, -1e5),
                ..default()
            },
            ViewInput {
                walk: DVec3::new(1.0, 1.0, 1.0),
                dt: 1e4,
                ..default()
            },
            ViewInput {
                walk: DVec3::new(-1.0, -1.0, -1.0),
                dt: 1e4,
                ..default()
            },
        ];
        let floor = view.floor().z;
        let (radius, height) = (view.radius(), view.height());
        for input in &hard {
            view.steer(input);
            let (eye, focus) = view.view(base);
            assert!(
                eye.z >= floor + OVER_FLOOR - 1e-9,
                "under the floor: {input:?}"
            );
            assert!(focus.z <= floor + height, "through the roof: {input:?}");
            let out = eye.truncate().distance(view.floor().truncate());
            assert!(
                out <= radius,
                "through the wall at {out} of {radius}: {input:?}"
            );
            assert!(view.distance >= NEAREST && view.pitch.abs() <= MAX_PITCH);
        }
    }
}
