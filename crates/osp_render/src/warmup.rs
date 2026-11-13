//! Shader warm-up: the pipelines the flight and map views need are made behind a loading screen
//! at startup, not in the middle of play.
//!
//! Bevy makes a pipeline (one material's shaders compiled for one mesh layout and view setup) the
//! first time something needs it, and on macOS it does so on the render thread and waits. The
//! driver keeps what it compiled, but a new build of the game starts it over: on the first start
//! after an install or an update, the scene's first frame took 7 s and the first map view 3 s
//! (the map view swaps TAA for FXAA, so every material is made again for it).
//!
//! So the view's camera starts switched off under a loading screen. Once that has been shown, the
//! view draws the scene, then the map, then the scene again, each until no pipeline has been made
//! for a moment and nothing holds it ([`Warmup::hold`]: the ground while its chunks build). Game
//! time stands still meanwhile.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use bevy::{
    prelude::*,
    render::{Render, RenderApp, RenderSystems, render_resource::PipelineCache, view::Msaa},
    text::{FontSource, FontWeight, TextLayoutInfo},
};
use osp_game::{
    camera::{CameraRig, MainCamera},
    profiler::LoadedAt,
};

/// Frames the loading screen is drawn with its text before the scene starts.
const SHOW_FRAMES: u32 = 3;
/// The longest the loading screen waits for its text (its font loading) before going on.
const SHOW_LIMIT: Duration = Duration::from_secs(3);
/// A view is done once no pipeline has been made for this long, nothing holds it...
const QUIET: Duration = Duration::from_millis(400);
/// ...and it has been drawn at least this many times. Loading ends after `LIMIT` whatever happens.
const MIN_FRAMES: u32 = 5;
const LIMIT: Duration = Duration::from_secs(120);
/// The loading screen draws over everything.
const SCREEN_ORDER: isize = 100;

const BACKGROUND: Color = Color::srgb(0.039, 0.075, 0.110);
const TEXT: Color = Color::srgb(0.894, 0.929, 0.957);
const TEXT_DIM: Color = Color::srgb(0.561, 0.639, 0.706);

pub struct WarmupPlugin;

impl Plugin for WarmupPlugin {
    fn build(&self, app: &mut App) {
        let progress = Progress::default();
        app.insert_resource(progress.clone())
            .insert_resource(Warmup {
                phase: Phase::Showing { frames: 0 },
                started: Instant::now(),
                held: false,
            })
            .add_systems(Startup, show_screen)
            .add_systems(PostStartup, hold_camera)
            .add_systems(
                PostUpdate,
                run.run_if(|warmup: Res<Warmup>| warmup.loading()),
            );
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.insert_resource(progress).add_systems(
                Render,
                count_pipelines
                    .in_set(RenderSystems::Cleanup)
                    .run_if(|progress: Res<Progress>| !progress.done.load(Ordering::Relaxed)),
            );
        }
    }
}

/// Where loading is at. Something the view needs that is still on its way (the ground's chunks,
/// say) calls [`Warmup::hold`] every frame until it is there.
#[derive(Resource)]
pub struct Warmup {
    phase: Phase,
    started: Instant,
    /// Held this frame.
    held: bool,
}

impl Warmup {
    /// Keep the loading screen up this frame.
    pub fn hold(&mut self) {
        self.held = true;
    }

    pub fn loading(&self) -> bool {
        !matches!(self.phase, Phase::Done)
    }
}

enum Phase {
    /// The loading screen is up and the scene isn't drawn yet; `frames` drawn with the text laid
    /// out.
    Showing {
        frames: u32,
    },
    /// The view draws behind the loading screen: at `step` 0 as it started (`first_map`), at 1
    /// the other way (flight or map), at 2 as it started again. `made`: pipelines so far, the
    /// last of them at `last_made`.
    Making {
        first_map: bool,
        step: u8,
        frames: u32,
        made: usize,
        last_made: Instant,
    },
    Done,
}

/// The render world's pipelines: how many there are, how many are still being made, and whether
/// loading is over (then it stops counting).
#[derive(Resource, Clone, Default)]
struct Progress {
    made: Arc<AtomicUsize>,
    waiting: Arc<AtomicUsize>,
    done: Arc<AtomicBool>,
}

