//! The lunar module by itself: from lunar orbit down to the surface and back up, flown with a
//! player's controls (the throttle, the stage key, RCS and pointing).

use osp_sim::{DVec3, gear};

use crate::{
    Mission,
    pilot::Pilot,
    world::{Area, Flight, PhaseSpec, World},
};

pub static MISSION: Mission = Mission {
    id: "lunar_module",
    title: "Lunar module: down to the Moon and back up",
    summary: "Eagle separated in lunar orbit and fell to 15 km. The descent engine burnt for \
              12 minutes against its speed, then the crew flew it down the last hundred metres \
              and touched down at under a metre a second. 21 hours later the ascent engine \
              lifted the cabin off the descent stage and into orbit.",
    scenario: "apollo11_landing",
    plan: PLAN,
    fly,
};

const PLAN: &[PhaseSpec] = &[
    PhaseSpec {
        id: "braking",
        title: "Powered descent",
        goal: "At 15 km the descent engine burns against the orbital speed of 1.7 km/s.",
    },
    PhaseSpec {
        id: "landing",
        title: "The last hundred metres",
        goal: "Nearly hovering, it comes straight down and touches down at under 3 m/s.",
    },
    PhaseSpec {
        id: "surface",
        title: "On the Moon",
        goal: "It stands on its legs for 21 h 36 min with its batteries holding.",
    },
    PhaseSpec {
        id: "ascent",
        title: "Back to orbit",
        goal: "The ascent engine leaves the descent stage on the ground and reaches orbit.",
    },
];

/// How high the braking phase aims to end, m.
const GATE: f64 = 150.0;
/// Horizontal speed under which it comes straight down, m/s.
const HOVER_SPEED: f64 = 3.0;
/// Sink at touchdown it aims for, m/s.
const TOUCHDOWN: f64 = 1.0;
/// How far the feet are below the centre of mass, roughly, m.
const FEET: f64 = 4.0;
/// The real stay on the surface, s.
const STAY: f64 = 77_760.0;
/// Height the ascent flies for, m, and the low point of the orbit that will do.
const ORBIT_HEIGHT: f64 = 18_000.0;
const ORBIT_PERIAPSIS: f64 = 12_000.0;

/// What a pilot sees: height over the ground, speed across it and up, gravity, and how much the
/// engine could push now.
struct Reading {
    height: f64,
    up: DVec3,
    across: DVec3,
    climb: f64,
    /// Gravity, and gravity less what going round the Moon takes off it, m/s².
    weight_alone: f64,
    weight: f64,
    push: f64,
}

fn read(w: &World, thrust: f64) -> Reading {
    let (system, now) = (&w.universe.system, w.now());
    let v = w.vessel();
    let body = system.body(v.parent());
    let state = v.state_at(system, now);
    let r = state.position.length();
    let up = state.position / r;
    let fixed = body.orientation(now).inverse() * up;
    let ground = state.velocity - body.surface_velocity(state.position);
    let climb = ground.dot(up);
    let across = ground - up * climb;
    Reading {
        height: r - body.radius - body.terrain_height(fixed) - FEET,
        up,
        across,
        climb,
        weight_alone: body.gm / (r * r),
        weight: body.gm / (r * r) - across.length_squared() / r,
        push: thrust / v.flight.mass.max(1.0),
    }
}

/// Set the throttle and point the nose so as to push by `wanted`, m/s².
fn push(w: &mut World, pilot: &mut Pilot, reading: &Reading, wanted: DVec3) {
    let throttle = (wanted.length() / reading.push).clamp(0.0, 1.0);
    w.vessel_mut().set_throttle(throttle);
    pilot.point(w, wanted.normalize_or(reading.up));
}

