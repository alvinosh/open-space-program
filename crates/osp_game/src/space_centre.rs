//! The space centre: where a game is run from, a view of the Cape with its buildings
//! (`space_centres` records, `osp_sim::space_centre`). A new game opens here, and so do Continue and
//! Load (a quicksave, a moment in flight, goes back to flying); the pause menu comes here with Space centre (`centre.open`), and leaving the editor comes
//! back. Pointing at a building lights it and names it; a click on it, or its row in the panel
//! (`centre.enter:<id>`), goes in: the Vehicle Assembly to the editor for rockets on the pad, the
//! Hangar to the editor for planes on the runway (`editor.open:<pad|runway>`, which closes the view).
//! In a career, Research opens the tech tree over the view (`tree.open`), and Mission Control shows
//! the contracts in the panel in place of the buildings (`centre.inside`), Back or Esc going back to
//! them; in a sandbox both say they are only in a career. The Tracking Station is not built yet.
//!
//! The view circles the buildings slowly until the player moves it: a drag turns it, the wheel
//! zooms, all within bounds. What is drawn is the game's own world at the Cape, with time standing
//! still: the view looks from a vessel with no parts standing there, only on show, which saves
//! leave out (`Universe::put_on_show`). `centre.fly` goes back to flying the vessel the game was
//! on. Esc and Back open the pause menu over it in a game, and its Resume comes back here (a new game
//! still to start over it is dropped); with no game, before a new game's first launch, they go back
//! to the title screen, as nothing has started yet.

use bevy::{
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    math::DVec3,
    prelude::*,
};
use osp_sim::{
    body::BodyId,
    data::{BuildingDef, BuildingUse, SpaceCentreDef},
    space_centre::{self, Standing},
    terrain,
    vessel::{Vessel, VesselId},
};
use osp_ui::{
    CLICK_SLOP, LayoutVisibility, UiAction, UiData, UiDocuments, UiRecord, ViewOverlay, record,
};

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera, update_camera},
    career::CareerTree,
    content::GameContent,
    dialog::Dialog,
    editing::{EditorActions, EditorStatus},
    frame::{RenderFrame, to_render},
    menu::{Menu, menu_actions, whereabouts},
    orbit_lines::ViewPointer,
    sim::{Sim, WorldData},
};

/// The panel on the left.
const LAYOUT: &str = "space_centre";
/// Where the view first looks from, rad: from the south-south-west, the sun behind it after midday,
/// a little above the ground.
const START_YAW: f64 = -1.9;
const START_PITCH: f64 = 0.24;
/// How far the view first stands from what it looks at, m, and the nearest and furthest it goes.
const START_DISTANCE: f64 = 1_250.0;
const NEAREST: f64 = 220.0;
const FURTHEST: f64 = 6_000.0;
/// How high over the middle of the buildings the view looks, m.
const LOOK_HEIGHT: f64 = 40.0;
/// The lowest and steepest the view looks down, rad.
const LOWEST: f64 = 0.06;
const STEEPEST: f64 = 1.3;
/// The view stays this high over the ground, m.
const OVER_GROUND: f64 = 20.0;
/// After this long without the player moving it, s, the view circles by itself this fast, rad/s.
const IDLE: f64 = 4.0;
const CIRCLING: f64 = 0.025;
/// How far the view turns for a pixel of the mouse's travel, rad.
const TURN_PER_PIXEL: f64 = 0.005;
/// How much nearer a notch of the wheel brings the view.
const ZOOM_STEP: f64 = 0.88;
/// How wide the panel on the left is, px (`menu_rail` in the theme).
const PANEL_PX: f32 = 560.0;
/// The name over the building pointed at: how far above its roof, px.
const LABEL_RISE: f32 = 34.0;
const LABEL_FILL: Color = Color::srgba(0.043, 0.086, 0.125, 0.94);
const LABEL_TEXT: Color = Color::srgb(0.902, 0.941, 0.961);

pub struct SpaceCentrePlugin;

