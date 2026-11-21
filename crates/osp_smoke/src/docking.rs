//! Docking in lunar orbit: Columbia turns round, flies the last metres to Eagle's ascent stage on
//! its RCS and captures, the two fly as one, and Eagle is let go. Flown as a player flies it: the
//! translation keys are on or off and change only a few times a second, on the numbers the docking
//! window shows. The first try comes in too fast, bumps and bounces off.

use osp_sim::{
    DVec3,
    attitude::SasMode,
    docking::{MAX_ANGLE, MAX_CLOSING, MAX_DRIFT, MAX_OFF_AXIS, PortApproach},
    staging::{Action, Activation},
    vessel::Vessel,
};

use crate::{
    Mission,
    world::{Abort, Area, Flight, PhaseSpec, World},
};

pub static MISSION: Mission = Mission {
    id: "docking",
    title: "Apollo 11: docking in lunar orbit",
    summary: "Eagle's ascent stage came back up to Columbia in lunar orbit. Collins turned the \
              CSM round, flew the last metres on the RCS and caught Eagle's drogue on his probe. \
              Armstrong and Aldrin came through the tunnel, and Eagle was let go.",
    scenario: "apollo11_docking",
    mode: None,
    plan: PLAN,
    fly,
};

const PLAN: &[PhaseSpec] = &[
    PhaseSpec {
        id: "turn",
        title: "Turning round",
        goal: "SAS turns the CSM's probe to face the drogue on Eagle.",
    },
    PhaseSpec {
        id: "bump",
        title: "Too fast",
        goal: "Coming in at 0.8 m/s, the probe does not catch: the two bump and bounce apart.",
    },
    PhaseSpec {
        id: "approach",
        title: "The last metres",
        goal: "On the keys, down Eagle's axis to a capture at under 0.3 m/s.",
    },
    PhaseSpec {
        id: "docked",
        title: "Docked",
        goal: "The two fly as one, on the service module's engine.",
    },
    PhaseSpec {
        id: "undock",
        title: "Letting go",
        goal: "Eagle is itself again, and the CSM backs away.",
    },
];

/// The speed it closes at from afar, m/s, and in the last metres.
const CRUISE: f64 = 1.5;
const FINAL: f64 = 0.12;
/// A pilot's slope: how fast to close per metre still to go, 1/s.
const SLOPE: f64 = 0.03;
/// How far from the axis it may be and still close, m, and share of what is left to go.
const CORRIDOR: f64 = 0.3;
const CORRIDOR_SHARE: f64 = 0.1;
/// The speed across per metre off the axis it corrects at (1/s), and the fastest it slides, m/s.
const SIDE_GAIN: f64 = 0.15;
const SIDE_SPEED: f64 = 0.08;
/// How far off the speed it wants it lets a speed get before it touches a key, m/s.
const CLOSING_SLACK: f64 = 0.03;
const SIDE_SLACK: f64 = 0.02;
/// How often it looks at the numbers and changes the keys it is holding, s.
const LOOK: f64 = 0.25;
/// How fast it comes in on the try that is too fast, m/s, from this far out.
const TOO_FAST: f64 = 0.8;
const CARELESS_FROM: f64 = 10.0;

pub(crate) fn approach(w: &World) -> Option<PortApproach> {
    let index = w.universe.find_vessel(w.focus)?;
    w.universe.port_approach(index, w.now())
}

/// Which way to hold a key to bring `have` to `want`: none within `slack`.
fn key(want: f64, have: f64, slack: f64) -> f64 {
    if have < want - slack {
        1.0
    } else if have > want + slack {
        -1.0
    } else {
        0.0
    }
}

/// A pilot's hands on the translation keys: the docking window's numbers, in the pilot's axes.
pub(crate) struct Hands {
    next_look: f64,
    /// Closing at this, whatever the gap.
    careless: Option<f64>,
}

impl Hands {
    pub(crate) fn new() -> Self {
        Self {
            next_look: 0.0,
            careless: None,
        }
    }