/// One tick of the descent. `true` once the feet have touched.
fn descend(w: &mut World, pilot: &mut Pilot, engine_thrust: f64) -> bool {
    if w.vessel().wheels_down() {
        w.vessel_mut().set_throttle(0.0);
        pilot.point(w, DVec3::ZERO);
        w.vessel_mut().rotation_input = DVec3::ZERO;
        return true;
    }
    let reading = read(w, engine_thrust);
    let speed = reading.across.length();
    let wanted = if speed > HOVER_SPEED {
        // Against the speed, losing height no faster than will bring it to the gate as the speed
        // runs out.
        let left = (speed / (0.85 * reading.push)).max(20.0);
        let sink = -((reading.height - GATE) / left).clamp(-5.0, 120.0);
        let lift = (reading.weight + (sink - reading.climb) / 10.0).clamp(0.0, 0.9 * reading.push);
        let against = (reading.push * reading.push - lift * lift)
            .sqrt()
            .min(speed);
        reading.up * lift - reading.across.normalize_or_zero() * against
    } else {
        // Straight down, slower the lower it gets.
        let sink = -(0.1 * reading.height).clamp(TOUCHDOWN, 25.0);
        let lift = reading.weight + (sink - reading.climb) / 3.0;
        reading.up * lift.max(0.2) - reading.across * 0.3
    };
    push(w, pilot, &reading, wanted);
    false
}

