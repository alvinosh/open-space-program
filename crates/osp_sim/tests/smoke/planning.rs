//! Flight planning as the player does it on the map: pick a target, let the transfer planner
//! find a departure, aim the burn at the target's periapsis, then let the burn executor fly
//! it. These mirror what `osp_game`'s transfer planner and flight-plan windows do.

use osp_sim::{
    DVec3,
    attitude::SasMode,
    body::BodyId,
    plan::Burn,
    prediction::predict,
    rendezvous::Target,
    time::SimTime,
    transfer::{
        Course, Transfer, TransferRequest, aim_periapsis, departure_burn, hohmann_time, porkchop,
    },
};

use crate::world::World;

/// How far ahead the map predicts a trajectory, s.
const PREDICTION_HORIZON: f64 = 12.0 * 86_400.0;
/// The transfer planner's grid.
const COLUMNS: usize = 72;
const ROWS: usize = 54;

/// What the transfer planner finds for the Moon (or another body in the same sphere of
/// influence) leaving within `window` seconds.
pub struct Departure {
    pub transfer: Transfer,
    pub burn: Burn,
}

/// F4: the cheapest transfer to `target` leaving between `after` and `after + window`
/// seconds from now, as a burn.
pub fn transfer_to(w: &mut World, target: BodyId, after: f64, window: f64) -> Option<Departure> {
    let system = &w.universe.system;
    let now = w.now();
    let vessel = w.vessel();
    let orbit = vessel.orbit(system, now);
    let arrival = system.body(target).orbit?;
    let (r1, r2) = (orbit.semi_major_axis(), arrival.semi_major_axis());
    let t_h = hohmann_time(orbit.mu(), r1, r2);
    let period = orbit.period()?;
    let request = TransferRequest {
        departure: orbit,
        arrival,
        depart_from: now.add_seconds(after.max(60.0)),
        depart_span: window,
        tof_min: 0.5 * t_h,
        tof_max: 1.5 * t_h,
        columns: COLUMNS,
        rows: ROWS,
        parking: None,
        floor: system.body(vessel.parent()).coast_floor(),
        phase_search: (window > 2.0 * period).then_some(period),
    };
    let chop = porkchop(&request);
    let (_, _, transfer) = chop.best()?;
    let burn = departure_burn(&orbit, transfer)?;
    Some(Departure {
        transfer: *transfer,
        burn,
    })
}

/// Puts `burn` in the plan and aims it so the trajectory passes `target` at periapsis
/// altitude `altitude`. Returns the altitude the aim reached.
pub fn aim(w: &mut World, index: usize, target: BodyId, altitude: f64) -> Option<f64> {
    let system = &w.universe.system;
    let now = w.now();
    let vessel = w.vessel();
    let burns = vessel.burns_ahead(system, now);
    let last = burns.last().map_or(now, |b| b.time);
    let course = Course {
        system,
        frame: vessel.parent(),
        orbit: vessel.orbit(system, now),
        from: now,
        horizon: last.add_seconds(PREDICTION_HORIZON),
    };
    let goal = system.body(target).radius + altitude;
    let (burn, reached) = aim_periapsis(&course, &burns, index, target, goal)?;
    w.vessel_mut().plan.set(index, burn);
    Some(reached - system_radius(w, target))
}

fn system_radius(w: &World, body: BodyId) -> f64 {
    w.universe.system.body(body).radius
}

/// The periapsis altitude the map predicts over `target` for the trajectory as it stands
/// (with the planned burns), m; `None` without an encounter.
pub fn predicted_periapsis(w: &World, target: BodyId) -> Option<f64> {
    let system = &w.universe.system;
    let now = w.now();
    let vessel = w.vessel();
    let burns = vessel.burns_ahead(system, now);
    let patches = predict(
        system,
        vessel.parent(),
        vessel.orbit(system, now),
        now,
        now.add_seconds(PREDICTION_HORIZON),
        16,
        &burns,
    );
    patches
        .iter()
        .find(|p| p.frame == target)
        .map(|p| p.orbit.periapsis() - system.body(target).radius)
}

/// Targets a body, as shift-click on the map does.
pub fn target_body(w: &mut World, body: BodyId) {
    w.vessel_mut().target = Some(Target::Body(body));
}