impl Plugin for SpaceCentrePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpaceCentre>()
            .add_systems(Startup, spawn_label)
            .add_systems(
                Update,
                (
                    // Before the editor's: Esc there comes back here, and must not go on to
                    // leave here too.
                    centre_actions
                        .in_set(FrameSet::Input)
                        .after(menu_actions)
                        .before(EditorActions),
                    keep_it.in_set(FrameSet::Input).after(centre_actions),
                    drive_view.in_set(FrameSet::Camera).after(update_camera),
                    point_at_buildings.in_set(FrameSet::Render),
                    publish_centre,
                    publish_tracking,
                ),
            );
    }
}

/// The space centre while it is open.
struct Showing {
    def: SpaceCentreDef,
    body: BodyId,
    /// Where each building stands, as `def.buildings` lists them, at its level in `levels`.
    standing: Vec<Standing>,
    levels: Vec<usize>,
    /// The middle of the layout on the ground, and its axes there: right and forward along the
    /// layout's heading, and up. Body-fixed.
    middle: DVec3,
    right: DVec3,
    forward: DVec3,
    up: DVec3,
    /// Which way the view looks from at what it looks at (round from the layout's right, and up),
    /// rad, and how far, m.
    yaw: f64,
    pitch: f64,
    distance: f64,
    /// What it looks at, from the middle: right, forward and up, m.
    look_at: DVec3,
    /// How long since the player last moved the view, s.
    idle: f64,
    /// The left button went down on the view, not on the panel: it turns the view, or clicks.
    pressed_at: Option<Vec2>,
    dragged: bool,
    /// The building under the pointer, by its index.
    pointed: Option<usize>,
    /// The building whose screen the panel shows in place of the list of buildings: Mission
    /// Control's contracts, or the Tracking Station's vessels with the map.
    inside: Option<BuildingUse>,
    /// The vessel the stand-in was put on show for, in focus while the view is of the Cape.
    stand_in: VesselId,
    /// The Tracking Station lists debris too.
    debris: bool,
    /// What just could not be done, or what a building is not yet.
    notice: String,
}

impl Showing {
    /// A point from the middle in the layout's axes (right, forward, up), body-fixed.
    fn fixed(&self, local: DVec3) -> DVec3 {
        self.middle + self.right * local.x + self.forward * local.y + self.up * local.z
    }

    /// Where the view is and what it looks at, from the middle in the layout's axes.
    fn eye_and_focus(&self) -> (DVec3, DVec3) {
        let around = DVec3::new(
            self.yaw.cos() * self.pitch.cos(),
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
        );
        let mut eye = self.look_at + around * self.distance;
        eye.z = eye.z.max(OVER_GROUND);
        (eye, self.look_at)
    }
}

/// The space centre, open or not.
#[derive(Resource, Default)]
pub struct SpaceCentre {
    showing: Option<Showing>,
    /// Where each building is on the screen while it is open, by id: the middle of its front, for
    /// playtests to aim at as a player would.
    spots: Vec<(String, Vec2)>,
}

impl SpaceCentre {
    pub fn is_open(&self) -> bool {
        self.showing.is_some()
    }

    /// The id of the building under the pointer.
    pub fn pointed(&self) -> Option<&str> {
        let showing = self.showing.as_ref()?;
        let index = showing.pointed?;
        Some(showing.def.buildings.get(index)?.id.as_str())
    }

    /// Where building `id` is on the screen, while the view is open.
    pub fn spot(&self, id: &str) -> Option<Vec2> {
        (self.spots.iter())
            .find(|(b, _)| b == id)
            .map(|(_, at)| *at)
    }

    /// Leave the building whose screen the panel shows, back to the list of buildings: whether
    /// there was one (Esc and Back do this first). From the Tracking Station the view is of the
    /// Cape again.
    pub fn leave_building(&mut self, sim: &mut Sim, rig: &mut CameraRig) -> bool {
        let Some(showing) = self.showing.as_mut() else {
            return false;
        };
        match showing.inside.take() {
            Some(BuildingUse::Tracking) => {
                sim.universe.focus = Some(showing.stand_in);
                rig.map_view = false;
                rig.follow = None;
                rig.held = true;
                true
            }
            Some(_) => true,
            None => false,
        }
    }

