//! The transfer planner (F4): a porkchop plot of delta-v against departure time and flight
//! time to the target, and a burn made from the transfer picked on it.
//!
//! Within one sphere of influence (a moon, another vessel) the burn is the exact Lambert
//! departure from the vessel's orbit. To another planet the plot is of the planet-to-planet
//! transfer and the burn is the ejection from the vessel's parking orbit (see
//! [`osp_sim::transfer`]).
//!
//! | Action | Effect |
//! | --- | --- |
//! | `transfer.compute` | Plot transfers to the current target |
//! | `transfer.best` | Pick the cheapest |
//! | `transfer.create` | Plan the departure burn of the picked transfer |

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
    body::BodyId,
    orbit::Orbit,
    rendezvous::Target,
    transfer::{
        Porkchop, TransferRequest, departure_burn, ejection_burn, hohmann_time, porkchop,
        synodic_period,
    },
};
use osp_ui::{UiAction, UiData, WidgetContext, WidgetRegistry};

use crate::{
    FrameSet,
    flight_plan::PlanUi,
    sim::{Sim, TimeWarp},
};

const COLUMNS: usize = 72;
const ROWS: usize = 54;
/// Longest departure window plotted, s.
const MAX_WINDOW: f64 = 2.0 * 365.25 * 86_400.0;

pub struct TransferPlannerPlugin;

impl Plugin for TransferPlannerPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut()
            .resource_mut::<WidgetRegistry>()
            .register("porkchop", porkchop_widget);
        app.init_resource::<Planner>()
            .add_systems(Startup, make_image)
            .add_systems(Update, planner_actions.in_set(FrameSet::Input))
            .add_systems(
                Update,
                (attach_image, finish_task, pick_cell, paint, publish)
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
            "transfer.compute" => match request(&sim) {
                Ok((request, meta)) => {
                    let task = AsyncComputeTaskPool::get().spawn(async move { porkchop(&request) });
                    planner.task = Some((task, meta));
                }
                Err(why) => warp.notice = Some((why.into(), real)),
            },
            "transfer.best" => {
                if let Some(plot) = &planner.chop {
                    planner.selected = plot.chop.best().map(|(c, r, _)| (c, r));
                    planner.dirty = true;
                }
            }
            "transfer.create" => {
                let Some(plot) = &planner.chop else { continue };
                let Some(transfer) = planner
                    .selected
                    .and_then(|(c, r)| plot.chop.cell(c, r))
                    .copied()
                else {
                    continue;
                };
                let meta = &plot.meta;
                let burn = if meta.interplanetary {
                    transfer
                        .excess
                        .and_then(|v| ejection_burn(&meta.vessel_orbit, v, transfer.departure))
                } else {
                    departure_burn(&meta.vessel_orbit, &transfer)
                };
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
    if vessel.is_landed() {
        return Err("Transfer planner: get into orbit first");
    }
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
    let plot = planner.chop.as_ref();
    data.set("transfer.has", plot.is_some());
    let Some(plot) = plot else { return };
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
