//! Owns the `Universe` and advances it each frame according to the time-warp setting.
//!
//! Low warp runs fixed physics ticks from an accumulator; the leftover fraction of a tick
//! becomes the render time offset, so rendering is smooth at any frame rate while physics
//! stays on an exact, frame-rate-independent tick grid. High warp jumps on rails and stops
//! exactly at events that need attention.

use std::{collections::HashMap, sync::Arc};

use bevy::prelude::*;
use osp_sim::{
    body::BodyId,
    data::LaunchSiteDef,
    system::SolarSystem,
    time::{SimTime, TICKS_PER_SECOND},
    universe::{PHYSICS_STEP, SimEvent, Universe},
    vessel::{Vessel, VesselEventKind},
};

use osp_terrain::Dataset;

use crate::{FrameSet, content::GameContent};

/// Warp rates. Rates up to `MAX_PHYSICS_WARP` run physics ticks; above that the universe
/// advances on rails, which requires every vessel to be coasting or landed.
pub const WARP_RATES: [f64; 10] = [
    1.0, 2.0, 3.0, 4.0, 10.0, 50.0, 100.0, 1_000.0, 10_000.0, 100_000.0,
];
pub const MAX_PHYSICS_WARP: f64 = 4.0;
/// Longest real frame time we simulate; avoids huge catch-up after a hitch.
const MAX_FRAME_TIME: f64 = 0.1;
/// Physics ticks per frame before the simulation is allowed to fall behind real time.
const MAX_TICKS_PER_FRAME: u32 = 64;

pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        let scenario = std::env::var("OSP_SCENARIO").unwrap_or_else(|_| "pad".into());
        let content = &app.world().resource::<GameContent>().0;
        let game = crate::content::new_game(content, &scenario)
            .unwrap_or_else(|e| panic!("cannot start scenario `{scenario}`: {e}"));
        for d in &game.diagnostics {
            warn!("{d}");
        }
        // Keep problems visible in the Mods window (F2) too.
        app.world_mut()
            .resource_mut::<GameContent>()
            .0
            .diagnostics
            .extend(game.diagnostics);
        let universe = game.universe;
        let datasets = game
            .datasets
            .into_iter()
            .filter_map(|(name, data)| Some((universe.system.find(&name)?, data)))
            .collect();
        app.insert_resource(WorldData {
            datasets,
            sites: game.sites,
        });
        let render_time = universe.time();
        app.insert_resource(Sim {
            universe,
            accumulator: 0.0,
            render_time,
            lagging: false,
        })
        .init_resource::<TimeWarp>()
        .add_message::<SimEventMessage>()
        .add_systems(Update, advance_simulation.in_set(FrameSet::Simulate));
    }
}

/// Static data about the world that rendering needs alongside the simulation.
#[derive(Resource)]
pub struct WorldData {
    /// Baked surface data (colour, land cover, ...) by body.
    pub datasets: HashMap<BodyId, Arc<Dataset>>,
    pub sites: Vec<LaunchSiteDef>,
}

#[derive(Resource)]
pub struct Sim {
    pub universe: Universe,
    /// Simulated seconds not yet consumed by a physics tick.
    accumulator: f64,
    /// The instant being rendered this frame (between physics ticks at low warp).
    pub render_time: SimTime,
    /// True if physics could not keep up with the requested warp this frame.
    pub lagging: bool,
}

impl Sim {
    /// The vessel the player is flying. Vessel switching is not implemented yet.
    pub fn active_vessel(&self) -> &Vessel {
        &self.universe.vessels[0]
    }

    pub fn active_vessel_mut(&mut self) -> &mut Vessel {
        &mut self.universe.vessels[0]
    }

    /// The active vessel, mutably, with the solar system it flies in (for commands that
    /// need positions, like staging and SAS).
    pub fn active_vessel_in_system(&mut self) -> (&mut Vessel, &SolarSystem) {
        (&mut self.universe.vessels[0], &self.universe.system)
    }
}

