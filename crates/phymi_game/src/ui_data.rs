//! Publishes game state into the UI data store, for layouts (and mods) to bind to.
//!
//! Every key is listed in `docs/modding.md`; in game, press F12 for the data inspector.

use bevy::prelude::*;
use phymi_mods::Severity;
use phymi_sim::{
    constants::G0,
    events::EventKind,
    time::SimTime,
    vessel::{Motion, Situation},
};
use phymi_ui::{Formatters, UiData, UiValue, record};

use crate::{
    FrameSet,
    camera::CameraRig,
    content::GameContent,
    orbit_lines::Prediction,
    sim::{Sim, SimEventMessage, TimeWarp},
};

/// How long warp notices stay up, real seconds.
const NOTICE_DURATION: f64 = 4.0;
const LOG_LINES: usize = 6;

pub struct UiDataPlugin;

impl Plugin for UiDataPlugin {
    fn build(&self, app: &mut App) {
        let mut formatters = app.world_mut().resource_mut::<Formatters>();
        formatters.register("date", |v, _| {
            v.as_number()
                .map_or("-".into(), |s| format_date(SimTime::from_seconds(s)))
        });
        formatters.register("clock", |v, _| {
            v.as_number()
                .map_or("-".into(), |s| format_clock(SimTime::from_seconds(s)))
        });
        app.add_systems(
            Update,
            (publish_flight, publish_mods).in_set(FrameSet::Render),
        );
    }
}

fn publish_flight(
    mut events: MessageReader<SimEventMessage>,
    sim: Res<Sim>,
    warp: Res<TimeWarp>,
    rig: Res<CameraRig>,
    prediction: Res<Prediction>,
    time: Res<Time>,
    mut data: ResMut<UiData>,
) {
    let system = &sim.universe.system;
    let now = sim.render_time;
    let vessel = sim.active_vessel();
    let body = system.body(vessel.parent());
    let state = vessel.state_at(system, now);
    let orbit = vessel.orbit(system, now);
    let altitude = state.position.length() - body.radius;
    let (lat, lon) = body.lat_lon(state.position, now);
    let mass = vessel.flight.mass.max(1.0);
    let local_g = body.gm / state.position.length_squared();

    data.set("time.ut", now.as_seconds());
    data.set("time.warp", warp.rate());
    data.set(
        "time.warp_mode",
        if warp.on_rails() { "rails" } else { "physics" },
    );
    data.set("time.lagging", sim.lagging);
    let notice = warp
        .notice
        .as_ref()
        .filter(|(_, shown)| time.elapsed_secs_f64() - shown < NOTICE_DURATION)
        .map(|(n, _)| n.clone());
    data.set("time.notice", notice.unwrap_or_default());
    data.set("view.map", rig.map_view);
    for SimEventMessage(event) in events.read() {
        let text = sim.universe.vessels[event.vessel]
            .log
            .last()
            .map(|entry| entry.message.clone())
            .unwrap_or_default();
        data.set("sim.last_event", text);
        data.set("sim.last_event_time", event.event.time.as_seconds());
    }

    data.set("vessel.name", vessel.name.as_str());
    data.set("vessel.body", body.name.as_str());
    data.set(
        "vessel.situation",
        match vessel.situation(system, now) {
            Situation::Landed => "Landed",
            Situation::Flying => "Flying",
            Situation::SubOrbital => "Sub-orbital",
            Situation::Orbiting => "Orbiting",
            Situation::Escaping => "Escaping",
        },
    );
    data.set(
        "vessel.motion",
        match vessel.motion() {
            Motion::Landed { .. } => "landed",
            Motion::Coasting(_) => "coasting",
            Motion::Integrated(_) => "integrated",
        },
    );
    data.set("vessel.landed", vessel.is_landed());
    data.set("vessel.altitude", altitude);
    data.set("vessel.vertical_speed", state.radial_speed());
    data.set(
        "vessel.surface_speed",
        (state.velocity - body.surface_velocity(state.position)).length(),
    );
    data.set("vessel.orbital_speed", state.velocity.length());
    data.set("vessel.latitude", lat.to_degrees());
    data.set("vessel.longitude", lon.to_degrees());
    data.set("vessel.mach", vessel.flight.mach);
    data.set("vessel.dynamic_pressure", vessel.flight.dynamic_pressure);
    data.set("vessel.static_pressure", vessel.flight.static_pressure);
    data.set("vessel.throttle", vessel.propulsion.throttle());
    data.set("vessel.thrust", vessel.flight.thrust);
    data.set("vessel.twr", vessel.flight.thrust / (mass * local_g));
    data.set("vessel.mass", mass);
    data.set("vessel.accel_g", vessel.flight.proper_acceleration / G0);
    data.set(
        "vessel.engines_running",
        vessel.propulsion.any_engine_running(),
    );

    let stages = &vessel.propulsion.stages;
    let rows: Vec<_> = stages
        .iter()
        .zip(vessel.propulsion.stage_info())
        .enumerate()
        .map(|(i, (stage, info))| {
            record([
                ("name", info.name.into()),
                ("active", (i == 0 && stage.active).into()),
                ("delta_v", info.delta_v_vac.into()),
                ("burn_time", info.burn_time.into()),
                ("fuel", stage.fuel_fraction().into()),
                ("has_fuel", stage.fuel_fraction().is_some().into()),
                (
                    "ignitions",
                    stage.engines.first().and_then(|e| e.ignitions_left).into(),
                ),
                ("engines", stage.engines.len().into()),
            ])
        })
        .collect();
    data.set("vessel.stage_count", rows.len());
    data.set(
        "vessel.delta_v",
        vessel
            .propulsion
            .stage_info()
            .iter()
            .map(|s| s.delta_v_vac)
            .sum::<f64>(),
    );
    data.set("vessel.stages", rows);

    let log: Vec<_> = vessel
        .log
        .iter()
        .rev()
        .take(LOG_LINES)
        .map(|entry| {
            record([
                ("time", entry.time.as_seconds().into()),
                ("message", entry.message.as_str().into()),
            ])
        })
        .collect();
    data.set("vessel.log", log);

    data.set("orbit.apoapsis", orbit.apoapsis().map(|a| a - body.radius));
    data.set("orbit.periapsis", orbit.periapsis() - body.radius);
    data.set(
        "orbit.escape",
        orbit.apoapsis().is_none_or(|a| a > body.soi),
    );
    data.set(
        "orbit.inclination",
        orbit.inclination_to(body.pole()).to_degrees(),
    );
    data.set("orbit.eccentricity", orbit.eccentricity());
    data.set("orbit.semi_major_axis", orbit.semi_major_axis());
    data.set("orbit.period", orbit.period());
    let events: Vec<_> = prediction
        .patches
        .iter()
        .filter_map(|patch| {
            let event = patch.ending?;
            let label = match event.kind {
                EventKind::SoiExit => format!("Escape {}", system.body(patch.frame).name),
                EventKind::SoiEntry(b) => format!("Encounter {}", system.body(b).name),
                EventKind::Descent => format!("Descent into {}", system.body(patch.frame).name),
            };
            Some(record([
                ("label", label.into()),
                ("time_to", event.time.seconds_since(now).into()),
            ]))
        })
        .collect();
    data.set("orbit.events", events);
}