    pub(crate) fn fly(&mut self, w: &mut World, seen: &PortApproach) {
        if w.t() < self.next_look {
            return;
        }
        self.next_look = w.t() + LOOK;
        let a = &seen.alignment;
        let (system, now) = (&w.universe.system, w.now());
        let turned = w.vessel().attitude_at(system, now);
        let (right, up) = (turned * DVec3::X, turned * DVec3::NEG_Y);
        let off = a.off_axis.length();
        let closing = match self.careless {
            Some(speed) => speed,
            None if off > CORRIDOR + CORRIDOR_SHARE * a.gap.max(0.0) => 0.0,
            None => (SLOPE * a.gap).clamp(FINAL, CRUISE),
        };
        let side = |axis: DVec3| {
            let want = (-SIDE_GAIN * a.off_axis.dot(axis)).clamp(-SIDE_SPEED, SIDE_SPEED);
            key(want, a.drift.dot(axis), SIDE_SLACK)
        };
        let keys = DVec3::new(
            side(right),
            -side(up),
            key(closing, a.closing, CLOSING_SLACK),
        );
        w.vessel_mut().translation_input = keys;
    }
}

/// `name`'s vessel, once it is one.
fn other<'a>(w: &'a World, name: &str) -> Option<&'a Vessel> {
    w.universe.vessels.iter().find(|v| v.name == name)
}