    /// In the Tracking Station: the vessel it shows on the map, in focus while it does.
    fn track(&mut self, sim: &mut Sim, rig: &mut CameraRig, id: VesselId) {
        if sim.universe.vessel(id).is_none() {
            return;
        }
        sim.universe.focus = Some(id);
        rig.follow = None;
        rig.map_view = true;
        rig.held = false;
        if let Some(showing) = &mut self.showing {
            showing.inside = Some(BuildingUse::Tracking);
        }
    }

    /// The vessel the Tracking Station shows, if it is open.
    fn tracked(&self, sim: &Sim) -> Option<VesselId> {
        let showing = self.showing.as_ref()?;
        (showing.inside == Some(BuildingUse::Tracking))
            .then_some(sim.universe.focus)
            .flatten()
            .filter(|&id| id != showing.stand_in)
    }

    fn is_tracking(&self) -> bool {
        (self.showing.as_ref()).is_some_and(|s| s.inside == Some(BuildingUse::Tracking))
    }

    /// Say `notice` in the panel.
    pub fn tell(&mut self, notice: impl Into<String>) {
        if let Some(showing) = &mut self.showing {
            showing.notice = notice.into();
        }
    }

    /// Open the view of the game's first space centre, from a vessel put on show there: the vessel
    /// in focus now is the one to go back to flying. If it was open already it opens afresh.
    pub fn open(
        &mut self,
        sim: &mut Sim,
        rig: &mut CameraRig,
        ui: &mut LayoutVisibility,
        content: &GameContent,
    ) -> Result<(), String> {
        let registry = &content.0.registry;
        let def = (registry.all::<SpaceCentreDef>("space_centres", &mut Vec::new()))
            .into_iter()
            .next()
            .ok_or("the game has no space centre")?;
        let system = &sim.universe.system;
        let body = (system.find(&def.body))
            .ok_or_else(|| format!("there is no `{}` for the space centre", def.body))?;
        let standing = space_centre::standing(&def, system.body(body), &[]);
        let middle = space_centre::middle(&def, system.body(body));
        let (up, right, forward) = terrain::site_axes(&space_centre::layout(&def));
        let time = sim.universe.time();
        let stand_in = Vessel::empty_on_ground(system, body, middle, time);
        let stand_in = sim.universe.put_on_show(stand_in);
        rig.held = true;
        rig.map_view = false;
        rig.follow = None;
        sim.paused = true;
        ui.hidden_groups.insert("flight".into());
        ui.layouts.insert(LAYOUT.into(), true);
        self.showing = Some(Showing {
            def,
            body,
            standing,
            levels: Vec::new(),
            middle,
            right,
            forward,
            up,
            yaw: START_YAW,
            pitch: START_PITCH,
            distance: START_DISTANCE,
            look_at: DVec3::Z * LOOK_HEIGHT,
            idle: 0.0,
            pressed_at: None,
            dragged: false,
            pointed: None,
            inside: None,
            stand_in,
            debris: false,
            notice: String::new(),
        });
        Ok(())
    }

    /// Close the view: the vessel on show goes, and the one behind it is in focus again unless
    /// another has been put in focus since. The camera is the flight camera's again; what shows
    /// next is up to what closed it.
    pub fn close(&mut self, sim: &mut Sim, rig: &mut CameraRig, ui: &mut LayoutVisibility) {
        if self.showing.take().is_none() {
            return;
        }
        sim.universe.take_off_show();
        rig.held = false;
        ui.layouts.insert(LAYOUT.into(), false);
        self.spots.clear();
    }

    /// The vessel to go back to flying from here, if the game has one.
    pub fn behind(&self, sim: &Sim) -> Option<VesselId> {
        let (_, behind) = sim.universe.on_show()?;
        behind.filter(|&id| sim.universe.vessel(id).is_some())
    }
}

