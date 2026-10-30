//! Part windows, as in KSP: right-click a part in flight view and a window opens beside the
//! pointer with what the part is and what it is doing: an engine's thrust and state, what
//! a tank holds and how much is left, how hard the structure is working. A click anywhere
//! else closes it unless it is pinned; pinned windows stay, one per part, until closed.
//! Right-clicking the part again closes its window too.
//!
//! The window is the `part_info` layout, an instanced popup (see `osp_ui::layout`), and
//! the parts with a window open publish their data under `part_info.<vessel>-<part>.`. A
//! window closes when its part leaves the vessel (staged or broken off) or the vessel is
//! gone, and when a scenario starts.
//!
//! Visuals carry the part they show ([`ShowsPart`]), on them or on what they hang from.

use std::collections::BTreeMap;

use bevy::{
    math::DVec3,
    picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings},
    prelude::*,
};
use osp_sim::{
    data::{ShellProfile, TankShape},
    gear::Leg,
    parts::{Part, PartId, PartKind, PartTree},
    propulsion::Section,
    staging::Activation,
    structure::{self, Mode},
    vessel::{Vessel, VesselId},
};
use osp_ui::{
    CLICK_SLOP, LayoutVisibility, UiAction, UiData, UiValue, instance_data, instance_id, record,
};

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera},
    controls::InputLocked,
    menu::Menu,
    orbit_lines::ViewPointer,
    part_highlight::PartMarks,
    sim::{Sim, SimRestarted},
};

/// The window's layout, opened once per part.
pub(crate) const LAYOUT: &str = "part_info";

pub struct PartInfoPlugin;

impl Plugin for PartInfoPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (pick_part, open_tile_part).in_set(FrameSet::Input))
            .add_systems(
                Update,
                publish_parts
                    .after(crate::part_highlight::sync_marks)
                    .in_set(FrameSet::Render),
            );
    }
}

/// The part a visual shows, for picking it.
#[derive(Component, Clone, Copy)]
pub struct ShowsPart {
    pub vessel: VesselId,
    pub part: PartId,
}

/// The parts of craft stage `stage` that `kind` picks, in the order its [`Section`] lists
/// them (it is gathered from the tree in this order).
pub fn stage_parts(tree: &PartTree, stage: usize, kind: fn(&PartKind) -> bool) -> Vec<PartId> {
    tree.stage(stage)
        .filter(|p| kind(&p.kind))
        .map(|p| p.id)
        .collect()
}

/// The part of each gear leg of a stack, in the order of [`osp_sim::gear::legs`]. A part
/// marked `mirror` has two legs (craft files list each leg as a part of its own).
pub fn leg_parts(tree: &PartTree, stages: &[Section]) -> Vec<Option<PartId>> {
    let mut parts = Vec::new();
    for stage in stages {
        let ids = stage_parts(tree, stage.id, is_gear);
        for (g, def) in stage.gear.iter().enumerate() {
            let legs = Leg::of(def, stage.diameter / 2.0, 0.0).len();
            parts.extend(std::iter::repeat_n(ids.get(g).copied(), legs));
        }
    }
    parts
}

/// The left half of a mirrored wing: the stage's `Mirror` part across the vessel's plane
/// of symmetry from `wing`.
pub fn mirror_half(tree: &PartTree, wing: &Part) -> Option<PartId> {
    let across = wing.position * DVec3::new(-1.0, 1.0, 1.0);
    tree.stage(wing.stage)
        .filter(|p| matches!(p.kind, PartKind::Mirror))
        .min_by(|a, b| {
            let (a, b) = (a.position.distance(across), b.position.distance(across));
            a.total_cmp(&b)
        })
        .map(|p| p.id)
}

pub fn is_wing(kind: &PartKind) -> bool {
    matches!(kind, PartKind::Wing(_))
}

pub fn is_fins(kind: &PartKind) -> bool {
    matches!(kind, PartKind::Fins(_))
}

fn is_gear(kind: &PartKind) -> bool {
    matches!(kind, PartKind::Gear(_))
}

fn is_tank(kind: &PartKind) -> bool {
    matches!(kind, PartKind::Tank(_))
}

/// A part's window key: `<vessel>-<part>`.
pub(crate) fn key(vessel: VesselId, part: PartId) -> String {
    format!("{}-{}", vessel.0, part.0)
}

