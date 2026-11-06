//! The transfer planner (F4): a porkchop plot of delta-v against departure time and flight
//! time to the target, and a burn made from the transfer picked on it.
//!
//! Within one sphere of influence (a moon, another vessel) the burn is the exact Lambert
//! departure from the vessel's orbit. To another planet the plot is of the planet-to-planet
//! transfer and the burn is the ejection from the vessel's parking orbit (see
//! [`osp_sim::transfer`]).
//!
//! From the ground there is no orbit to plot from yet. There it says when to launch for a target
//! around the same body, and which way to fly to end up in the plane that reaches it, at any time and
//! at the best times to come.
//!
//! | Action | Effect |
//! | --- | --- |
//! | `transfer.compute` | Plot transfers to the current target, or from the ground list launch times |
//! | `transfer.best` | Pick the cheapest |
//! | `transfer.create` | Plan the departure burn of the picked transfer |
//! | `transfer.alarm` | Set an alarm for just before the picked transfer's departure burn starts |
//! | `transfer.alarm_launch:<index>` | Set an alarm for just before that launch time |
//!
//! It does not warp: its times go on the alarm clock (`crate::alarm_clock`), which warps to them.

use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    input::mouse::MouseButton,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    tasks::{AsyncComputeTaskPool, Task, block_on, poll_once},
    ui::RelativeCursorPosition,
};
use osp_sim::{
    alarm::AlarmTime,
    body::BodyId,
    orbit::Orbit,
    plan::Burn,
    rendezvous::Target,
    transfer::{
        LaunchRequest, LaunchWindow, Porkchop, TransferRequest, departure_burn, ejection_burn,
        hohmann_time, porkchop, synodic_period,
    },
    vessel::VesselId,
};
use osp_ui::{UiAction, UiData, UiRecord, WidgetContext, WidgetRegistry, record};

use crate::{
    FrameSet,
    alarm_clock::set_alarm,
    flight_plan::{PlanUi, WARP_TO_BURN},
    sim::{Sim, SimRestarted, TimeWarp},
    targeting::target_name,
};

const COLUMNS: usize = 72;
const ROWS: usize = 54;
/// Longest departure window plotted, s.
const MAX_WINDOW: f64 = 2.0 * 365.25 * 86_400.0;
/// How many launch times are listed.
const LAUNCH_TIMES: usize = 4;
/// An alarm for a launch time rings this long before it, s, to get ready.
const BEFORE_LAUNCH: f64 = 60.0;
/// A plane is only said to be missed by more than this, rad.
const WORTH_SAYING: f64 = 0.05 * std::f64::consts::PI / 180.0;

pub struct TransferPlannerPlugin;

impl Plugin for TransferPlannerPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut()
            .resource_mut::<WidgetRegistry>()
            .register("porkchop", porkchop_widget);
        app.init_resource::<Planner>()
            .add_systems(Startup, make_image)
            .add_systems(
                Update,
                (forget_on_restart, planner_actions)
                    .chain()
                    .in_set(FrameSet::Input),
            )
            .add_systems(
                Update,
                (
                    attach_image,
                    finish_task,
                    keep_launch_times,
                    pick_cell,
                    paint,
                    publish,
                )
                    .chain()
                    .in_set(FrameSet::Render),
            );
    }
}

#[derive(Resource, Default)]
struct Planner {
    chop: Option<Plot>,
    /// A plot being computed off the main thread, and what it is for.
    task: Option<(Task<Porkchop>, Meta)>,
    selected: Option<(usize, usize)>,
    image: Handle<Image>,
    /// The plot changed and needs painting.
    dirty: bool,
    /// What a vessel on the ground gets in place of a plot.
    launch: Option<LaunchTimes>,
}

/// When to launch for a target, from where a vessel stands.
struct LaunchTimes {
    vessel: VesselId,
    target: Target,
    request: LaunchRequest,
    windows: Vec<LaunchWindow>,
}

struct Plot {
    chop: Porkchop,
    meta: Meta,
}

/// What a plot was computed for.
#[derive(Clone, Copy)]
struct Meta {
    target: Target,
    /// Vessel orbit at the time (to make the burn), and the central body of the plot.
    vessel_orbit: Orbit,
    central: BodyId,
    interplanetary: bool,
}

