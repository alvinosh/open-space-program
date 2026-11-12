//! Publishes game state into the UI data store, for layouts (and mods) to bind to.
//!
//! Every key is listed in `docs/modding.md`; in game, press F12 for the data inspector.

use std::collections::BTreeMap;

use bevy::{math::DVec3, prelude::*};
use osp_mods::Severity;
use osp_sim::{
    constants::G0,
    data::TankShape,
    events::EventKind,
    parts::PartId,
    propulsion::{Group, Propulsion},
    staging::Activation,
    structure::{Mode, Part, PartLoad},
    time::SimTime,
    vessel::{Motion, Situation, VesselId},
};
use osp_ui::{Formatters, UiData, UiValue, record};

use crate::{
    FrameSet,
    camera::CameraRig,
    content::{GameContent, Library, PlayerMods, can_offer, mode_of},
    controls::SequenceEdits,
    editing::EditorStatus,
    orbit_lines::Prediction,
    part_highlight::PartMarks,
    sim::{MAX_PHYSICS_WARP, Sim, SimEventMessage, TimeWarp, WARP_RATES},
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
            (publish_flight, publish_mods, publish_link_change).in_set(FrameSet::Render),
        );
    }
}

/// What the flight UI remembers between frames.
#[derive(Default)]
struct FlightMemory {
    /// Latest log entry and the real time it appeared.
    message: Option<(SimTime, String, f64)>,
}