fn fly(w: &mut World) -> Flight {
    let mass = w
        .vessel()
        .flight
        .mass
        .max(w.vessel().mass_properties().mass);
    w.within(
        Area::Build,
        "It weighs what Eagle weighed",
        mass,
        14_950.0..=15_260.0,
        "kg",
        "15,103 kg at undocking",
    );
    let legs = gear::legs(&w.vessel().propulsion).len();
    w.check(
        Area::Landing,
        "It has four legs",
        legs == 4,
        format!("{legs}"),
    );

    w.phase("braking");
    // Coast to the low point, with the engine pointing the way it will fire.
    let radius = w.universe.system.body(w.vessel().parent()).radius;
    let reached = w.coast(3_600.0, |w| {
        let (system, now) = (&w.universe.system, w.now());
        let v = w.vessel();
        let state = v.state_at(system, now);
        state.position.length() - radius < 16_000.0
    });
    w.must(Area::Orbit, "It comes down to the low point", reached, "")?;
    let mut pilot = Pilot::new();
    w.vessel_mut().set_throttle(0.0);
    w.stage();
    // Against the way it is going, and the propellant settled with the RCS before it lights.
    w.fly(45.0, |w| {
        let reading = read(w, 1.0);
        pilot.point(w, -reading.across.normalize_or_zero());
        false
    });
    let thrust = {
        let v = w.vessel();
        v.propulsion
            .engines
            .iter()
            .find(|e| e.def.id.starts_with("LMDE"))
            .map_or(0.0, |e| e.def.thrust_vac)
    };
    w.check(
        Area::Propulsion,
        "The descent engine is on board",
        thrust > 0.0,
        "",
    );
    w.vessel_mut().translation_input = DVec3::Z;
    w.fly(4.0, |w| {
        let reading = read(w, 1.0);
        pilot.point(w, -reading.across.normalize_or_zero());
        false
    });
    w.vessel_mut().set_throttle(1.0);
    w.fly(2.0, |w| {
        let reading = read(w, 1.0);
        pilot.point(w, -reading.across.normalize_or_zero());
        false
    });
    w.vessel_mut().translation_input = DVec3::ZERO;
    let lit = w.vessel().propulsion.engines.iter().any(|e| e.running);
    w.must(
        Area::Propulsion,
        "The descent engine lights on RCS ullage",
        lit,
        "",
    )?;
    let mut hovering = false;
    let touched = w.fly(1_500.0, |w| {
        let done = descend(w, &mut pilot, thrust);
        if !hovering && read(w, thrust).across.length() < HOVER_SPEED {
            hovering = true;
            w.phase("landing");
        }
        done
    });
    let log: Vec<String> = w.vessel().log.iter().map(|e| e.message.clone()).collect();
    let crashed = log.iter().find(|m| m.starts_with("Crashed")).cloned();
    w.must(
        Area::Landing,
        "It touches down",
        touched && crashed.is_none(),
        crashed.unwrap_or_default(),
    )?;
    let sink = log
        .iter()
        .find_map(|m| m.strip_prefix("Touched down at "))
        .and_then(|m| m.split(' ').next())
        .and_then(|m| m.parse::<f64>().ok())
        .unwrap_or(f64::INFINITY);
    w.within(
        Area::Landing,
        "It touches down softly",
        sink,
        0.0..=3.0,
        "m/s",
        "Eagle: 0.5 m/s",
    );
    // Standing still.
    let stopped = w.fly(30.0, |w| w.vessel().is_landed());
    w.must(Area::Landing, "It comes to rest on its legs", stopped, "")?;

    w.phase("surface");
    let (system, now) = (&w.universe.system, w.now());
    let v = w.vessel();
    let state = v.state_at(system, now);
    let nose = v.attitude_state(system, now).nose();
    let tilt = nose.angle_between(state.position.normalize()).to_degrees();
    w.within(
        Area::Landing,
        "It stands upright",
        tilt,
        0.0..=12.0,
        "degrees",
        "Eagle: 4.5 degrees",
    );
    let waited = w.coast(STAY, |_| false);
    let browned = w.vessel().log.iter().any(|e| e.message.contains("power"));
    w.check(Area::Power, "The batteries hold for the stay", !browned, "");
    w.check(
        Area::Landing,
        "Nothing stops the stay short",
        waited || w.vessel().is_landed(),
        "",
    );
    w.check(
        Area::Landing,
        "It is still on its legs",
        w.vessel().is_landed(),
        "",
    );

    w.phase("ascent");
    w.vessel_mut().set_throttle(1.0);
    w.stage();
    let aps = {
        let v = w.vessel();
        v.propulsion
            .engines
            .iter()
            .find(|e| e.def.id == "LMAE")
            .map_or(0.0, |e| e.def.thrust_vac)
    };
    w.must(
        Area::Staging,
        "The ascent engine is what is left",
        aps > 0.0,
        "",
    )?;
    let others = w.universe.vessels.len();
    w.check(
        Area::Staging,
        "The descent stage stays behind",
        others == 2,
        format!("{others} vessels"),
    );
    let lifted = w.fly(10.0, |w| {
        let reading = read(w, aps);
        pilot.point(w, reading.up);
        reading.climb > 1.0
    });
    w.must(Area::Landing, "It lifts off", lifted, "")?;
    // Up, then over to the east, holding a climb until it is well clear, until it is in orbit.
    let in_orbit = w.fly(900.0, |w| {
        let reading = read(w, aps);
        let (system, now) = (&w.universe.system, w.now());
        let v = w.vessel();
        let body = system.body(v.parent());
        let periapsis = v.orbit(system, now).periapsis() - body.radius;
        let pole = body.orientation(now) * DVec3::Z;
        let east = pole.cross(reading.up).normalize_or_zero();
        if periapsis >= ORBIT_PERIAPSIS {
            w.vessel_mut().set_throttle(0.0);
            return true;
        }
        let aim = if reading.height < 100.0 {
            reading.up
        } else {
            // Climbing no faster than will bring it to its height as the speed comes up, and
            // levelling off as it does, for the orbit's low point is only as high as that.
            let circular = (reading.weight_alone * (reading.height + radius)).sqrt();
            let left = (circular - reading.across.length()).max(0.0) / reading.push;
            let climb = ((ORBIT_HEIGHT - reading.height) / left.max(20.0)).clamp(0.0, 60.0)
                * (left / 60.0).min(1.0);
            let most = 0.95 * reading.push;
            let lift = (reading.weight + (climb - reading.climb) / 10.0).clamp(-0.3 * most, most);
            let along = (reading.push * reading.push - lift * lift).sqrt();
            reading.up * lift + east * along
        };
        pilot.point(w, aim);
        false
    });
    w.must(Area::Orbit, "The ascent stage reaches orbit", in_orbit, "")?;
    w.gap(
        Area::Docking,
        "Back to the command module",
        "It has reached orbit and stops there: nothing can dock yet (T040), so it cannot get the \
         crew back to the command module.",
    );
    Ok(())
}
