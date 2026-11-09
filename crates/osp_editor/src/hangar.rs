//! The hangar: where a vessel is chosen, built and looked over before it flies. New game (`game.new`) and
//! Fly a new vessel (`editor.open`) come here with the hangar empty. The player picks a craft from
//! the list (`editor.show:<id>`), which then gets out of the way and comes back with `editor.list`.
//! They walk round it, pick another if they like, change its staging with the same window as in
//! flight, then launch it from the pad, the runway or orbit (`editor.launch`). A pad or runway with
//! a vessel on it takes no launch until that is cleared (`editor.clear`, asked once). Back or Esc
//! goes back to the menu with the game as it was. Building is in the crate's other modules.
//!
//! Nothing the player did to the craft on show is thrown away without asking: while it has changes
//! since it was put on show, putting another on show, New rocket or New plane, Back and Esc ask
//! first (they send `editor.ask_discard:<action>`, which asks the frame after, so the press that
//! asked does not also close the question). Discard sends `editor.discard:<action>`, which lets
//! that action through once.
//!
//! A launch is remembered (`osp_game::editing::LastLaunch`) until the game is left or another launch replaces it. The
//! pause menu can go back to it: `game.revert:launch` to the moment it left, `game.revert:hangar` to
//! here with the same craft, staging and place, as if it had never gone (`ask.revert:<to>` asks
//! first). Saves are not touched, so what was saved since can still be loaded.
//!
//! A game always has a vessel in it. With the last one destroyed, here or from the menu's Vessels
//! screen (`vessel.destroy:<id>`, asked once), the hangar is what is left: going back from it leaves
//! the game for the title screen, as there is nothing to go back to.
//!
//! What is on show is a real vessel (`Vessel::on_display`), the one the player flies for now, so
//! the vessel renderer draws it and the staging window edits it as they do in flight. It stands far
//! over the star's pole, with time stopped, inside a hangar drawn around it that hides everything
//! else in the game, and goes when the editor closes. An empty hangar has a vessel with no parts in
//! it, so the camera, the lights and the rest of the game always have one to go by. What launches
//! is a new vessel built from the same craft, with the sequences the player left it with.
//!
//! The camera here is its own: the flight camera lets go while the editor is open
//! ([`CameraRig::held`]). Right or left drag looks around, middle drag or WASD, Q and E moves,
//! the wheel zooms, all of it inside the hangar's dome.

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
use osp_mods::ModManifest;
use osp_sim::{
    builder::Builder,
    data::{CraftDef, LaunchSiteDef, LaunchSiteStartDef, OrbitStartDef, PartCatalog, StartDef},
    propulsion::Propulsion,
    time::SimTime,
    vessel::{Vessel, VesselId},
};
use osp_ui::{LayoutVisibility, PointerOverUi, UiAction, UiData, UiDocuments, UiRecord, record};

use osp_game::{
    FrameSet,
    camera::{CameraRig, MainCamera, update_camera},
    content::{GameContent, PlayerMods, catalog},
    controls::InputLocked,
    dialog::{Dialog, Question},
    editing::{EditorActions, EditorStatus, LastLaunch, LaunchPlace as Place, LaunchPoint},
    frame::{RenderFrame, Sunlight, to_render},
    menu::{Menu, go_to_title, menu_actions},
    saves::Saves,
    sim::{Sim, SimRestarted, TimeWarp},
};

use crate::{
    building::Placing,
    moving::Moving,
    shaping::{Shaping, craft_origin},
};

/// The layout with the launch button.
const LAYOUT: &str = "editor";
/// The id of a craft started from nothing.
const NEW_CRAFT: &str = "new_vessel";
/// What stays on the screen of what flight shows, as the editor hides the rest: the staging
/// window, its menu and a part's window (to split a part off).
const KEPT_LAYOUTS: [&str; 3] = ["staging", "staging_menu", "part_info"];
/// The staging window, the dock it goes in here, and a part's window.
const STAGING: &str = "staging";
const STAGING_DOCK: &str = "editor_staging";
const PART_WINDOW: &str = "part_info";
/// How far the view starts from the middle of an empty hangar, m.
const EMPTY_VIEW: f64 = 45.0;
/// How far over the floor the first part of a new craft floats, m, besides its own size (half of it
/// for a plane): room for what goes under it.
const FIRST_CLEARANCE: f64 = 2.0;
/// How far from the middle of the floor the craft can be carried, m, if it is not bigger.
const MOVE_ROOM: f64 = 20.0;
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
/// The hall is a dome: how far out in it the camera goes and what it looks at can be, as shares of
/// the way from the middle of the floor to the dome.
const WALL_REACH: f64 = 0.92;
const FOCUS_REACH: f64 = 0.85;
/// Which way the view first looks from at what is on show, rad.
const START_YAW: f64 = std::f64::consts::PI * 1.25;
const START_PITCH: f64 = 0.12;

pub struct HangarPlugin;

impl Plugin for HangarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Editor>().add_systems(
            Update,
            (
                // After the menu, which leaves Esc to the editor while the editor is open.
                editor_actions
                    .in_set(FrameSet::Input)
                    .in_set(EditorActions)
                    .after(menu_actions),
                revert_actions.in_set(FrameSet::Input).after(editor_actions),
                // As soon as it opens or closes, for the menu and flight to go by.
                tell_the_game.in_set(FrameSet::Input).after(revert_actions),
                drive_camera.in_set(FrameSet::Camera).after(update_camera),
                place_hangar.in_set(FrameSet::Render),
                refit_hangar.in_set(FrameSet::Render).before(place_hangar),
                // After what lights the world, which would take this for a night in deep space.
                light_hangar.after(FrameSet::Render),
                publish_editor,
            ),
        );
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

/// The editor while it is open, and what is on show.
struct Showing {
    /// The craft on show as the player has it, with what they did to it; none in an empty hangar.
    craft: Option<Builder>,
    /// The vessel standing in the hangar, with no parts in it when there is no craft.
    vessel: VesselId,
    /// The vessel to go back to flying; none once there is nothing left to fly.
    flown: Option<VesselId>,
    /// The vessel to go back to is not the one the player left: the camera finds it afresh.
    flown_changed: bool,
    /// The box round the craft as it stands, from its root, sim axes.
    low: DVec3,
    high: DVec3,
    /// Where the root is, from the middle of the floor: the craft stays where it is put, and is
    /// lifted only when a part would go through the floor.
    anchor: DVec3,
    /// The hangar, how big it was built (radius and height, m), and the ambient light to go back to
    /// when it goes.
    hangar: Entity,
    hangar_size: (f64, f64),
    ambient: f32,
    /// What it will weigh by, m/s²: gravity on the ground where it would launch.
    gravity: f64,
    place: Place,
    notice: String,
    yaw: f64,
    pitch: f64,
    distance: f64,
    /// What the camera looks at, from the middle of the floor.
    look_at: DVec3,
    /// The left button went down off the UI, so it drags the view.
    dragging: bool,
    /// Whether the staging window was open, to leave it so.
    staging_was_open: bool,
    /// The list of vessels is showing.
    list_open: bool,
    /// The action the player said may throw away the changes to the craft (`editor.discard`): it
    /// goes ahead once without asking.
    discard: Option<String>,
}

impl Showing {
    fn size(&self) -> f64 {
        (self.high - self.low).max_element()
    }

