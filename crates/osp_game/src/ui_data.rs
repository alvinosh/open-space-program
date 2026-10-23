//! Publishes game state into the UI data store, for layouts (and mods) to bind to.
//!
//! Every key is listed in `docs/modding.md`; in game, press F12 for the data inspector.

use bevy::{math::DVec3, prelude::*};
use osp_mods::Severity;
use osp_sim::{
    constants::G0,
    data::TankShape,
    events::EventKind,
    structure::{Mode, Part, PartLoad},
    time::SimTime,
    vessel::{Motion, Situation, VesselEventKind},
};
use osp_ui::{Formatters, UiData, UiValue, record};

use crate::{
    FrameSet,
    camera::CameraRig,
    content::GameContent,
    orbit_lines::Prediction,
    sim::{Sim, SimEventMessage, TimeWarp},
};

/// How long warp notices and screen messages stay up, real seconds.
const NOTICE_DURATION: f64 = 4.0;
const LOG_LINES: usize = 8;

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
        // Compass heading, three digits: 090.
        formatters.register("heading", |v, _| {
            v.as_number()
                .filter(|h| h.is_finite())
                .map_or("---".into(), |h| format!("{:03.0}", h.rem_euclid(360.0)))
        });
        app.add_systems(
            Update,
            (publish_flight, publish_mods).in_set(FrameSet::Render),
        );
    }
}

/// What the flight UI remembers between frames.
#[derive(Default)]
struct FlightMemory {
    liftoff: Option<SimTime>,
    /// Latest log entry and the real time it appeared.
    message: Option<(SimTime, String, f64)>,
}

