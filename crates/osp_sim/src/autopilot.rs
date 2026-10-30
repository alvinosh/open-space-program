//! Flight computer: flies planned burns ([`crate::plan`]).
//!
//! The plan treats a burn as an impulse. The executor turns it into a finite burn centred on
//! the burn's time: it points along the burn's Δv, lights the engines half the estimated
//! burn time early, and steers along whatever Δv is still to be given (inertial, fixed when
//! the burn starts), throttling down so it stops on it. Engines that need ullage get it from
//! the RCS translating forward just before ignition; an engine is never lit unsettled, as
//! that would waste an ignition. Afterwards the engines are cut, SAS holds the attitude and
//! the burn is removed from the plan.

use glam::DVec3;

use crate::{constants::G0, plan::Burn, propulsion::Propulsion, time::SimTime};

/// Start the RCS ullage push this long before ignition, s.
pub const ULLAGE_LEAD: f64 = 3.0;
/// Time warp hands over to physics this long before the executor needs it, s.
pub const WARP_MARGIN: f64 = 10.0;
/// The nose must be this close to the burn direction to light the engines, rad.
const ALIGNED: f64 = 0.03;
/// Throttle so that what is left would take this long at the current setting, s.
const FINISH_TIME: f64 = 1.5;
/// Stop steering along the remaining Δv once it is this small a share of the burn (the
/// direction of a nearly finished burn is noise).
const FREEZE_SHARE: f64 = 0.02;

/// What the active stage can do right now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Capability {
    /// Full-throttle thrust at the current ambient pressure, N.
    pub thrust: f64,
    /// Full-throttle propellant flow, kg/s.
    pub mass_flow: f64,
    /// Thrust share at the lowest throttle setting, 0..1.
    pub min_throttle: f64,
    pub needs_ullage: bool,
}

impl Capability {
    /// Of the bottom stages ([`Propulsion::bottom`]) that are armed, if they have an engine
    /// that can still light.
    pub fn of(propulsion: &Propulsion, pressure: f64) -> Option<Self> {
        Self::of_bottom_stage(propulsion, pressure, true)
    }

    /// Of the bottom stages whether or not they are armed yet (for estimates).
    pub fn estimate(propulsion: &Propulsion, pressure: f64) -> Option<Self> {
        Self::of_bottom_stage(propulsion, pressure, false)
    }

    fn of_bottom_stage(propulsion: &Propulsion, pressure: f64, armed: bool) -> Option<Self> {
        let usable: Vec<_> = propulsion
            .bottom()
            .filter(|s| propulsion.armed(s.id) || !armed)
            .flat_map(|s| propulsion.engines_of(s.id))
            .filter(|e| e.running || e.ignitions_left != Some(0))
            .collect();
        if usable.is_empty() {
            return None;
        }
        Some(Self {
            thrust: usable
                .iter()
                .map(|e| e.max_mass_flow() * e.isp(pressure) * G0)
                .sum(),
            mass_flow: usable.iter().map(|e| e.max_mass_flow()).sum(),
            min_throttle: usable
                .iter()
                .map(|e| e.def.min_throttle.clamp(0.0, 1.0))
                .fold(0.0, f64::max),
            needs_ullage: usable.iter().any(|e| e.def.requires_ullage && !e.running),
        })
    }

    /// Time to give `delta_v` at full throttle starting at `mass`, s (rocket equation).
    pub fn burn_time(&self, mass: f64, delta_v: f64) -> f64 {
        if self.mass_flow <= 0.0 || self.thrust <= 0.0 {
            return f64::INFINITY;
        }
        let exhaust = self.thrust / self.mass_flow;
        mass / self.mass_flow * (1.0 - (-delta_v / exhaust).exp())
    }
}

/// Where a burn starts and how long it takes, for the vessel as it is now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BurnTiming {
    pub start: SimTime,
    pub duration: f64,
}

impl BurnTiming {
    pub fn of(burn: &Burn, capability: &Capability, mass: f64) -> Self {
        let duration = capability.burn_time(mass, burn.delta_v.length());
        Self {
            start: burn.time.add_seconds(-(duration.min(1e9)) / 2.0),
            duration,
        }
    }
}