    /// The box round the craft, from the middle of the floor.
    fn bounds(&self) -> (DVec3, DVec3) {
        (self.anchor + self.low, self.anchor + self.high)
    }

    /// The middle of the craft, from the middle of the floor.
    fn middle(&self) -> DVec3 {
        self.anchor + (self.low + self.high) / 2.0
    }

    fn radius(&self) -> f64 {
        let (low, high) = self.bounds();
        let out = low.truncate().abs().max(high.truncate().abs()).length();
        (self.size() * HANGAR_RADIUS)
            .max(HANGAR_LEAST_RADIUS)
            .max(out * 1.5)
    }

    fn height(&self) -> f64 {
        (self.size() * HANGAR_HEIGHT)
            .max(HANGAR_LEAST_HEIGHT)
            .max(self.bounds().1.z * 1.3)
    }

    /// Keep the craft over the floor and near the middle of the hall.
    fn keep_in(&mut self) {
        let middle = (self.low + self.high) / 2.0;
        let room = self.size().max(MOVE_ROOM);
        let across = (self.anchor + middle).truncate().clamp_length_max(room);
        self.anchor.x = across.x - middle.x;
        self.anchor.y = across.y - middle.y;
        let top = room * 2.0 + HANGAR_LEAST_HEIGHT;
        self.anchor.z = self.anchor.z.min(top - self.high.z).max(-self.low.z);
    }

    /// Stand the craft in the middle of the floor, `clearance` metres over it.
    fn stand_at(&mut self, clearance: f64) {
        let middle = (self.low + self.high) / 2.0;
        self.anchor = DVec3::new(-middle.x, -middle.y, clearance - self.low.z);
        self.keep_in();
    }