/// What the panel offers for a building.
fn building_meta(opens: BuildingUse, career: bool) -> &'static str {
    match opens {
        BuildingUse::Rockets => "Build rockets for the pad",
        BuildingUse::Planes => "Build planes for the runway",
        BuildingUse::Research | BuildingUse::Missions if !career => "Only in a career",
        BuildingUse::Research => "Unlock parts with science",
        BuildingUse::Missions => "Contracts that pay",
        BuildingUse::Tracking => "Every vessel in flight",
    }
}

#[allow(clippy::too_many_arguments)]
fn centre_actions(
    mut commands: Commands,
    mut actions: MessageReader<UiAction>,
    mut centre: ResMut<SpaceCentre>,
    mut menu: ResMut<Menu>,
    mut sim: ResMut<Sim>,
    mut rig: ResMut<CameraRig>,
    mut ui: ResMut<LayoutVisibility>,
    content: Res<GameContent>,
) {
    for action in actions.read() {
        match action.name() {
            // A new game starts here (`menu` has taken it as new).
            "game.new" => {
                // Not back to the menu's root, which would drop the new game.
                menu.show(false, &mut sim, &mut ui);
                if let Err(e) = centre.open(&mut sim, &mut rig, &mut ui, &content) {
                    error!("cannot open the space centre: {e}");
                    menu.notice = format!("Cannot start: {e}");
                    menu.show(true, &mut sim, &mut ui);
                }
            }
            "centre.open" => {
                menu.back_to_root();
                menu.show(false, &mut sim, &mut ui);
                if centre.is_open() {
                    continue;
                }
                if let Err(e) = centre.open(&mut sim, &mut rig, &mut ui, &content) {
                    error!("cannot open the space centre: {e}");
                    menu.notice = format!("Cannot go there: {e}");
                    menu.show(true, &mut sim, &mut ui);
                }
            }
            "centre.enter" => {
                let Some(showing) = &mut centre.showing else {
                    continue;
                };
                let Some(id) = action.arg() else { continue };
                let Some(building) = showing.def.buildings.iter().find(|b| b.id == id) else {
                    warn!("no building `{id}` at {}", showing.def.display_name());
                    continue;
                };
                let career = career_game(&menu, &sim);
                showing.notice.clear();
                match building.opens {
                    // The editor opens and closes the view (`osp_editor::hangar`).
                    BuildingUse::Rockets => {
                        commands.write_message(UiAction("editor.open:pad".into()));
                    }
                    BuildingUse::Planes => {
                        commands.write_message(UiAction("editor.open:runway".into()));
                    }
                    // The tech tree, over the view (`tree_screen`); Back comes back here.
                    BuildingUse::Research if career => {
                        commands.write_message(UiAction("tree.open".into()));
                    }
                    // The contracts, in the panel in place of the buildings.
                    BuildingUse::Missions if career => showing.inside = Some(BuildingUse::Missions),
                    // The map, on the vessel the game was on or the first there is; or on the
                    // Cape with nothing in flight (before a new game's first launch, too).
                    BuildingUse::Tracking => {
                        showing.inside = Some(BuildingUse::Tracking);
                        rig.map_view = true;
                        rig.held = false;
                        let behind = sim.universe.on_show().and_then(|(_, b)| b);
                        let first = (sim.universe.in_game())
                            .find(|v| !v.is_debris())
                            .map(|v| v.id());
                        let playing = menu.playing && !menu.new_game;
                        if let Some(id) = behind.or(first).filter(|_| playing) {
                            centre.track(&mut sim, &mut rig, id);
                        }
                    }
                    opens => {
                        showing.notice = format!(
                            "{}: {}",
                            building.display_name(),
                            building_meta(opens, career).to_lowercase()
                        );
                    }
                }
            }
            "centre.fly" if centre.is_open() => {
                if centre.behind(&sim).is_none() || !menu.playing || menu.new_game {
                    centre.tell("There is nothing in flight to go back to");
                    continue;
                }
                centre.close(&mut sim, &mut rig, &mut ui);
                rig.frame(sim.active_vessel().propulsion.length());
                menu.back_to_root();
                menu.show(false, &mut sim, &mut ui);
            }
            // Flown from the menu's list of vessels (`targeting` flies it), or left for another game
            // or back to a launch from the menu over the view.
            // A vessel picked in the Tracking Station: the map on it.
            "centre.track" => {
                let id = action.arg().and_then(|id| id.parse().ok()).map(VesselId);
                if let Some(id) = id {
                    centre.track(&mut sim, &mut rig, id);
                }
            }
            "centre.debris" => {
                if let Some(showing) = &mut centre.showing {
                    showing.debris = !showing.debris;
                }
            }
            // Flown from the Tracking Station or the menu's list of vessels (`targeting` flies it):
            // flight, in its own view.
            "vessel.fly" if centre.is_open() => {
                centre.close(&mut sim, &mut rig, &mut ui);
                rig.map_view = false;
                menu.back_to_root();
                menu.show(false, &mut sim, &mut ui);
            }
            // Left for another game, or back to a launch, from the menu over the view.
            "game.exit" | "game.revert" | "game.start_tutorial" if centre.is_open() => {
                centre.close(&mut sim, &mut rig, &mut ui);
            }
            _ => {}
        }
    }
}

