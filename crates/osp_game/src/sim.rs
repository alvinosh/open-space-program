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
/// What `cargo run` starts without `OSP_SCENARIO`.
const DEFAULT_SCENARIO: &str = "moon_plane";
/// Longest real frame time we simulate; avoids huge catch-up after a hitch.
const MAX_FRAME_TIME: f64 = 0.1;
/// Physics ticks per frame before the simulation is allowed to fall behind real time.
const MAX_TICKS_PER_FRAME: u32 = 64;
/// Warping to a time, the rate is what would get there in this many real seconds, so the
/// remaining time shrinks exponentially and the warp slows smoothly on the way in.
const WARP_TO_LEAD: f64 = 0.6;
/// Fastest automatic warp, ×.
const MAX_AUTO_WARP: f64 = 1e8;

pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        let scenario = std::env::var("OSP_SCENARIO").unwrap_or_else(|_| {
            // The scripted captures were written for a launch from the pad.
            if std::env::var_os("OSP_CAPTURE").is_some() {
                "pad".into()
            } else {
                DEFAULT_SCENARIO.into()
            }
        });
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
            active: 0,
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
    /// Index of the vessel the player is flying.
    active: usize,
    /// Simulated seconds not yet consumed by a physics tick.
    accumulator: f64,
    /// The instant being rendered this frame (between physics ticks at low warp).
    pub render_time: SimTime,
    /// True if physics could not keep up with the requested warp this frame.
    pub lagging: bool,
}

impl Sim {
    /// The vessel the player is flying.
    pub fn active_vessel(&self) -> &Vessel {
        &self.universe.vessels[self.active]
    }

    pub fn active_vessel_mut(&mut self) -> &mut Vessel {
        &mut self.universe.vessels[self.active]
    }

    /// The active vessel, mutably, with the solar system it flies in (for commands that
    /// need positions, like staging and SAS).
    pub fn active_vessel_in_system(&mut self) -> (&mut Vessel, &SolarSystem) {
        (
            &mut self.universe.vessels[self.active],
            &self.universe.system,
        )
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    /// Fly another vessel; `step` +1 / −1 cycles through them. The one left behind lets
    /// go of the controls (its throttle, SAS and burn executor carry on).
    pub fn switch_vessel(&mut self, step: isize) {
        let count = self.universe.vessels.len() as isize;
        if count > 1 {
            let left = self.active_vessel_mut();
            left.rotation_input = bevy::math::DVec3::ZERO;
            left.translation_input = bevy::math::DVec3::ZERO;
            self.active = (self.active as isize + step).rem_euclid(count) as usize;
        }
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
    /// Warping to this time (and what is there), choosing the rate automatically.
    target: Option<(SimTime, String)>,
    /// The rate an automatic warp chose, overriding the warp level.
    auto_rate: Option<f64>,
}

impl TimeWarp {
    pub fn rate(&self) -> f64 {
        self.auto_rate.unwrap_or(WARP_RATES[self.index])
    }

    pub fn on_rails(&self) -> bool {
        self.rate() > MAX_PHYSICS_WARP
    }

    /// Where an automatic warp is heading, if one is.
    pub fn target(&self) -> Option<&(SimTime, String)> {
        self.target.as_ref()
    }

    /// Warp to `time` (labelled `what` for notices), slowing down on the way in.
    pub fn warp_to(&mut self, time: SimTime, what: &str, sim: &Sim, now: f64) {
        if time <= sim.universe.time() {
            self.notify(&format!("{what} has already passed"), now);
            return;
        }
        self.target = Some((time, what.to_owned()));
        self.notify(&format!("Warping to {what}"), now);
    }

    /// Pick the rate for an automatic warp; ends it on arrival.
    fn steer_to_target(&mut self, sim: &Sim, now: f64) {
        let Some((time, what)) = self.target.clone() else {
            return;
        };
        let remaining = time.seconds_since(sim.universe.time());
        if remaining <= 0.0 {
            self.target = None;
            self.auto_rate = None;
            self.index = 0;
            self.notify(&format!("Arrived: {what}"), now);
            return;
        }
        let limit = if sim.universe.can_use_rails() {
            MAX_AUTO_WARP
        } else {
            MAX_PHYSICS_WARP
        };
        self.auto_rate = Some((remaining / WARP_TO_LEAD).clamp(1.0, limit));
    }

    pub fn increase(&mut self, sim: &Sim, now: f64) {
        self.target = None;
        self.auto_rate = None;
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
        self.target = None;
        self.auto_rate = None;
        self.index = self.index.saturating_sub(1);
    }

    pub fn stop(&mut self, reason: &str, now: f64) {
        self.target = None;
        self.auto_rate = None;
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
    warp.steer_to_target(sim, now);

    if warp.on_rails() {
        if sim.universe.can_use_rails() {
            let mut target = sim.universe.time().add_seconds(real_dt * warp.rate());
            if let Some((end, _)) = warp.target() {
                target = target.min(*end);
            }
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
            VesselEventKind::Destroyed => "Warp stopped: structural failure".into(),
            VesselEventKind::BurnDue => "Warp stopped: burn coming up".into(),
            VesselEventKind::BurnEnded => "Warp stopped: burn ended".into(),
            _ => "Warp stopped".into(),
        })
}
