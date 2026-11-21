//! Watching a mission fly (`cargo run -- --watch <mission>`): the game shows the flight while its
//! script flies it, and it is the same flight `cargo smoke` flies headless.
//!
//! The script runs on a thread of its own, as it does in the tests: it is one function that flies
//! the whole mission and returns. Each time the flight has used up the slice of time the game gave
//! it, the script hands the universe to the game and waits; the game draws a frame of it and hands
//! it back with the next slice ([`Watch::show`]). Only one universe ever flies, held by the game
//! while it draws and by the script while it flies; the other side keeps a spare in its place. The
//! slices change nothing in the flight (`a_watched_flight_is_the_same_flight` holds it to that), but
//! the game must not change the universe while it holds it either.

use std::{
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::mpsc::{Receiver, SendError, Sender, TryRecvError, channel},
    thread,
    time::{Duration, Instant},
};

use osp_sim::universe::{SimEvent, Universe};

use crate::{Mission, world::World, write_up};

/// How fast a watched flight goes, in times real time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speed {
    /// Under physics: the launch, burns, docking, the landing, the entry.
    pub physics: f64,
    /// On rails: the coasts.
    pub rails: f64,
}

impl Default for Speed {
    fn default() -> Self {
        Self {
            physics: 4.0,
            rails: 10_000.0,
        }
    }
}

/// The longest the script flies for a frame. When it cannot keep up with the speed asked for, the
/// flight falls behind instead of the game stopping drawing.
const SLICE: Duration = Duration::from_millis(12);

/// What the game gives the script: the universe, and how long to fly it for.
struct Turn {
    universe: Universe,
    /// Real seconds: the speed says how much flight that is.
    real: f64,
    speed: Speed,
}

/// What the script gives the game.
enum Handoff {
    /// The flight so far, to show and give back.
    Frame {
        universe: Universe,
        events: Vec<SimEvent>,
        phase: Option<String>,
        rails: bool,
    },
    /// The flight is over: the universe is the game's now.
    Done { universe: Universe, summary: String },
    /// It could not start.
    Failed(String),
}

/// The game let go of the watch: the script stops where it is.
struct Stopped;

/// What became of a frame's turn.
#[derive(Debug)]
pub enum Shown {
    /// The flight has not begun (the script is loading the mods): what was showing still is.
    Loading,
    /// The flight's universe is showing for the first time: every vessel in it is new.
    Started,
    /// It flew on: what happened in the universe, the phase it is in, and whether it is coasting
    /// on rails.
    Flew {
        events: Vec<SimEvent>,
        phase: Option<String>,
        rails: bool,
    },
    /// It is over, and its universe is left as it ended. The report's summary.
    Finished(String),
    /// It cannot go on, and why. The universe is what it was before the last turn, or the one
    /// that was showing before the flight began.
    Stopped(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Loading,
    Showing,
    Over,
}

/// A mission flying on its own thread, for the game to show.
pub struct Watch {
    to_script: Sender<Turn>,
    from_script: Receiver<Handoff>,
    /// What fills the game's universe while the script has the flight's.
    spare: Option<Universe>,
    state: State,
}

impl Watch {
    /// Starts flying `mission` (loading the mods takes a moment: [`Shown::Loading`] until then).
    pub fn start(mission: &'static Mission) -> Watch {
        let (to_game, from_script) = channel();
        let (to_script, from_game) = channel();
        thread::Builder::new()
            .name(format!("watch {}", mission.id))
            .spawn(move || fly(mission, to_game, from_game))
            .expect("a thread for the script");
        Watch {
            to_script,
            from_script,
            spare: None,
            state: State::Loading,
        }
    }

    /// Lets the flight go on for `real` seconds at `speed`, in `universe`, the one the game
    /// shows. Waits for the script to fly them (never much longer than [`SLICE`]).
    pub fn show(&mut self, universe: &mut Universe, real: f64, speed: Speed) -> Shown {
        let handoff = match self.state {
            State::Over => return Shown::Stopped("the flight is over".into()),
            State::Loading => match self.from_script.try_recv() {
                Ok(handoff) => handoff,
                Err(TryRecvError::Empty) => return Shown::Loading,
                Err(TryRecvError::Disconnected) => {
                    return self.stop("the script stopped before the flight began");
                }
            },
            State::Showing => {
                let spare = self.spare.take().expect("the spare universe is here");
                let live = std::mem::replace(universe, spare);
                let turn = Turn {
                    universe: live,
                    real,
                    speed,
                };
                if let Err(SendError(turn)) = self.to_script.send(turn) {
                    self.spare = Some(std::mem::replace(universe, turn.universe));
                    return self.stop("the script has stopped");
                }
                match self.from_script.recv() {
                    Ok(handoff) => handoff,
                    // It went with the universe: what showed before the last turn is all there is.
                    Err(_) => return self.stop("the script stopped in the middle of the flight"),
                }
            }
        };
        match handoff {
            Handoff::Frame {
                universe: live,
                events,
                phase,
                rails,
            } => {
                self.spare = Some(std::mem::replace(universe, live));
                if self.state == State::Loading {
                    self.state = State::Showing;
                    Shown::Started
                } else {
                    Shown::Flew {
                        events,
                        phase,
                        rails,
                    }
                }
            }
            Handoff::Done {
                universe: live,
                summary,
            } => {
                *universe = live;
                self.state = State::Over;
                Shown::Finished(summary)
            }
            Handoff::Failed(why) => self.stop(&why),
        }
    }