/// Which of `building`'s levels stands (an index into its `levels`): its facility's level in a
/// career, a new one's first, the top one in a sandbox, which has every level; the first without a
/// facility.
pub fn level_of(building: &BuildingDef, career: &CareerTree, menu: &Menu, sim: &Sim) -> usize {
    let top = building.levels.len().saturating_sub(1);
    let Some(id) = &building.facility else {
        return 0;
    };
    let Some(facility) = career.facilities.iter().find(|f| &f.id == id) else {
        return 0;
    };
    match career.current(menu, sim) {
        Some(playing) => playing.level(facility).min(top),
        None if menu.new_game && menu.career => 0,
        None => top,
    }
}

/// Whether the game is a career: one in progress, or a new one about to start.
fn career_game(menu: &Menu, sim: &Sim) -> bool {
    if menu.new_game {
        menu.career
    } else {
        sim.universe.career.is_some()
    }
}

/// While the view is open, time stands still and the flight UI stays hidden, the menu over it or
/// not; its panel shows when the menu does not.
fn keep_it(
    mut centre: ResMut<SpaceCentre>,
    menu: Res<Menu>,
    editor: Res<EditorStatus>,
    mut sim: ResMut<Sim>,
    mut rig: ResMut<CameraRig>,
    mut ui: ResMut<LayoutVisibility>,
) {
    if !centre.is_open() {
        return;
    }
    // The game it was the view of has gone (another loaded, the title screen's put in its place).
    if sim.universe.on_show().is_none() {
        centre.showing = None;
        centre.spots.clear();
        rig.held = editor.is_open();
        ui.layouts.insert(LAYOUT.into(), false);
        return;
    }
    if !sim.paused {
        sim.paused = true;
    }
    if !ui.hidden_groups.contains("flight") {
        ui.hidden_groups.insert("flight".into());
    }
    if ui.shown(LAYOUT) == menu.open {
        ui.layouts.insert(LAYOUT.into(), !menu.open);
    }
}

