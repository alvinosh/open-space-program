//! Performance overlay (F10) and profiling reports.
//!
//! Every frame is timed: the whole frame, the main world's update on the CPU, the render
//! world's work on the render thread, and how long the render thread waited for the GPU to
//! hand back a swapchain image or accept the frame's commands (a long wait means the GPU,
//! or vsync, sets the pace). The overlay (`perf` layout, `perf.*` keys) shows these over
//! the last half second, with what is being drawn.
//!
//! Built with `--features profile`, Bevy traces every system and schedule; the overlay
//! then also lists the slowest systems, and on Vulkan and DX12 each render pass's GPU time.
//! (Not on Metal: its timestamp counters run out and the device is lost.)
//! `--features bevy/trace_tracy` streams the same spans to the Tracy profiler for a
//! timeline.
//!
//! `OSP_PROFILE=<file>` writes a report there on exit: frame time percentiles, a timeline
//! per second, the worst frames and, with `profile`, every system's mean and worst time.
//! Combine it with `OSP_CAPTURE` (and `OSP_UNCAPPED=1`) for a repeatable scripted run, or
//! with `OSP_PROFILE_SECONDS=<s>` to play in the window for that long and exit.

use std::{
    fmt::Write as _,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

use bevy::{
    diagnostic::DiagnosticsStore,
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        renderer::{RenderAdapterInfo, RenderGraph, RenderGraphSystems},
        view::prepare_windows,
    },
};
use osp_ui::{UiData, record};

use crate::{camera::ViewOutput, settings::GraphicsSettings};

/// The overlay is refreshed this often, s.
const PUBLISH_PERIOD: f64 = 0.5;
/// Rows in the overlay's lists.
const OVERLAY_ROWS: usize = 10;
/// Frames kept for the report (about 15 minutes at 100 fps).
const MAX_FRAMES: usize = 100_000;

pub struct ProfilerPlugin;

impl Plugin for ProfilerPlugin {
    fn build(&self, app: &mut App) {
        let shared = RenderTimes::default();
        #[cfg(all(feature = "profile", not(target_os = "macos")))]
        app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
        app.insert_resource(shared.clone())
            .insert_resource(Profile {
                report: std::env::var_os("OSP_PROFILE").map(PathBuf::from),
                ..default()
            })
            .init_resource::<MeshTriangles>()
            .add_systems(First, begin_main)
            .add_systems(Last, (count_triangles, end_frame, publish).chain())
            .add_systems(Last, write_report.run_if(on_message::<AppExit>));
        if let Some(seconds) = std::env::var("OSP_PROFILE_SECONDS")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
        {
            app.add_systems(
                Update,
                move |time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>| {
                    if time.elapsed_secs_f64() > seconds {
                        exit.write(AppExit::Success);
                    }
                },
            );
        }
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(shared)
                .init_resource::<RenderClock>()
                .add_systems(
                    Render,
                    (
                        begin_render.before(RenderSystems::ExtractCommands),
                        begin_acquire
                            .in_set(RenderSystems::PrepareViews)
                            .before(prepare_windows),
                        end_acquire
                            .in_set(RenderSystems::PrepareViews)
                            .after(prepare_windows),
                        end_render.after(RenderSystems::PostCleanup),
                    ),
                )
                .add_systems(
                    RenderGraph,
                    (
                        begin_submit
                            .after(RenderGraphSystems::Render)
                            .before(RenderGraphSystems::Submit),
                        end_submit
                            .after(RenderGraphSystems::Submit)
                            .before(RenderGraphSystems::Finish),
                    ),
                );
        }
    }
}

/// The render world's latest times, µs, written on the render thread.
#[derive(Resource, Clone, Default)]
struct RenderTimes(Arc<[AtomicU32; 3]>);

impl RenderTimes {
    const RENDER: usize = 0;
    /// Waiting for the next swapchain image.
    const ACQUIRE: usize = 1;
    /// Submitting the frame's commands, which blocks while the GPU is too far behind
    /// (and is where the wait shows up when there is no window to present to).
    const SUBMIT: usize = 2;

    fn store(&self, which: usize, duration: Duration) {
        self.0[which].store(duration.as_micros() as u32, Ordering::Relaxed);
    }