    fn stop(&mut self, why: &str) -> Shown {
        self.state = State::Over;
        Shown::Stopped(why.to_owned())
    }
}

/// The script's thread: flies `mission` from start to finish, handing the universe over as it goes.
fn fly(mission: &'static Mission, to_game: Sender<Handoff>, from_game: Receiver<Turn>) {
    let mut world =
        match catch_unwind(|| World::in_mode(mission.scenario, mission.mode, mission.plan)) {
            Ok(world) => world,
            Err(e) => {
                let why = format!("cannot start `{}`: {}", mission.id, panic_text(&*e));
                let _ = to_game.send(Handoff::Failed(why));
                return;
            }
        };
    let universe = &world.universe;
    let spare = Universe::new(universe.system.clone(), Vec::new(), universe.time());
    world.pacer = Some(Pacer {
        to_game: to_game.clone(),
        from_game,
        spare: Some(spare),
        left: 0.0,
        speed: Speed::default(),
        until: Instant::now(),
        events: Vec::new(),
        rails: false,
    });
    let flight = catch_unwind(AssertUnwindSafe(|| {
        // Shown on the pad before it flies.
        world.hand_over();
        (mission.fly)(&mut world)
    }));
    let stopped_early = match flight {
        Ok(flight) => flight.is_err(),
        Err(e) if e.is::<Stopped>() => return,
        Err(e) => {
            world.event(format!("The script panicked: {}", panic_text(&*e)));
            true
        }
    };
    world.pacer = None;
    let done = write_up(mission, &mut world, stopped_early);
    let summary = format!("{} (report: {})", done.summary, done.page.display());
    let _ = to_game.send(Handoff::Done {
        universe: world.universe,
        summary,
    });
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    (payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic".into())
}

/// The script's side of a watch, kept by the [`World`] it flies.
pub(crate) struct Pacer {
    to_game: Sender<Handoff>,
    from_game: Receiver<Turn>,
    /// What fills the world's universe while the game has the flight's.
    spare: Option<Universe>,
    /// Real seconds of the slice not yet flown; below zero, what a long step overran it by.
    left: f64,
    speed: Speed,
    /// When the slice ends, flown or not.
    until: Instant,
    /// What happened since the last frame.
    events: Vec<SimEvent>,
    /// The last step was on rails.
    rails: bool,
}

impl Pacer {
    /// `seconds` more of the flight have passed, on `rails` or by physics; hands the universe over
    /// when the slice is used up.
    pub(crate) fn passed(
        &mut self,
        universe: &mut Universe,
        seconds: f64,
        rails: bool,
        events: &[SimEvent],
        phase: Option<&str>,
    ) {
        self.events.extend_from_slice(events);
        self.rails = rails;
        let rate = if rails {
            self.speed.rails
        } else {
            self.speed.physics
        };
        self.left -= seconds / rate.max(1e-6);
        if self.left <= 0.0 || Instant::now() >= self.until {
            self.hand_over(universe, phase);
        }
    }

    /// Gives the game the universe for a frame, and waits for it back with the next slice; frame
    /// after frame while a step that overran is paid for.
    pub(crate) fn hand_over(&mut self, universe: &mut Universe, phase: Option<&str>) {
        loop {
            let spare = self.spare.take().expect("the spare universe is here");
            let frame = Handoff::Frame {
                universe: std::mem::replace(universe, spare),
                events: std::mem::take(&mut self.events),
                phase: phase.map(str::to_owned),
                rails: self.rails,
            };
            if self.to_game.send(frame).is_err() {
                resume_unwind(Box::new(Stopped));
            }
            let Ok(turn) = self.from_game.recv() else {
                resume_unwind(Box::new(Stopped));
            };
            self.spare = Some(std::mem::replace(universe, turn.universe));
            // Behind (the slice ran out of time first), what was not flown is dropped, as the game
            // drops it; ahead (a long step overran it), the next slices pay for it.
            self.left = self.left.min(0.0) + turn.real;
            self.speed = turn.speed;
            self.until = Instant::now() + SLICE;
            if self.left > 0.0 {
                return;
            }
        }
    }
}
