//! What a vessel's computer can fly by itself, for the parts of a flight no hand could: the way down
//! to a surface on an engine, and an entry at the angle its lift allows.
//!
//! The descent is a tool the player switches on: land by itself, hold a rate of descent, or hover.
//! It flies the engines' throttle and which way the vessel points, and hands back at touchdown.
//!
//! The entry rolls the vessel about the way it is going, which points its lift: up to stay high and
//! pull less, and over when it would otherwise climb back out.

use glam::DVec3;
use serde::{Deserialize, Serialize};

/// Height it brakes down to, m: the way down ends in a near hover this far up.
const GATE: f64 = 150.0;
/// Speed across the ground under which it comes straight down, m/s.
const HOVER_SPEED: f64 = 3.0;
/// The rate it touches down at, m/s.
const TOUCHDOWN: f64 = 1.0;

/// Below this much acceleration the air has not got hold of it yet, m/s².
const ENTRY_LIFT_ON: f64 = 0.5;
/// Further than this from heat shield first the thrusters turn it back, rad.
const ENTRY_TURNED_ROUND: f64 = 1.0;
/// What an entry tries to hold, m/s².
const ENTRY_TARGET: f64 = 4.0 * 9.80665;
/// How far ahead it looks, s, and the height the air thins by a factor of e over, m.
const ENTRY_LOOK_AHEAD: f64 = 15.0;
const ENTRY_SCALE_HEIGHT: f64 = 7_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum Guidance {
    #[default]
    Off,
    /// All the way down to the ground from wherever it is.
    Land,
    /// Sinking at this rate, m/s; climbing if negative.
    HoldRate(f64),
    /// Holding height, with the speed across the ground taken out.
    Hover,
    /// Rolling its lift through an entry.
    Entry,
}

/// What the descent's law needs to know.
#[derive(Debug, Clone, Copy)]
pub struct DescentView {
    /// Above the ground, feet to the ground, m.
    pub height: f64,
    /// Local vertical, unit, inertial axes.
    pub up: DVec3,
    /// Velocity across the ground, inertial axes, m/s.
    pub across: DVec3,
    /// Up, m/s.
    pub climb: f64,
    /// Gravity less what going round takes off it, m/s².
    pub weight: f64,
    /// What the engines can give, m/s².
    pub push: f64,
}

/// What the descent wants: the push, m/s², inertial axes.
pub fn descent_push(guidance: Guidance, view: &DescentView) -> Option<DVec3> {
    let speed = view.across.length();
    let holding = |sink: f64| {
        let lift = view.weight + (-sink - view.climb) / 3.0;
        view.up * lift.max(0.2) - view.across * 0.3
    };
    match guidance {
        Guidance::Off | Guidance::Entry => None,
        Guidance::HoldRate(sink) => Some(holding(sink)),
        Guidance::Hover => Some(holding(0.0)),
        Guidance::Land if speed > HOVER_SPEED => {
            // Against the speed, losing height no faster than will bring it to the gate as the
            // speed runs out.
            let left = (speed / (0.85 * view.push)).max(20.0);
            let sink = -((view.height - GATE) / left).clamp(-5.0, 120.0);
            let lift = (view.weight + (sink - view.climb) / 10.0).clamp(0.0, 0.9 * view.push);
            let against = (view.push * view.push - lift * lift).sqrt().min(speed);
            Some(view.up * lift - view.across.normalize_or_zero() * against)
        }
        // Straight down, slower the lower it gets.
        Guidance::Land => Some(holding((0.1 * view.height).clamp(TOUCHDOWN, 25.0))),
    }
}

/// What the entry's law needs to know.
#[derive(Debug, Clone, Copy)]
pub struct EntryView {
    /// How it moves through the air, unit, in its own axes.
    pub toward: DVec3,
    /// The local vertical in its own axes.
    pub up: DVec3,
    /// Up, m/s.
    pub climb: f64,
    /// What the air is doing to it, m/s².
    pub acceleration: f64,
    /// How it is turning, rad/s, in its own axes.
    pub turning: DVec3,
    /// The way its lift points, across the vessel, in its own axes: from the axis to the centre of
    /// mass, or `None` if it is on the axis.
    pub lift: Option<DVec3>,
    /// What its thrusters can turn it at, rad/s², about each axis.
    pub authority: DVec3,
}

/// The rotation to command through an entry: it damps what the air does not, and rolls the lift
/// towards up, or as far over as keeps it from climbing back out.
pub fn entry_command(view: &EntryView) -> DVec3 {
    let damp = |rate: f64, alpha: f64| (-rate / (1.0 * alpha.max(1e-3))).clamp(-1.0, 1.0);
    let mut command = DVec3::new(
        damp(view.turning.x, view.authority.x),
        damp(view.turning.y, view.authority.y),
        damp(view.turning.z, view.authority.z),
    );
    // Before the air takes over, or if it has been turned far round: heat shield first, which is
    // the apex pointing back along the way it is going.
    let shield = -view.toward;
    let off = DVec3::Z.angle_between(shield);
    if view.acceleration < ENTRY_LIFT_ON || off > ENTRY_TURNED_ROUND {
        let axis = DVec3::Z.cross(shield).normalize_or(DVec3::X);
        let error = axis * off;
        let steer = |error: f64, rate: f64, alpha: f64| {
            ((0.3 * error - rate) / alpha.max(1e-3)).clamp(-1.0, 1.0)
        };
        command.x = steer(error.x, view.turning.x, view.authority.x);
        command.y = steer(error.y, view.turning.y, view.authority.y);
    }
    let Some(lift) = view.lift else {
        return command;
    };
    if view.acceleration < ENTRY_LIFT_ON {
        return command;
    }
    // Where it wants its lift, about the way it is going. What it will be feeling a little ahead,
    // as the air thickens at the rate it is coming down: too much, and the lift goes up to hold it
    // high; too little, and it goes down so as not to leave the air again.
    let ahead = view.acceleration * (-view.climb * ENTRY_LOOK_AHEAD / ENTRY_SCALE_HEIGHT).exp();
    let wanted_up = ((ahead - ENTRY_TARGET) / (0.5 * ENTRY_TARGET)).clamp(-1.0, 1.0);
    let wanted_bank = wanted_up.acos();
    let up = (view.up - view.toward * view.up.dot(view.toward)).normalize_or_zero();
    let side = view.toward.cross(up);
    let lift_now = lift - view.toward * lift.dot(view.toward);
    let bank = lift_now.dot(side).atan2(lift_now.dot(up));
    // Whichever way round is shorter; the turn is about the vessel's own long axis.
    let mut error = wanted_bank - bank.abs();
    if bank < 0.0 {
        error = -error;
    }
    // Its long axis points back along the way it is going, so turning it about that axis takes the
    // bank the other way.
    let roll = (-error * 0.8 - view.turning.z) / view.authority.z.max(1e-3);
    command.z = roll.clamp(-1.0, 1.0);
    command
}