    /// Look at the middle of the craft, from far enough to see all of it.
    fn frame(&mut self) {
        self.look_at = self.middle();
        self.distance = self.size() * 1.15 + 6.0;
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

    /// Which way the view is from what it looks at.
    fn around(&self) -> DVec3 {
        self.away() * self.pitch.cos() + DVec3::Z * self.pitch.sin()
    }

    /// What the view looks at, from the middle of the floor.
    fn focus(&self) -> DVec3 {
        self.look_at
    }

    /// `at`, from the middle of the floor, in the hall's proportions: the dome is the unit sphere.
    fn in_dome(&self, at: DVec3) -> DVec3 {
        DVec3::new(
            at.x / self.radius(),
            at.y / self.radius(),
            at.z / self.height(),
        )
    }

    /// How far the view can stand from what it looks at, the way it looks from now, and still be
    /// inside the dome: beyond it, all there is to see is the outside of the wall.
    fn reach(&self) -> f64 {
        let from = self.in_dome(self.focus());
        let way = self.in_dome(self.around());
        let (a, half_b) = (way.length_squared(), from.dot(way));
        let c = from.length_squared() - WALL_REACH * WALL_REACH;
        let root = half_b * half_b - a * c;
        if root <= 0.0 {
            return NEAREST;
        }
        ((root.sqrt() - half_b) / a).max(NEAREST)
    }

    /// Move the view as the player asks, keeping it in the hangar. `distance` is how far they asked
    /// for: turning towards the roof brings the view in, and turning back takes it out again.
    fn steer(&mut self, input: &ViewInput) {
        self.yaw += input.turn.x * 0.005;
        self.pitch = (self.pitch + input.turn.y * 0.005).clamp(-MAX_PITCH, MAX_PITCH);

        let away = self.away();
        let right = DVec3::new(-away.y, away.x, 0.0);
        // Dragged, what is on the screen stays under the pointer.
        let per_pixel = self.distance * 0.0012;
        self.look_at += (DVec3::Z * input.drag.y + right * input.drag.x) * per_pixel;
        let step = self.distance.max(10.0) * 0.6 * input.dt;
        self.look_at +=
            (-away * input.walk.x + right * input.walk.y + DVec3::Z * input.walk.z) * step;

        // What it looks at stays over the floor and well inside the dome.
        let (radius, height) = (self.radius(), self.height());
        let focus = self.focus();
        let up = focus.z.clamp(OVER_FLOOR, height * FOCUS_REACH);
        let share = up / height;
        let across = radius * (FOCUS_REACH * FOCUS_REACH - share * share).max(0.0).sqrt();
        self.look_at = focus.truncate().clamp_length_max(across).extend(up);

        if input.zoom != 0.0 {
            // From where it is, however far it was asked to be.
            let reach = self.reach();
            let zoomed = self.distance.min(reach) * 0.88f64.powf(input.zoom);
            self.distance = zoomed.clamp(NEAREST, reach);
        }
    }

    /// Where the camera is and what it looks at, sim axes, given where the middle of the floor is.
    fn view(&self, floor: DVec3) -> (DVec3, DVec3) {
        let focus = floor + self.look_at;
        let mut eye = focus + self.around() * self.distance.min(self.reach());
        eye.z = eye.z.max(floor.z + OVER_FLOOR);
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

    /// Turn and zoom the view as the mouse would, for a scripted capture.
    pub fn look(&mut self, turn: DVec2, zoom: f64) {
        if let Some(showing) = self.0.as_mut() {
            showing.steer(&ViewInput {
                turn,
                zoom,
                ..default()
            });
        }
    }

    /// Whether the list of vessels is showing.
    pub fn list_open(&self) -> bool {
        self.0.as_ref().is_some_and(|showing| showing.list_open)
    }

    /// The id of the craft on show while the editor is open, empty for an empty hangar.
    pub fn craft(&self) -> Option<&str> {
        let showing = self.0.as_ref()?;
        Some(
            showing
                .craft
                .as_ref()
                .map_or("", |craft| craft.draft().id.as_str()),
        )
    }

    /// The craft on show and what was done to it, while there is one.
    pub(crate) fn builder(&self) -> Option<&Builder> {
        self.0.as_ref()?.craft.as_ref()
    }

    pub(crate) fn builder_mut(&mut self) -> Option<&mut Builder> {
        self.0.as_mut()?.craft.as_mut()
    }

    /// Whether `action` has to wait for the player to say the changes to the craft on show may be
    /// thrown away: it would put another craft in its place (`editor.show`, `editor.new`) or leave
    /// (Back, Esc), and there are changes the craft was not put on show with. If so the question is
    /// sent for (`editor.ask_discard:<action>`) and the caller does nothing more with `action`.
    /// The action the player agreed to (`editor.discard`) goes through, once.
    pub(crate) fn asks_first(&mut self, action: &UiAction, commands: &mut Commands) -> bool {
        let Some(showing) = self.0.as_mut() else {
            return false;
        };
        if showing.discard.as_deref() == Some(action.0.as_str()) {
            showing.discard = None;
            return false;
        }
        let Some(craft) = showing.craft.as_ref() else {
            return false;
        };
        let replaces = match action.name() {
            // Choosing the craft that is on show only puts the list away.
            "editor.show" => action.arg() != Some(craft.draft().id.as_str()),
            "editor.new" | "editor.back" | "menu.back" | "game.menu" => true,
            _ => false,
        };
        if !replaces || !craft.can_undo() {
            return false;
        }
        commands.write_message(UiAction(format!("editor.ask_discard:{}", action.0)));
        true
    }

    /// The vessel standing in the hangar.
    pub(crate) fn vessel(&self) -> Option<VesselId> {
        self.0.as_ref().map(|showing| showing.vessel)
    }

    /// Say this under the launch button, or nothing for "".
    pub(crate) fn set_notice(&mut self, text: impl Into<String>) {
        if let Some(showing) = self.0.as_mut() {
            showing.notice = text.into();
        }
    }

    /// Put `craft`, a change to what is on show, there in its place, with the staging the player
    /// gave it, while the view and the hall stay as they are.
    pub(crate) fn redraw(
        &mut self,
        craft: &CraftDef,
        catalog: &PartCatalog,
        sim: &mut Sim,
        ui: &mut LayoutVisibility,
    ) -> Result<(), String> {
        if craft.parts.is_empty() {
            let empty = Vessel::empty_display(&sim.universe.system, sim.universe.time());
            self.stand(empty, sim, ui);
            return Ok(());
        }
        let propulsion = Propulsion::from_craft(craft, catalog).map_err(|e| e.to_string())?;
        self.redraw_built(craft, propulsion, sim, ui);
        Ok(())
    }

    /// [`Editor::redraw`] with the parts of `craft` built already, as they can be elsewhere.
    pub(crate) fn redraw_built(
        &mut self,
        craft: &CraftDef,
        propulsion: Propulsion,
        sim: &mut Sim,
        ui: &mut LayoutVisibility,
    ) {
        let time = sim.universe.time();
        let vessel = Vessel::on_display_built(craft, propulsion, &sim.universe.system, time);
        self.stand(vessel, sim, ui);
    }

    /// Stand `vessel` in the hangar in place of what is there, with the staging the player gave it
    /// if its parts are the same.
    fn stand(&mut self, mut vessel: Vessel, sim: &mut Sim, ui: &mut LayoutVisibility) {
        let Some(showing) = self.0.as_mut() else {
            return;
        };
        vessel.lie_on_display(showing.place == Place::Runway);
        if let Some(left) = sim.universe.vessel(showing.vessel) {
            // A change of size leaves the parts as they were, so the staging still fits them.
            if left.propulsion.parts().parts().len() == vessel.propulsion.parts().parts().len() {
                vessel.propulsion.sequences = left.propulsion.sequences.clone();
            }
            vessel.flight.static_pressure = left.flight.static_pressure;
        }
        sim.universe.remove_vessel(showing.vessel);
        showing.vessel = sim.universe.add_vessel(vessel);
        sim.universe.focus = Some(showing.vessel);
        close_part_windows(ui);
    }

    /// Start a craft of nothing, to be built: a rocket for the pad, or for the runway a plane,
    /// built lying as it will stand there. The hangar empties and the view stays where it is.
    pub(crate) fn start_new(&mut self, plane: bool, sim: &mut Sim, ui: &mut LayoutVisibility) {
        let Some(showing) = self.0.as_mut() else {
            return;
        };
        if plane {
            showing.place = Place::Runway;
        } else if showing.place == Place::Runway {
            showing.place = Place::Pad;
        }
        showing.anchor = DVec3::ZERO;
        showing.craft = Some(Builder::of(CraftDef {
            id: NEW_CRAFT.into(),
            name: "New vessel".into(),
            parts: Vec::new(),
            sequences: None,
            triggers: Vec::new(),
            rcs_shut: Vec::new(),
            abort: Vec::new(),
        }));
        showing.list_open = false;
        showing.discard = None;
        showing.notice.clear();
        let empty = Vessel::empty_display(&sim.universe.system, sim.universe.time());
        self.stand(empty, sim, ui);
        self.settle(sim);
    }

    /// The first part of a craft being built is on show: it floats well over the floor, with
    /// room under it for what goes below, and the view, far out in the empty hangar, comes in to
    /// it.
    pub(crate) fn frame_first_part(&mut self, sim: &mut Sim) {
        self.settle(sim);
        if let Some(showing) = self.0.as_mut() {
            let size = showing.size();
            let clearance = if showing.place == Place::Runway {
                FIRST_CLEARANCE + 0.5 * size
            } else {
                FIRST_CLEARANCE + size
            };
            showing.stand_at(clearance);
            showing.frame();
        }
    }

    /// Where the root is from the middle of the floor, sim axes.
    pub(crate) fn anchor(&self) -> Option<DVec3> {
        self.0.as_ref().map(|showing| showing.anchor)
    }

    /// Move the whole craft to have its root `at` (from the middle of the floor, sim axes), as far
    /// as it stays over the floor and in the hall; the view keeps looking at the same place.
    pub(crate) fn carry_to(&mut self, at: DVec3) {
        if let Some(showing) = self.0.as_mut() {
            showing.anchor = at;
            showing.keep_in();
        }
    }

    /// The left button is no longer turning the view: what it pressed on was picked up.
    pub(crate) fn stop_view_drag(&mut self) {
        if let Some(showing) = self.0.as_mut() {
            showing.dragging = false;
        }
    }

    /// Show or hide the list of vessels.
    pub(crate) fn set_list_open(&mut self, open: bool) {
        if let Some(showing) = self.0.as_mut() {
            showing.list_open = open;
        }
    }

    /// What is on show has stopped changing: the view keeps looking at the same place on it as its
    /// size is measured again.
    pub(crate) fn settle(&mut self, sim: &mut Sim) {
        let Some(showing) = self.0.as_mut() else {
            return;
        };
        let Some(index) = sim
            .universe
            .vessels
            .iter()
            .position(|v| v.id() == showing.vessel)
        else {
            return;
        };
        (showing.low, showing.high) = extent(sim, index);
        // It stays where it is, lifted only so as not to go through the floor.
        showing.keep_in();
    }

    /// Show the craft standing, for the pad and for orbit, or lying, for the runway, as `place`
    /// says; stood on the floor afresh when that changes how it lies.
    pub(crate) fn set_place(
        &mut self,
        place: Place,
        catalog: &PartCatalog,
        sim: &mut Sim,
        ui: &mut LayoutVisibility,
    ) {
        let Some(showing) = self.0.as_mut() else {
            return;
        };
        let turned = (showing.place == Place::Runway) != (place == Place::Runway);
        showing.place = place;
        showing.notice.clear();
        if !turned {
            return;
        }
        let draft = showing.craft.as_ref().map(|b| b.draft().clone());
        if let Some(draft) = draft
            && let Err(e) = self.redraw(&draft, catalog, sim, ui)
        {
            error!("cannot show the craft: {e}");
        }
        self.settle(sim);
        if let Some(showing) = self.0.as_mut() {
            showing.stand_at(0.0);
            showing.frame();
        }
    }
}

/// Where a revert goes back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RevertTo {
    Launch,
    Hangar,
}

impl RevertTo {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "launch" => Some(Self::Launch),
            "hangar" => Some(Self::Hangar),
            _ => None,
        }
    }

    /// How its actions write it.
    fn id(self) -> &'static str {
        match self {
            Self::Launch => "launch",
            Self::Hangar => "hangar",
        }
    }
}

/// How long `seconds` of the game reads in the question: 40 s, 12 min, 3 h 5 min, 2 d 4 h.
fn time_since(seconds: f64) -> String {
    let seconds = seconds.max(0.0).round() as u64;
    match seconds {
        0..60 => format!("{seconds} s"),
        60..3_600 => format!("{} min", seconds / 60),
        3_600..86_400 => format!("{} h {} min", seconds / 3_600, seconds % 3_600 / 60),
        _ => format!("{} d {} h", seconds / 86_400, seconds % 86_400 / 3_600),
    }
}