/// `widget: "porkchop"`: shows the plot; size it with `style.width` / `height`. Click to
/// pick a transfer.
#[derive(Component)]
struct PorkchopView;

fn porkchop_widget(ctx: &mut WidgetContext) -> Entity {
    ctx.commands
        .entity(ctx.entity)
        .insert((PorkchopView, RelativeCursorPosition::default()));
    ctx.entity
}

fn make_image(mut planner: ResMut<Planner>, mut images: ResMut<Assets<Image>>) {
    let mut image = Image::new_fill(
        Extent3d {
            width: COLUMNS as u32,
            height: ROWS as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[12, 14, 18, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    planner.image = images.add(image);
}

fn attach_image(
    mut commands: Commands,
    planner: Res<Planner>,
    views: Query<Entity, (With<PorkchopView>, Without<ImageNode>)>,
) {
    for view in &views {
        commands
            .entity(view)
            .insert(ImageNode::new(planner.image.clone()));
    }
}

fn planner_actions(
    mut actions: MessageReader<UiAction>,
    mut sim: ResMut<Sim>,
    mut planner: ResMut<Planner>,
    mut plan_ui: ResMut<PlanUi>,
    mut warp: ResMut<TimeWarp>,
    time: Res<Time>,
) {
    let real = time.elapsed_secs_f64();
    for action in actions.read() {
        match action.name() {
            "transfer.compute" if sim.active_vessel().is_landed() => match launch_times(&sim) {
                Ok(times) => {
                    planner.task = None;
                    planner.chop = None;
                    planner.selected = None;
                    planner.launch = Some(times);
                }
                Err(why) => warp.notice = Some((why.into(), real)),
            },
            "transfer.compute" => match request(&sim) {
                Ok((request, meta)) => {
                    let task = AsyncComputeTaskPool::get().spawn(async move { porkchop(&request) });
                    planner.task = Some((task, meta));
                }
                Err(why) => warp.notice = Some((why.into(), real)),
            },
            "transfer.alarm_launch" => {
                let index: Option<usize> = action.arg().and_then(|index| index.parse().ok());
                let times = planner.launch.as_ref();
                let window =
                    index.and_then(|index| Some((times?.windows.get(index)?, times?.target)));
                let Some((window, target)) = window else {
                    warn!("no launch time in `{}`", action.0);
                    continue;
                };
                let time = window.time.add_seconds(-BEFORE_LAUNCH);
                let title = for_target("Launch", &sim, target);
                let vessel = Some(sim.active_id());
                set_alarm(
                    &mut sim,
                    &mut warp,
                    real,
                    title,
                    vessel,
                    AlarmTime::At(time),
                );
            }
            "transfer.alarm" => {
                let Some((burn, target)) = picked_burn(&planner) else {
                    continue;
                };
                let vessel = sim.active_vessel();
                let start = (vessel.burn_timing(&burn, 0.0)).map_or(burn.time, |t| t.start);
                let time = start.add_seconds(-WARP_TO_BURN);
                let title = for_target("Departure", &sim, target);
                let vessel = Some(vessel.id());
                set_alarm(
                    &mut sim,
                    &mut warp,
                    real,
                    title,
                    vessel,
                    AlarmTime::At(time),
                );
            }
            "transfer.best" => {
                if let Some(plot) = &planner.chop {
                    planner.selected = plot.chop.best().map(|(c, r, _)| (c, r));
                    planner.dirty = true;
                }
            }
            "transfer.create" => {
                let burn = picked_burn(&planner).map(|(burn, _)| burn);
                match burn.filter(|b| b.time > sim.universe.time()) {
                    Some(burn) => {
                        plan_ui.selected = Some(sim.active_vessel_mut().plan.add(burn));
                    }
                    None => {
                        warp.notice = Some(("That departure has passed: recompute".into(), real));
                    }
                }
            }
            _ => {}
        }
    }
}

/// The departure burn of the transfer picked on the plot, and the target it is for.
fn picked_burn(planner: &Planner) -> Option<(Burn, Target)> {
    let plot = planner.chop.as_ref()?;
    let (column, row) = planner.selected?;
    let transfer = plot.chop.cell(column, row)?;
    let meta = &plot.meta;
    let burn = if meta.interplanetary {
        let excess = transfer.excess?;
        ejection_burn(&meta.vessel_orbit, excess, transfer.departure)
    } else {
        departure_burn(&meta.vessel_orbit, transfer)
    };
    Some((burn?, meta.target))
}

/// "Launch for the Moon": an alarm's title, for what is done for a target.
fn for_target(what: &str, sim: &Sim, target: Target) -> String {
    match target_name(sim, target) {
        Some(name) => format!("{what} for {name}"),
        None => what.to_owned(),
    }
}

/// A game started afresh, or gone back to: what was planned was for what is gone.
fn forget_on_restart(mut restarted: MessageReader<SimRestarted>, mut planner: ResMut<Planner>) {
    if restarted.read().count() > 0 {
        planner.chop = None;
        planner.task = None;
        planner.selected = None;
        planner.launch = None;
    }
}

/// The launch times for the vessel being flown, while it is on the ground.
fn launch_times(sim: &Sim) -> Result<LaunchTimes, &'static str> {
    let system = &sim.universe.system;
    let now = sim.universe.time();
    let vessel = sim.active_vessel();
    let target = vessel
        .target
        .ok_or("Transfer planner: choose a target first")?;
    let on_ground = (target.vessel())
        .and_then(|id| sim.universe.vessel(id))
        .is_some_and(|other| other.is_landed());
    if on_ground {
        return Err("Transfer planner: the target is on the ground");
    }
    let (frame, arrival) = sim
        .universe
        .target_orbit(target, now)
        .ok_or("Transfer planner: the target has no orbit")?;
    // Between planets the launch hardly matters and the parking orbit is what the plot starts from.
    if frame != vessel.parent() {
        return Err("Transfer planner: for that target, get into orbit first");
    }
    let body = system.body(frame);
    let site = body.orientation(now).inverse() * vessel.state_at(system, now).position;
    let request = LaunchRequest::new(body, frame, site, arrival)
        .ok_or("Transfer planner: needs a closed orbit to go to")?;
    Ok(LaunchTimes {
        vessel: vessel.id(),
        target,
        request,
        windows: request.windows(system, now, LAUNCH_TIMES),
    })
}

/// They go when the vessel leaves the ground or the target changes, and move on as they pass.
fn keep_launch_times(sim: Res<Sim>, mut planner: ResMut<Planner>) {
    let Some(times) = &mut planner.launch else {
        return;
    };
    let vessel = sim.active_vessel();
    let same = times.vessel == vessel.id() && Some(times.target) == vessel.target;
    if !same || !vessel.is_landed() {
        planner.launch = None;
        return;
    }
    let now = sim.render_time;
    if times.windows.first().is_some_and(|w| w.time <= now) {
        times.windows = times
            .request
            .windows(&sim.universe.system, now, LAUNCH_TIMES);
    }
}

/// `transfer.windows[]`, and what `transfer.now_*` says about leaving now.
fn window_record(index: usize, window: &LaunchWindow, now: osp_sim::time::SimTime) -> UiRecord {
    record([
        ("index", index.into()),
        ("in", window.time.seconds_since(now).into()),
        ("heading", heading_text(window.heading).into()),
        ("inclination", angle_text(window.inclination).into()),
        ("extra", window.extra.into()),
        ("misses", (window.miss > WORTH_SAYING).into()),
        ("miss", angle_text(window.miss).into()),
    ])
}

/// How a heading reads on the navball: 090.
fn heading_text(radians: f64) -> String {
    format!("{:03.0}", radians.to_degrees().rem_euclid(360.0))
}

fn angle_text(radians: f64) -> String {
    format!("{:.1}", radians.to_degrees())
}

/// Pick up a finished plot.
fn finish_task(mut planner: ResMut<Planner>) {
    let Some((task, _)) = &mut planner.task else {
        return;
    };
    let Some(chop) = block_on(poll_once(task)) else {
        return;
    };
    let (_, meta) = planner.task.take().unwrap();
    planner.selected = chop.best().map(|(c, r, _)| (c, r));
    planner.chop = Some(Plot { chop, meta });
    planner.dirty = true;
}

/// Set up the grid for the active vessel's target.
fn request(sim: &Sim) -> Result<(TransferRequest, Meta), &'static str> {
    let system = &sim.universe.system;
    let now = sim.universe.time();
    let vessel = sim.active_vessel();
    let target = vessel
        .target
        .ok_or("Transfer planner: choose a target first")?;
    let (target_frame, arrival) = sim
        .universe
        .target_orbit(target, now)
        .ok_or("Transfer planner: the target has no orbit")?;
    let vessel_orbit = vessel.orbit(system, now);
    let parent = vessel.parent();
    let planet = system.body(parent);
    let (central, departure, parking) = if target_frame == parent {
        (parent, vessel_orbit, None)
    } else if planet.parent == Some(target_frame) {
        let orbit = planet
            .orbit
            .ok_or("Transfer planner: no orbit to leave from")?;
        let radius = vessel.state_at(system, now).position.length();
        (target_frame, orbit, Some((planet.gm, radius)))
    } else {
        return Err("Transfer planner: the target must orbit the same body as you, or its parent");
    };
    let mu = departure.mu();
    let (r1, r2) = (departure.semi_major_axis(), arrival.semi_major_axis());
    if !(r1 > 0.0 && r2 > 0.0) {
        return Err("Transfer planner: needs closed orbits at both ends");
    }
    let t_h = hohmann_time(mu, r1, r2);
    let (p1, p2) = (departure.period().unwrap(), arrival.period().unwrap());
    let mut window = synodic_period(p1, p2).min(MAX_WINDOW).max(p1);
    // A target on another plane is cheap to reach only when its arrival point lies in the
    // parking plane, which takes up to half its period to come round.
    let tilt = departure.normal().angle_between(arrival.normal());
    if parking.is_none() && tilt > 0.5f64.to_radians() {
        window = window.max(p2 / 2.0).min(MAX_WINDOW);
    }
    // Over many parking orbits, each departure time stands for the best moment in its orbit.
    let phase_search = (parking.is_none() && window > 2.0 * p1).then_some(p1);
    let request = TransferRequest {
        departure,
        arrival,
        depart_from: now.add_seconds(60.0),
        depart_span: window,
        tof_min: 0.5 * t_h,
        tof_max: 1.5 * t_h,
        columns: COLUMNS,
        rows: ROWS,
        parking,
        floor: system.body(central).coast_floor(),
        phase_search,
    };
    let meta = Meta {
        target,
        vessel_orbit,
        central,
        interplanetary: parking.is_some(),
    };
    Ok((request, meta))
}

fn pick_cell(
    mut planner: ResMut<Planner>,
    buttons: Res<ButtonInput<MouseButton>>,
    views: Query<&RelativeCursorPosition, With<PorkchopView>>,
) {
    if !buttons.just_pressed(MouseButton::Left) || planner.chop.is_none() {
        return;
    }
    for cursor in &views {
        let Some(at) = cursor.normalized.filter(|_| cursor.cursor_over) else {
            continue;
        };
        // (0, 0) is the centre, (0.5, 0.5) the bottom right; short flights at the bottom.
        let column = ((at.x + 0.5) * COLUMNS as f32).clamp(0.0, COLUMNS as f32 - 1.0) as usize;
        let from_top = ((at.y + 0.5) * ROWS as f32).clamp(0.0, ROWS as f32 - 1.0) as usize;
        planner.selected = Some((column, ROWS - 1 - from_top));
        planner.dirty = true;
    }
}

/// Colour by total delta-v on a log scale from the cheapest (blue) to four times that (red).
fn paint(mut planner: ResMut<Planner>, mut images: ResMut<Assets<Image>>) {
    if !planner.dirty {
        return;
    }
    planner.dirty = false;
    let Some(plot) = &planner.chop else { return };
    let Some(mut image) = images.get_mut(&planner.image) else {
        return;
    };
    let best = plot.chop.best().map_or(1.0, |(_, _, t)| t.total()).max(1.0);
    let ramp = [
        [0.12, 0.2, 0.75],
        [0.1, 0.7, 0.9],
        [0.2, 0.8, 0.3],
        [0.95, 0.85, 0.2],
        [0.9, 0.2, 0.15],
    ];
    let mut data = vec![0u8; COLUMNS * ROWS * 4];
    for row in 0..ROWS {
        for column in 0..COLUMNS {
            let color = match plot.chop.cell(column, row) {
                Some(t) => {
                    let x = ((t.total() / best).ln() / 4f64.ln()).clamp(0.0, 1.0) * 3.999;
                    let (i, f) = (x as usize, (x.fract()) as f32);
                    let (a, b) = (ramp[i], ramp[(i + 1).min(4)]);
                    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * f)
                }
                None => [0.04, 0.04, 0.05],
            };
            let color = if planner.selected == Some((column, row)) {
                [1.0, 1.0, 1.0]
            } else {
                color
            };
            let pixel = ((ROWS - 1 - row) * COLUMNS + column) * 4;
            for k in 0..3 {
                data[pixel + k] = (color[k].powf(1.0 / 2.2) * 255.0).round() as u8;
            }
            data[pixel + 3] = 255;
        }
    }
    image.data = Some(data);
}