fn publish_flight(
    mut events: MessageReader<SimEventMessage>,
    sim: Res<Sim>,
    warp: Res<TimeWarp>,
    rig: Res<CameraRig>,
    prediction: Res<Prediction>,
    marks: Res<PartMarks>,
    edits: Res<SequenceEdits>,
    editor: Res<EditorStatus>,
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
    let local_g = (editor.gravity()).unwrap_or(body.gm / state.position.length_squared());

    data.set("time.ut", now.as_seconds());
    data.set("time.warp", warp.rate());
    data.set("time.warp_label", warp_label(warp.rate()));
    // One entry per warp level, lit up to the current rate (the warp strip).
    let rate = warp.rate();
    data.set(
        "time.warp_levels",
        WARP_RATES
            .iter()
            .map(|&r| {
                record([
                    ("lit", (r <= rate).into()),
                    ("rails", (r > MAX_PHYSICS_WARP).into()),
                ])
            })
            .collect::<Vec<_>>(),
    );
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
    }
    let liftoff = vessel.lifted_off();
    data.set("time.launched", liftoff.is_some());
    data.set(
        "time.met",
        liftoff.map_or(0.0, |t| now.seconds_since(t).max(0.0)),
    );

    data.set("vessel.name", vessel.name.as_str());
    data.set("vessel.body", body.name.as_str());
    let situation = vessel.situation(system, now);
    data.set(
        "vessel.situation",
        match situation {
            Situation::Landed => "Landed",
            Situation::Flying => "Flying",
            Situation::SubOrbital => "Sub-orbital",
            Situation::Orbiting => "Orbiting",
            Situation::Escaping => "Escaping",
        },
    );
    // Out of the atmosphere (or off the ground of an airless world): the Orbit window's cue.
    data.set(
        "vessel.in_space",
        !matches!(situation, Situation::Landed | Situation::Flying),
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
        vessel
            .propulsion
            .bottom()
            .flat_map(|s| vessel.propulsion.engines_of(s.id))
            .map(|e| e.def.min_throttle.clamp(0.0, 1.0))
            .fold(0.0, f64::max),
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
    let attitude = vessel.control_attitude(system, now);
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
    data.set("vessel.has_radio", vessel.radio().is_some());
    data.set("vessel.commandable", vessel.commandable());
    let link = vessel.link();
    data.set("vessel.link", link.is_some());
    data.set(
        "vessel.link_station",
        link.map_or("", |l| l.station.as_str()),
    );
    data.set("vessel.link_margin", link.map_or(0.0, |l| l.margin));
    data.set("vessel.link_delay", link.map_or(0.0, |l| l.delay));
    data.set("vessel.link_hops", link.map_or(0, |l| l.hops));
    data.set("vessel.is_relay", vessel.relay_radio().is_some());
    let rcs_fuel = vessel
        .propulsion
        .sections
        .iter()
        .filter_map(|s| vessel.propulsion.rcs_fraction(s.id))
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
    let moments = props.inertia.diagonal();
    let inertia = moments.max(DVec3::splat(1e-9));
    data.set("attitude.com", props.center_of_mass.z);
    data.set("attitude.moi_pitch", moments.x);
    data.set("attitude.moi_yaw", moments.y);
    data.set("attitude.moi_roll", moments.z);
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
    let (gimbal, range) = vessel
        .propulsion
        .bottom()
        .flat_map(|s| vessel.propulsion.engines_of(s.id))
        .fold((0.0f64, 0.0f64), |(g, r), e| {
            (g.max(e.deflection.length()), r.max(e.def.gimbal_range))
        });
    data.set("attitude.gimbal", gimbal.to_degrees());
    data.set("attitude.gimbal_range", range);
    data.set(
        "attitude.wheels",
        vessel
            .propulsion
            .sections
            .iter()
            .map(|s| s.wheel_torque)
            .sum::<f64>(),
    );
    let rcs = osp_sim::rcs::torque_authority(
        &vessel.propulsion,
        props.center_of_mass.z,
        vessel.flight.static_pressure,
    );
    data.set("attitude.rcs", rcs.x);

    let propulsion = &vessel.propulsion;
    let stages = &propulsion.sections;
    let stage_info = vessel.propulsion.stage_info();
    let rows: Vec<_> = stages
        .iter()
        .zip(&stage_info)
        .enumerate()
        .map(|(i, (stage, info))| {
            record([
                ("name", info.name.as_str().into()),
                (
                    "active",
                    (propulsion.armed(stage.id) && propulsion.clear(i)).into(),
                ),
                ("delta_v", info.delta_v_vac.into()),
                ("burn_time", info.burn_time.into()),
                ("fuel", propulsion.fuel_fraction(stage.id).into()),
                (
                    "has_fuel",
                    propulsion.fuel_fraction(stage.id).is_some().into(),
                ),
                (
                    "ignitions",
                    propulsion
                        .engines_of(stage.id)
                        .next()
                        .and_then(|e| e.ignitions_left)
                        .into(),
                ),
                ("engines", stage.engines.into()),
            ])
        })
        .collect();
    data.set("vessel.stage_count", rows.len());
    data.set(
        "vessel.delta_v",
        stage_info.iter().map(|s| s.delta_v_vac).sum::<f64>(),
    );
    data.set("vessel.stages", rows);
    publish_sequences(&mut data, vessel, mass * local_g, &marks, &edits);
    publish_resources(&mut data, propulsion);
    publish_power(&mut data, &vessel.power_state(system, now));

    // Procedural tanks and fins: what each is made of and how hard it is working.
    data.set("vessel.destroyed", vessel.is_destroyed());
    {
        use osp_sim::{
            guidance::Guidance,
            systems::{Cabin, Crew, HeatShield, Parachute, Stage, Thermal},
        };
        let systems = &vessel.propulsion.systems;
        data.set("vessel.can_abort", vessel.propulsion.can_abort());
        data.set("vessel.aborting", vessel.propulsion.aborting());
        data.set(
            "vessel.guidance",
            match vessel.guidance {
                Guidance::Off => "off",
                Guidance::Land => "landing",
                Guidance::HoldRate(_) => "holding a rate",
                Guidance::Hover => "hovering",
                Guidance::Entry => "entry",
            },
        );
        data.set("vessel.guidance_on", vessel.guidance != Guidance::Off);
        data.set("vessel.has_seats", systems.all::<Crew>().next().is_some());
        data.set("vessel.crew_aboard", f64::from(systems.crew_aboard()));
        data.set("vessel.crew_lost", f64::from(systems.crew_lost()));
        // Who is aboard, and how anyone not well is doing.
        let people: Vec<String> = (systems.all::<Crew>())
            .flat_map(|(_, crew)| {
                crew.people
                    .iter()
                    .map(|person| match crew.stage_of(person) {
                        (Stage::Fine, _) => person.name.clone(),
                        (stage, of) => format!(
                            "{} ({}{})",
                            person.name,
                            stage.word(),
                            of.map(|of| format!(": {of}")).unwrap_or_default()
                        ),
                    })
            })
            .collect();
        data.set("vessel.crew_people", people.join(", "));
        // What the crew can reach, not what the rocket under them burns.
        let cabin = systems
            .all::<Crew>()
            .map(|(part, _)| part)
            .collect::<Vec<_>>();
        // Each thing they need: what there is for them, lockers and all, and how long it lasts. What
        // they breathe from the cabin's air is what tops it up.
        let aboard = f64::from(systems.crew_aboard());
        let (needs, gases) = (systems.all::<Crew>())
            .find(|(_, crew)| !crew.needs.is_empty())
            .map(|(_, crew)| (crew.needs.clone(), crew.gases.clone()))
            .unwrap_or_default();
        let supply = |resource: &str| {
            (gases.iter())
                .find(|(gas, _)| gas == resource)
                .and_then(|(_, from)| from.clone())
                .unwrap_or_else(|| resource.to_owned())
        };
        let supplies: Vec<String> = (needs.iter())
            .map(|need| {
                let kg: f64 = (cabin.iter())
                    .map(|&part| vessel.propulsion.held_for(part, &supply(&need.resource)))
                    .sum();
                let days = kg / (need.daily * aboard).max(1e-9);
                if aboard > 0.0 {
                    format!("{} {kg:.1} kg ({days:.1} d)", need.called())
                } else {
                    format!("{} {kg:.1} kg", need.called())
                }
            })
            .collect();
        data.set("vessel.supplies", supplies.join(", "));
        data.set("vessel.has_supplies", !needs.is_empty());
        // Each cabin's air: its pressure, its oxygen and its CO2, and whether it is going.
        let tree = vessel.propulsion.parts();
        let cabins: Vec<String> = (systems.all::<Cabin>())
            .filter(|(_, cabin)| !cabin.gases.is_empty())
            .map(|(part, cabin)| {
                let name = tree.get(part).map_or("Cabin", |p| p.name.as_str());
                let kpa = |gas: &str| cabin.partial_of(gas).unwrap_or(0.0) / 1e3;
                let state = if cabin.venting {
                    ", let down"
                } else if cabin.hole > 0.0 {
                    ", leaking"
                } else {
                    ""
                };
                format!(
                    "{name}: {:.0} kPa, oxygen {:.1}, CO2 {:.2}{state}",
                    cabin.pressure() / 1e3,
                    kpa("oxygen"),
                    kpa("CO2"),
                )
            })
            .collect();
        data.set("vessel.cabins", cabins.join("\n"));
        data.set("vessel.has_air", !cabins.is_empty());
        // Radiation here, outside and in the first crew's part, and what each person has taken.
        let exposed = (systems.all::<Crew>()).find(|(_, crew)| crew.radiation.is_some());
        data.set("vessel.has_radiation", exposed.is_some());
        if let Some((_, crew)) = exposed {
            use osp_sim::radiation::MSV_PER_HOUR;
            let here = vessel.environment(system, now);
            let outside = here.radiation.total() / MSV_PER_HOUR;
            let inside = (crew.through(here.air_column)).map_or(0.0, |through| {
                here.radiation.weighted(&through) / MSV_PER_HOUR
            });
            data.set(
                "vessel.radiation",
                format!("{outside:.3} mSv/h outside, {inside:.3} inside"),
            );
            let doses: Vec<String> = (systems.all::<Crew>())
                .flat_map(|(_, crew)| &crew.people)
                .map(|person| format!("{} {:.1} mSv", person.name, person.dose * 1e3))
                .collect();
            data.set("vessel.doses", doses.join(", "));
        }
        let hottest = (systems.all::<Thermal>())
            .map(|(_, thermal)| thermal.temperature)
            .fold(0.0, f64::max);
        data.set("vessel.temperature", hottest);
        let shield = systems.all::<HeatShield>().next().map(|(_, shield)| shield);
        data.set("vessel.has_shield", shield.is_some());
        data.set(
            "vessel.ablator",
            shield.map_or(0.0, |shield| shield.ablator),
        );
        data.set(
            "vessel.heat_flux",
            shield.map_or(0.0, |shield| shield.flux / 1e3),
        );
        let open = (systems.all::<Parachute>())
            .filter(|(_, chute)| chute.canopy().is_some())
            .count();
        data.set(
            "vessel.has_parachutes",
            systems.all::<Parachute>().next().is_some(),
        );
        data.set("vessel.parachutes_open", open as f64);
        data.set("vessel.afloat", vessel.is_afloat());
    }
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
        for (k, tank) in propulsion.tanks_of(stage.id).enumerate() {
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

/// How long until the focused vessel's link changes, while the comms window is open. It looks an
/// hour ahead, every few seconds: finding it takes more than a frame should.
fn publish_link_change(
    sim: Res<Sim>,
    shown: Res<osp_ui::runtime::LayoutVisibility>,
    time: Res<Time>,
    mut next_look: Local<f64>,
    mut data: ResMut<UiData>,
) {
    if !shown.shown("comms_info") || time.elapsed_secs_f64() < *next_look {
        return;
    }
    *next_look = time.elapsed_secs_f64() + 5.0;
    let now = sim.universe.time();
    let change = sim.universe.next_link_change(now.add_seconds(3600.0));
    data.set(
        "vessel.link_next",
        change.map_or(0.0, |at| at.seconds_since(now)),
    );
}

fn publish_mods(
    content: Res<GameContent>,
    library: Res<Library>,
    player: Res<PlayerMods>,
    mut data: ResMut<UiData>,
) {
    if !content.is_changed() && !player.is_changed() {
        return;
    }
    let content = &content.0;
    let resolved = osp_mods::resolved(&content.available, &player.choices);
    let available: Vec<_> = (content.available.iter())
        .map(|m| {
            let missing = osp_mods::missing_dependency(&content.available, &m.id);
            let on = resolved.get(&m.id).copied().unwrap_or(false) && missing.is_none();
            let state = match missing {
                Some(mod_id) => format!("Needs {mod_id}"),
                None if on => "On".to_owned(),
                None => "Off".to_owned(),
            };
            record([
                ("id", m.id.as_str().into()),
                ("name", m.name.as_str().into()),
                ("description", m.description.as_str().into()),
                ("on", on.into()),
                ("state", state.into()),
            ])
        })
        .collect();
    data.set("mods.available", available);
    let mode = mode_of(&library.modes, &content.available, &player.choices);
    let modes: Vec<_> = (library.modes.iter())
        .filter(|m| can_offer(m, &content.available))
        .map(|m| {
            record([
                ("id", m.id.as_str().into()),
                ("name", m.name.as_str().into()),
                ("description", m.description.as_str().into()),
                ("on", mode.is_some_and(|mode| mode.id == m.id).into()),
            ])
        })
        .collect();
    data.set("mods.modes", modes);
    data.set("mods.custom", mode.is_none());
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

/// `<vessel>:<part>,<part>,...`, which `part.group` and the `hover` of a tile take.
fn group_window(vessel: VesselId, parts: &[PartId]) -> String {
    let parts: Vec<_> = parts.iter().map(|p| p.0.to_string()).collect();
    format!("{}:{}", vessel.0, parts.join(","))
}

/// The activation sequences, next first (`vessel.sequences`), each with an icon per group
/// of identical parts in it (`items`, see `Propulsion::groups`), and the thrust-to-weight
/// ratio the bottom stage's engines give at full throttle here (`vessel.stage_twr`).
/// `weight` is the vessel's weight here, N. A tile wears the colour of the part window
/// that marks any of its parts (`mark`), and a right-click on it opens the window of its
/// first part (`window`, for `part.group`).
fn publish_sequences(
    data: &mut UiData,
    vessel: &osp_sim::vessel::Vessel,
    weight: f64,
    marks: &PartMarks,
    edits: &SequenceEdits,
) {
    let propulsion = &vessel.propulsion;
    let stages = &propulsion.sections;
    let sequences = &propulsion.sequences;
    // An icon for identical parts: how many, and the fuel feeding them (engines) or the
    // RCS's, and what it does and how it burns (`caption`, `tip`).
    let icon = |seq: usize, index: usize, group: &Group| {
        let stage = stages.iter().find(|s| s.id == group.section);
        let stage_name = stage.map_or("", |s| s.name.as_str());
        let count = group.members.len();
        let fuel = match group.kind {
            "engines" => propulsion.feed_fraction(&group.parts),
            "rcs" => stage.and_then(|s| propulsion.rcs_fraction(s.id)),
            _ => None,
        };
        let engines: Vec<_> = (group.parts.iter())
            .filter_map(|p| propulsion.engines.iter().find(|e| e.part == *p))
            .collect();
        let first = engines.first().map(|e| &e.def);
        let solid = first.is_some_and(|d| d.is_solid());
        // Made for vacuum: gives far less thrust at sea level.
        let vacuum = first.is_some_and(|d| d.isp_vac > 0.0 && d.isp_sl / d.isp_vac < 0.6);
        let caption = group_caption(group.kind, group.verb, count, &group.look, stage_name);
        let tip = match first {
            Some(def) => {
                let thrust: f64 = engines.iter().map(|e| e.def.thrust_vac).sum();
                let mut tip = format!(
                    "{caption} · {} · Isp {:.0} s",
                    format_thrust(thrust),
                    def.isp_vac
                );
                if let Some(time) = propulsion.burn_time_left(&group.parts) {
                    tip += &format!(" · burn {}", format_burn(time));
                }
                if let Some(fuel) = fuel {
                    tip += &format!(" · fuel {:.0}%", fuel * 100.0);
                }
                tip
            }
            None => match fuel {
                Some(fuel) => format!("{caption} · fuel {:.0}%", fuel * 100.0),
                None => caption.clone(),
            },
        };
        let mark = marks.color_of_parts(vessel.id(), &group.parts);
        record([
            ("seq", seq.into()),
            ("index", index.into()),
            ("kind", group.kind.into()),
            ("window", group_window(vessel.id(), &group.parts).into()),
            ("mark", mark.unwrap_or_default().into()),
            (
                "is_engines",
                (group.kind == "engines" && !solid && !vacuum).into(),
            ),
            (
                "is_vacuum",
                (group.kind == "engines" && vacuum && !solid).into(),
            ),
            ("is_solid", (group.kind == "engines" && solid).into()),
            ("is_decoupler", (group.kind == "decoupler").into()),
            ("is_fairing", (group.kind == "fairing").into()),
            ("is_rcs", (group.kind == "rcs").into()),
            ("is_cutoff", (group.kind == "cutoff").into()),
            ("is_throttle", (group.kind == "throttle").into()),
            // What no icon was drawn for: a system's own action.
            (
                "is_other",
                (!matches!(
                    group.kind,
                    "engines" | "decoupler" | "fairing" | "rcs" | "cutoff" | "throttle"
                ))
                .into(),
            ),
            ("stage", stage_name.into()),
            ("count", count.into()),
            ("multiple", (count > 1).into()),
            ("fuel", fuel.into()),
            ("has_fuel", fuel.is_some().into()),
            ("color", stage_color(group.section).into()),
            ("caption", caption.into()),
            ("tip", tip.into()),
        ])
    };
    let rows: Vec<_> = sequences
        .list()
        .iter()
        .enumerate()
        .map(|(i, seq)| {
            let groups = propulsion.groups(seq);
            let items: Vec<_> = (groups.iter().enumerate())
                .map(|(j, g)| icon(i, j, g))
                .collect();
            let lines = tile_lines(&items);
            // Every part of the row, for hovering its number.
            let row_parts: Vec<PartId> = (groups.iter())
                .flat_map(|g| g.parts.iter().copied())
                .collect();
            let window = match row_parts.is_empty() {
                true => String::new(),
                false => group_window(vessel.id(), &row_parts),
            };
            let caption = (items.iter())
                .filter_map(|item| match item.get("caption") {
                    Some(UiValue::Text(text)) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" · ");
            record([
                ("index", i.into()),
                ("number", sequences.number(i).into()),
                ("next", (i == 0).into()),
                // Rows stack from the next at the bottom: up is later.
                ("above", (i + 1).into()),
                ("below", i.saturating_sub(1).into()),
                ("can_up", (i + 1 < sequences.list().len()).into()),
                ("can_down", (i > 0).into()),
                ("window", window.into()),
                ("empty", seq.is_empty().into()),
                ("caption", caption.into()),
                ("lines", lines.into()),
                ("items", items.into()),
            ])
        })
        .collect();
    publish_sequence_menu(data, propulsion, edits);
    data.set("vessel.can_undo_sequences", edits.can_undo(vessel));
    data.set("vessel.sequence_count", rows.len());
    // What the stage key does next, under the stack.
    let next = rows.first().and_then(|row| row.get("caption"));
    data.set("vessel.next_caption", next.cloned().unwrap_or_default());
    data.set("vessel.sequences", rows);
    // A fired sequence leaves the list, but the engines it lit keep burning: they stay
    // under the stack, with the propellant they have left, until their stage falls away.
    let lit: Vec<Activation> = (propulsion.engines.iter())
        .filter(|e| propulsion.engine_armed(e.part))
        .map(|e| Activation::engine(e.part))
        .collect();
    let armed: Vec<_> = (propulsion.groups(&lit).iter())
        .map(|g| icon(0, 0, g))
        .collect();
    data.set("vessel.armed_count", armed.len());
    data.set("vessel.armed_lines", tile_lines(&armed));
    data.set("vessel.armed", armed);
    data.set("vessel.armed_number", sequences.number(0) - 1);
    let pressure = vessel.flight.static_pressure;
    let full: f64 = propulsion
        .bottom()
        .flat_map(|s| propulsion.engines_of(s.id))
        .map(|e| e.max_mass_flow() * e.isp(pressure) * G0)
        .sum();
    data.set(
        "vessel.stage_twr",
        if weight > 0.0 { full / weight } else { 0.0 },
    );
}

/// What the menu of a row in the staging window offers (`staging_menu.*`): to add a sequence
/// each side of it, to move it, to merge it with the one below, and to split one part off each
/// group of several (`splits`).
fn publish_sequence_menu(data: &mut UiData, propulsion: &Propulsion, edits: &SequenceEdits) {
    let sequences = &propulsion.sequences;
    let Some((row, seq)) = edits.menu.and_then(|i| Some((i, sequences.list().get(i)?))) else {
        return;
    };
    let count = sequences.list().len();
    data.set("staging_menu.row", row);
    data.set("staging_menu.number", sequences.number(row));
    data.set("staging_menu.above", row + 1);
    data.set("staging_menu.below", row.saturating_sub(1));
    data.set("staging_menu.can_up", row + 1 < count);
    data.set("staging_menu.can_down", row > 0);
    let splits: Vec<_> = (propulsion.groups(seq).iter().enumerate())
        .filter(|(_, g)| g.members.len() > 1)
        .map(|(j, g)| {
            record([
                ("group", j.into()),
                ("label", format!("Split one {} off", g.look).into()),
            ])
        })
        .collect();
    data.set("staging_menu.splits", splits);
}

/// Most tiles to a line in the staging window.
const TILES_PER_LINE: usize = 4;

/// A row's tiles in lines of [`TILES_PER_LINE`] (`tiles` each), so a row is as wide as its
/// longest line and as tall as its lines, which a wrapping container cannot do.
fn tile_lines(tiles: &[osp_ui::UiRecord]) -> Vec<osp_ui::UiRecord> {
    tiles
        .chunks(TILES_PER_LINE)
        .map(|line| record([("tiles", line.to_vec().into())]))
        .collect()
}

/// What a group of identical parts does when its sequence fires: "Ignite 9 × K-1 Booster",
/// "Decouple S-IC". `name` is the engines' record, `stage` the stage the parts belong to.
fn group_caption(kind: &str, verb: &str, count: usize, name: &str, stage: &str) -> String {
    let times = if count > 1 {
        format!("{count} × ")
    } else {
        String::new()
    };
    match kind {
        "engines" => format!("Ignite {times}{name}"),
        "decoupler" => format!("Decouple {stage}"),
        "fairing" => format!("Jettison {stage} fairing"),
        "rcs" => "RCS on".to_owned(),
        "cutoff" => format!("Cut off {times}{name}"),
        "throttle" => format!("Limit thrust of {times}{name}"),
        // A system's own action: it says what it is called.
        _ => format!("{verb} {times}{name}"),
    }
}

/// 914 kN, 7.6 MN.
fn format_thrust(newtons: f64) -> String {
    if newtons >= 1e6 {
        format!("{:.1} MN", newtons / 1e6)
    } else {
        format!("{:.0} kN", newtons / 1e3)
    }
}

/// 42 s, 2:42.
fn format_burn(seconds: f64) -> String {
    let s = seconds.round() as u64;
    if s < 60 {
        format!("{s} s")
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Bar colours that tell stages apart, by [`osp_sim::propulsion::Section::id`] so a stage keeps
/// its colour as the ones below fall away: its propellant in the Resources window and its
/// fuel bars in the sequence stack.
const STAGE_COLORS: [&str; 6] = [
    "#5fb8ff", "#ffa45c", "#b98cff", "#4fd6c4", "#ff7aa8", "#e6d35a",
];

fn stage_color(id: usize) -> &'static str {
    STAGE_COLORS[id % STAGE_COLORS.len()]
}

/// Every propellant on board and the charge in its batteries, a row per stage that holds it, top of
/// the stack first (`vessel.resources`): how much is left, whether the stage's engines or RCS burn
/// it, and the stage's colour.
fn publish_resources(data: &mut UiData, propulsion: &osp_sim::propulsion::Propulsion) {
    let tree = propulsion.parts();
    let mut charge_by_stage: BTreeMap<usize, (f64, f64)> = BTreeMap::new();
    for (part, held, most) in propulsion.systems.stores() {
        let stage = tree.get(part).map_or(0, |p| p.stage);
        let total = charge_by_stage.entry(stage).or_default();
        *total = (total.0 + held, total.1 + most);
    }
    let rows: Vec<_> = propulsion
        .sections
        .iter()
        .enumerate()
        .rev()
        .flat_map(|(i, stage)| {
            let charge = charge_by_stage.get(&stage.id).map(|&(held, most)| {
                record([
                    ("name", "Charge".into()),
                    ("stage", stage.name.as_str().into()),
                    ("color", stage_color(stage.id).into()),
                    ("bottom", (i == 0).into()),
                    // J: what `|energy` takes.
                    ("amount", (held * 1e3).into()),
                    ("capacity", (most * 1e3).into()),
                    ("fraction", (held / most.max(1e-9)).into()),
                    ("charge", true.into()),
                    ("engines", false.into()),
                    ("rcs", false.into()),
                    ("tag", "".into()),
                ])
            });
            let propellants = propulsion.propellants(stage.id).into_iter().map(move |p| {
                let engines = propulsion
                    .engines_of(stage.id)
                    .any(|e| e.def.propellants.contains_key(&p.name));
                let rcs = propulsion
                    .rcs_of(stage.id)
                    .any(|b| b.def.propellants.contains_key(&p.name));
                record([
                    ("name", p.name.as_str().into()),
                    ("stage", stage.name.as_str().into()),
                    ("color", stage_color(stage.id).into()),
                    ("bottom", (i == 0).into()),
                    ("amount", p.amount.into()),
                    ("capacity", p.capacity.into()),
                    ("fraction", (p.amount / p.capacity.max(1e-9)).into()),
                    ("charge", false.into()),
                    ("engines", engines.into()),
                    ("rcs", rcs.into()),
                    // Marks RCS-only propellant in the window.
                    ("tag", if rcs && !engines { "RCS" } else { "" }.into()),
                ])
            });
            charge.into_iter().chain(propellants)
        })
        .collect();
    data.set("vessel.resource_count", rows.len());
    data.set("vessel.resources", rows);
}

/// `vessel.has_power` and what is said about it: how long until it runs out if it is running down
/// (`vessel.power_draining`, `vessel.power_left`), holding (`vessel.power_steady`), its batteries
/// flat with everything still running on what it makes (`vessel.power_flat`), some of its loads off
/// (`vessel.power_shedding`) or what it cannot do without off (`vessel.power_out`), and how much of it
/// is left (`vessel.power_fraction`).
fn publish_power(data: &mut UiData, power: &osp_sim::systems::PowerState) {
    let has_store = power.capacity > 0.0;
    data.set(
        "vessel.has_power",
        has_store || power.browned_out || power.shedding,
    );
    data.set("vessel.power_out", power.browned_out);
    data.set(
        "vessel.power_shedding",
        power.shedding && !power.browned_out,
    );
    let draining = power.runs_out_in.filter(|_| !power.browned_out);
    data.set("vessel.power_draining", draining.is_some());
    data.set("vessel.power_left", draining.unwrap_or(0.0));
    let holding = has_store && draining.is_none() && !power.shedding && !power.browned_out;
    data.set("vessel.power_supplied", power.supplied);
    data.set(
        "vessel.power_steady",
        holding && !power.is_flat() && !power.supplied,
    );
    data.set("vessel.power_flat", holding && power.is_flat());
    data.set(
        "vessel.power_fraction",
        power.charge / power.capacity.max(1e-9),
    );
}

/// How a warp rate reads: times real time, or above a day a second, as time a second.
pub fn warp_label(rate: f64) -> String {
    const DAY: f64 = 86_400.0;
    const YEAR: f64 = 365.25 * DAY;
    let per_second = |amount: f64, unit: &str| {
        let whole = (amount - amount.round()).abs() < 0.05;
        if amount < 99.5 && !whole {
            format!("{amount:.1} {unit}/s")
        } else {
            format!("{amount:.0} {unit}/s")
        }
    };
    if rate >= 0.995 * YEAR {
        per_second(rate / YEAR, "y")
    } else if rate > 100_000.0 {
        per_second(rate / DAY, "d")
    } else {
        format!("{rate:.0}x")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warp_reads_as_times_and_then_as_time_a_second() {
        assert_eq!(warp_label(1.0), "1x");
        assert_eq!(warp_label(100_000.0), "100000x");
        assert_eq!(warp_label(1e6), "11.6 d/s");
        assert_eq!(warp_label(1e7), "116 d/s");
        assert_eq!(warp_label(365.25 * 86_400.0), "1 y/s");
        assert_eq!(warp_label(2.5 * 365.25 * 86_400.0), "2.5 y/s");
        assert_eq!(warp_label(100.0 * 365.25 * 86_400.0), "100 y/s");
    }

    #[test]
    fn a_group_says_what_it_does() {
        assert_eq!(
            group_caption("engines", "Do it", 5, "F-1", "S-IC"),
            "Ignite 5 × F-1"
        );
        assert_eq!(
            group_caption("engines", "Do it", 1, "J-2", "S-II"),
            "Ignite J-2"
        );
        assert_eq!(
            group_caption("decoupler", "Do it", 1, "", "S-IC"),
            "Decouple S-IC"
        );
        assert_eq!(
            group_caption("fairing", "Do it", 2, "", "Payload"),
            "Jettison Payload fairing"
        );
        assert_eq!(group_caption("rcs", "Do it", 1, "", "S-IVB"), "RCS on");
    }

    #[test]
    fn a_tile_and_a_row_name_the_parts_to_mark() {
        let boosters = [PartId(7), PartId(8), PartId(9)];
        let window = group_window(VesselId(2), &boosters);
        assert_eq!(window, "2:7,8,9");
        // What a hover of that text marks.
        let text = format!("part.group:{window}");
        assert_eq!(
            crate::part_highlight::group_of(&text),
            Some((VesselId(2), boosters.to_vec()))
        );
        // One booster split off into a tile of its own marks just that one.
        let one = group_window(VesselId(2), &boosters[..1]);
        assert_eq!(one, "2:7");
    }

    #[test]
    fn thrust_and_burn_time_are_short() {
        assert_eq!(format_thrust(914_000.0), "914 kN");
        assert_eq!(format_thrust(7_600_000.0), "7.6 MN");
        assert_eq!(format_burn(42.0), "42 s");
        assert_eq!(format_burn(162.0), "2:42");
    }
}