/// Whether to go back to `to`, losing all of the game since `name` left, `since` seconds ago.
fn revert_question(to: RevertTo, name: &str, since: f64) -> Question {
    let title = match to {
        RevertTo::Launch => "Revert to launch?",
        RevertTo::Hangar => "Revert to hangar?",
    };
    Question::destructive(
        title.to_owned(),
        &format!(
            "Everything since {name} launched, {} ago, will be lost. Saves stay as they are.",
            time_since(since)
        ),
        "Revert",
        format!("game.revert:{}", to.id()),
    )
}

/// Whether to throw away what was done to `craft` and go on with `then`.
fn discard_question(craft: &CraftDef, then: &str) -> Question {
    let (title, text) = if craft.id == NEW_CRAFT {
        (
            "Discard the vessel you are building?".to_owned(),
            "It has not been launched, so it will be lost.".to_owned(),
        )
    } else {
        (
            format!("Discard your changes to {}?", craft.name),
            format!(
                "They have not been launched, so they will be lost. {} itself stays as it was.",
                craft.name
            ),
        )
    };
    Question::destructive(title, &text, "Discard", format!("editor.discard:{then}"))
}

/// `editor.vessels` (name, and in `craft` each one's id, name, parts and selected): every craft
/// there is to fly, a group per mod in load order, by name in each.
fn vessel_groups(content: &GameContent, shown: Option<&str>) -> Vec<UiRecord> {
    let registry = &content.0.registry;
    (content.0.mods.iter())
        .filter_map(|source| {
            let mut craft: Vec<_> = (registry.records("craft"))
                .filter(|r| r.source == source.manifest.id)
                .map(|r| {
                    let name = (r.value.get("name").and_then(|n| n.as_str()))
                        .filter(|name| !name.is_empty())
                        .unwrap_or(&r.id);
                    let parts = r.value.get("parts").and_then(|p| p.as_array());
                    (name.to_owned(), r.id.clone(), parts.map_or(0, Vec::len))
                })
                .collect();
            if craft.is_empty() {
                return None;
            }
            craft.sort();
            let craft: Vec<_> = craft
                .into_iter()
                .map(|(name, id, parts)| {
                    let selected = shown == Some(id.as_str());
                    record([
                        ("id", id.into()),
                        ("name", name.into()),
                        ("parts", parts.into()),
                        ("selected", selected.into()),
                    ])
                })
                .collect();
            Some(record([
                ("name", group_name(&source.manifest).into()),
                ("craft", craft.into()),
            ]))
        })
        .collect()
}

/// What a mod's craft are headed with: its name, up to any colon.
fn group_name(manifest: &ModManifest) -> &str {
    let name = if manifest.name.is_empty() {
        &manifest.id
    } else {
        &manifest.name
    };
    name.split(':').next().unwrap_or(name)
}