fn publish_flight(
    mut events: MessageReader<SimEventMessage>,
    sim: Res<Sim>,
    warp: Res<TimeWarp>,
    rig: Res<CameraRig>,
    prediction: Res<Prediction>,
    time: Res<Time>,
    mut memory: Local<FlightMemory>,
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
    let active = vessel.id();
    for SimEventMessage(event) in events.read() {
        // Other vessels' news is named after them; debris only matters while flown.
        let Some(from) = sim
            .universe
            .vessel(event.vessel)
            .filter(|v| v.id() == active || !v.is_debris())
        else {
            continue;
        };
        let message = from.log.last().map_or("", |entry| entry.message.as_str());
        let text = if from.id() == active {
            message.to_owned()
        } else {
            format!("{}: {message}", from.name)
        };
        data.set("sim.last_event", text);
        data.set("sim.last_event_time", event.event.time.as_seconds());
        if event.vessel == active && event.event.kind == VesselEventKind::Liftoff {
            memory.liftoff.get_or_insert(event.event.time);
        }
    }
    data.set("time.launched", memory.liftoff.is_some());
    data.set(
        "time.met",
        memory
            .liftoff
            .map_or(0.0, |t| now.seconds_since(t).max(0.0)),
    );

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
    data.set(
        "vessel.angle_of_attack",
        vessel.flight.angle_of_attack.to_degrees(),
    );
    data.set("vessel.sideslip", vessel.flight.sideslip.to_degrees());
    data.set("vessel.lift", vessel.flight.lift);
    data.set("vessel.drag", vessel.flight.drag);
    data.set("vessel.static_pressure", vessel.flight.static_pressure);
    data.set("vessel.throttle", vessel.propulsion.throttle());
    // What the engines actually give (spooling, slewing, tailing off), and the lowest
    // share they can hold while running.
    data.set("vessel.engine_level", vessel.propulsion.engine_level());
    data.set(
        "vessel.min_throttle",
        vessel.propulsion.stages.first().map_or(0.0, |s| {
            s.engines
                .iter()
                .map(|e| e.def.min_throttle.clamp(0.0, 1.0))
                .fold(0.0, f64::max)
        }),
    );
    data.set("vessel.thrust", vessel.flight.thrust);
    data.set("vessel.twr", vessel.flight.thrust / (mass * local_g));
    data.set("vessel.mass", mass);
    data.set("vessel.accel_g", vessel.flight.proper_acceleration / G0);
    data.set(
        "vessel.engines_running",
        vessel.propulsion.any_engine_running(),
    );

    // Newest log line as a screen message for a few seconds.
    let real = time.elapsed_secs_f64();
    if let Some(entry) = vessel.log.last()
        && memory
            .message
            .as_ref()
            .is_none_or(|(t, m, _)| (*t, m.as_str()) != (entry.time, entry.message.as_str()))
    {
        memory.message = Some((entry.time, entry.message.clone(), real));
    }
    let message = memory
        .message
        .as_ref()
        .filter(|(_, _, shown)| real - shown < NOTICE_DURATION)
        .map_or(String::new(), |(_, m, _)| m.clone());
    data.set("screen.message", message);

    // Attitude, SAS and the navball readouts.
    let attitude = vessel.attitude_state(system, now);
    let sas = vessel.sas();
    data.set("vessel.sas", sas.enabled());
    let (mode, label) = if sas.enabled() {
        (sas.mode().id(), sas.mode().label())
    } else {
        ("off", "Off")
    };
    data.set("vessel.sas_mode", mode);
    data.set("vessel.sas_mode_label", label);
    data.set("vessel.rcs", vessel.rcs());
    let rcs_fuel = vessel
        .propulsion
        .stages
        .iter()
        .filter_map(|s| s.rcs_fraction())
        .reduce(f64::min);
    data.set("vessel.has_rcs", rcs_fuel.is_some());
    data.set("vessel.rcs_fuel", rcs_fuel);
    data.set("vessel.rcs_mass", vessel.propulsion.rcs_propellant_mass());
    data.set("vessel.rcs_delta_v", vessel.propulsion.rcs_delta_v());
    let frame = vessel.nav_frame();
    data.set("nav.frame", frame.label());
    let speed = match frame {
        osp_sim::attitude::NavFrame::Surface => {
            state.velocity - body.surface_velocity(state.position)
        }
        osp_sim::attitude::NavFrame::Orbit => state.velocity,
        osp_sim::attitude::NavFrame::Target => {
            state.velocity - vessel.target_state().map_or(DVec3::ZERO, |t| t.velocity)
        }
    };
    data.set("nav.speed", speed.length());
    let up = state.position.normalize();
    let north = (body.pole() - up * body.pole().dot(up)).normalize_or(DVec3::X);
    let east = north.cross(up);
    let (nose, feet, right) = (
        attitude.nose(),
        attitude.orientation * DVec3::Y,
        attitude.orientation * DVec3::X,
    );
    let pitch = nose.dot(up).clamp(-1.0, 1.0).asin();
    // Pointing straight up or down, the heading is where pitching down would go.
    let toward = if nose.dot(up).abs() < 0.999 {
        nose
    } else {
        feet
    };
    let heading = toward.dot(east).atan2(toward.dot(north));
    let roll = if nose.dot(up).abs() < 0.999 {
        (-right.dot(up)).atan2(-feet.dot(up))
    } else {
        f64::NAN
    };
    data.set("nav.heading", heading.to_degrees().rem_euclid(360.0));
    data.set("nav.pitch", pitch.to_degrees());
    data.set("nav.roll", roll.to_degrees());

    let control = vessel.flight.control;
    data.set("control.pitch", control.x);
    data.set("control.yaw", control.y);
    data.set("control.roll", control.z);
    let w = attitude.angular_velocity;
    data.set("attitude.rate_pitch", w.x.to_degrees());
    data.set("attitude.rate_yaw", w.y.to_degrees());
    data.set("attitude.rate_roll", w.z.to_degrees());
    let props = vessel.mass_properties();
    let authority = vessel.flight.torque_authority;
    let inertia = props.inertia.diagonal().max(DVec3::splat(1e-9));
    data.set("attitude.com", props.center_of_mass);
    data.set("attitude.moi_pitch", props.inertia.pitch);
    data.set("attitude.moi_yaw", props.inertia.yaw);
    data.set("attitude.moi_roll", props.inertia.roll);
    data.set("attitude.torque_pitch", authority.x);
    data.set("attitude.torque_roll", authority.z);
    data.set(
        "attitude.accel_pitch",
        (authority.x / inertia.x).to_degrees(),
    );
    data.set(
        "attitude.accel_roll",
        (authority.z / inertia.z).to_degrees(),
    );
    let engines = vessel.propulsion.stages.first().map(|s| &s.engines);
    let (gimbal, range) = engines
        .into_iter()
        .flatten()
        .fold((0.0f64, 0.0f64), |(g, r), e| {
            (g.max(e.deflection.length()), r.max(e.def.gimbal_range))
        });
    data.set("attitude.gimbal", gimbal.to_degrees());
    data.set("attitude.gimbal_range", range);
    data.set(
        "attitude.wheels",
        vessel
            .propulsion
            .stages
            .iter()
            .map(|s| s.wheel_torque)
            .sum::<f64>(),
    );
    let rcs = osp_sim::rcs::torque_authority(
        &vessel.propulsion,
        props.center_of_mass,
        vessel.flight.static_pressure,
    );
    data.set("attitude.rcs", rcs.x);

    let stages = &vessel.propulsion.stages;
    let stage_info = vessel.propulsion.stage_info();
    let rows: Vec<_> = stages
        .iter()
        .zip(&stage_info)
        .enumerate()
        .map(|(i, (stage, info))| {
            record([
                ("name", info.name.as_str().into()),
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
        stage_info.iter().map(|s| s.delta_v_vac).sum::<f64>(),
    );
    data.set("vessel.stages", rows);
    publish_sequences(&mut data, vessel, mass * local_g);
    publish_resources(&mut data, stages);

    // Procedural tanks and fins: what each is made of and how hard it is working.
    data.set("vessel.destroyed", vessel.is_destroyed());
    data.set("vessel.structure_load", vessel.flight.structure_load);
    let load_of = |stage: usize, part: Part| {
        vessel
            .structure
            .iter()
            .find(|l| l.stage == stage && l.part == part)
    };
    let mode = |load: Option<&PartLoad>| {
        load.map(|l| match l.mode {
            Mode::Buckling => "buckling",
            Mode::Burst => "pressure",
            Mode::Bending => "bending",
        })
    };
    let mut parts = Vec::new();
    for (s, stage) in stages.iter().enumerate() {
        for (k, tank) in stage.tanks.iter().enumerate() {
            let load = load_of(s, Part::Tank(k));
            let size = match tank.shape {
                TankShape::Cylinder => format!("{:.2} x {:.2} m", 2.0 * tank.radius, tank.length),
                TankShape::Sphere => format!("{:.2} m sphere", 2.0 * tank.radius),
            };
            let contents: Vec<_> = tank
                .contents
                .iter()
                .map(|c| format!("{} {:.2} t", c.resource, c.capacity / 1e3))
                .collect();
            parts.push(record([
                ("stage", stage.name.as_str().into()),
                ("name", tank.tank_type.as_str().into()),
                ("tank", true.into()),
                ("size", size.into()),
                ("material", tank.material.as_str().into()),
                ("mass", tank.dry_mass.into()),
                ("volume", tank.volume.into()),
                ("utilization", tank.utilization.into()),
                ("contents", contents.join(", ").into()),
                ("wall", (tank.wall * 1e3).into()),
                ("max_pressure", (tank.max_pressure / 1e5).into()),
                ("max_axial_load", tank.max_axial_load.into()),
                ("max_fin_force", UiValue::Null),
                ("load", load.map(|l| l.fraction).into()),
                ("mode", mode(load).into()),
            ]));
        }
        if let Some(fins) = &stage.fins {
            let load = load_of(s, Part::Fins);
            let d = &fins.def;
            parts.push(record([
                ("stage", stage.name.as_str().into()),
                ("name", format!("{} fins", d.count).into()),
                ("tank", false.into()),
                (
                    "size",
                    format!(
                        "{:.2} m2 each, span {:.2} m, {:.0} mm thick",
                        fins.area,
                        d.span,
                        d.thickness * 1e3
                    )
                    .into(),
                ),
                ("material", d.material.as_str().into()),
                ("mass", fins.mass.into()),
                ("volume", UiValue::Null),
                ("utilization", UiValue::Null),
                ("contents", UiValue::Null),
                ("wall", UiValue::Null),
                ("max_pressure", UiValue::Null),
                ("max_axial_load", UiValue::Null),
                ("max_fin_force", fins.max_force.into()),
                ("load", load.map(|l| l.fraction).into()),
                ("mode", mode(load).into()),
            ]));
        }
    }
    data.set("vessel.parts", parts);

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
            let event = patch.ending?.event()?;
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

/// The activation sequences, next first (`vessel.sequences`), each with the activations
/// in it (`items`), and the thrust-to-weight ratio the bottom stage's engines give at full
/// throttle here (`vessel.stage_twr`). `weight` is the vessel's weight here, N.
fn publish_sequences(data: &mut UiData, vessel: &osp_sim::vessel::Vessel, weight: f64) {
    use osp_sim::staging::Activation;
    let propulsion = &vessel.propulsion;
    let stages = &propulsion.stages;
    let sequences = &propulsion.sequences;
    let rows: Vec<_> = sequences
        .list()
        .iter()
        .enumerate()
        .map(|(i, seq)| {
            let items: Vec<_> = seq
                .iter()
                .enumerate()
                .map(|(j, &a)| {
                    let stage = stages.iter().find(|s| s.id == a.stage());
                    let (count, fuel) = match (a, stage) {
                        (Activation::Engines(_), Some(s)) => (s.engines.len(), s.fuel_fraction()),
                        (Activation::Rcs(_), Some(s)) => (s.rcs.len(), s.rcs_fraction()),
                        _ => (1, None),
                    };
                    record([
                        ("seq", i.into()),
                        ("index", j.into()),
                        ("kind", a.kind().into()),
                        ("is_engines", matches!(a, Activation::Engines(_)).into()),
                        ("is_decoupler", matches!(a, Activation::Decoupler(_)).into()),
                        ("is_fairing", matches!(a, Activation::Fairing(_)).into()),
                        ("is_rcs", matches!(a, Activation::Rcs(_)).into()),
                        ("stage", stage.map_or("", |s| s.name.as_str()).into()),
                        ("count", count.into()),
                        ("multiple", (count > 1).into()),
                        ("fuel", fuel.into()),
                        ("has_fuel", fuel.is_some().into()),
                        ("color", stage.map_or("", |s| stage_color(s.id)).into()),
                    ])
                })
                .collect();
            record([
                ("index", i.into()),
                ("number", sequences.number(i).into()),
                ("next", (i == 0).into()),
                ("empty", seq.is_empty().into()),
                ("items", items.into()),
            ])
        })
        .collect();
    data.set("vessel.sequence_count", rows.len());
    data.set("vessel.sequences", rows);
    // A fired sequence leaves the list, but the engines it lit keep burning: they stay
    // under the stack, with the propellant they have left, until their stage falls away.
    let armed: Vec<_> = stages
        .iter()
        .filter(|s| s.active && !s.engines.is_empty())
        .map(|s| {
            let fuel = s.fuel_fraction();
            record([
                ("stage", s.name.as_str().into()),
                ("count", s.engines.len().into()),
                ("multiple", (s.engines.len() > 1).into()),
                ("fuel", fuel.into()),
                ("has_fuel", fuel.is_some().into()),
                ("color", stage_color(s.id).into()),
            ])
        })
        .collect();
    data.set("vessel.armed_count", armed.len());
    data.set("vessel.armed", armed);
    data.set("vessel.armed_number", sequences.number(0) - 1);
    let pressure = vessel.flight.static_pressure;
    let full: f64 = stages.first().map_or(0.0, |s| {
        s.engines
            .iter()
            .map(|e| e.max_mass_flow() * e.isp(pressure) * G0)
            .sum()
    });
    data.set(
        "vessel.stage_twr",
        if weight > 0.0 { full / weight } else { 0.0 },
    );
}

/// Bar colours that tell stages apart, by [`osp_sim::propulsion::Stage::id`] so a stage keeps
/// its colour as the ones below fall away: its propellant in the Resources window and its
/// fuel bars in the sequence stack.
const STAGE_COLORS: [&str; 6] = [
    "#5fb8ff", "#ffa45c", "#b98cff", "#4fd6c4", "#ff7aa8", "#e6d35a",
];

fn stage_color(id: usize) -> &'static str {
    STAGE_COLORS[id % STAGE_COLORS.len()]
}

/// Every propellant on board, a row per stage that holds it, top of the stack first
/// (`vessel.resources`): how much is left, whether the stage's engines or RCS burn it, and
/// the stage's colour.
fn publish_resources(data: &mut UiData, stages: &[osp_sim::propulsion::Stage]) {
    let rows: Vec<_> = stages
        .iter()
        .enumerate()
        .rev()
        .flat_map(|(i, stage)| {
            stage.propellants.iter().map(move |p| {
                let engines = stage
                    .engines
                    .iter()
                    .any(|e| e.def.propellants.contains_key(&p.name));
                let rcs = stage
                    .rcs
                    .iter()
                    .any(|b| b.def.propellants.contains_key(&p.name));
                record([
                    ("name", p.name.as_str().into()),
                    ("stage", stage.name.as_str().into()),
                    ("color", stage_color(stage.id).into()),
                    ("bottom", (i == 0).into()),
                    ("amount", p.amount.into()),
                    ("capacity", p.capacity.into()),
                    ("fraction", (p.amount / p.capacity.max(1e-9)).into()),
                    ("engines", engines.into()),
                    ("rcs", rcs.into()),
                    // Marks RCS-only propellant in the window.
                    ("tag", if rcs && !engines { "RCS" } else { "" }.into()),
                ])
            })
        })
        .collect();
    data.set("vessel.resource_count", rows.len());
    data.set("vessel.resources", rows);
}