/// The first planned burn's Δv in m/s, and when it is due.
pub fn next_burn(w: &World) -> Option<(f64, SimTime)> {
    w.vessel()
        .plan
        .burns()
        .first()
        .map(|b| (b.delta_v.length(), b.time))
}

/// Turns the vessel onto the first planned burn well ahead of it, as a pilot does when the
/// stack is slow to turn: warps to `lead` seconds before the burn starts, switches SAS to
/// point along the burn and flies physics until the nose is on it. The angle off the burn,
/// degrees, when it settles or `limit` seconds are up.
pub fn point_at_burn(w: &mut World, lead: f64, limit: f64) -> Option<f64> {
    let burn = *w.vessel().plan.burns().first()?;
    let timing = w.vessel().burn_timing(&burn, 0.0)?;
    let wait = timing.start.seconds_since(w.now()) - lead;
    if wait > 1.0 {
        w.warp(wait);
    }
    w.sas(SasMode::Burn);
    let off = |w: &World| {
        let v = w.vessel();
        let system = &w.universe.system;
        let nose = v.attitude_at(system, w.now()) * DVec3::Z;
        let rate = v.attitude_state(system, w.now()).angular_velocity.length();
        v.burn_vector(system, w.now())
            .and_then(DVec3::try_normalize)
            .map(|d| (nose.angle_between(d).to_degrees(), rate))
    };
    w.fly(limit, |w| {
        off(w).is_some_and(|(angle, rate)| angle < 0.7 && rate < 2e-3)
    });
    off(w).map(|(angle, _)| angle)
}

/// Engages the burn executor, rides rails until the first planned burn needs physics, and
/// returns there: the next thing to do is `fly_burn`. `false` if something else stopped the
/// warp first (a crash, a descent).
pub fn warp_to_burn(w: &mut World, limit: f64) -> bool {
    w.with_vessel(|v, system, now| v.engage_burn(system, now));
    let until = w.now().add_seconds(limit);
    while w.universe.can_use_rails() && w.now() < until {
        if !w.warp(3_600.0) {
            break;
        }
    }
    !w.universe.can_use_rails() && w.vessel().executor().engaged()
}

/// Flies the burn under physics until the executor finishes it. `false` on a timeout.
pub fn fly_burn(w: &mut World, limit: f64) -> bool {
    let done = w.fly(limit, |w| !w.vessel().executor().engaged());
    // The engines wind down over a few more ticks, before rails are allowed again.
    w.fly(30.0, |w| w.universe.can_use_rails());
    done
}

/// A small burn by hand: points along the first planned burn with SAS, pushes the stack
/// forward on RCS and stops when the velocity has changed by the burn's Δv (read off the
/// orbit, as the map's predicted trajectory does), then drops the burn from the plan. The
/// Δv given, m/s, or `None` if the stack could not be pointed or the RCS ran dry.
pub fn rcs_trim(w: &mut World, limit: f64) -> Option<f64> {
    let burn = *w.vessel().plan.burns().first()?;
    let wanted = burn.delta_v.length();
    let direction = {
        let system = &w.universe.system;
        let at = w.vessel().state_at(system, burn.time);
        burn.inertial(&at)?.normalize()
    };
    // Aligned, and at rest.
    w.sas(SasMode::Burn);
    let aligned = w.fly(300.0, |w| {
        let v = w.vessel();
        let system = &w.universe.system;
        let nose = v.attitude_at(system, w.now()) * DVec3::Z;
        let rate = v.attitude_state(system, w.now()).angular_velocity.length();
        nose.angle_between(direction).to_degrees() < 0.5 && rate < 2e-3
    });
    if !aligned {
        return None;
    }
    let (system, start) = (&w.universe.system, w.now());
    let before = w.vessel().orbit(system, start);
    let given = |w: &World| {
        let system = &w.universe.system;
        let at = w.now();
        (w.vessel().orbit(system, at).state_at(at).velocity - before.state_at(at).velocity)
            .dot(direction)
    };
    w.vessel_mut().translation_input = DVec3::Z;
    w.fly(limit, |w| given(w) >= wanted - 0.02);
    w.vessel_mut().translation_input = DVec3::ZERO;
    // Settle: the RCS stops, rotation is damped.
    w.fly(10.0, |_| false);
    let got = given(w);
    w.vessel_mut().plan.remove(0);
    w.sas(SasMode::Stability);
    Some(got)
}