/// What the executor wants this tick.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Command {
    /// Where to point the nose (inertial), if anywhere.
    pub direction: Option<DVec3>,
    /// Throttle to set; `None` leaves it alone.
    pub throttle: Option<f64>,
    /// Push forward on RCS to settle propellant.
    pub ullage: bool,
    /// The burn is over (or cannot be flown), with a log message.
    pub finished: Option<Finish>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Finish {
    Done { residual: f64 },
    Aborted(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Phase {
    #[default]
    Idle,
    /// Engaged, pointing, waiting for the start time.
    Waiting,
    /// Engines lit (or being lit); `remaining` is the Δv still to give.
    Burning {
        remaining: DVec3,
        total: f64,
        aim: DVec3,
    },
}

/// The burn executor's state. One per vessel; it flies the vessel's first planned burn.
#[derive(Debug, Clone, Default)]
pub struct BurnExecutor {
    phase: Phase,
}

/// What the executor needs to know each tick.
pub struct Situation<'a> {
    pub time: SimTime,
    pub burn: &'a Burn,
    /// The burn's Δv in inertial axes, on the trajectory as it is now.
    pub delta_v: DVec3,
    pub capability: Option<Capability>,
    pub mass: f64,
    pub nose: DVec3,
    pub settled: bool,
    pub has_rcs: bool,
    /// Engines of the bottom stage are burning.
    pub running: bool,
}

impl BurnExecutor {
    pub fn engaged(&self) -> bool {
        self.phase != Phase::Idle
    }

    pub fn burning(&self) -> bool {
        matches!(self.phase, Phase::Burning { .. })
    }

    pub fn engage(&mut self) {
        if self.phase == Phase::Idle {
            self.phase = Phase::Waiting;
        }
    }

    pub fn disengage(&mut self) {
        self.phase = Phase::Idle;
    }

    /// Δv still to give, inertial, once the burn has started.
    pub fn remaining(&self) -> Option<DVec3> {
        match self.phase {
            Phase::Burning { remaining, .. } => Some(remaining),
            _ => None,
        }
    }

    /// Direction the burn wants the nose (inertial), if one is being flown.
    pub fn aim(&self) -> Option<DVec3> {
        match self.phase {
            Phase::Burning { aim, .. } => Some(aim),
            _ => None,
        }
    }

    /// Decide this tick's commands, before the engines run.
    pub fn command(&mut self, s: &Situation) -> Command {
        let mut command = Command::default();
        match self.phase {
            Phase::Idle => {}
            Phase::Waiting => {
                let Some(capability) = s.capability else {
                    return self.abort("no armed engine can burn; stage first");
                };
                let total = s.delta_v.length();
                if total <= 0.0 {
                    return self.finish(0.0);
                }
                let aim = s.delta_v / total;
                command.direction = Some(aim);
                // The executor owns the throttle once engaged; engines light at the start.
                command.throttle = Some(0.0);
                let timing = BurnTiming::of(s.burn, &capability, s.mass);
                if !timing.duration.is_finite() {
                    return self.abort("the active stage has no thrust");
                }
                let unsettled = capability.needs_ullage && !s.settled;
                if unsettled && s.time >= timing.start.add_seconds(-ULLAGE_LEAD) {
                    if !s.has_rcs {
                        return self.abort("propellant unsettled and no RCS to settle it");
                    }
                    command.ullage = true;
                }
                if s.time < timing.start {
                    return command;
                }
                if s.nose.angle_between(aim) > ALIGNED || unsettled {
                    // Late is better than lighting off-axis or unsettled.
                    return command;
                }
                self.phase = Phase::Burning {
                    remaining: s.delta_v,
                    total,
                    aim,
                };
                return self.command(s);
            }
            Phase::Burning {
                remaining,
                total,
                aim,
            } => {
                let Some(capability) = s.capability else {
                    return self.abort("out of propellant or ignitions");
                };
                let left = remaining.length();
                let full = capability.thrust / s.mass;
                let min = capability.min_throttle;
                // Thrust share that would finish in FINISH_TIME, within the engine's range.
                let share = (left / (full * FINISH_TIME)).clamp(min, 1.0);
                command.direction = Some(aim);
                // Never 0 while burning: that shuts the engines down.
                command.throttle = Some(share.max(1e-6));
                command.ullage = capability.needs_ullage && !s.settled;
                if left < FREEZE_SHARE * total {
                    return command;
                }
                self.phase = Phase::Burning {
                    remaining,
                    total,
                    aim: remaining / left,
                };
                command.direction = Some(remaining / left);
            }
        }
        command
    }

    /// Count the Δv the engines gave this tick (inertial, m/s); ends the burn when what is
    /// left is no more than the engines' tail-off would still give (`tail_off`, m/s) or
    /// would be overshot by more than this tick's share.
    pub fn account(&mut self, given: DVec3, running: bool, tail_off: f64) -> Option<Finish> {
        let Phase::Burning {
            remaining,
            total,
            aim,
        } = self.phase
        else {
            return None;
        };
        if !running {
            return self
                .abort("engines stopped before the burn was complete")
                .finished;
        }
        let left = remaining - given;
        // Past the target along the burn (counting the tail-off still to come), or closer
        // than another tick would get.
        if left.dot(aim) <= 0.5 * given.length() + tail_off {
            return self.finish((left - aim * tail_off).length()).finished;
        }
        self.phase = Phase::Burning {
            remaining: left,
            total,
            aim,
        };
        None
    }

    fn finish(&mut self, residual: f64) -> Command {
        self.phase = Phase::Idle;
        Command {
            throttle: Some(0.0),
            finished: Some(Finish::Done { residual }),
            ..Default::default()
        }
    }

    fn abort(&mut self, why: &str) -> Command {
        self.phase = Phase::Idle;
        Command {
            throttle: Some(0.0),
            finished: Some(Finish::Aborted(why.to_owned())),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burn_time_follows_the_rocket_equation() {
        let c = Capability {
            thrust: 100e3,
            mass_flow: 100e3 / (300.0 * G0),
            min_throttle: 0.0,
            needs_ullage: false,
        };
        // Small burns approach Δv · m / F.
        let t = c.burn_time(10_000.0, 1.0);
        assert!((t - 0.1).abs() < 1e-4, "{t}");
        // Big ones take less than that, as the vessel lightens.
        let t = c.burn_time(10_000.0, 1_000.0);
        assert!(t < 100.0 && t > 80.0, "{t}");
    }
}