/// Move the view as the player asks, or circle slowly, and put the camera there.
#[allow(clippy::too_many_arguments)]
fn drive_view(
    mut centre: ResMut<SpaceCentre>,
    mut rig: ResMut<CameraRig>,
    menu: Res<Menu>,
    dialog: Res<Dialog>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    pointer: ViewPointer,
    time: Res<Time<Real>>,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    camera: Single<(&mut Transform, &Camera, &Projection), With<MainCamera>>,
) {
    let Some(showing) = centre.showing.as_mut() else {
        return;
    };
    let (mut transform, camera, projection) = camera.into_inner();
    // The Tracking Station's map has the flight camera's map view, aimed past the panel.
    if showing.inside == Some(BuildingUse::Tracking) {
        aim_past_panel(&mut transform, camera, projection);
        return;
    }
    let dt = time.delta_secs_f64();
    let free = !menu.open && !dialog.is_open();
    let over_view = pointer.position();
    let mut moved = false;
    if free {
        if buttons.just_pressed(MouseButton::Left) {
            showing.pressed_at = over_view;
            showing.dragged = false;
        }
        if let Some(from) = showing.pressed_at
            && buttons.pressed(MouseButton::Left)
            && pointer
                .anywhere()
                .is_some_and(|at| at.distance(from) > CLICK_SLOP)
        {
            showing.dragged = true;
        }
        let turning = (showing.pressed_at.is_some() && buttons.pressed(MouseButton::Left))
            || (over_view.is_some() && buttons.pressed(MouseButton::Right));
        if turning && motion.delta != Vec2::ZERO {
            let turn = motion.delta.as_dvec2() * TURN_PER_PIXEL;
            showing.yaw -= turn.x;
            showing.pitch = (showing.pitch + turn.y).clamp(LOWEST, STEEPEST);
            moved = true;
        }
        if over_view.is_some() && scroll.delta.y != 0.0 {
            let notches = match scroll.unit {
                MouseScrollUnit::Line => f64::from(scroll.delta.y),
                MouseScrollUnit::Pixel => f64::from(scroll.delta.y) / 40.0,
            };
            showing.distance =
                (showing.distance * ZOOM_STEP.powf(notches)).clamp(NEAREST, FURTHEST);
            moved = true;
        }
    }
    showing.idle = if moved { 0.0 } else { showing.idle + dt };
    if showing.idle > IDLE {
        showing.yaw += CIRCLING * dt;
    }
    showing.yaw = showing.yaw.rem_euclid(std::f64::consts::TAU);

    // Body-fixed, then as the body is turned now, from the render origin.
    let turned = sim
        .universe
        .system
        .body(showing.body)
        .orientation(frame.time);
    let (eye, focus) = showing.eye_and_focus();
    let at = |local: DVec3| frame.relative(showing.body, turned * showing.fixed(local));
    let (eye, focus) = (at(eye), at(focus));
    rig.position = eye;
    let up = to_render(turned * showing.up);
    *transform = Transform::from_translation(to_render(eye)).looking_at(to_render(focus), up);
    aim_past_panel(&mut transform, camera, projection);
}

/// Turn the view so that what it looks at is in the middle of what the panel leaves of the
/// screen, not behind the panel.
fn aim_past_panel(transform: &mut Transform, camera: &Camera, projection: &Projection) {
    if let (Projection::Perspective(lens), Some(size)) =
        (projection, camera.logical_viewport_size())
        && size.x > PANEL_PX
    {
        let across = (lens.fov / 2.0).tan() * size.x / size.y;
        let off = PANEL_PX / 2.0 / (size.x / 2.0);
        transform.rotate_local_y((off * across).atan());
    }
}