    fn load(&self, which: usize) -> f32 {
        self.0[which].load(Ordering::Relaxed) as f32 / 1000.0
    }
}

#[derive(Resource)]
struct RenderClock {
    render: Instant,
    acquire: Instant,
    submit: Instant,
}

impl Default for RenderClock {
    fn default() -> Self {
        Self {
            render: Instant::now(),
            acquire: Instant::now(),
            submit: Instant::now(),
        }
    }
}

fn begin_render(mut clock: ResMut<RenderClock>) {
    clock.render = Instant::now();
}

fn begin_acquire(mut clock: ResMut<RenderClock>) {
    clock.acquire = Instant::now();
}

fn end_acquire(clock: Res<RenderClock>, times: Res<RenderTimes>) {
    times.store(RenderTimes::ACQUIRE, clock.acquire.elapsed());
}

fn begin_submit(mut clock: ResMut<RenderClock>) {
    clock.submit = Instant::now();
}

fn end_submit(clock: Res<RenderClock>, times: Res<RenderTimes>) {
    times.store(RenderTimes::SUBMIT, clock.submit.elapsed());
}

fn end_render(clock: Res<RenderClock>, times: Res<RenderTimes>) {
    times.store(RenderTimes::RENDER, clock.render.elapsed());
}

/// One frame's times, ms.
#[derive(Clone, Copy, Default)]
struct FrameSample {
    /// Seconds since startup, at the end of the frame.
    at: f32,
    frame: f32,
    main: f32,
    render: f32,
    /// Waiting for the GPU: for a swapchain image, or to submit.
    gpu_wait: f32,
}

#[derive(Resource, Default)]
struct Profile {
    report: Option<PathBuf>,
    main_start: Option<Instant>,
    frames: Vec<FrameSample>,
    /// Frames since the overlay was last refreshed.
    window: Vec<FrameSample>,
    since_publish: f64,
    /// Render passes: path, sum of the per-refresh averages (CPU, GPU) and how many.
    passes: Vec<(String, f64, f64, u32)>,
    counts: Counts,
}

/// What is in the world, refreshed with the overlay.
#[derive(Clone, Copy, Default)]
struct Counts {
    entities: u32,
    meshes: u32,
    visible_meshes: u32,
    /// In the visible meshes, each counted once (shadow casters are drawn again into each
    /// cascade they reach).
    visible_triangles: u64,
    lights: u32,
}

/// Triangles per mesh, counted when each is added or changed: meshes kept only in the
/// render world lose their data in the main world once extracted.
#[derive(Resource, Default)]
struct MeshTriangles(bevy::platform::collections::HashMap<AssetId<Mesh>, u32>);

fn count_triangles(
    mut events: MessageReader<AssetEvent<Mesh>>,
    meshes: Res<Assets<Mesh>>,
    mut counts: ResMut<MeshTriangles>,
) {
    for event in events.read() {
        match *event {
            AssetEvent::Added { id } | AssetEvent::Modified { id } => {
                let Some(mesh) = meshes.get(id) else { continue };
                let corners = match mesh.try_indices_option() {
                    Ok(Some(indices)) => indices.len(),
                    Ok(None) => mesh
                        .try_attribute(Mesh::ATTRIBUTE_POSITION)
                        .map_or(0, |positions| positions.len()),
                    Err(_) => continue,
                };
                counts.0.insert(id, (corners / 3) as u32);
            }
            AssetEvent::Removed { id } | AssetEvent::Unused { id } => {
                counts.0.remove(&id);
            }
            AssetEvent::LoadedWithDependencies { .. } => {}
        }
    }
}

fn begin_main(mut profile: ResMut<Profile>) {
    profile.main_start = Some(Instant::now());
}

fn end_frame(time: Res<Time<Real>>, times: Res<RenderTimes>, mut profile: ResMut<Profile>) {
    let main = profile
        .main_start
        .map_or(0.0, |start| start.elapsed().as_secs_f32() * 1000.0);
    let sample = FrameSample {
        at: time.elapsed_secs(),
        frame: time.delta_secs() * 1000.0,
        main,
        render: times.load(RenderTimes::RENDER),
        gpu_wait: times.load(RenderTimes::ACQUIRE) + times.load(RenderTimes::SUBMIT),
    };
    if profile.report.is_some() && profile.frames.len() < MAX_FRAMES {
        profile.frames.push(sample);
    }
    profile.window.push(sample);
    #[cfg(feature = "profile")]
    spans::end_frame(sample.at, sample.main);
}