/// Simulation events, re-broadcast for UI and mods.
#[derive(Message, Clone, Copy, Debug)]
pub struct SimEventMessage(pub SimEvent);

#[derive(Resource, Default)]
pub struct TimeWarp {
    index: usize,
    /// Last warp-related notice and the real time it was shown.
    pub notice: Option<(String, f64)>,
}

impl TimeWarp {
    pub fn rate(&self) -> f64 {
        WARP_RATES[self.index]
    }

    pub fn on_rails(&self) -> bool {
        self.rate() > MAX_PHYSICS_WARP
    }

    pub fn increase(&mut self, sim: &Sim, now: f64) {
        let next = (self.index + 1).min(WARP_RATES.len() - 1);
        if WARP_RATES[next] > MAX_PHYSICS_WARP && !sim.universe.can_use_rails() {
            self.notify(
                "Cannot warp faster: under power, in atmosphere or near terrain",
                now,
            );
            return;
        }
        self.index = next;
    }

    /// Jump straight to a warp level (scripts and UI); same rails checks as `increase`.
    pub fn set_index(&mut self, index: usize, sim: &Sim, now: f64) {
        while self.index < index.min(WARP_RATES.len() - 1) {
            let before = self.index;
            self.increase(sim, now);
            if self.index == before {
                break;
            }
        }
        self.index = self.index.min(index);
    }

    pub fn decrease(&mut self) {
        self.index = self.index.saturating_sub(1);
    }

    pub fn stop(&mut self, reason: &str, now: f64) {
        self.index = 0;
        self.notify(reason, now);
    }

    fn notify(&mut self, message: &str, now: f64) {
        self.notice = Some((message.to_owned(), now));
    }
}

fn advance_simulation(
    time: Res<Time>,
    mut sim: ResMut<Sim>,
    mut warp: ResMut<TimeWarp>,
    mut messages: MessageWriter<SimEventMessage>,
) {
    let real_dt = time.delta_secs_f64().min(MAX_FRAME_TIME);
    let now = time.elapsed_secs_f64();
    let sim = &mut *sim;

    if warp.on_rails() {
        if sim.universe.can_use_rails() {
            let target = sim.universe.time().add_seconds(real_dt * warp.rate());
            let outcome = sim.universe.advance_rails(target);
            if outcome.interrupted {
                warp.stop(&stop_reason(&outcome.events), now);
            }
            messages.write_batch(outcome.events.into_iter().map(SimEventMessage));
            sim.accumulator = 0.0;
            sim.render_time = sim.universe.time();
            sim.lagging = false;
            return;
        }
        warp.stop("Warp stopped: physics required", now);
    }

    let step = PHYSICS_STEP.as_seconds();
    sim.accumulator += real_dt * warp.rate();
    let mut ticks = 0;
    while sim.accumulator >= step && ticks < MAX_TICKS_PER_FRAME {
        let events = sim.universe.step_physics();
        sim.accumulator -= step;
        ticks += 1;
        messages.write_batch(events.into_iter().map(SimEventMessage));
    }
    sim.lagging = sim.accumulator >= step;
    if sim.lagging {
        // Drop the backlog instead of spiralling; the simulation runs slower than requested.
        sim.accumulator = 0.0;
    }
    let offset = (sim.accumulator * TICKS_PER_SECOND as f64) as i64;
    sim.render_time = sim.universe.time().add_ticks(offset);
}

fn stop_reason(events: &[SimEvent]) -> String {
    events
        .iter()
        .rev()
        .find(|e| e.event.kind.stops_warp())
        .map_or("Warp stopped".into(), |e| match e.event.kind {
            VesselEventKind::Descent { .. } => {
                "Warp stopped: entering atmosphere / low altitude".into()
            }
            VesselEventKind::Landed { .. } => "Warp stopped: landed".into(),
            VesselEventKind::Crashed { .. } => "Warp stopped: crashed".into(),
            _ => "Warp stopped".into(),
        })
}