/// Find the building under the pointer; a click on it goes in. Put each building's spot on the
/// screen for the playtests, and the name of the one pointed at over it.
#[allow(clippy::too_many_arguments)]
fn point_at_buildings(
    mut commands: Commands,
    mut centre: ResMut<SpaceCentre>,
    menu: Res<Menu>,
    dialog: Res<Dialog>,
    buttons: Res<ButtonInput<MouseButton>>,
    pointer: ViewPointer,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    docs: Res<UiDocuments>,
    career: Res<CareerTree>,
    camera: Single<(&Camera, &Transform), With<MainCamera>>,
    mut label: Single<(&mut Text, &mut TextFont, &mut Node, &mut Visibility), With<BuildingLabel>>,
    mut font_set: Local<bool>,
) {
    let (text, font, node, visibility) = &mut *label;
    let centre = &mut *centre;
    let Some(showing) = centre.showing.as_mut() else {
        **visibility = Visibility::Hidden;
        return;
    };
    // On the Tracking Station's map there are no buildings to point at.
    if showing.inside == Some(BuildingUse::Tracking) {
        showing.pointed = None;
        showing.pressed_at = None;
        centre.spots.clear();
        **visibility = Visibility::Hidden;
        return;
    }
    // Each building as big as its level makes it: an upgrade makes it bigger at once.
    let levels: Vec<usize> = (showing.def.buildings.iter())
        .map(|building| level_of(building, &career, &menu, &sim))
        .collect();
    if levels != showing.levels {
        let body = sim.universe.system.body(showing.body);
        showing.standing = space_centre::standing(&showing.def, body, &levels);
        showing.levels = levels;
    }
    let (camera, transform) = *camera;
    let eye = GlobalTransform::from(*transform);
    let turned = sim
        .universe
        .system
        .body(showing.body)
        .orientation(frame.time);
    let on_screen = |fixed: DVec3| {
        let at = frame.relative(showing.body, turned * fixed);
        camera.world_to_viewport(&eye, to_render(at)).ok()
    };
    // Each building's front, half way up, for a playtest to point at as a player would.
    centre.spots = (showing.def.buildings.iter().zip(&showing.standing))
        .filter_map(|(building, standing)| {
            let front = standing.middle() + standing.forward * (standing.half.y * 0.5);
            Some((building.id.clone(), on_screen(front)?))
        })
        .collect();

    let free = !menu.open && !dialog.is_open();
    let ray = (pointer.position().filter(|_| free))
        .and_then(|cursor| camera.viewport_to_world(&eye, cursor).ok());
    showing.pointed = ray.and_then(|ray| {
        // The ray in the body's fixed axes, from the view.
        let way = DVec3::new(
            f64::from(ray.direction.x),
            -f64::from(ray.direction.z),
            f64::from(ray.direction.y),
        );
        let from = showing.fixed(showing.eye_and_focus().0);
        let way = turned.inverse() * way;
        (showing.standing.iter().enumerate())
            .filter_map(|(index, standing)| Some((index, standing.hit(from, way)?)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    });
    if free && buttons.just_released(MouseButton::Left) {
        let clicked = showing.pressed_at.is_some() && !showing.dragged;
        showing.pressed_at = None;
        if clicked && let Some(index) = showing.pointed {
            let id = &showing.def.buildings[index].id;
            commands.write_message(UiAction(format!("centre.enter:{id}")));
        }
    }

    // Its name over its roof.
    let shown = showing.pointed.and_then(|index| {
        let building = showing.def.buildings.get(index)?;
        let at = on_screen(showing.standing[index].top())?;
        Some((building.display_name().to_owned(), at))
    });
    match shown {
        Some((name, at)) => {
            if text.0 != name {
                text.0 = name;
            }
            if docs.is_changed() || !*font_set {
                **font = docs.text_font(15.0);
                *font_set = true;
            }
            node.left = Val::Px(at.x);
            node.top = Val::Px(at.y - LABEL_RISE);
            **visibility = Visibility::Inherited;
        }
        None => **visibility = Visibility::Hidden,
    }
}

/// The name shown over the building pointed at.
#[derive(Component)]
struct BuildingLabel;

fn spawn_label(mut commands: Commands) {
    commands.spawn((
        BuildingLabel,
        Text::new(""),
        TextColor(LABEL_TEXT),
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::axes(Val::Px(9.0), Val::Px(4.0)),
            border_radius: BorderRadius::all(Val::Px(3.0)),
            ..default()
        },
        // Centred over the point it names.
        UiTransform::from_translation(Val2::percent(-50.0, 0.0)),
        BackgroundColor(LABEL_FILL),
        Pickable::IGNORE,
        ViewOverlay,
        GlobalZIndex(-4),
        Visibility::Hidden,
    ));
}

/// `centre.*`, see `layouts/space_centre.json5`.
fn publish_centre(
    centre: Res<SpaceCentre>,
    menu: Res<Menu>,
    sim: Res<Sim>,
    mut data: ResMut<UiData>,
    mut published: Local<
        Option<(
            bool,
            Option<usize>,
            String,
            bool,
            Option<VesselId>,
            Option<BuildingUse>,
        )>,
    >,
) {
    let showing = centre.showing.as_ref();
    let behind = centre.behind(&sim);
    let career = career_game(&menu, &sim);
    let now = (
        showing.is_some(),
        showing.and_then(|s| s.pointed),
        showing.map_or(String::new(), |s| s.notice.clone()),
        career,
        behind.filter(|_| menu.playing && !menu.new_game),
        showing.and_then(|s| s.inside),
    );
    if published.as_ref() == Some(&now) {
        return;
    }
    *published = Some(now);
    data.set("centre.open", showing.is_some());
    let Some(showing) = showing else {
        return;
    };
    data.set("centre.name", showing.def.display_name());
    let buildings: Vec<UiRecord> = (showing.def.buildings.iter().enumerate())
        .map(|(index, building)| {
            record([
                ("id", building.id.as_str().into()),
                ("name", building.display_name().into()),
                ("meta", building_meta(building.opens, career).into()),
                ("pointed", (showing.pointed == Some(index)).into()),
            ])
        })
        .collect();
    data.set("centre.buildings", buildings);
    data.set(
        "centre.pointed",
        (showing.pointed)
            .and_then(|i| showing.def.buildings.get(i))
            .map_or("", |b| b.display_name()),
    );
    // Not before a new game's first launch: what is behind it then is not the game's.
    let flown =
        (behind.filter(|_| menu.playing && !menu.new_game)).and_then(|id| sim.universe.vessel(id));
    data.set("centre.can_fly", flown.is_some());
    data.set("centre.flown_name", flown.map_or("", |v| v.name.as_str()));
    data.set("centre.notice", showing.notice.as_str());
    let inside = match showing.inside {
        Some(BuildingUse::Missions) => "missions",
        Some(BuildingUse::Tracking) => "tracking",
        _ => "",
    };
    data.set("centre.inside", inside);
}

/// `centre.vessels` and what goes with them, for the Tracking Station: every vessel in the game
/// (debris with `centre.debris`), each `id`, `name`, `where`, `debris` and `tracked`; the one shown
/// on the map (`centre.tracked`, `centre.tracked_name`, `centre.has_tracked`), whether it can be
/// recovered (`centre.can_recover`), and whether debris is listed (`centre.debris`).
fn publish_tracking(
    centre: Res<SpaceCentre>,
    menu: Res<Menu>,
    sim: Res<Sim>,
    world: Res<WorldData>,
    mut data: ResMut<UiData>,
    mut published: Local<Option<(Vec<VesselId>, Option<VesselId>, bool)>>,
) {
    if !centre.is_tracking() {
        *published = None;
        return;
    }
    let debris = centre.showing.as_ref().is_some_and(|s| s.debris);
    let tracked = centre.tracked(&sim);
    // Before a new game's first launch, what is there is not the game's.
    let playing = menu.playing && !menu.new_game;
    let listed: Vec<&Vessel> = (sim.universe.in_game())
        .filter(|v| playing && (debris || !v.is_debris()))
        .collect();
    let now = (listed.iter().map(|v| v.id()).collect(), tracked, debris);
    if published.as_ref() == Some(&now) {
        return;
    }
    *published = Some(now);
    let rows: Vec<UiRecord> = (listed.iter())
        .map(|vessel| {
            record([
                ("id", vessel.id().0.into()),
                ("name", vessel.name.as_str().into()),
                ("where", whereabouts(vessel, &sim).into()),
                ("debris", vessel.is_debris().into()),
                ("tracked", (Some(vessel.id()) == tracked).into()),
            ])
        })
        .collect();
    data.set("centre.has_vessels", !rows.is_empty());
    data.set("centre.vessels", rows);
    data.set("centre.debris", debris);
    let shown = tracked.and_then(|id| sim.universe.vessel(id));
    data.set("centre.has_tracked", shown.is_some());
    data.set("centre.tracked", tracked.map_or(0, |id| id.0));
    data.set("centre.tracked_name", shown.map_or("", |v| v.name.as_str()));
    let recover = tracked.is_some_and(|id| sim.universe.recoverable(id, &world.sites));
    data.set("centre.can_recover", recover);
}