/// Gravity and the air's pressure on the ground at the site launches go from by default, m/s² and
/// Pa: what the craft's figures are for.
fn ground_at_launch(sim: &Sim, all: &[LaunchSiteDef]) -> (f64, f64) {
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

/// The box round `vessel`'s parts as it stands, from its root, sim axes.
fn extent(sim: &mut Sim, index: usize) -> (DVec3, DVec3) {
    let universe = &mut sim.universe;
    let vessel = &universe.vessels[index];
    let tree = vessel.propulsion.parts();
    let attitude = vessel.attitude_at(&universe.system, universe.time());
    let base = tree.root().map_or(DVec3::ZERO, |root| root.position);
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

/// The site a launch from `place` goes from, and whether it needs that site's runway. An orbit
/// stands on nothing.
fn launch_ground(place: Place, all: &[LaunchSiteDef]) -> Option<(&LaunchSiteDef, bool)> {
    let (pad, runway) = sites(all);
    match place {
        Place::Pad => pad.map(|site| (site, false)),
        Place::Runway => runway.map(|site| (site, true)),
        Place::Orbit => None,
    }
}

/// What stands where a launch from `place` would. A new game has nothing standing anywhere: what
/// is there now goes when it starts.
fn in_the_way(place: Place, all: &[LaunchSiteDef], sim: &Sim, new_game: bool) -> Vec<VesselId> {
    if new_game {
        return Vec::new();
    }
    launch_ground(place, all).map_or(Vec::new(), |(site, runway)| {
        sim.universe.on_launch_ground(site, runway)
    })
}

/// What the player reads about what is in the way: the name, and how many more with it. The names
/// of vessels come before those of debris.
fn named(ids: &[VesselId], sim: &Sim) -> String {
    let vessels = ids.iter().filter_map(|&id| sim.universe.vessel(id));
    let mut vessels: Vec<&Vessel> = vessels.collect();
    vessels.sort_by_key(|vessel| vessel.is_debris());
    match vessels.as_slice() {
        [] => String::new(),
        [only] => only.name.clone(),
        [first, rest @ ..] => format!("{} and {} more", first.name, rest.len()),
    }
}

/// A vessel to fly other than these and what is on show: debris is not one.
fn next_to_fly(sim: &Sim, except: &[VesselId]) -> Option<VesselId> {
    (sim.universe.vessels.iter())
        .filter(|vessel| !vessel.is_debris() && !except.contains(&vessel.id()))
        .map(Vessel::id)
        .next()
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
pub(crate) fn editor_actions(
    mut commands: Commands,
    mut actions: MessageReader<UiAction>,
    mut editor: ResMut<Editor>,
    mut menu: ResMut<Menu>,
    mut sim: ResMut<Sim>,
    mut warp: ResMut<TimeWarp>,
    mut rig: ResMut<CameraRig>,
    mut ui: ResMut<LayoutVisibility>,
    mut ambient: ResMut<GlobalAmbientLight>,
    (mut saves, mut mods): (ResMut<Saves>, ResMut<PlayerMods>),
    (mut meshes, mut materials, mut images): (
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
        ResMut<Assets<Image>>,
    ),
    mut restarted: MessageWriter<SimRestarted>,
    content: Res<GameContent>,
    docs: Res<UiDocuments>,
    (mut dialog, mut shaping, mut placing, mut moving): (
        ResMut<Dialog>,
        ResMut<Shaping>,
        ResMut<Placing>,
        ResMut<Moving>,
    ),
    mut last_launch: ResMut<LastLaunch>,
) {
    for action in actions.read() {
        match action.name() {
            // Esc and Back close the question being asked, and nothing more.
            "game.menu" | "menu.back" | "editor.back" if dialog.is_open() => {}
            // Esc puts back a part in hand, then lets go of the part picked, before it leaves.
            "game.menu" if editor.0.is_some() && placing.let_go() => {}
            "game.menu" if editor.0.is_some() && moving.let_go() => {}
            "game.menu" if editor.0.is_some() && shaping.let_go() => {}
            // The last one leaves only the hangar for a new one.
            "vessel.destroy" if editor.0.is_none() => {
                let id = action.arg().and_then(|id| id.parse().ok()).map(VesselId);
                let Some(id) = id.filter(|&id| sim.universe.vessel(id).is_some()) else {
                    warn!("no vessel to destroy in `{}`", action.0);
                    continue;
                };
                let name = sim
                    .universe
                    .vessel(id)
                    .map_or(String::new(), |v| v.name.clone());
                let next = next_to_fly(&sim, &[id]);
                info!("destroyed `{name}`");
                if next.is_some() {
                    if id == sim.active_id() {
                        sim.fly(next.unwrap_or(id));
                        rig.follow = None;
                        rig.frame(sim.active_vessel().propulsion.length());
                    }
                    sim.universe.remove_vessel(id);
                    menu.notice = format!("Destroyed {name}");
                    continue;
                }
                sim.universe.remove_vessel(id);
                let registry = &content.0.registry;
                let all = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
                open_hangar(
                    &mut commands,
                    &mut editor,
                    &mut menu,
                    &mut sim,
                    &mut rig,
                    &mut ui,
                    &mut ambient,
                    (&mut meshes, &mut materials, &mut images),
                    &all,
                    None,
                );
            }
            "game.new" | "editor.open" if editor.0.is_none() => {
                let registry = &content.0.registry;
                let all = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
                let flown = Some(sim.active_id());
                open_hangar(
                    &mut commands,
                    &mut editor,
                    &mut menu,
                    &mut sim,
                    &mut rig,
                    &mut ui,
                    &mut ambient,
                    (&mut meshes, &mut materials, &mut images),
                    &all,
                    flown,
                );
            }
            // Sent by an action that would throw away changes to the craft (Editor::asks_first).
            "editor.ask_discard" => {
                let craft = editor.builder().map(Builder::draft);
                if let (Some(then), Some(craft)) = (action.arg(), craft) {
                    dialog.ask(discard_question(craft, then));
                }
            }
            // The player said the changes may go: the action that asked goes ahead.
            "editor.discard" => {
                let (Some(then), Some(showing)) = (action.arg(), editor.0.as_mut()) else {
                    continue;
                };
                showing.discard = Some(then.to_owned());
                commands.write_message(UiAction(then.to_owned()));
            }
            "ask.clear" => {
                let Some(showing) = editor.0.as_ref() else {
                    continue;
                };
                let registry = &content.0.registry;
                let all = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
                let ids = in_the_way(showing.place, &all, &sim, menu.new_game);
                if !ids.is_empty() {
                    dialog.ask(Question::destructive(
                        format!("Destroy {}?", named(&ids, &sim)),
                        "A destroyed vessel cannot be brought back.",
                        "Destroy",
                        "editor.clear".to_owned(),
                    ));
                }
            }
            // Whatever stands where the launch would go is destroyed.
            "editor.clear" => {
                let Some(showing) = editor.0.as_mut() else {
                    continue;
                };
                let registry = &content.0.registry;
                let all = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
                let ids = in_the_way(showing.place, &all, &sim, menu.new_game);
                info!("cleared `{}`", named(&ids, &sim));
                for &id in &ids {
                    sim.universe.remove_vessel(id);
                }
                if showing.flown.is_some_and(|flown| ids.contains(&flown)) {
                    showing.flown = next_to_fly(&sim, &[showing.vessel]);
                    showing.flown_changed = true;
                }
                showing.notice.clear();
            }
            "editor.show" if editor.asks_first(action, &mut commands) => {}
            // Put another craft on show in place of what is there.
            "editor.show" => {
                let Some(showing) = editor.0.as_mut() else {
                    continue;
                };
                let Some(id) = action.arg() else { continue };
                if showing
                    .craft
                    .as_ref()
                    .is_some_and(|craft| craft.draft().id == id)
                {
                    showing.list_open = false;
                    continue;
                }
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
                        showing.notice = format!("Cannot show it: {e}");
                        continue;
                    }
                };
                let all = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
                let place = place_for(&vessel, sites(&all).1.is_some(), showing.place);
                put_on_show(
                    showing,
                    craft,
                    vessel,
                    place,
                    &mut commands,
                    &mut sim,
                    &mut ui,
                    (&mut meshes, &mut materials, &mut images),
                );
            }
            "editor.list" => {
                if let Some(showing) = editor.0.as_mut() {
                    showing.list_open = !showing.list_open;
                }
            }
            "editor.place" => {
                if let Some(place) = action.arg().and_then(Place::parse) {
                    let catalog = catalog(&content.0.registry, &mut Vec::new());
                    editor.set_place(place, &catalog, &mut sim, &mut ui);
                }
            }
            // Esc closes a popup first. Otherwise it and Back go to the menu they came from.
            "game.menu" if editor.0.is_some() && ui.dismiss_popups(&docs) => {}
            "editor.back" | "menu.back" | "game.menu"
                if editor.asks_first(action, &mut commands) => {}
            "editor.back" | "menu.back" | "game.menu" => {
                let Some(showing) = editor.0.take() else {
                    continue;
                };
                close(&showing, &mut commands, &mut rig, &mut ui, &mut ambient);
                menu.new_game = false;
                let Some(flown) = showing.flown else {
                    // Nothing left to fly, so nothing to go back to.
                    go_to_title(
                        &mut menu,
                        &mut sim,
                        &mut warp,
                        &mut rig,
                        &mut ui,
                        &mut saves,
                        &mut last_launch,
                        &content,
                        &mut restarted,
                    );
                    continue;
                };
                sim.universe.focus = Some(flown);
                sim.universe.remove_vessel(showing.vessel);
                if showing.flown_changed {
                    rig.frame(sim.active_vessel().propulsion.length());
                }
                menu.show(true, &mut sim, &mut ui);
            }
            "editor.launch" => {
                let Some(showing) = editor.0.as_mut() else {
                    continue;
                };
                let Some(craft) = showing.craft.as_ref().map(|b| b.draft().clone()) else {
                    showing.notice = "Choose a vessel first".into();
                    continue;
                };
                let registry = &content.0.registry;
                let all = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
                if !in_the_way(showing.place, &all, &sim, menu.new_game).is_empty() {
                    showing.notice = "Clear it before launching".into();
                    continue;
                }
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
                    universe.launch(&all, &catalog, &craft, &start, sequences.clone(), new_game);
                let id = match launched {
                    Ok(id) => id,
                    Err(e) => {
                        error!("cannot launch `{}`: {e}", craft.id);
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
                info!("launched `{}` from {}", craft.id, showing.place.id());
                rig.frame(showing.size());
                sim.restarted();
                *warp = TimeWarp::default();
                restarted.write(SimRestarted);
                if menu.new_game {
                    saves.forget_current();
                    mods.new_game_starts();
                }
                last_launch.set(LaunchPoint {
                    game: sim.universe.snapshot(),
                    launched: id,
                    name: sim.active_vessel().name.clone(),
                    time,
                    place: showing.place,
                    // A new game has nothing in it from before.
                    flown: if menu.new_game { None } else { showing.flown },
                    craft,
                    sequences,
                });
                menu.back_to_root();
                menu.playing = true;
                menu.show(false, &mut sim, &mut ui);
                // The game is in the list of saves from its first launch on.
                saves.save(&sim);
            }
            _ => {}
        }
    }
}

/// Where a craft put on show goes from: what rolls on wheels from the runway, if there is one, and
/// is shown lying as it will stand there; anything else from where the last went, or the pad if
/// that was the runway.
fn place_for(vessel: &Vessel, runway: bool, now: Place) -> Place {
    let rolls = (osp_sim::gear::legs(&vessel.propulsion).iter()).any(|leg| !leg.foot);
    match now {
        _ if rolls && runway => Place::Runway,
        Place::Runway => Place::Pad,
        other => other,
    }
}

/// Put `vessel`, built from `craft`, on show in place of what is there, going from `place`.
fn put_on_show(
    showing: &mut Showing,
    craft: CraftDef,
    vessel: Vessel,
    place: Place,
    commands: &mut Commands,
    sim: &mut Sim,
    ui: &mut LayoutVisibility,
    (meshes, materials, images): (
        &mut Assets<Mesh>,
        &mut Assets<StandardMaterial>,
        &mut Assets<Image>,
    ),
) {
    let pressure =
        (sim.universe.vessel(showing.vessel)).map_or(0.0, |left| left.flight.static_pressure);
    showing.place = place;
    let mut vessel = vessel;
    vessel.lie_on_display(showing.place == Place::Runway);
    sim.universe.remove_vessel(showing.vessel);
    showing.vessel = sim.universe.add_vessel(vessel);
    sim.universe.focus = Some(showing.vessel);
    sim.active_vessel_mut().flight.static_pressure = pressure;
    let index = sim.active_index();
    (showing.low, showing.high) = extent(sim, index);
    showing.craft = Some(Builder::of(craft));
    showing.discard = None;
    showing.notice.clear();
    // Every craft is first seen the same way, with the list out of its way.
    showing.list_open = false;
    showing.yaw = START_YAW;
    showing.pitch = START_PITCH;
    showing.stand_at(0.0);
    showing.frame();
    let (radius, height) = (showing.radius(), showing.height());
    // The hall fits what is in it.
    commands.entity(showing.hangar).despawn();
    showing.hangar = spawn_hangar(
        commands,
        meshes,
        materials,
        images,
        radius as f32,
        height as f32,
    );
    showing.hangar_size = (radius, height);
    close_part_windows(ui);
    ui.layouts.insert(STAGING.into(), true);
}

/// `ask.revert:<launch|hangar>` and `game.revert:<launch|hangar>`. If the game cannot go back,
/// nothing changes and the menu says why.
#[allow(clippy::too_many_arguments)]
fn revert_actions(
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
    (mut meshes, mut materials, mut images): (
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
        ResMut<Assets<Image>>,
    ),
    mut restarted: MessageWriter<SimRestarted>,
    content: Res<GameContent>,
    mut dialog: ResMut<Dialog>,
    last_launch: Res<LastLaunch>,
) {
    for action in actions.read() {
        if !matches!(action.name(), "ask.revert" | "game.revert") {
            continue;
        }
        let to = action.arg().and_then(RevertTo::parse);
        let (Some(to), Some(point)) = (to, last_launch.point()) else {
            warn!("nothing to revert to in `{}`", action.0);
            continue;
        };
        if editor.is_open() {
            continue;
        }
        if action.name() == "ask.revert" {
            let since = sim.universe.time().seconds_since(point.time);
            dialog.ask(revert_question(to, &point.name, since));
            continue;
        }
        // What goes on show is built first, as it comes from the records and can fail.
        let registry = &content.0.registry;
        let display = match to {
            RevertTo::Launch => None,
            RevertTo::Hangar => {
                let catalog = catalog(registry, &mut Vec::new());
                let system = &sim.universe.system;
                match Vessel::on_display(&point.craft, &catalog, system, point.time) {
                    Ok(vessel) => Some(vessel),
                    Err(e) => {
                        error!("cannot put `{}` back on show: {e}", point.craft.id);
                        menu.notice = format!("Cannot revert: {e}");
                        continue;
                    }
                }
            }
        };
        if let Err(e) = sim.universe.restore(point.game.clone()) {
            error!("cannot revert: {e}");
            menu.notice = format!("Cannot revert: {e}");
            continue;
        }
        info!("reverted `{}` to {}", point.name, to.id());
        sim.restarted();
        *warp = TimeWarp::default();
        rig.map_view = false;
        rig.follow = None;
        restarted.write(SimRestarted);
        saves.notice.clear();
        menu.back_to_root();
        let Some(mut display) = display else {
            rig.frame(sim.active_vessel().propulsion.length());
            menu.show(false, &mut sim, &mut ui);
            continue;
        };
        if let Some(sequences) = &point.sequences {
            display.propulsion.sequences = sequences.clone();
        }
        sim.universe.remove_vessel(point.launched);
        let all = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
        open_hangar(
            &mut commands,
            &mut editor,
            &mut menu,
            &mut sim,
            &mut rig,
            &mut ui,
            &mut ambient,
            (&mut meshes, &mut materials, &mut images),
            &all,
            point.flown,
        );
        let Some(showing) = editor.0.as_mut() else {
            continue;
        };
        // The camera was on what is gone.
        showing.flown_changed = true;
        put_on_show(
            showing,
            point.craft.clone(),
            display,
            point.place,
            &mut commands,
            &mut sim,
            &mut ui,
            (&mut meshes, &mut materials, &mut images),
        );
    }
}

/// Open the editor with the hangar empty, going back to `flown`.
#[allow(clippy::too_many_arguments)]
fn open_hangar(
    commands: &mut Commands,
    editor: &mut Editor,
    menu: &mut Menu,
    sim: &mut Sim,
    rig: &mut CameraRig,
    ui: &mut LayoutVisibility,
    ambient: &mut GlobalAmbientLight,
    (meshes, materials, images): (
        &mut Assets<Mesh>,
        &mut Assets<StandardMaterial>,
        &mut Assets<Image>,
    ),
    all: &[LaunchSiteDef],
    flown: Option<VesselId>,
) {
    let (gravity, pressure) = ground_at_launch(sim, all);
    let time = sim.universe.time();
    let empty = Vessel::empty_display(&sim.universe.system, time);
    let vessel = sim.universe.add_vessel(empty);
    sim.universe.focus = Some(vessel);
    sim.active_vessel_mut().flight.static_pressure = pressure;
    let index = sim.active_index();
    let (low, high) = extent(sim, index);
    let mut showing = Showing {
        craft: None,
        vessel,
        flown,
        flown_changed: false,
        low,
        high,
        anchor: DVec3::ZERO,
        hangar: Entity::PLACEHOLDER,
        hangar_size: (0.0, 0.0),
        ambient: ambient.brightness,
        gravity,
        place: Place::Pad,
        notice: String::new(),
        yaw: START_YAW,
        pitch: START_PITCH,
        distance: EMPTY_VIEW,
        look_at: DVec3::Z * OVER_FLOOR,
        dragging: false,
        staging_was_open: ui.shown(STAGING),
        list_open: true,
        discard: None,
    };
    showing.hangar = spawn_hangar(
        commands,
        meshes,
        materials,
        images,
        showing.radius() as f32,
        showing.height() as f32,
    );
    showing.hangar_size = (showing.radius(), showing.height());
    ambient.brightness = HANGAR_AMBIENT;
    rig.held = true;
    rig.map_view = false;
    rig.follow = None;
    menu.notice.clear();
    menu.show(false, sim, ui);
    sim.paused = true;
    ui.hidden_groups.insert("flight".into());
    ui.kept.extend(KEPT_LAYOUTS.map(str::to_owned));
    ui.layouts.insert(LAYOUT.into(), true);
    ui.layouts.insert(STAGING.into(), false);
    ui.docks.insert(STAGING.into(), STAGING_DOCK.into());
    editor.0 = Some(showing);
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
    close_part_windows(ui);
    ui.kept.clear();
    ui.docks.remove(STAGING);
    ui.layouts.insert(STAGING.into(), showing.staging_was_open);
    ui.layouts.insert(LAYOUT.into(), false);
}

/// The windows of parts of what was on show.
fn close_part_windows(ui: &mut LayoutVisibility) {
    let open: Vec<String> = ui.instances(PART_WINDOW).map(str::to_owned).collect();
    for key in open {
        ui.layouts.insert(format!("{PART_WINDOW}:{key}"), false);
    }
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
    (shaping, placing): (Res<Shaping>, Res<Placing>),
    mut camera: Single<&mut Transform, With<MainCamera>>,
) {
    let Some(showing) = editor.0.as_mut() else {
        return;
    };
    let Some(vessel) = sim.universe.vessel(showing.vessel) else {
        return;
    };
    // A press on a handle shapes the part instead.
    if buttons.just_pressed(MouseButton::Left) {
        showing.dragging = !over_ui.0 && !shaping.holds_pointer();
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
        // Q and E turn a part picked or in hand instead.
        let turning = shaping.picked().is_some() || placing.is_holding();
        input.walk = DVec3::new(
            axis(KeyCode::KeyW, KeyCode::KeyS),
            axis(KeyCode::KeyD, KeyCode::KeyA),
            if turning {
                0.0
            } else {
                axis(KeyCode::KeyE, KeyCode::KeyQ)
            },
        );
        input.dt = time.delta_secs_f64();
    }
    showing.steer(&input);
    let root = craft_origin(vessel, &sim.universe.system, frame.time);
    let floor = frame.relative(vessel.parent(), root) - showing.anchor;
    let (eye, focus) = showing.view(floor);
    rig.position = eye;
    **camera = Transform::from_translation(to_render(eye)).looking_at(to_render(focus), Vec3::Y);
}

/// The hall grows with a craft being built, so it always fits, and shrinks back when much of it
/// is taken off.
fn refit_hangar(
    mut commands: Commands,
    mut editor: ResMut<Editor>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(showing) = editor.0.as_mut() else {
        return;
    };
    let (radius, height) = (showing.radius(), showing.height());
    let (built_radius, built_height) = showing.hangar_size;
    let outgrown = radius > built_radius * 1.001 || height > built_height * 1.001;
    let too_big = radius < built_radius * 0.6 && height < built_height * 0.6;
    if !(outgrown || too_big) {
        return;
    }
    commands.entity(showing.hangar).despawn();
    showing.hangar = spawn_hangar(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        radius as f32,
        height as f32,
    );
    showing.hangar_size = (radius, height);
}

/// Keep the floor where it is under the craft.
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
    let root = craft_origin(vessel, &sim.universe.system, frame.time);
    let floor = frame.relative(vessel.parent(), root) - showing.anchor;
    for mut transform in &mut hangars {
        transform.translation = to_render(floor);
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
/// Whether the editor is open, and what the craft on show weighs by, for the rest of the game.
fn tell_the_game(editor: Res<Editor>, mut status: ResMut<EditorStatus>) {
    let gravity = editor.gravity();
    if status.gravity() != gravity {
        status.set(gravity);
    }
}

fn publish_editor(
    editor: Res<Editor>,
    menu: Res<Menu>,
    sim: Res<Sim>,
    content: Res<GameContent>,
    mut published: Local<Option<(VesselId, Place, String, bool, bool, usize)>>,
    mut data: ResMut<UiData>,
) {
    let Some(showing) = &editor.0 else {
        *published = None;
        return;
    };
    let now = (
        showing.vessel,
        showing.place,
        showing.notice.clone(),
        menu.new_game,
        showing.list_open,
        sim.universe.vessels.len(),
    );
    if published.as_ref() == Some(&now) && !content.is_changed() {
        return;
    }
    *published = Some(now);
    let heading = if menu.new_game {
        "New game"
    } else {
        "Fly a new vessel"
    };
    data.set("editor.heading", heading);
    data.set("editor.has_vessel", showing.craft.is_some());
    data.set("editor.list_open", showing.list_open);
    data.set(
        "editor.vessels",
        vessel_groups(&content, editor.craft().filter(|id| !id.is_empty())),
    );
    if let Some(vessel) = sim.universe.vessel(showing.vessel) {
        let propulsion = &vessel.propulsion;
        // A craft being built is called what it is called even with nothing in it yet.
        let draft = showing.craft.as_ref().map(|b| b.draft());
        let name = draft.map_or(vessel.name.as_str(), |d| {
            if d.name.is_empty() {
                d.id.as_str()
            } else {
                d.name.as_str()
            }
        });
        data.set("editor.name", name);
        // Plus zero, so that nothing weighs 0 kg rather than -0.
        data.set("editor.mass", propulsion.mass() + 0.0);
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
    let orbit = format!("{:.0} km around {body}", ORBIT_ALTITUDE / 1e3);
    let place_name = match showing.place {
        Place::Pad => pad.map_or("", LaunchSiteDef::display_name),
        Place::Runway => runway.map_or("", LaunchSiteDef::display_name),
        Place::Orbit => orbit.as_str(),
    };
    data.set("editor.place_name", place_name);
    data.set("editor.orbit", orbit.as_str());
    let blocking = in_the_way(showing.place, &all, &sim, menu.new_game);
    let names = named(&blocking, &sim);
    let occupied = if blocking.is_empty() {
        String::new()
    } else {
        format!("Occupied by {names}")
    };
    data.set("editor.occupied", occupied);
    let has_parts = (showing.craft.as_ref()).is_some_and(|c| !c.draft().parts.is_empty());
    data.set("editor.has_parts", has_parts);
    data.set("editor.can_launch", has_parts && blocking.is_empty());
    let what = match showing.place {
        Place::Runway => "Clear runway",
        Place::Pad | Place::Orbit => "Clear pad",
    };
    data.set("editor.clear_label", what);
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
    fn a_launch_stands_on_the_pad_or_the_runway_and_an_orbit_on_nothing() {
        let all = [site("strip", false, true), site("a", true, false)];
        let on = |place| launch_ground(place, &all).map(|(s, runway)| (s.id.as_str(), runway));
        assert_eq!(on(Place::Pad), Some(("a", false)));
        assert_eq!(on(Place::Runway), Some(("strip", true)));
        assert_eq!(on(Place::Orbit), None);
        assert!(launch_ground(Place::Runway, &[site("a", true, false)]).is_none());
    }

    #[test]
    fn a_revert_asks_about_where_it_goes() {
        let mut dialog = Dialog::default();
        dialog.ask(revert_question(RevertTo::Hangar, "Saturn V", 300.0));
        assert_eq!(dialog.title(), "Revert to hangar?");
        dialog.ask(revert_question(RevertTo::Launch, "Saturn V", 300.0));
        assert_eq!(dialog.title(), "Revert to launch?");
        assert_eq!(RevertTo::parse("hangar"), Some(RevertTo::Hangar));
        assert_eq!(
            RevertTo::parse(RevertTo::Launch.id()),
            Some(RevertTo::Launch)
        );
        assert_eq!(RevertTo::parse("orbit"), None);
    }

    #[test]
    fn the_time_since_a_launch_reads_naturally() {
        assert_eq!(time_since(40.2), "40 s");
        assert_eq!(time_since(720.0), "12 min");
        assert_eq!(time_since(3.0 * 3_600.0 + 300.0), "3 h 5 min");
        assert_eq!(time_since(2.0 * 86_400.0 + 4.0 * 3_600.0), "2 d 4 h");
        assert_eq!(time_since(-5.0), "0 s");
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
            craft: Some(Builder::of(CraftDef {
                id: "test".into(),
                name: String::new(),
                parts: Vec::new(),
                sequences: None,
                triggers: Vec::new(),
                rcs_shut: Vec::new(),
                abort: Vec::new(),
            })),
            vessel: VesselId(0),
            flown: Some(VesselId(0)),
            flown_changed: false,
            low: DVec3::new(-5.0, -5.0, -2.0),
            high: DVec3::new(5.0, 5.0, 98.0),
            anchor: DVec3::new(0.0, 0.0, 2.0),
            hangar: Entity::PLACEHOLDER,
            hangar_size: (0.0, 0.0),
            ambient: 0.0,
            gravity: STANDARD_GRAVITY,
            place: Place::Pad,
            notice: String::new(),
            yaw: 0.0,
            pitch: 0.1,
            distance: 120.0,
            look_at: DVec3::new(0.0, 0.0, 50.0),
            dragging: false,
            staging_was_open: true,
            list_open: false,
            discard: None,
        }
    }

    /// How far out in the dome the camera stands, 1 at the dome, with the floor's middle at the
    /// origin.
    fn out_in_dome(view: &Showing) -> f64 {
        let (eye, _) = view.view(DVec3::ZERO);
        view.in_dome(eye).length()
    }

    #[test]
    fn zoomed_right_out_the_view_stays_under_the_dome_whichever_way_it_looks() {
        // A small craft in the smallest hall, where the roof is lowest for its width, and a tall one.
        let small = Showing {
            low: DVec3::new(-1.0, -1.0, 0.0),
            high: DVec3::new(1.0, 1.0, 15.0),
            anchor: DVec3::ZERO,
            look_at: DVec3::new(0.0, 0.0, 7.5),
            ..showing()
        };
        for craft in [small, showing()] {
            let mut view = craft;
            for step in 0..=60 {
                view.pitch = -MAX_PITCH + 2.0 * MAX_PITCH * f64::from(step) / 60.0;
                for yaw in 0..12 {
                    view.yaw = f64::from(yaw) * 0.5;
                    view.steer(&ViewInput {
                        zoom: -500.0,
                        ..default()
                    });
                    let out = out_in_dome(&view);
                    assert!(
                        out <= WALL_REACH + 1e-6,
                        "through the dome at {out}, pitch {}",
                        view.pitch
                    );
                }
            }
        }
    }

    #[test]
    fn however_far_it_is_dragged_what_it_looks_at_stays_inside_and_the_view_inside_with_it() {
        let mut view = showing();
        for drag in [DVec2::splat(1e6), DVec2::splat(-1e6), DVec2::new(1e6, -1e6)] {
            view.steer(&ViewInput {
                drag,
                zoom: -500.0,
                ..default()
            });
            let focus = view.in_dome(view.focus()).length();
            assert!(focus <= FOCUS_REACH + 1e-6, "looking at {focus}");
            assert!(out_in_dome(&view) <= WALL_REACH + 1e-6);
            assert!(
                view.focus().z >= OVER_FLOOR - 1e-6,
                "looking under the floor"
            );
        }
    }

    #[test]
    fn turning_towards_the_roof_brings_the_view_in_and_turning_back_takes_it_out_again() {
        let mut view = showing();
        view.steer(&ViewInput {
            zoom: -500.0,
            ..default()
        });
        let far = view.view(DVec3::ZERO);
        let far = far.0.distance(far.1);
        view.steer(&ViewInput {
            turn: DVec2::new(0.0, 250.0),
            ..default()
        });
        let above = view.view(DVec3::ZERO);
        assert!(above.0.distance(above.1) < far * 0.9, "should have come in");
        view.steer(&ViewInput {
            turn: DVec2::new(0.0, -250.0),
            ..default()
        });
        let back = view.view(DVec3::ZERO);
        assert!(
            (back.0.distance(back.1) - far).abs() < 1e-6,
            "should be back out"
        );
    }

    #[test]
    fn zooming_in_answers_at_once_however_far_out_it_was_asked_to_be() {
        let mut view = showing();
        view.steer(&ViewInput {
            zoom: -500.0,
            ..default()
        });
        let far = view.distance;
        assert!(
            far <= view.reach() + 1e-6,
            "zooming out must stop at the dome"
        );
        view.steer(&ViewInput {
            zoom: 1.0,
            ..default()
        });
        assert!(view.distance < far);
    }

    #[test]
    fn an_empty_hangar_stands_a_vessel_with_nothing_in_it() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let content = osp_mods::load(&root.join("assets/core"), None);
        let registry = &content.registry;
        let mut diagnostics = Vec::new();
        let system = osp_sim::system::SolarSystem::from_def(&osp_sim::data::SystemDef {
            bodies: registry.all("bodies", &mut diagnostics),
        })
        .unwrap();
        let vessel = Vessel::empty_display(&system, SimTime::from_seconds(0.0));
        assert!(vessel.propulsion.parts().parts().is_empty());
        assert_eq!(vessel.propulsion.length(), 0.0);
        assert!(vessel.propulsion.mass().abs() < 1e-9);
    }

    #[test]
    fn the_list_of_vessels_goes_by_mod_and_marks_the_one_on_show() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let content = GameContent(osp_mods::load(
            &root.join("assets/core"),
            Some(&root.join("mods")),
        ));
        let groups = vessel_groups(&content, Some("glider"));
        let mut data = UiData::default();
        data.set("groups", groups);
        let text = |key: &str| format!("{:?}", data.get(key));
        assert!(
            text("groups[0].name").contains("Core"),
            "{}",
            text("groups[0].name")
        );
        let first = data.list_len("groups[0].craft");
        let total: usize = (0..data.list_len("groups"))
            .map(|g| data.list_len(&format!("groups[{g}].craft")))
            .sum();
        assert!(first > 0 && total > first, "{first} of {total}");
        let mut lit = 0;
        for g in 0..data.list_len("groups") {
            for c in 0..data.list_len(&format!("groups[{g}].craft")) {
                if data.condition(&format!("groups[{g}].craft[{c}].selected")) {
                    lit += 1;
                    assert!(
                        data.condition(&format!("groups[{g}].craft[{c}].id == glider")),
                        "the wrong one is lit"
                    );
                }
            }
        }
        assert_eq!(lit, 1);
    }

    #[test]
    fn a_mod_is_headed_by_its_name_up_to_any_colon() {
        let manifest = |id: &str, name: &str| {
            serde_json::from_value::<ModManifest>(serde_json::json!({ "id": id, "name": name }))
                .unwrap()
        };
        assert_eq!(
            group_name(&manifest("r", "Realism: parts and history")),
            "Realism"
        );
        assert_eq!(group_name(&manifest("c", "Core")), "Core");
        assert_eq!(group_name(&manifest("plain", "")), "plain");
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
        let floor = 0.0;
        let (radius, height) = (view.radius(), view.height());
        for input in &hard {
            view.steer(input);
            let (eye, focus) = view.view(base);
            assert!(
                eye.z >= floor + OVER_FLOOR - 1e-9,
                "under the floor: {input:?}"
            );
            assert!(focus.z <= floor + height, "through the roof: {input:?}");
            assert!(out_in_dome(&view) <= 1.0, "through the dome: {input:?}");
            let out = eye.truncate().length();
            assert!(
                out <= radius,
                "through the wall at {out} of {radius}: {input:?}"
            );
            assert!(view.distance >= NEAREST && view.pitch.abs() <= MAX_PITCH);
        }
    }
}