/// Means over a set of frames, ms.
#[derive(Default)]
struct Summary {
    frame: f32,
    worst: f32,
    main: f32,
    render: f32,
    gpu_wait: f32,
}

impl Summary {
    fn of(samples: &[FrameSample]) -> Self {
        let n = samples.len().max(1) as f32;
        let mean = |f: fn(&FrameSample) -> f32| samples.iter().map(f).sum::<f32>() / n;
        Self {
            frame: mean(|s| s.frame),
            worst: samples.iter().map(|s| s.frame).fold(0.0, f32::max),
            main: mean(|s| s.main),
            render: mean(|s| s.render),
            gpu_wait: mean(|s| s.gpu_wait),
        }
    }

    /// What limits the frame rate. The render thread blocks for the next swapchain image,
    /// or to submit, when the GPU (or vsync) is behind; otherwise the slower of the two CPU
    /// threads sets the pace (they run in parallel, a frame apart).
    fn bound(&self, vsync: bool) -> &'static str {
        let render_cpu = self.render - self.gpu_wait;
        if self.gpu_wait > 2.0 && self.gpu_wait > 0.25 * self.frame {
            if vsync { "GPU or vsync" } else { "GPU" }
        } else if self.main.max(render_cpu) < 0.6 * self.frame {
            "GPU or frame cap"
        } else if self.main >= render_cpu {
            "CPU (main thread)"
        } else {
            "CPU (render thread)"
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn publish(
    time: Res<Time<Real>>,
    mut profile: ResMut<Profile>,
    mut data: ResMut<UiData>,
    diagnostics: Res<DiagnosticsStore>,
    settings: Res<GraphicsSettings>,
    adapter: Option<Res<RenderAdapterInfo>>,
    entities: Query<()>,
    meshes: Query<(&Mesh3d, &ViewVisibility)>,
    triangles: Res<MeshTriangles>,
    lights: Query<(), Or<(With<PointLight>, With<SpotLight>, With<DirectionalLight>)>>,
) {
    profile.since_publish += time.delta_secs_f64();
    if profile.since_publish < PUBLISH_PERIOD {
        return;
    }
    profile.since_publish = 0.0;
    let window = std::mem::take(&mut profile.window);
    let summary = Summary::of(&window);
    let visible = || meshes.iter().filter(|(_, v)| v.get());
    let counts = Counts {
        entities: entities.iter().len() as u32,
        meshes: meshes.iter().len() as u32,
        visible_meshes: visible().count() as u32,
        visible_triangles: visible()
            .filter_map(|(mesh, _)| triangles.0.get(&mesh.id()))
            .map(|&n| u64::from(n))
            .sum(),
        lights: lights.iter().len() as u32,
    };
    profile.counts = counts;

    data.set("perf.fps", 1000.0 / summary.frame.max(0.001));
    data.set("perf.frame_ms", summary.frame);
    data.set("perf.worst_ms", summary.worst);
    data.set("perf.main_ms", summary.main);
    data.set("perf.render_ms", summary.render);
    data.set("perf.gpu_wait_ms", summary.gpu_wait);
    data.set("perf.bound", summary.bound(vsync(&settings)));
    data.set("perf.entities", counts.entities);
    data.set("perf.meshes", counts.meshes);
    data.set("perf.visible_meshes", counts.visible_meshes);
    data.set("perf.triangles_m", counts.visible_triangles as f64 / 1e6);
    data.set("perf.lights", counts.lights);
    if let Some(adapter) = adapter {
        data.set("perf.gpu", gpu_description(&adapter));
    }

    // Render passes, slowest first (by GPU time where measured).
    let mut passes: Vec<(String, f64, Option<f64>)> = Vec::new();
    for diagnostic in diagnostics.iter() {
        let path = diagnostic.path().as_str();
        let Some(pass) = path
            .strip_prefix("render/")
            .and_then(|p| p.strip_suffix("/elapsed_cpu"))
        else {
            continue;
        };
        let Some(cpu) = diagnostic.average() else {
            continue;
        };
        let gpu = diagnostics
            .get(&bevy::diagnostic::DiagnosticPath::new(format!(
                "render/{pass}/elapsed_gpu"
            )))
            .and_then(|d| d.average());
        passes.push((pass.to_owned(), cpu, gpu));
    }
    passes.sort_by(|a, b| b.2.unwrap_or(b.1).total_cmp(&a.2.unwrap_or(a.1)));
    if profile.report.is_some() {
        for (pass, cpu, gpu) in &passes {
            let gpu = gpu.unwrap_or(f64::NAN);
            match profile.passes.iter_mut().find(|p| &p.0 == pass) {
                Some(p) => {
                    p.1 += cpu;
                    p.2 += gpu;
                    p.3 += 1;
                }
                None => profile.passes.push((pass.clone(), *cpu, gpu, 1)),
            }
        }
    }
    data.set("perf.gpu_timed", passes.iter().any(|p| p.2.is_some()));
    data.set(
        "perf.passes",
        passes
            .iter()
            .take(OVERLAY_ROWS)
            .map(|(pass, cpu, gpu)| {
                record([
                    ("name", pass.as_str().into()),
                    ("cpu_ms", (*cpu).into()),
                    ("gpu_ms", (*gpu).into()),
                ])
            })
            .collect::<Vec<_>>(),
    );

    data.set("perf.systems_traced", cfg!(feature = "profile"));
    #[cfg(feature = "profile")]
    data.set(
        "perf.systems",
        spans::take_window()
            .into_iter()
            .filter(|s| s.kind == spans::Kind::System)
            .take(OVERLAY_ROWS)
            .map(|s| {
                record([
                    ("name", short_name(&s.name).into()),
                    ("ms", s.mean_ms.into()),
                    ("max_ms", s.max_ms.into()),
                ])
            })
            .collect::<Vec<_>>(),
    );
}

/// Whether vsync paces the frames (`OSP_UNCAPPED` turns it off whatever the setting).
fn vsync(settings: &GraphicsSettings) -> bool {
    settings.vsync && std::env::var_os("OSP_UNCAPPED").is_none()
}

fn gpu_description(adapter: &RenderAdapterInfo) -> String {
    format!(
        "{} ({:?}, {:?})",
        adapter.name, adapter.backend, adapter.device_type
    )
}

/// `osp::planet::update_lod` -> `planet::update_lod`; generic arguments are dropped.
#[cfg(feature = "profile")]
fn short_name(name: &str) -> String {
    let base = name.split('<').next().unwrap_or(name);
    let mut parts: Vec<&str> = base.split("::").collect();
    if parts.len() > 2 {
        parts.drain(..parts.len() - 2);
    }
    let crate_name = base.split("::").next().unwrap_or("");
    let prefix = if crate_name == "osp" { "" } else { crate_name };
    if prefix.is_empty() || parts.first() == Some(&prefix) {
        parts.join("::")
    } else {
        format!("{prefix}: {}", parts.join("::"))
    }
}

fn percentile(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f32 * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn write_report(
    profile: Res<Profile>,
    settings: Res<GraphicsSettings>,
    adapter: Option<Res<RenderAdapterInfo>>,
    window: Option<Single<&Window>>,
    (output, images): (Res<ViewOutput>, Res<Assets<Image>>),
) {
    let Some(path) = &profile.report else { return };
    // The first second is loading: shaders compile and the ground streams in.
    let frames: Vec<FrameSample> = profile
        .frames
        .iter()
        .copied()
        .filter(|f| f.at > 1.0)
        .collect();
    let mut out = String::new();
    let duration = frames.last().map_or(0.0, |f| f.at) - frames.first().map_or(0.0, |f| f.at);
    let _ = writeln!(out, "# OSP profile\n");
    if let Some(adapter) = &adapter {
        let _ = writeln!(out, "GPU: {}", gpu_description(adapter));
    }
    match output.0.as_ref().and_then(|image| images.get(image)) {
        Some(image) => {
            let _ = writeln!(
                out,
                "View: {}x{} px (capture)",
                image.width(),
                image.height()
            );
        }
        None => {
            if let Some(window) = window {
                let _ = writeln!(
                    out,
                    "View: {}x{} px (window, scale {})",
                    window.physical_width(),
                    window.physical_height(),
                    window.scale_factor()
                );
            }
        }
    }
    let _ = writeln!(
        out,
        "Scenario: {}",
        std::env::var("OSP_SCENARIO").unwrap_or_default()
    );
    let _ = writeln!(out, "Graphics: {}", settings.describe());
    let _ = writeln!(
        out,
        "Systems traced: {}",
        if cfg!(feature = "profile") {
            "yes"
        } else {
            "no (build with --features profile)"
        }
    );
    let _ = writeln!(
        out,
        "{} frames over {:.1} s, after the first second\n",
        frames.len(),
        duration
    );

    let summary = Summary::of(&frames);
    let mut sorted: Vec<f32> = frames.iter().map(|f| f.frame).collect();
    sorted.sort_by(f32::total_cmp);
    let _ = writeln!(out, "## Frames\n");
    let _ = writeln!(
        out,
        "frame  mean {:.2} ms ({:.0} fps)  median {:.2}  p95 {:.2}  p99 {:.2}  max {:.2}",
        summary.frame,
        1000.0 / summary.frame.max(0.001),
        percentile(&sorted, 0.5),
        percentile(&sorted, 0.95),
        percentile(&sorted, 0.99),
        summary.worst
    );
    let _ = writeln!(
        out,
        "main world (CPU) mean {:.2} ms; render world (CPU) mean {:.2} ms, of which {:.2} ms waiting for the GPU",
        summary.main, summary.render, summary.gpu_wait
    );
    let _ = writeln!(out, "limited by: {}", summary.bound(vsync(&settings)));
    let over = |ms: f32| sorted.iter().filter(|&&f| f > ms).count();
    let _ = writeln!(
        out,
        "frames over 16.7 ms: {}, over 33.3 ms: {}, over 50 ms: {}\n",
        over(16.7),
        over(33.3),
        over(50.0)
    );

    let _ = writeln!(out, "## Timeline (per second, ms)\n");
    let _ = writeln!(
        out,
        "{:>5} {:>6} {:>7} {:>7} {:>7} {:>7} {:>7}",
        "t s", "fps", "frame", "worst", "main", "render", "wait"
    );
    let mut second = 0;
    let mut start = 0;
    for i in 0..=frames.len() {
        let next_second = frames.get(i).map(|f| f.at as i32);
        if next_second == Some(second) {
            continue;
        }
        if i > start {
            let s = Summary::of(&frames[start..i]);
            let _ = writeln!(
                out,
                "{:>5} {:>6.0} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>7.2}",
                second,
                1000.0 / s.frame.max(0.001),
                s.frame,
                s.worst,
                s.main,
                s.render,
                s.gpu_wait
            );
        }
        start = i;
        second = next_second.unwrap_or(0);
    }

    let _ = writeln!(out, "\n## Worst frames\n");
    let mut worst: Vec<&FrameSample> = frames.iter().collect();
    worst.sort_by(|a, b| b.frame.total_cmp(&a.frame));
    for f in worst.iter().take(10) {
        let _ = writeln!(
            out,
            "t {:>7.2} s  frame {:>7.2} ms  main {:>7.2}  render {:>7.2}  wait {:>7.2}",
            f.at, f.frame, f.main, f.render, f.gpu_wait
        );
    }

    let counts = profile.counts;
    let _ = writeln!(
        out,
        "\n## World (at exit)\n\nentities {}, meshes {} ({} visible, {} triangles), lights {}",
        counts.entities,
        counts.meshes,
        counts.visible_meshes,
        counts.visible_triangles,
        counts.lights
    );

    if !profile.passes.is_empty() {
        let _ = writeln!(
            out,
            "\n## Render passes (mean ms; GPU only on Vulkan/DX12)\n"
        );
        let mut passes: Vec<_> = profile
            .passes
            .iter()
            .map(|(name, cpu, gpu, n)| (name, cpu / *n as f64, gpu / *n as f64))
            .collect();
        passes.sort_by(|a, b| {
            let key = |p: &(&String, f64, f64)| if p.2.is_nan() { p.1 } else { p.2 };
            key(b).total_cmp(&key(a))
        });
        for (name, cpu, gpu) in passes {
            let _ = writeln!(out, "{cpu:>8.3} cpu {gpu:>8.3} gpu  {name}");
        }
    }

    #[cfg(feature = "profile")]
    {
        let _ = writeln!(
            out,
            "\n## Hitches (slowest main-thread frames and their slowest systems, ms)\n"
        );
        for hitch in spans::hitches() {
            let _ = writeln!(out, "t {:>7.2} s  main {:.2} ms", hitch.at, hitch.main_ms);
            for (name, ms) in hitch.systems {
                let _ = writeln!(out, "    {ms:>8.2}  {}", short_name(&name));
            }
        }
        let all = spans::totals();
        for (kind, title) in [
            (spans::Kind::Schedule, "Schedules"),
            (spans::Kind::System, "Systems"),
            (
                spans::Kind::Commands,
                "Commands (applying spawns, inserts, despawns)",
            ),
        ] {
            let _ = writeln!(
                out,
                "\n## {title} (mean ms per frame it ran, worst frame ms, frames)\n"
            );
            for s in all.iter().filter(|s| s.kind == kind).take(60) {
                let _ = writeln!(
                    out,
                    "{:>8.3} {:>8.2} {:>6}  {}",
                    s.mean_ms, s.max_ms, s.frames, s.name
                );
            }
        }
    }

    match std::fs::write(path, out) {
        Ok(()) => info!("profile written to {}", path.display()),
        Err(e) => error!("cannot write the profile to {}: {e}", path.display()),
    }
}

/// Times Bevy's `system`, `system_commands` and `schedule` spans (traced with
/// `--features profile`).
#[cfg(feature = "profile")]
pub mod spans {
    use std::{
        collections::HashMap,
        sync::{LazyLock, Mutex},
        time::Instant,
    };

    use bevy::log::{
        BoxedLayer,
        tracing::{
            Subscriber,
            field::{Field, Visit},
            span,
        },
        tracing_subscriber::{Layer, layer::Context, registry::LookupSpan},
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum Kind {
        System,
        Commands,
        Schedule,
    }

    #[derive(Default)]
    struct Entry {
        /// Nanoseconds in the current frame.
        current: u64,
        /// Since the overlay last read it: nanoseconds, frames it ran in, worst frame.
        window: (u64, u32, u64),
        /// Whole run: nanoseconds, frames it ran in, worst frame.
        total: (u64, u32, u64),
    }

    /// Main-thread hitches kept, with this many of each one's slowest systems.
    const HITCHES: usize = 12;
    const HITCH_SYSTEMS: usize = 8;

    #[derive(Default)]
    struct Table {
        names: Vec<(Kind, String)>,
        index: HashMap<(u8, String), usize>,
        entries: Vec<Entry>,
        /// The slowest main-thread frames: when, how long (ms), and their slowest systems
        /// (index, ns).
        hitches: Vec<(f32, f32, Vec<(usize, u64)>)>,
    }

    pub struct Hitch {
        pub at: f32,
        pub main_ms: f32,
        pub systems: Vec<(String, f64)>,
    }

    static TABLE: LazyLock<Mutex<Table>> = LazyLock::new(Default::default);

    pub struct Stat {
        pub kind: Kind,
        pub name: String,
        pub mean_ms: f64,
        pub max_ms: f64,
        pub frames: u32,
    }

    /// Fold the frame's times into the window and the run, and keep the frame's slowest
    /// systems if its main thread (`main_ms`) was among the slowest. Render-world systems
    /// run alongside, a frame behind, so they land in the frame they finished in.
    pub fn end_frame(at: f32, main_ms: f32) {
        let mut table = TABLE.lock().unwrap();
        let slowest_kept = table.hitches.last().map_or(0.0, |h| h.1);
        if at > 1.0 && (table.hitches.len() < HITCHES || main_ms > slowest_kept) {
            let mut systems: Vec<(usize, u64)> = table
                .entries
                .iter()
                .enumerate()
                .filter(|(i, e)| e.current > 0 && table.names[*i].0 == Kind::System)
                .map(|(i, e)| (i, e.current))
                .collect();
            systems.sort_by_key(|s| std::cmp::Reverse(s.1));
            systems.truncate(HITCH_SYSTEMS);
            table.hitches.push((at, main_ms, systems));
            table.hitches.sort_by(|a, b| b.1.total_cmp(&a.1));
            table.hitches.truncate(HITCHES);
        }
        for e in &mut table.entries {
            if e.current > 0 {
                e.window.0 += e.current;
                e.window.1 += 1;
                e.window.2 = e.window.2.max(e.current);
                e.total.0 += e.current;
                e.total.1 += 1;
                e.total.2 = e.total.2.max(e.current);
                e.current = 0;
            }
        }
    }

    fn stats(table: &Table, pick: impl Fn(&Entry) -> (u64, u32, u64)) -> Vec<Stat> {
        let mut stats: Vec<Stat> = table
            .entries
            .iter()
            .zip(&table.names)
            .filter_map(|(e, (kind, name))| {
                let (ns, frames, max) = pick(e);
                (frames > 0).then(|| Stat {
                    kind: *kind,
                    name: name.clone(),
                    mean_ms: ns as f64 / frames as f64 / 1e6,
                    max_ms: max as f64 / 1e6,
                    frames,
                })
            })
            .collect();
        stats.sort_by(|a, b| b.mean_ms.total_cmp(&a.mean_ms));
        stats
    }

    /// Slowest first, since the last call.
    pub fn take_window() -> Vec<Stat> {
        let mut table = TABLE.lock().unwrap();
        let stats = stats(&table, |e| e.window);
        for e in &mut table.entries {
            e.window = (0, 0, 0);
        }
        stats
    }

    /// Slowest first, over the whole run.
    pub fn totals() -> Vec<Stat> {
        stats(&TABLE.lock().unwrap(), |e| e.total)
    }

    /// The slowest main-thread frames, slowest first.
    pub fn hitches() -> Vec<Hitch> {
        let table = TABLE.lock().unwrap();
        table
            .hitches
            .iter()
            .map(|(at, main_ms, systems)| Hitch {
                at: *at,
                main_ms: *main_ms,
                systems: systems
                    .iter()
                    .map(|(i, ns)| (table.names[*i].1.clone(), *ns as f64 / 1e6))
                    .collect(),
            })
            .collect()
    }

    /// For `LogPlugin::custom_layer`.
    pub fn layer(_app: &mut bevy::app::App) -> Option<BoxedLayer> {
        Some(Box::new(SpanTimer))
    }

    struct SpanTimer;

    /// Index into the table, and when the span was entered.
    struct Timed(usize, Option<Instant>);

    struct NameVisitor(String);

    impl Visit for NameVisitor {
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "name" {
                self.0 = value.to_owned();
            }
        }

        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "name" {
                self.0 = format!("{value:?}");
            }
        }
    }

    impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for SpanTimer {
        fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &span::Id, ctx: Context<'_, S>) {
            let kind = match attrs.metadata().name() {
                "system" => Kind::System,
                "system_commands" => Kind::Commands,
                "schedule" => Kind::Schedule,
                _ => return,
            };
            let mut visitor = NameVisitor(String::new());
            attrs.record(&mut visitor);
            let index = {
                let mut table = TABLE.lock().unwrap();
                let key = (kind as u8, visitor.0);
                match table.index.get(&key) {
                    Some(&i) => i,
                    None => {
                        let i = table.entries.len();
                        table.names.push((kind, key.1.clone()));
                        table.entries.push(Entry::default());
                        table.index.insert(key, i);
                        i
                    }
                }
            };
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(Timed(index, None));
            }
        }

        fn on_enter(&self, id: &span::Id, ctx: Context<'_, S>) {
            if let Some(span) = ctx.span(id)
                && let Some(timed) = span.extensions_mut().get_mut::<Timed>()
            {
                timed.1 = Some(Instant::now());
            }
        }

        fn on_exit(&self, id: &span::Id, ctx: Context<'_, S>) {
            let Some(span) = ctx.span(id) else { return };
            let mut extensions = span.extensions_mut();
            let Some(timed) = extensions.get_mut::<Timed>() else {
                return;
            };
            let Some(entered) = timed.1.take() else {
                return;
            };
            let ns = entered.elapsed().as_nanos() as u64;
            let index = timed.0;
            drop(extensions);
            TABLE.lock().unwrap().entries[index].current += ns;
        }
    }
}