fn count_pipelines(cache: Res<PipelineCache>, progress: Res<Progress>) {
    progress
        .made
        .store(cache.pipelines().count(), Ordering::Relaxed);
    progress
        .waiting
        .store(cache.waiting_pipelines().count(), Ordering::Relaxed);
}

/// The loading screen and its camera.
#[derive(Component)]
struct LoadingScreen;

#[derive(Component)]
struct LoadingText;

fn show_screen(
    mut commands: Commands,
    mut time: ResMut<Time<Virtual>>,
    mut loaded_at: ResMut<LoadedAt>,
) {
    time.pause();
    loaded_at.0 = None;
    let camera = commands
        .spawn((
            LoadingScreen,
            Camera2d,
            Camera {
                order: SCREEN_ORDER,
                clear_color: ClearColorConfig::Custom(BACKGROUND),
                ..default()
            },
            Msaa::Off,
        ))
        .id();
    let text = |size: f32, weight: u16, color: Color| {
        (
            TextFont {
                font: FontSource::Families("Barlow".into()),
                weight: FontWeight(weight),
                ..TextFont::from_font_size(size)
            },
            TextColor(color),
        )
    };
    commands
        .spawn((
            LoadingScreen,
            UiTargetCamera(camera),
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(10),
                ..default()
            },
        ))
        .with_children(|screen| {
            screen.spawn((
                LoadingText,
                Text::new("PREPARING SHADERS"),
                text(26.0, 600, TEXT),
            ));
            screen.spawn((
                Text::new("The first start after an update takes a little longer."),
                text(16.0, 500, TEXT_DIM),
            ));
        });
}

/// The view's camera stays off until the loading screen has been shown.
fn hold_camera(mut camera: Single<&mut Camera, With<MainCamera>>) {
    camera.is_active = false;
}

#[allow(clippy::too_many_arguments)]
fn run(
    mut commands: Commands,
    mut warmup: ResMut<Warmup>,
    progress: Res<Progress>,
    mut rig: ResMut<CameraRig>,
    mut time: ResMut<Time<Virtual>>,
    real: Res<Time<Real>>,
    mut loaded_at: ResMut<LoadedAt>,
    text: Query<&TextLayoutInfo, With<LoadingText>>,
    mut camera: Single<&mut Camera, With<MainCamera>>,
    screen: Query<Entity, With<LoadingScreen>>,
) {
    let warmup = &mut *warmup;
    let held = std::mem::take(&mut warmup.held);
    let now = Instant::now();
    let over_time = warmup.started.elapsed() > LIMIT;
    let made_now = progress.made.load(Ordering::Relaxed);
    match &mut warmup.phase {
        Phase::Showing { frames } => {
            if text.iter().any(|layout| !layout.glyphs.is_empty()) {
                *frames += 1;
            }
            if *frames < SHOW_FRAMES && warmup.started.elapsed() < SHOW_LIMIT {
                return;
            }
            debug!(
                "shader warm-up: loading screen shown after {:.2} s",
                warmup.started.elapsed().as_secs_f32()
            );
            camera.is_active = true;
            warmup.phase = Phase::Making {
                first_map: rig.map_view,
                step: 0,
                frames: 0,
                made: made_now,
                last_made: now,
            };
        }
        Phase::Making {
            first_map,
            step,
            frames,
            made,
            last_made,
        } => {
            *frames += 1;
            if made_now != *made || held {
                *made = made_now;
                *last_made = now;
            }
            let settled = *frames >= MIN_FRAMES
                && progress.waiting.load(Ordering::Relaxed) == 0
                && now.duration_since(*last_made) >= QUIET;
            if !settled && !over_time {
                return;
            }
            debug!(
                "shader warm-up: view {step} done after {:.2} s, {frames} frames, {made_now} pipelines",
                warmup.started.elapsed().as_secs_f32()
            );
            if *step < 2 && !over_time {
                // Now the other view, then back to the first.
                *step += 1;
                rig.map_view = *first_map != (*step == 1);
                *frames = 0;
                *last_made = now;
                return;
            }
            rig.map_view = *first_map;
            for entity in &screen {
                commands.entity(entity).despawn();
            }
            time.unpause();
            progress.done.store(true, Ordering::Relaxed);
            loaded_at.0 = Some(real.elapsed_secs());
            info!(
                "shaders ready after {:.1} s: {made_now} pipelines",
                warmup.started.elapsed().as_secs_f32()
            );
            warmup.phase = Phase::Done;
        }
        Phase::Done => {}
    }
}