pub(crate) fn parse_key(key: &str) -> Option<(VesselId, PartId)> {
    let (vessel, part) = key.split_once('-')?;
    Some((VesselId(vessel.parse().ok()?), PartId(part.parse().ok()?)))
}

/// A right-click in place (a drag turns the camera) on a part in flight view opens its
/// window, or closes it if it is open.
#[allow(clippy::too_many_arguments)]
fn pick_part(
    buttons: Res<ButtonInput<MouseButton>>,
    rig: Res<CameraRig>,
    locked: Res<InputLocked>,
    menu: Res<Menu>,
    pointer: ViewPointer,
    camera: Single<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut ray_cast: MeshRayCast,
    parts: Query<&ShowsPart>,
    parents: Query<&ChildOf>,
    mut shown: ResMut<LayoutVisibility>,
    mut press: Local<Option<Vec2>>,
) {
    if rig.map_view || locked.0 || menu.open {
        *press = None;
        return;
    }
    let cursor = pointer.position();
    if buttons.just_pressed(MouseButton::Right) {
        *press = cursor;
    }
    if !buttons.just_released(MouseButton::Right) {
        return;
    }
    let (Some(from), Some(at)) = (press.take(), cursor) else {
        return;
    };
    if from.distance(at) >= CLICK_SLOP {
        return;
    }
    let (camera, eye) = *camera;
    let Ok(ray) = camera.viewport_to_world(eye, at) else {
        return;
    };
    // What a mesh shows is tagged on it or above it (a model's meshes are deep in its
    // scene); plumes and the rest of the world are not parts.
    let tagged = |e: Entity| {
        std::iter::once(e)
            .chain(parents.iter_ancestors(e))
            .find(|&a| parts.contains(a))
    };
    let filter = |e: Entity| tagged(e).is_some();
    let settings = MeshRayCastSettings::default().with_filter(&filter);
    let Some((mesh, _)) = ray_cast.cast_ray(ray, &settings).first() else {
        return;
    };
    let picked = tagged(*mesh)
        .and_then(|tag| parts.get(tag).ok())
        .map(|shows| (shows.vessel, shows.part));
    if let Some((vessel, part)) = picked {
        shown.toggle(&instance_id(LAYOUT, &key(vessel, part)));
    }
}

/// A right-click on a tile of the staging window (`part.group:<vessel>:<part>,<part>,...`)
/// opens the window of the first part of the tile, or closes it if it is open, and marks all
/// of the tile's parts, not only that one.
fn open_tile_part(
    mut actions: MessageReader<UiAction>,
    mut shown: ResMut<LayoutVisibility>,
    mut marks: ResMut<PartMarks>,
) {
    for action in actions.read().filter(|a| a.name() == "part.group") {
        let Some((vessel, parts)) = parse_group(action.arg().unwrap_or_default()) else {
            warn!("cannot read `{}`", action.0);
            continue;
        };
        let Some(&first) = parts.first() else {
            continue;
        };
        let (key, id) = (key(vessel, first), instance_id(LAYOUT, &key(vessel, first)));
        if !shown.shown(&id) {
            marks.open_for_tile(&key, vessel, parts);
        }
        shown.toggle(&id);
    }
}

/// The vessel and parts `<vessel>:<part>,<part>,...` names.
fn parse_group(arg: &str) -> Option<(VesselId, Vec<PartId>)> {
    let (vessel, parts) = arg.split_once(':')?;
    let parts: Option<Vec<_>> = parts
        .split(',')
        .map(|p| Some(PartId(p.parse().ok()?)))
        .collect();
    Some((VesselId(vessel.parse().ok()?), parts?))
}