fn fly(w: &mut World) -> Flight {
    let eagle_mass = other(w, "Eagle").map_or(0.0, |e| e.propulsion.mass());
    w.within(
        Area::Build,
        "Eagle's ascent stage weighs what it did",
        eagle_mass,
        2_000.0..=5_000.0,
        "kg",
        "full tanks, as if it had not just flown up; Apollo 11's came back at about 2.6 t",
    );
    let columbia_mass = w.vessel().propulsion.mass();
    let columbia_reach: f64 = (w.vessel().propulsion.stage_info().iter())
        .map(|row| row.delta_v_vac)
        .sum();
    let start = approach(w);
    w.must(
        Area::Docking,
        "The CSM's probe has Eagle's drogue to go for",
        start.is_some(),
        "",
    )?;
    let start = start.ok_or(Abort)?;
    w.event(format!(
        "{:.0} m from the drogue, facing away from it",
        start.alignment.distance()
    ));

    w.phase("turn");
    w.with_vessel(|v, system, now| {
        v.set_rcs(true);
        v.set_sas_mode(system, now, SasMode::TargetPort);
    });
    let turned = w.fly(600.0, |w| {
        approach(w).is_some_and(|a| a.alignment.angle < 1f64.to_radians())
    });
    let angle = approach(w).map_or(180.0, |a| a.alignment.angle.to_degrees());
    w.must(
        Area::Control,
        "SAS turns the probe to face the drogue",
        turned,
        format!("{angle:.1} degrees off"),
    )?;
    w.end_phase();

    w.phase("bump");
    // Holding the key too long on the way in.
    let mut hands = Hands::new();
    let near = w.fly(1_800.0, |w| {
        let Some(seen) = approach(w) else {
            return false;
        };
        hands.fly(w, &seen);
        seen.alignment.gap < CARELESS_FROM
    });
    w.must(Area::Docking, "It gets near the drogue", near, "")?;
    hands.careless = Some(TOO_FAST);
    let mut deepest = f64::INFINITY;
    let mut hit = 0.0;
    let bumped = w.fly(120.0, |w| {
        let Some(seen) = approach(w) else {
            return true;
        };
        hands.fly(w, &seen);
        deepest = deepest.min(seen.alignment.gap);
        hit = seen.alignment.closing.max(hit);
        w.vessel()
            .log
            .iter()
            .any(|e| e.message.starts_with("Bumped"))
    });
    w.with_vessel(|v, _, _| v.translation_input = DVec3::ZERO);
    let told = w
        .vessel()
        .log
        .iter()
        .rev()
        .find(|e| e.message.starts_with("Bumped"));
    let told = told.map_or(String::new(), |e| e.message.clone());
    w.must(
        Area::Docking,
        "Coming in too fast, it bumps instead of going through",
        bumped && other(w, "Eagle").is_some(),
        format!("hit at {hit:.2} m/s; {told}"),
    )?;
    w.check(
        Area::Docking,
        "The pilot is told why",
        told.contains("Too fast"),
        told.clone(),
    );
    w.within(
        Area::Docking,
        "The ports do not pass each other",
        -deepest,
        -1.0..=0.5,
        "m past",
        "they stop at the port",
    );
    // A few seconds to see what it did: they part, and Eagle, spun by the hit, takes the spin out on
    // the attitude hold it is flying.
    w.fly(5.0, |_| false);
    let eagle_after_hit = other(w, "Eagle").map_or(0.0, |e| e.propulsion.mass());
    w.within(
        Area::Docking,
        "Eagle's thrusters take out the spin of the hit",
        eagle_mass - eagle_after_hit,
        0.0..=2.0,
        "kg of RCS propellant",
        "it holds its attitude, so a hit off its centre of mass costs a little",
    );
    let after = approach(w).ok_or(Abort)?;
    w.check(
        Area::Docking,
        "They bounce apart",
        after.alignment.closing < 0.0 && after.alignment.gap > 0.0,
        format!(
            "{:.2} m apart, parting at {:.2} m/s",
            after.alignment.gap, -after.alignment.closing
        ),
    );
    w.end_phase();

    w.phase("approach");
    let mut hands = Hands::new();
    let mut last = None;
    let joined = w.fly(3_600.0, |w| {
        if w.vessel().propulsion.joined_parts().next().is_some() {
            return true;
        }
        let Some(seen) = approach(w) else {
            return false;
        };
        hands.fly(w, &seen);
        last = Some(seen.alignment);
        false
    });
    w.with_vessel(|v, system, now| {
        v.translation_input = DVec3::ZERO;
        v.set_sas_mode(system, now, SasMode::Stability);
    });
    w.must(
        Area::Docking,
        "On the keys, at a sensible speed, the ports capture",
        joined,
        last.map_or(String::new(), |a| {
            format!(
                "still {:.2} m off, {:.2} m/s: {}",
                a.distance(),
                a.closing,
                a.problem().map_or("no problem", |p| p.label())
            )
        }),
    )?;
    let last = last.ok_or(Abort)?;
    w.within(
        Area::Docking,
        "It closes slowly enough",
        last.closing,
        0.0..=MAX_CLOSING,
        "m/s",
        "the funnel takes up to 0.5 m/s; Apollo's procedures asked for under 0.3",
    );
    w.within(
        Area::Docking,
        "It is on the drogue's axis",
        last.off_axis.length(),
        0.0..=MAX_OFF_AXIS,
        "m",
        "the funnel takes 0.4 m; Apollo's procedures asked for 0.3",
    );
    w.within(
        Area::Docking,
        "It faces the drogue",
        last.angle.to_degrees(),
        0.0..=MAX_ANGLE.to_degrees(),
        "degrees",
        "the funnel takes 15; Apollo's procedures asked for 10",
    );
    w.within(
        Area::Docking,
        "It is not sliding across",
        last.drift.length(),
        0.0..=MAX_DRIFT,
        "m/s",
        "the funnel takes 0.3 m/s; Apollo's procedures asked for 0.15",
    );
    w.end_phase();

    w.phase("docked");
    let reach = |w: &World| -> f64 {
        (w.vessel().propulsion.stage_info().iter())
            .map(|row| row.delta_v_vac)
            .sum()
    };
    let with_eagle = reach(w);
    w.check(
        Area::Docking,
        "The delta-v readout counts Eagle as a load",
        with_eagle > 0.5 * columbia_reach && with_eagle < columbia_reach,
        format!("{with_eagle:.0} m/s with it, {columbia_reach:.0} m/s alone"),
    );
    let both = w.vessel().propulsion.mass();
    let burnt = columbia_mass - both + eagle_mass;
    w.check(
        Area::Docking,
        "The two weigh what they did, less the RCS it took",
        (0.0..=200.0).contains(&burnt),
        format!("{both:.0} kg, {burnt:.1} kg of RCS propellant gone"),
    );
    w.check(
        Area::Docking,
        "Eagle is no longer a vessel of its own",
        other(w, "Eagle").is_none(),
        "",
    );
    let networks = w.vessel().propulsion.parts().power_groups();
    let main = networks.iter().map(Vec::len).max().unwrap_or(0);
    let parts = w.vessel().propulsion.parts().parts().len();
    w.check(
        Area::Power,
        "Power passes through the port",
        main * 2 > parts,
        format!("{main} of {parts} parts on the largest network"),
    );
    // The service module's engine, with Eagle on the nose: it is why they could come home.
    let lander_engines: Vec<_> = (w.vessel().propulsion.engines.iter())
        .filter(|e| e.direction.z < 0.0)
        .map(|e| e.part)
        .collect();
    w.with_vessel(|v, _, _| v.translation_input = DVec3::Z);
    w.fly(10.0, |_| false);
    w.with_vessel(|v, _, _| {
        v.translation_input = DVec3::ZERO;
        v.set_throttle(1.0);
    });
    let mut peak: f64 = 0.0;
    w.fly(3.0, |w| {
        peak = peak.max(w.vessel().flight.thrust);
        false
    });
    w.with_vessel(|v, _, _| v.set_throttle(0.0));
    w.fly(3.0, |_| false);
    w.within(
        Area::Propulsion,
        "The service module's engine pushes the two",
        peak,
        85_000.0..=105_000.0,
        "N",
        "the SPS: 91 kN",
    );
    w.check(
        Area::Propulsion,
        "Eagle's engines stay off",
        (w.vessel().propulsion.engines.iter())
            .filter(|e| lander_engines.contains(&e.part))
            .all(|e| e.thrust == 0.0 && !e.running),
        "",
    );
    // Flown from Eagle instead: its engines and its axes are the ones that count.
    let lander = (w.vessel().propulsion.parts().parts().iter())
        .find(|p| p.name == "Ascent Stage")
        .map(|p| p.id)
        .ok_or(Abort)?;
    let from = |part| Activation::new(part, Action::CONTROL);
    let root = w.vessel().propulsion.parts().root().ok_or(Abort)?.id;
    w.universe.perform(w.focus, from(lander));
    let bottom: Vec<String> = (w.vessel().propulsion.bottom())
        .map(|s| s.name.clone())
        .collect();
    let nose = w.vessel().propulsion.control_rotation() * DVec3::Z;
    w.check(
        Area::Control,
        "Flown from Eagle, its stage and its axes count",
        bottom.iter().all(|n| n == "Ascent Stage") && nose.z < -0.99,
        format!("bottom of the stack: {bottom:?}, nose {nose}"),
    );
    w.universe.perform(w.focus, from(root));
    w.end_phase();

    w.phase("undock");
    let port = w.vessel().propulsion.joined_parts().next().ok_or(Abort)?;
    let done = w.universe.perform(w.focus, Activation::undock(port));
    w.must(Area::Docking, "The port lets go", done, "")?;
    let left = other(w, "Eagle").cloned();
    w.must(
        Area::Docking,
        "Eagle is a vessel of its own again",
        left.is_some(),
        "",
    )?;
    let eagle = left.ok_or(Abort)?;
    w.within(
        Area::Docking,
        "Eagle weighs what it did",
        eagle.propulsion.mass(),
        eagle_after_hit * 0.9999..=eagle_after_hit * 1.0001,
        "kg",
        "its thrusters stayed off while the CSM flew the two",
    );
    w.check(
        Area::Docking,
        "Eagle's port is free again",
        eagle.propulsion.docking_ports().iter().all(|p| p.free),
        "",
    );
    w.check(
        Area::Docking,
        "The CSM's port is free again",
        w.vessel().propulsion.docking_ports().iter().all(|p| p.free),
        "",
    );
    // Their springs part them, along the line of the ports, and nothing has moved yet.
    let (opening, stood) = {
        let (system, now) = (&w.universe.system, w.now());
        let (a, b) = (
            w.vessel().state_at(system, now),
            eagle.state_at(system, now),
        );
        (
            (a.velocity - b.velocity).length(),
            (a.position - b.position).length(),
        )
    };
    w.within(
        Area::Docking,
        "The springs part them",
        opening,
        0.2..=1.0,
        "m/s",
        "2,000 N s between 30 t and 5 t",
    );
    w.within(
        Area::Docking,
        "They start where they were",
        stood,
        3.0..=20.0,
        "m between their centres of mass",
        "nothing jumps as they let go",
    );
    // Backing away on the RCS, without catching it again.
    w.with_vessel(|v, _, _| v.translation_input = DVec3::NEG_Z);
    let name = eagle.name.clone();
    let apart = |w: &World| {
        let now = w.now();
        other(w, &name).map_or(0.0, |e| {
            let system = &w.universe.system;
            (e.state_at(system, now).position - w.vessel().state_at(system, now).position).length()
        })
    };
    let away = w.fly(300.0, |w| apart(w) > 60.0);
    w.with_vessel(|v, _, _| v.translation_input = DVec3::ZERO);
    let reached = apart(w);
    w.check(
        Area::Docking,
        "The CSM pulls away",
        away && w.vessel().propulsion.joined_parts().next().is_none(),
        format!("{reached:.0} m apart"),
    );
    let met =
        (w.universe.mission.as_ref()).is_some_and(|m| m.goals().iter().all(|g| g.met_at.is_some()));
    w.check(
        Area::Docking,
        "The scenario's goals are met",
        met,
        "dock with Eagle, let it go and back off",
    );
    w.gap(
        Area::Crew,
        "Crew moving through the tunnel",
        "The port has no tunnel and a crew seat belongs to one pod, so Armstrong and Aldrin are \
         still on Eagle when it goes. (T049)",
    );
    w.end_phase();
    Ok(())
}
