//! The Universe Manifest (F11): every body and vessel, as in Kitten Space Agency. Rows are
//! filtered by kind (the filters stay set while the window is closed). Any row can be
//! followed: the camera looks at it instead of the active vessel (a body in map view), and
//! distances and speeds are measured from it. Vessels other than the one flown can be
//! destroyed, or recovered once landed on a world with a launch site; Clear Debris removes
//! every piece of debris. Removing anything asks first.
//!
//! | Action | Effect |
//! | --- | --- |
//! | `manifest.filter:<bodies\|vessels\|debris>` | Show or hide that kind of row |
//! | `manifest.follow:<key>`, `manifest.follow_body:<name>` | Follow a vessel (the active one: stop following) or a body |
//! | `manifest.remove:<key>` | Recover or destroy a vessel, once confirmed |
//! | `manifest.clear_debris` | Remove all debris, once confirmed |
//! | `manifest.confirm`, `manifest.cancel` | Answer the question asked |

use bevy::prelude::*;
use osp_sim::{
    body::BodyId,
    orbit::StateVector,
    rendezvous::Target,
    vessel::{Situation, Vessel, VesselId},
};
use osp_ui::{UiAction, UiData, UiRecord, record};

use crate::{
    FrameSet,
    camera::{CameraRig, Follow},
    sim::{Sim, TimeWarp, WorldData},
};

pub struct ManifestPlugin;

impl Plugin for ManifestPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ManifestUi>()
            .add_systems(Update, manifest_actions.in_set(FrameSet::Input))
            .add_systems(Update, publish_manifest.in_set(FrameSet::Render));
    }
}

/// What the manifest shows, and the question it is asking.
#[derive(Resource)]
struct ManifestUi {
    bodies: bool,
    vessels: bool,
    debris: bool,
    confirm: Option<Confirm>,
}