fn publish_mods(content: Res<GameContent>, mut data: ResMut<UiData>) {
    if !content.is_changed() {
        return;
    }
    let content = &content.0;
    let mods: Vec<_> = content
        .mods
        .iter()
        .map(|m| {
            record([
                ("id", m.manifest.id.as_str().into()),
                ("name", m.manifest.name.as_str().into()),
                ("version", m.manifest.version.as_str().into()),
                ("authors", m.manifest.authors.join(", ").into()),
                ("description", m.manifest.description.as_str().into()),
            ])
        })
        .collect();
    let count = |s| {
        content
            .diagnostics
            .iter()
            .filter(|d| d.severity == s)
            .count()
    };
    let problems: Vec<_> = content
        .diagnostics
        .iter()
        .filter(|d| d.severity != Severity::Info)
        .map(|d| {
            record([
                ("error", (d.severity == Severity::Error).into()),
                ("text", d.to_string().into()),
            ])
        })
        .collect();
    data.set("mods.count", mods.len());
    data.set("mods.list", mods);
    data.set("mods.errors", count(Severity::Error));
    data.set("mods.warnings", count(Severity::Warning));
    data.set("mods.problems", UiValue::List(problems));
}

pub fn format_clock(ut: SimTime) -> String {
    let s = (ut.as_seconds() + 43_200.0).rem_euclid(86_400.0) as u64;
    format!("{:02}:{:02}:{:02}", s / 3_600, s / 60 % 60, s % 60)
}

/// UT (seconds since J2000, 2000-01-01 12:00) as a calendar date. Ignores leap seconds.
pub fn format_date(ut: SimTime) -> String {
    let since_midnight = ut.as_seconds() + 43_200.0;
    let days = since_midnight.div_euclid(86_400.0) as i64 + 10_957; // days since 1970-01-01
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {}", format_clock(ut))
}
