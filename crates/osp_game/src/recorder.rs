//! Flight recorder, for chasing glitches seen while playing.
//!
//! `OSP_RECORD=<file.csv>` writes a row per frame about the vessel being flown: its flight
//! data and rotation, what the pilot and SAS ask of it, where the camera sits, and how the
//! frame went (its length, the physics ticks it ran, the render time between ticks). G
//! (`debug.mark`) marks the row where something looked wrong. New log messages (staging,
//! fins tearing off, ...) go in the last column. Rows are flushed as they go, so the file
//! survives a crash.

use std::{
    fmt::Write as _,
    fs::File,
    io::{BufWriter, Write as _},
    path::PathBuf,
};

use bevy::{
    math::{DVec2, DVec3},
    prelude::*,
};
use osp_sim::{time::SimTime, universe::PHYSICS_STEP};
use osp_ui::UiAction;

use crate::{
    FrameSet,
    camera::CameraRig,
    sim::{Sim, TimeWarp},
};

/// Rows between flushes.
const FLUSH_EVERY: u64 = 25;

const HEADER: &str = "frame,real_s,frame_ms,ut_s,render_ms,ticks,warp,lagging,mark,vessel,\
altitude_m,airspeed_ms,mach,q_pa,aoa_deg,sideslip_deg,wx,wy,wz,nose_pitch_deg,\
nose_heading_deg,input_x,input_y,input_z,cmd_x,cmd_y,cmd_z,sas,sas_mode,rcs,throttle,\
thrust_kn,gimbal_x_deg,gimbal_y_deg,load,map,cam_up,cam_east,cam_north,note";

pub struct RecorderPlugin;

impl Plugin for RecorderPlugin {
    fn build(&self, app: &mut App) {
        let Some(path) = std::env::var_os("OSP_RECORD").map(PathBuf::from) else {
            return;
        };
        let file = File::create(&path)
            .unwrap_or_else(|e| panic!("cannot write the recording {}: {e}", path.display()));
        let mut out = BufWriter::new(file);
        let _ = writeln!(out, "{HEADER}");
        info!(
            "flight recorder: writing {} (G marks a moment)",
            path.display()
        );
        app.insert_resource(Recorder {
            out,
            rows: 0,
            start: None,
            last: None,
            noted: None,
        })
        .add_systems(Update, record.in_set(FrameSet::Render));
    }
}

#[derive(Resource)]
struct Recorder {
    out: BufWriter<File>,
    rows: u64,
    /// Universe time of the first row; `ut_s` counts from it.
    start: Option<SimTime>,
    /// Universe time at the last row.
    last: Option<SimTime>,
    /// Time of the newest log message already written.
    noted: Option<SimTime>,
}

fn record(
    mut recorder: ResMut<Recorder>,
    mut actions: MessageReader<UiAction>,
    sim: Res<Sim>,
    mut warp: ResMut<TimeWarp>,
    rig: Res<CameraRig>,
    time: Res<Time<Real>>,
) {
    let mark = actions.read().any(|a| a.name() == "debug.mark");
    let now = time.elapsed_secs_f64();
    if mark {
        warp.notice = Some(("Marked in the flight recording".into(), now));
    }
    if sim.paused && !mark {
        return;
    }
    let recorder = &mut *recorder;
    let ut = sim.universe.time();
    // Started afresh (another scenario): count from here.
    if recorder.last.is_some_and(|last| ut < last) {
        recorder.start = None;
        recorder.noted = None;
    }
    let start = *recorder.start.get_or_insert(ut);
    let ticks = recorder.last.map_or(0.0, |last| {
        ut.seconds_since(last) / PHYSICS_STEP.as_seconds()
    });
    recorder.last = Some(ut);

    let system = &sim.universe.system;
    let v = sim.active_vessel();
    let body = system.body(v.parent());
    let state = v.state_at(system, ut);
    let up = state.position.normalize();
    let east = body.pole().cross(up).normalize_or_zero();
    let north = up.cross(east);
    let airspeed = (state.velocity - body.surface_velocity(state.position)).length();
    let w = v.attitude_state(system, ut).angular_velocity;
    // The nose as drawn this frame, against the local horizon.
    let nose = v.attitude_at(system, sim.render_time) * DVec3::Z;
    let nose_pitch = nose.dot(up).clamp(-1.0, 1.0).asin().to_degrees();
    let nose_heading = nose.dot(east).atan2(nose.dot(north)).to_degrees();
    let gimbal = v
        .propulsion
        .engines
        .first()
        .map_or(DVec2::ZERO, |e| e.deflection);
    let f = &v.flight;
    let note: Vec<&str> = v
        .log
        .iter()
        .filter(|e| recorder.noted.is_none_or(|t| e.time > t))
        .map(|e| e.message.as_str())
        .collect();
    if let Some(newest) = v.log.last() {
        recorder.noted = Some(newest.time);
    }

    let mut row = String::new();
    let _ = write!(
        row,
        "{},{now:.4},{:.2},{:.3},{:.2},{ticks:.1},{},{},{},\"{}\",",
        recorder.rows,
        time.delta_secs_f64() * 1e3,
        ut.seconds_since(start),
        sim.render_time.seconds_since(ut) * 1e3,
        warp.rate(),
        u8::from(sim.lagging),
        u8::from(mark),
        v.name.replace('"', "'"),
    );
    let _ = write!(
        row,
        "{:.1},{airspeed:.2},{:.4},{:.1},{:.3},{:.3},{:.5},{:.5},{:.5},{nose_pitch:.4},{nose_heading:.4},",
        state.position.length() - body.radius,
        f.mach,
        f.dynamic_pressure,
        f.angle_of_attack.to_degrees(),
        f.sideslip.to_degrees(),
        w.x,
        w.y,
        w.z,
    );
    let input = v.rotation_input;
    let _ = write!(
        row,
        "{},{},{},{:.3},{:.3},{:.3},{},{},{},{:.3},{:.1},{:.3},{:.3},{:.3},",
        input.x,
        input.y,
        input.z,
        f.control.x,
        f.control.y,
        f.control.z,
        u8::from(v.sas().enabled()),
        v.sas().mode().id(),
        u8::from(v.rcs()),
        v.propulsion.throttle(),
        f.thrust / 1e3,
        gimbal.x.to_degrees(),
        gimbal.y.to_degrees(),
        f.structure_load,
    );
    let _ = write!(
        row,
        "{},{:.2},{:.2},{:.2},\"{}\"",
        u8::from(rig.map_view),
        rig.position.dot(up),
        rig.position.dot(east),
        rig.position.dot(north),
        note.join("; ").replace('"', "'"),
    );

    if let Err(e) = writeln!(recorder.out, "{row}") {
        warn!("flight recorder: {e}");
    }
    recorder.rows += 1;
    if mark || recorder.rows % FLUSH_EVERY == 0 {
        let _ = recorder.out.flush();
    }
}