impl Default for ManifestUi {
    fn default() -> Self {
        Self {
            bodies: true,
            vessels: true,
            debris: true,
            confirm: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Confirm {
    Remove(VesselId),
    ClearDebris,
}

/// Landed on a world with a launch site, where it can be fetched.
fn recoverable(vessel: &Vessel, sim: &Sim, world: &WorldData) -> bool {
    let system = &sim.universe.system;
    let body = &system.body(vessel.parent()).name;
    vessel.situation(system, sim.render_time) == Situation::Landed
        && world.sites.iter().any(|s| &s.body == body)
}

fn manifest_actions(
    mut actions: MessageReader<UiAction>,
    mut sim: ResMut<Sim>,
    mut ui: ResMut<ManifestUi>,
    mut rig: ResMut<CameraRig>,
    mut warp: ResMut<TimeWarp>,
    world: Res<WorldData>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();
    // A question about something that has gone (crashed, say) is moot.
    let moot = match ui.confirm {
        Some(Confirm::Remove(id)) => sim.universe.vessel(id).is_none(),
        Some(Confirm::ClearDebris) => !sim
            .universe
            .vessels
            .iter()
            .any(|v| v.is_debris() && v.id() != sim.active_id()),
        None => false,
    };
    if moot {
        ui.confirm = None;
    }
    for action in actions.read() {
        let arg = action.arg().unwrap_or_default();
        let vessel = arg.parse().ok().map(VesselId);
        match action.name() {
            "manifest.filter" => match arg {
                "bodies" => ui.bodies = !ui.bodies,
                "vessels" => ui.vessels = !ui.vessels,
                "debris" => ui.debris = !ui.debris,
                _ => warn!("unknown manifest filter `{arg}`"),
            },
            "manifest.follow" => {
                if let Some(id) = vessel.filter(|&id| sim.universe.vessel(id).is_some()) {
                    rig.set_follow(Some(Follow::Vessel(id)), &sim);
                }
            }
            "manifest.follow_body" => {
                if let Some(body) = sim.universe.system.find(arg) {
                    rig.set_follow(Some(Follow::Body(body)), &sim);
                }
            }
            "manifest.remove" => {
                ui.confirm = vessel
                    .filter(|&id| id != sim.active_id() && sim.universe.vessel(id).is_some())
                    .map(Confirm::Remove);
            }
            "manifest.clear_debris" => ui.confirm = Some(Confirm::ClearDebris),
            "manifest.cancel" => ui.confirm = None,
            "manifest.confirm" => {
                let notice = match ui.confirm.take() {
                    Some(Confirm::Remove(id)) => {
                        let Some(vessel) = sim.universe.vessel(id) else {
                            continue;
                        };
                        let verb = if recoverable(vessel, &sim, &world) {
                            "Recovered"
                        } else {
                            "Destroyed"
                        };
                        let notice = format!("{verb} {}", vessel.name);
                        sim.universe.remove_vessel(id);
                        notice
                    }
                    Some(Confirm::ClearDebris) => match sim.universe.clear_debris() {
                        1 => "Cleared 1 piece of debris".to_owned(),
                        count => format!("Cleared {count} pieces of debris"),
                    },
                    None => continue,
                };
                warp.notice = Some((notice, now));
            }
            _ => {}
        }
    }
}

fn publish_manifest(
    sim: Res<Sim>,
    rig: Res<CameraRig>,
    ui: Res<ManifestUi>,
    world: Res<WorldData>,
    mut data: ResMut<UiData>,
) {
    let system = &sim.universe.system;
    let now = sim.render_time;
    let active = sim.active_vessel();
    let target = active.target;
    // Distances and speeds are from what the camera follows.
    let (frame, reference) = match rig.follow {
        Some(Follow::Body(body)) => (body, StateVector::default()),
        _ => {
            let viewed = rig.viewed_vessel(&sim);
            (viewed.parent(), viewed.state_at(system, now))
        }
    };
    let relative = |body: BodyId, state: StateVector| {
        let r = system.transform(state, body, frame, now) - reference;
        (r.position.length(), r.velocity.length())
    };
    let followed = |what: Follow| match rig.follow {
        Some(follow) => follow == what,
        None => what == Follow::Vessel(active.id()),
    };

    let mut rows: Vec<UiRecord> = Vec::new();
    if ui.bodies {
        for body in system.bodies() {
            let apsides = body.orbit.map(|o| (o.apoapsis(), o.periapsis()));
            let (distance, speed) = relative(body.id, StateVector::default());
            rows.push(record([
                ("name", body.name.as_str().into()),
                ("kind", "body".into()),
                ("is_body", true.into()),
                ("is_debris", false.into()),
                ("key", body.name.as_str().into()),
                (
                    "parent",
                    body.parent
                        .map_or("-".to_owned(), |p| system.body(p).name.clone())
                        .into(),
                ),
                ("apoapsis", apsides.and_then(|a| a.0).into()),
                ("periapsis", apsides.map(|a| a.1).into()),
                ("distance", distance.into()),
                ("speed", speed.into()),
                ("is_target", (target == Some(Target::Body(body.id))).into()),
                ("is_active", false.into()),
                ("is_followed", followed(Follow::Body(body.id)).into()),
                ("target_body", (body.id != active.parent()).into()),
                ("target_vessel", false.into()),
                ("can_fly", false.into()),
                ("can_recover", false.into()),
                ("can_destroy", false.into()),
                ("confirming", false.into()),
                ("confirm_text", "".into()),
            ]));
        }
    }
    let mut debris = 0;
    for other in &sim.universe.vessels {
        let id = other.id();
        let is_active = id == active.id();
        if other.is_debris() && !is_active {
            debris += 1;
        }
        let shown = if other.is_debris() {
            ui.debris
        } else {
            ui.vessels
        };
        if !shown && !is_active {
            continue;
        }
        let body = system.body(other.parent());
        let orbit = (!other.is_landed()).then(|| other.orbit(system, now));
        let (distance, speed) = relative(other.parent(), other.state_at(system, now));
        let recover = !is_active && recoverable(other, &sim, &world);
        let confirming = ui.confirm == Some(Confirm::Remove(id));
        let kind = if other.is_debris() {
            "debris"
        } else {
            "vessel"
        };
        rows.push(record([
            ("name", other.name.as_str().into()),
            ("kind", kind.into()),
            ("is_body", false.into()),
            ("is_debris", other.is_debris().into()),
            ("key", id.0.to_string().into()),
            ("parent", body.name.as_str().into()),
            (
                "apoapsis",
                orbit
                    .and_then(|o| o.apoapsis())
                    .map(|a| a - body.radius)
                    .into(),
            ),
            (
                "periapsis",
                orbit.map(|o| o.periapsis() - body.radius).into(),
            ),
            ("distance", distance.into()),
            ("speed", speed.into()),
            ("is_target", (target == Some(Target::Vessel(id))).into()),
            ("is_active", is_active.into()),
            ("is_followed", followed(Follow::Vessel(id)).into()),
            ("target_body", false.into()),
            ("target_vessel", (!is_active).into()),
            ("can_fly", (!is_active).into()),
            ("can_recover", recover.into()),
            ("can_destroy", (!is_active && !recover).into()),
            ("confirming", confirming.into()),
            (
                "confirm_text",
                if recover { "Recover?" } else { "Destroy?" }.into(),
            ),
        ]));
    }
    data.set("manifest.items", rows);
    data.set("manifest.show_bodies", ui.bodies);
    data.set("manifest.show_vessels", ui.vessels);
    data.set("manifest.show_debris", ui.debris);
    data.set("manifest.debris_count", debris as f64);
    data.set(
        "manifest.confirm_clear",
        ui.confirm == Some(Confirm::ClearDebris),
    );
    data.set(
        "manifest.following",
        match rig.follow {
            Some(Follow::Body(body)) => system.body(body).name.clone(),
            Some(Follow::Vessel(_)) => rig.viewed_vessel(&sim).name.clone(),
            None => String::new(),
        },
    );
}