fn publish(sim: Res<Sim>, planner: Res<Planner>, mut data: ResMut<UiData>) {
    let now = sim.render_time;
    let system = &sim.universe.system;
    data.set("transfer.computing", planner.task.is_some());
    data.set("transfer.on_ground", planner.launch.is_some());
    if let Some(times) = &planner.launch {
        let leaving = times.request.at(system, now);
        data.set("transfer.launch_far", times.request.is_far());
        data.set("transfer.launch_flight", times.request.flight);
        data.set("transfer.now_heading", heading_text(leaving.heading));
        data.set("transfer.now_inclination", angle_text(leaving.inclination));
        data.set("transfer.now_extra", leaving.extra);
        data.set("transfer.now_misses", leaving.miss > WORTH_SAYING);
        data.set("transfer.now_miss", angle_text(leaving.miss));
        data.set("transfer.has_windows", !times.windows.is_empty());
        let windows: Vec<_> = (times.windows.iter().enumerate())
            .map(|(index, window)| window_record(index, window, now))
            .collect();
        data.set("transfer.windows", windows);
    }
    let plot = planner.chop.as_ref();
    data.set("transfer.has", plot.is_some());
    let Some(plot) = plot else {
        data.set("transfer.picked", false);
        data.set("transfer.stale", false);
        return;
    };
    let r = &plot.chop.request;
    let meta = &plot.meta;
    data.set(
        "transfer.kind",
        if meta.interplanetary {
            format!("Interplanetary, around {}", system.body(meta.central).name)
        } else {
            format!("Around {}", system.body(meta.central).name)
        },
    );
    data.set(
        "transfer.stale",
        sim.active_vessel().target != Some(meta.target),
    );
    data.set("transfer.window_from", r.depart_from.seconds_since(now));
    data.set(
        "transfer.window_to",
        r.depart_from.add_seconds(r.depart_span).seconds_since(now),
    );
    data.set("transfer.tof_min", r.tof_min);
    data.set("transfer.tof_max", r.tof_max);
    data.set(
        "transfer.best_total",
        plot.chop.best().map(|(_, _, t)| t.total()),
    );
    let picked = planner.selected.and_then(|(c, r)| plot.chop.cell(c, r));
    data.set("transfer.picked", picked.is_some());
    if let Some(t) = picked {
        data.set("transfer.departure_in", t.departure.seconds_since(now));
        data.set("transfer.tof", t.tof);
        data.set("transfer.depart_dv", t.depart_dv);
        data.set("transfer.arrive_dv", t.arrive_dv);
        data.set("transfer.total", t.total());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heading_reads_as_on_the_navball() {
        assert_eq!(heading_text(90f64.to_radians()), "090");
        assert_eq!(heading_text(7f64.to_radians()), "007");
        assert_eq!(heading_text(0.0), "000");
    }
}