/// Fill in the open windows, and close those whose part is gone.
fn publish_parts(
    sim: Res<Sim>,
    marks: Res<PartMarks>,
    mut restarted: MessageReader<SimRestarted>,
    mut shown: ResMut<LayoutVisibility>,
    mut data: ResMut<UiData>,
) {
    // A new scenario numbers its vessels from the start again.
    let restart = restarted.read().count() > 0;
    let keys: Vec<String> = shown.instances(LAYOUT).map(str::to_owned).collect();
    for key in keys {
        let found = parse_key(&key)
            .filter(|_| !restart)
            .and_then(|(vessel, part)| {
                let vessel = sim.universe.vessel(vessel)?;
                Some((vessel, vessel.propulsion.parts().get(part)?))
            });
        match found {
            Some((vessel, part)) => {
                let prefix = instance_data(LAYOUT, &key);
                publish_part(&mut data, &prefix, vessel, part);
                // The colour that ties the window to its part on the vessel.
                let color: &str = marks.color_of_window(&key).unwrap_or_default();
                data.set(format!("{prefix}mark"), UiValue::from(color));
            }
            None => {
                shown.layouts.insert(instance_id(LAYOUT, &key), false);
            }
        }
    }
}

/// What a part is and what it is doing, as fields under `prefix`. `type` says which of
/// the per-kind fields there are.
fn publish_part(data: &mut UiData, prefix: &str, vessel: &Vessel, part: &Part) {
    let mut set = |field: &str, value: UiValue| data.set(format!("{prefix}{field}"), value);
    let tree = vessel.propulsion.parts();
    let stages = &vessel.propulsion.sections;
    // Its stage's place in the stack, bottom first, and its place among the stage's parts
    // of its kind: where the stage keeps its state.
    let index = stages.iter().position(|s| s.id == part.stage);
    let stage = index.map(|i| &stages[i]);
    let nth = |kind: fn(&PartKind) -> bool| {
        stage_parts(tree, part.stage, kind)
            .iter()
            .position(|&id| id == part.id)
            .unwrap_or(0)
    };
    let load = |which: structure::Part| {
        let load = index.and_then(|s| {
            vessel
                .structure
                .iter()
                .find(|l| l.stage == s && l.part == which)
        });
        let mode = load.map(|l| match l.mode {
            Mode::Buckling => "buckling",
            Mode::Burst => "pressure",
            Mode::Bending => "bending",
        });
        (load.map(|l| l.fraction).into(), mode.into())
    };
    // The left half of a mirrored wing shows its wing, which flies and weighs both.
    let kind = match part.kind {
        PartKind::Mirror => tree
            .stage(part.stage)
            .find(|w| is_wing(&w.kind) && mirror_half(tree, w) == Some(part.id))
            .map_or(&part.kind, |w| &w.kind),
        _ => &part.kind,
    };

    let (type_, label, mass) = match kind {
        PartKind::Hull { structure_mass, .. } => ("hull", "Stage hull", *structure_mass),
        PartKind::Decoupler => ("decoupler", "Decoupler", 0.0),
        PartKind::Tank(tank) => ("tank", "Tank", tank.dry_mass),
        PartKind::Engine(def) => ("engine", "Engine", def.mass),
        PartKind::Rcs(def) => ("rcs", "RCS thrusters", def.mass),
        PartKind::ReactionWheel(def) => ("wheel", "Reaction wheel", def.mass),
        PartKind::Pod(def) => ("pod", "Crew pod", def.mass),
        // Each fin is a part of its own, carrying the whole set.
        PartKind::Fins(fins) => {
            let parts = stage_parts(tree, part.stage, is_fins).len().max(1);
            ("fins", "Fin", fins.mass / parts as f64)
        }
        PartKind::Wing(def) => {
            let halves = if def.mirror { 2.0 } else { 1.0 };
            ("wing", "Wing", def.mass / halves)
        }
        PartKind::Gear(def) => ("gear", "Landing gear", def.mass),
        PartKind::Shell(shell) => ("shell", "Shell", shell.mass),
        PartKind::FairingHalf { mass, .. } => ("fairing", "Fairing half", *mass),
        PartKind::Mirror => ("wing", "Wing", 0.0),
        PartKind::Structure { mass } => ("structure", "Structure", *mass),
    };
    let catalog = match kind {
        PartKind::Engine(def) => Some(&def.catalog),
        PartKind::Pod(def) => Some(&def.catalog),
        _ => None,
    };
    let title = catalog.and_then(|c| c.title.as_deref());
    set("name", title.unwrap_or(&part.name).into());
    set("type", type_.into());
    set("kind", label.into());
    set("mass", mass.into());
    set("vessel", vessel.name.as_str().into());
    set("stage", stage.map_or("", |s| s.name.as_str()).into());
    let text = |pick: fn(&osp_sim::data::CatalogInfo) -> &Option<String>| {
        catalog.and_then(|c| pick(c).as_deref()).unwrap_or_default()
    };
    set("manufacturer", text(|c| &c.manufacturer).into());
    set("description", text(|c| &c.description).into());
    // What can be done to it now: a button each, `do` being the `part.do` action's argument.
    let actions: Vec<_> = (vessel.part_actions(part.id).into_iter())
        .map(|a| {
            let target = tree.get(a.part()).map_or("", |p| p.name.as_str());
            let label = match a {
                Activation::Decoupler(p) if p != part.id => format!("{} {target}", a.verb()),
                _ => a.verb().to_string(),
            };
            let id = format!("{}:{}:{}", vessel.id().0, a.kind(), a.part().0);
            record([("label", label.into()), ("do", id.into())])
        })
        .collect();
    set("has_actions", (!actions.is_empty()).into());
    set("actions", actions.into());

    match kind {
        PartKind::Hull {
            diameter, length, ..
        } => {
            set("diameter", (*diameter).into());
            set("length", (*length).into());
        }
        PartKind::Decoupler | PartKind::Structure { .. } => {}
        PartKind::Tank(tank) => {
            let size = match tank.shape {
                TankShape::Cylinder => {
                    format!("{:.2} x {:.2} m", 2.0 * tank.radius, tank.length)
                }
                TankShape::Sphere => format!("{:.2} m sphere", 2.0 * tank.radius),
            };
            set("size", size.into());
            set("material", tank.material.as_str().into());
            set("volume", tank.volume.into());
            set("wall", (tank.wall * 1e3).into());
            set("max_pressure", (tank.max_pressure / 1e5).into());
            set("max_axial_load", tank.max_axial_load.into());
            // What it holds now.
            let live = vessel.propulsion.tanks.iter().find(|t| t.part == part.id);
            let left = |resource: &str| live.map_or(0.0, |t| t.fill(resource));
            let mut held = 0.0;
            let rows: Vec<_> = tank
                .contents
                .iter()
                .map(|c| {
                    let fraction = left(&c.resource);
                    held += c.capacity * fraction;
                    record([
                        ("name", c.resource.as_str().into()),
                        ("amount", (c.capacity * fraction).into()),
                        ("capacity", c.capacity.into()),
                        ("fraction", fraction.into()),
                    ])
                })
                .collect();
            set("resources", rows.into());
            set("contents_mass", held.into());
            let (load, mode) = load(structure::Part::Tank(nth(is_tank)));
            set("load", load);
            set("mode", mode);
        }
        PartKind::Engine(def) => {
            let engine = vessel.propulsion.engines.iter().find(|e| e.part == part.id);
            let armed = vessel.propulsion.firing().contains(&part.id);
            // Armed, but a stage under it keeps its exhaust in until it falls away.
            let blocked = !armed && vessel.propulsion.engine_armed(part.id);
            let status = match engine {
                Some(e) if e.running && e.spooling() => "Starting",
                Some(e) if e.running => "Running",
                Some(e) if e.level > 0.0 => "Shutting down",
                Some(e) if e.ignitions_left == Some(0) => "No ignitions left",
                Some(_) if armed => "Armed",
                Some(_) if blocked => "Blocked by the stage below",
                _ => "Off",
            };
            set("status", status.into());
            set("lit", engine.is_some_and(|e| e.level > 0.0).into());
            set("thrust", engine.map_or(0.0, |e| e.thrust).into());
            set("level", engine.map_or(0.0, |e| e.level).into());
            set("thrust_vac", def.thrust_vac.into());
            let sea_level = def.thrust_vac * def.isp_sl / def.isp_vac.max(1e-9);
            set("thrust_sl", sea_level.into());
            let pressure = vessel.flight.static_pressure;
            set(
                "isp",
                engine.map_or(def.isp_vac, |e| e.isp(pressure)).into(),
            );
            set("isp_vac", def.isp_vac.into());
            set("isp_sl", def.isp_sl.into());
            set("min_throttle", def.min_throttle.into());
            let gimbal = engine.map_or(0.0, |e| e.deflection.length().to_degrees());
            set("gimbal", gimbal.into());
            set("gimbal_range", def.gimbal_range.into());
            let ignitions = engine.map_or(def.ignitions, |e| e.ignitions_left);
            set("ignitions", ignitions.into());
            set("limited", ignitions.is_some().into());
            set("propellants", propellants(&def.propellants).into());
        }
        PartKind::Rcs(def) => {
            let block = vessel.propulsion.rcs.iter().find(|b| b.part == part.id);
            let firing = block.map_or(0.0, |b| b.firing.iter().copied().fold(0.0, f64::max));
            set("on", vessel.rcs().into());
            set("firing", firing.into());
            set("nozzle_thrust", def.thrust.into());
            set("isp_vac", def.isp_vac.into());
            set("isp_sl", def.isp_sl.into());
            set("propellants", propellants(&def.propellants).into());
        }
        PartKind::ReactionWheel(def) => set("torque", def.torque.into()),
        PartKind::Pod(def) => {
            set("crew", def.crew.into());
            set("diameter", def.diameter.into());
            set("height", def.height.into());
        }
        PartKind::Fins(fins) => {
            let d = &fins.def;
            set("count", d.count.into());
            set("area", fins.area.into());
            set("span", d.span.into());
            set("thickness", (d.thickness * 1e3).into());
            set("material", d.material.as_str().into());
            set("max_fin_force", fins.max_force.into());
            let (load, mode) = load(structure::Part::Fins);
            set("load", load);
            set("mode", mode);
        }
        PartKind::Wing(def) => {
            set("span", def.span.into());
            set(
                "area",
                ((def.root_chord + def.tip_chord) / 2.0 * def.span).into(),
            );
            set("sweep", def.sweep.into());
            set("mirrored", def.mirror.into());
            set("control", def.control.is_some().into());
            let deflection = def.control.as_ref().map(|c| c.deflection);
            set("deflection", deflection.into());
        }
        PartKind::Gear(def) => {
            // How far into its travel the strut is: the most of its legs.
            let travel = leg_parts(tree, stages)
                .iter()
                .zip(vessel.gear_compression())
                .filter(|(id, _)| **id == Some(part.id))
                .map(|(_, &c)| c)
                .fold(0.0, f64::max);
            set("compression", (travel / def.stroke.max(1e-9)).into());
            set("length", def.length.into());
            set("stroke", def.stroke.into());
            set("rated_mass", def.load.into());
            set("brakes", def.brakes.into());
            set("steering", def.steering.into());
            set("mirrored", def.mirror.into());
        }
        PartKind::Shell(shell) => {
            set("profile", profile(shell.profile).into());
            set("length", shell.length.into());
            set("bottom_diameter", (2.0 * shell.bottom_radius).into());
            set("top_diameter", (2.0 * shell.top_radius).into());
        }
        PartKind::FairingHalf { radius, length, .. } => {
            set("diameter", (2.0 * radius).into());
            set("length", (*length).into());
        }
        // Without its wing: only the name.
        PartKind::Mirror => {}
    }
}

/// What an engine or RCS block burns: `"LqdOxygen + Kerosene"`.
fn propellants(ratios: &BTreeMap<String, f64>) -> String {
    ratios
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" + ")
}

fn profile(profile: ShellProfile) -> &'static str {
    match profile {
        ShellProfile::Conic => "Conic",
        ShellProfile::Ogive => "Tangent ogive",
        ShellProfile::Elliptical => "Elliptical",
        ShellProfile::Parabolic => "Parabolic",
        ShellProfile::Haack => "Von Karman",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_names_its_vessel_and_parts() {
        assert_eq!(
            parse_group("3:7,8,9"),
            Some((VesselId(3), vec![PartId(7), PartId(8), PartId(9)]))
        );
        assert_eq!(parse_group("3:7"), Some((VesselId(3), vec![PartId(7)])));
        assert_eq!(parse_group("3:"), None);
        assert_eq!(parse_group("3"), None);
    }

    #[test]
    fn keys_name_a_part_of_a_vessel() {
        let key = key(VesselId(12), PartId(7));
        assert_eq!(key, "12-7");
        assert_eq!(parse_key(&key), Some((VesselId(12), PartId(7))));
        assert_eq!(parse_key("12"), None);
    }
}
