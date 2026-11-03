//! What a flight sounds like, from what the simulation says is happening.
//!
//! Space is silent: everything but the alert tone needs air to carry it ([`air_carries`]), both
//! where the sound is made and where the vessel being flown is.
//!
//! - Engines roar through the air, from recordings ([`recordings`]). A jet turns about half a
//!   percent of its power into noise, so how loud it is follows from its thrust and exhaust speed,
//!   the distance to the camera and the air around it. The size of the jet sets how fast the
//!   recording plays, so a cluster of big engines is deeper than a small one. Distance and thin
//!   air move it to the takes recorded from further away and take the highs out, and a solid is
//!   brighter. Lighting an engine plays a take's own attack and cutting one its own tail.
//! - The vessel being flown also shakes under its thrust: the most distant take, muffled, as you
//!   would hear it through the hull.
//! - Wind is synthesised: it follows the air's speed and dynamic pressure, in gusts that move its
//!   pitch and differ in each ear, and through Mach 1 the hull shakes with it.
//! - Separating stages, separation motors, collisions and RCS thrusters make no sound.
//!
//! [`Mix`] is what the game tells the synthesiser (shared levels, read on the audio thread);
//! [`Synth`] makes the sound from it. A scripted capture stays silent. To listen without flying,
//! `cargo test -p osp_game render_sound_samples -- --ignored` writes scenes to `target/sound`.

mod recordings;

use std::{
    f32::consts::{PI, TAU},
    num::NonZero,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use bevy::{
    audio::{AddAudioSource, ChannelCount, Decodable, SampleRate, Source},
    prelude::*,
};
use osp_sim::{constants::G0, propulsion::Engine};

use self::recordings::{Bank, CLOSE, DISTANT, Looper, Tail, VERY_FAR};
use crate::{
    FrameSet,
    alerts::ActiveAlerts,
    camera::{CameraRig, MainCamera},
    content,
    frame::RenderFrame,
    menu::Menu,
    settings::{AudioSettings, InterfaceSettings},
    sim::Sim,
};

/// Share of a jet's power that goes into noise.
const ACOUSTIC_EFFICIENCY: f64 = 0.005;
/// Sound pressure level that plays at full scale, dB: a Saturn V from about 100 m.
const FULL_SCALE: f64 = 150.0;
/// How many dB down the level is for a tenth of the amplitude. Twice as many as in the air, so a
/// launch from 10 km can be heard without the one from 100 m deafening.
const DB_PER_TENTH: f64 = 40.0;
/// Air at sea level, Pa.
const SEA_LEVEL: f64 = 101_325.0;
/// With less air than this nothing can be heard, Pa (Earth at about 65 km), and with this much the
/// air carries sound as well as it ever does (about 20 km).
const AIR_SILENT: f64 = 10.0;
const AIR_FULL: f64 = 5_000.0;
/// How far away the map view listens from, m, and how much quieter it is on top.
const MAP_DISTANCE: f64 = 300.0;
const MAP_QUIET: f32 = 0.35;
/// A vessel's hull is heard from this close, m.
const HULL_REACH: f64 = 500.0;
/// The acceleration at which the hull shakes at its loudest, m/s².
const HULL_FULL: f64 = 30.0;
/// The dynamic pressure at which the wind is at its loudest, Pa, and the power of its share of
/// that the level follows: gentler than the speed itself, or a slow plane would be silent.
const WIND_FULL: f64 = 40_000.0;
const WIND_CURVE: f64 = 0.35;
/// Where a jet's noise peaks, as a Strouhal number, and the lowest and highest that is let go, Hz.
const STROUHAL: f64 = 0.2;
const PEAK_LOWEST: f32 = 55.0;
const PEAK_HIGHEST: f32 = 700.0;
/// The peak the recordings play at their own speed for, Hz (a first stage of nine kerosene
/// engines), and the slowest and fastest they play: outside that a jet's size stops sounding like
/// its size and starts sounding like a tape.
const PEAK_AS_RECORDED: f32 = 200.0;
const SLOWEST: f32 = 0.75;
const FASTEST: f32 = 1.35;
/// How much further a throttled engine's level falls, as a power of the thrust share, on top of
/// what the physics says, which is hardly any: full to a tenth should be heard from the seat.
const THROTTLE_SWING: f64 = 0.35;
/// How much slower a jet at no thrust at all would play.
const THROTTLE_RATE: f32 = 0.1;
/// From this distance on a jet starts to sound distant, and at the other it is as far as it gets, m.
const DISTANCE_NEAR: f64 = 300.0;
const DISTANCE_FAR: f64 = 20_000.0;
/// How many tenths of the sea-level air it takes for a jet to sound as far as it gets.
const THIN_AIR_DECADES: f64 = 3.0;
/// How high a jet is still heard from close up, the distance that halves that, m, and the least.
const REACH_CLOSE: f64 = 16_000.0;
const ABSORPTION: f64 = 400.0;
const REACH_LOWEST: f32 = 200.0;
/// Below this share of what was running a moment ago, the engines count as cut.
const CUT_SHARE: f32 = 0.5;
/// A cut quieter than this has no tail worth playing.
const CUT_LEAST: f32 = 0.02;
/// Least seconds between alert tones, however often an alert comes and goes.
const ALERT_GAP: f64 = 10.0;

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<FlightSound>()
            .init_resource::<FlightMix>()
            .add_systems(Startup, start)
            .add_systems(Update, listen.in_set(FrameSet::Render));
    }
}

/// What the game has the synthesiser play.
#[derive(Resource, Default, Clone)]
struct FlightMix(Arc<Mix>);

#[derive(Asset, TypePath, Clone)]
struct FlightSound(Arc<Mix>);

impl Decodable for FlightSound {
    type Decoder = Synth;

    fn decoder(&self) -> Synth {
        Synth::new(self.0.clone())
    }
}

fn start(mut commands: Commands, mut sounds: ResMut<Assets<FlightSound>>, mix: Res<FlightMix>) {
    if std::env::var_os("OSP_CAPTURE").is_some() {
        return;
    }
    // Decoding takes a moment, and the game has nothing to play in the first one.
    let shared = mix.0.clone();
    std::thread::spawn(move || {
        let bank = Bank::load(&content::asset_root().join("core/sounds"));
        let _ = shared.bank.set(Arc::new(bank));
    });
    // It never ends, so it plays once for as long as the game runs.
    commands.spawn(AudioPlayer(sounds.add(FlightSound(mix.0.clone()))));
}

/// A jet as a source of noise.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Jet {
    /// Thrust, N.
    thrust: f64,
    /// Exhaust speed, m/s.
    exhaust: f64,
    /// Exit diameter, m.
    diameter: f64,
}

impl Jet {
    fn acoustic_power(&self) -> f64 {
        ACOUSTIC_EFFICIENCY * 0.5 * self.thrust * self.exhaust
    }
}

/// How much sound air at `pressure` Pa carries, 0..1. The levels alone would never get to silence:
/// they are played with half their range, so a rocket at 100 km would still be heard.
fn air_carries(pressure: f64) -> f32 {
    let x = (pressure.max(1e-9) / AIR_SILENT).log10() / (AIR_FULL / AIR_SILENT).log10();
    let x = x.clamp(0.0, 1.0) as f32;
    x * x * (3.0 - 2.0 * x)
}

/// Sound pressure level of `power` W of noise `distance` m away with air at `pressure` Pa, dB.
fn level(power: f64, distance: f64, pressure: f64) -> f64 {
    let intensity = power / (4.0 * std::f64::consts::PI * distance * distance)
        * (pressure / SEA_LEVEL).min(1.0);
    10.0 * (intensity.max(1e-30) / 1e-12).log10()
}

/// The amplitude to play a level at, 0..1.
fn amplitude(level: f64) -> f32 {
    10f64.powf((level - FULL_SCALE) / DB_PER_TENTH).min(1.0) as f32
}

/// Where the noise of jets of `area` m² of exit in all, leaving at `exhaust` m/s, peaks, Hz.
fn roar_peak(exhaust: f64, area: f64) -> f32 {
    let diameter = (4.0 * area / std::f64::consts::PI).sqrt().max(0.05);
    ((STROUHAL * exhaust / diameter) as f32).clamp(PEAK_LOWEST, PEAK_HIGHEST)
}

/// How fast the recordings play for jets whose noise peaks at `peak` Hz.
fn roar_rate(peak: f32) -> f32 {
    (peak / PEAK_AS_RECORDED).sqrt().clamp(SLOWEST, FASTEST)
}

/// How far off a jet sounds, 0..1, from `distance` m away with air at `pressure` Pa: thin air is
/// like distance, taking the body out of it long before it goes quiet.
fn roar_far(distance: f64, pressure: f64) -> f32 {
    let away = (distance / DISTANCE_NEAR).max(1.0).log10() / (DISTANCE_FAR / DISTANCE_NEAR).log10();
    let thin = (SEA_LEVEL / pressure.max(1e-3)).max(1.0).log10() / THIN_AIR_DECADES;
    away.max(thin).clamp(0.0, 1.0) as f32
}

/// How high a roar is still heard from `distance` m, Hz.
fn roar_reach(distance: f64) -> f32 {
    ((REACH_CLOSE / (1.0 + distance / ABSORPTION)) as f32).max(REACH_LOWEST)
}

/// What is heard of a vessel's lit engines from `distance` m away.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Heard {
    amplitude: f32,
    /// The amplitude of the engines still running: without those in their tail-off.
    running: f32,
    rate: f32,
    far: f32,
    reach: f32,
    /// How much of it is a solid's, 0..1.
    solid: f32,
}

/// Whether an engine counts as one to hear: not the small motors that fire as a stage separates,
/// which are part of the separation and make no sound.
fn is_heard(engine: &Engine) -> bool {
    !engine.is_separation_motor()
}

fn engines_heard(engines: &[Engine], pressure: f64, distance: f64) -> Option<Heard> {
    let (mut power, mut full, mut running) = (0.0, 0.0, 0.0);
    let (mut area, mut flow, mut solid) = (0.0, 0.0, 0.0);
    for engine in engines.iter().filter(|e| is_heard(e) && e.level > 0.0) {
        let jet = Jet {
            thrust: engine.thrust,
            exhaust: engine.isp(pressure) * G0,
            diameter: engine.def.exit_diameter.max(0.2),
        };
        let noise = jet.acoustic_power();
        power += noise;
        full += noise / engine.level;
        area += std::f64::consts::PI * jet.diameter * jet.diameter / 4.0;
        flow += noise * jet.exhaust;
        if engine.running {
            running += noise;
        }
        if engine.is_solid() {
            solid += noise;
        }
    }
    (power > 0.0).then(|| {
        let open = power / full;
        let swing = air_carries(pressure) * open.powf(THROTTLE_SWING) as f32;
        let rate = roar_rate(roar_peak(flow / power, area));
        Heard {
            amplitude: swing * amplitude(level(power, distance, pressure)),
            running: swing * amplitude(level(running, distance, pressure)),
            rate: rate * (1.0 - THROTTLE_RATE * (1.0 - open as f32)),
            far: roar_far(distance, pressure),
            reach: roar_reach(distance),
            solid: (solid / power) as f32,
        }
    })
}

/// What the game last told the synthesiser, to tell what changed.
#[derive(Default)]
struct Memory {
    /// Engines running anywhere, and the amplitude of what they add up to.
    running: usize,
    running_level: f32,
    alerts: usize,
    /// When the alert tone last sounded, real seconds.
    alert_tone: Option<f64>,
}

/// Work out what the camera hears and tell the synthesiser.
#[allow(clippy::too_many_arguments)]
fn listen(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    camera: Single<&Transform, With<MainCamera>>,
    menu: Res<Menu>,
    settings: Res<AudioSettings>,
    interface: Res<InterfaceSettings>,
    alerts: Res<ActiveAlerts>,
    time: Res<Time>,
    mix: Res<FlightMix>,
    mut memory: Local<Memory>,
) {
    let mix = &mix.0;
    let system = &sim.universe.system;
    let map = if rig.map_view { MAP_QUIET } else { 1.0 };
    let share = |percent: u32| percent as f32 / 100.0;
    let engines = share(settings.engines) * map;
    let now = time.elapsed_secs_f64();
    let flown = sim.active_id();
    let (mut roar, mut heard_running, mut running) = (0.0f32, 0.0f32, 0);
    let (mut rate, mut far, mut reach, mut solid) = (1.0, 0.0, REACH_LOWEST, 0.0);
    let (mut hull, mut wind, mut wind_tone, mut buffet) = (0.0f32, 0.0, 0.0, 0.0);
    for vessel in &sim.universe.vessels {
        let at = frame.to_render(
            vessel.parent(),
            vessel.state_at(system, frame.time).position,
        );
        let distance = if rig.map_view {
            MAP_DISTANCE
        } else {
            f64::from(at.distance(camera.translation)).max(1.0)
        };
        running += (vessel.propulsion.engines.iter())
            .filter(|e| is_heard(e) && e.running)
            .count();
        let engines_here = &vessel.propulsion.engines;
        let pressure = vessel.flight.static_pressure;
        if let Some(heard) = engines_heard(engines_here, pressure, distance) {
            // Noises add by power, and the louder says what it sounds like.
            if heard.amplitude >= roar {
                (rate, far, reach, solid) = (heard.rate, heard.far, heard.reach, heard.solid);
            }
            roar = roar.hypot(heard.amplitude);
            heard_running = heard_running.hypot(heard.running);
        }
        if vessel.id() != flown {
            continue;
        }
        let near = 1.0 / (1.0 + distance / HULL_REACH);
        if vessel.flight.mass > 0.0 {
            let thrust: f64 = (engines_here.iter())
                .filter(|e| is_heard(e))
                .map(|e| e.thrust)
                .sum();
            let shake = (thrust / vessel.flight.mass / HULL_FULL).sqrt();
            hull = shake.min(1.0) as f32 * near as f32;
        }
        let pressure = (vessel.flight.dynamic_pressure / WIND_FULL).max(0.0);
        wind = pressure.powf(WIND_CURVE).min(1.0) as f32 * near as f32;
        let speed = 340.0 * vessel.flight.mach;
        wind_tone = (120.0 + 0.8 * speed).min(1_100.0) as f32;
        let through = ((vessel.flight.mach - 1.0) / 0.25).powi(2);
        buffet = (-through).exp() as f32 * wind;
    }
    if running < memory.running
        && memory.running_level > CUT_LEAST
        && heard_running < CUT_SHARE * memory.running_level
    {
        mix.cut(memory.running_level * engines);
    }
    (memory.running, memory.running_level) = (running, heard_running);
    let rested = memory.alert_tone.is_none_or(|last| now - last >= ALERT_GAP);
    if alerts.0.len() > memory.alerts && interface.alert_tone && rested {
        mix.beep();
        memory.alert_tone = Some(now);
    }
    memory.alerts = alerts.0.len();
    let blowing = share(settings.wind) * map;
    let effects = share(settings.effects);
    let master = if sim.paused || menu.open {
        0.0
    } else {
        share(settings.master)
    };
    mix.set(Voice::Roar, roar * engines);
    // With nothing heard these stay as they were, for a tail still playing out.
    if roar > 0.0 {
        mix.set(Voice::RoarRate, rate);
        mix.set(Voice::RoarFar, far);
        mix.set(Voice::RoarReach, reach);
        mix.set(Voice::RoarSolid, solid);
    }
    mix.set(Voice::Hull, hull * engines);
    mix.set(Voice::Wind, wind * blowing);
    mix.set(Voice::WindTone, wind_tone.max(100.0));
    mix.set(Voice::Buffet, buffet * blowing);
    let here = air_carries(sim.active_vessel().flight.static_pressure);
    mix.set(Voice::Air, here);
    mix.set(Voice::Effects, effects);
    mix.set(Voice::Master, master);
}

/// A level the synthesiser reads.
#[derive(Clone, Copy)]
enum Voice {
    Roar,
    RoarRate,
    RoarFar,
    RoarReach,
    RoarSolid,
    Hull,
    Wind,
    WindTone,
    Buffet,
    /// How much sound the air where the vessel being flown is carries: all but the alert tone.
    Air,
    Effects,
    Master,
}

const VOICES: [Voice; 12] = [
    Voice::Roar,
    Voice::RoarRate,
    Voice::RoarFar,
    Voice::RoarReach,
    Voice::RoarSolid,
    Voice::Hull,
    Voice::Wind,
    Voice::WindTone,
    Voice::Buffet,
    Voice::Air,
    Voice::Effects,
    Voice::Master,
];

impl Voice {
    /// Seconds it takes to rise and to fall: the simulation already spools the jets up and down,
    /// wind builds and dies away, the air thins.
    fn follows(self) -> (f32, f32) {
        match self {
            Voice::Roar => (0.1, 0.12),
            Voice::RoarRate
            | Voice::RoarFar
            | Voice::RoarReach
            | Voice::RoarSolid
            | Voice::WindTone => (0.5, 0.5),
            Voice::Hull | Voice::Buffet => (0.3, 0.5),
            Voice::Wind => (0.6, 0.8),
            Voice::Air => (0.3, 0.3),
            Voice::Effects | Voice::Master => (0.05, 0.05),
        }
    }

    /// A setting stands where it is first told to, and where it is told to as the engines are
    /// lit, rather than sweeping there.
    fn is_tone(self) -> bool {
        matches!(
            self,
            Voice::RoarRate
                | Voice::RoarFar
                | Voice::RoarReach
                | Voice::RoarSolid
                | Voice::WindTone
        )
    }
}

/// What the game has the synthesiser play: levels, and one-off sounds waiting to go off.
#[derive(Default)]
pub struct Mix {
    levels: [AtomicU32; VOICES.len()],
    beep: AtomicU32,
    cut: AtomicU32,
    /// The recordings, once they are decoded.
    bank: OnceLock<Arc<Bank>>,
}

impl Mix {
    fn set(&self, voice: Voice, value: f32) {
        self.levels[voice as usize].store(value.to_bits(), Ordering::Relaxed);
    }

    fn get(&self, voice: Voice) -> f32 {
        f32::from_bits(self.levels[voice as usize].load(Ordering::Relaxed))
    }

    /// The alert tone, once.
    fn beep(&self) {
        self.beep.store(1, Ordering::Relaxed);
    }

    fn take_beep(&self) -> bool {
        self.beep.swap(0, Ordering::Relaxed) != 0
    }

    /// Engines that were roaring at this level have been cut.
    fn cut(&self, level: f32) {
        self.cut.store(level.to_bits(), Ordering::Relaxed);
    }

    fn take_cut(&self) -> f32 {
        f32::from_bits(self.cut.swap(0, Ordering::Relaxed))
    }
}

const RATE: u32 = 44_100;
/// Frames between looks at the mix.
const CONTROL_FRAMES: u32 = 32;
/// Under this a voice is not worth working out.
const SILENT: f32 = 1e-3;
/// How much of a cut's level its tail starts at: the jet's own tail-off is still dying under it.
const TAIL_LEVEL: f32 = 0.85;
/// How loud each voice is at a level of 1, against the recordings' own level.
const HULL_LEVEL: f32 = 1.3;
const BUFFET_LEVEL: f32 = 0.85;
const WIND_RMS: f32 = 0.075;
/// How fast the hull plays the most distant take, and what it lets through, Hz.
const HULL_RATE: f64 = 0.85;
const HULL_TOP: f32 = 900.0;
const HULL_FLOOR: f32 = 30.0;
/// Nothing lower than this reaches any speaker, Hz.
const ROAR_FLOOR: f32 = 25.0;
/// How much is lifted above this, Hz, for speakers that cannot play what is mostly bass.
const PRESENCE_EDGE: f32 = 250.0;
const PRESENCE_LIFT: f32 = 0.4;
/// How much a solid lifts what is above this, Hz: the crackle in the recordings.
const SOLID_EDGE: f32 = 900.0;
const SOLID_LIFT: f32 = 2.4;
/// The wind: a rush of noise that falls away steeply above its tone, a narrow band that howls
/// above that, up to a point past which it would whistle, and a rumble under them, with how much
/// damping the two filters have and how much of each is heard.
const WIND_BODY_DAMPING: f32 = 1.1;
const WIND_HOWL_DAMPING: f32 = 0.25;
const WIND_HOWL_ABOVE: f32 = 1.6;
const WIND_HOWL_TOP: f32 = 1_300.0;
const WIND_RUMBLE_TOP: f32 = 140.0;
const WIND_MIX: [f32; 3] = [0.86, 0.2, 0.46];
/// The wind's slow gusts and fast flutter, each in its own ear (Hz, depth), and how far a gust
/// moves its pitch, as a share. A faster gust is a higher note, which is what makes it wind.
const WIND_GUST: (f32, f32) = (0.35, 0.42);
const WIND_FLUTTER: (f32, f32) = (2.3, 0.16);
const WIND_PITCH_SWING: f32 = 0.32;
/// How the hull shakes through Mach 1 (Hz, depth).
const BUFFET_SHAKE: (f32, f32) = (9.0, 0.6);
/// The alert tone: two notes, Hz, each for this long, seconds, at this level.
const BEEP_NOTES: [f32; 2] = [740.0, 587.0];
const BEEP_NOTE: f32 = 0.11;
const BEEP_LEVEL: f32 = 0.11;
/// White noise in -1..1 has this RMS.
const WHITE_RMS: f32 = 0.577;

/// A one-pole low-pass filter.
#[derive(Default, Clone, Copy)]
struct Pole(f32);

impl Pole {
    fn tick(&mut self, coefficient: f32, input: f32) -> f32 {
        self.0 += coefficient * (input - self.0);
        self.0
    }
}

/// The filter coefficient for a cutoff of `hz`.
fn coefficient(hz: f32) -> f32 {
    1.0 - (-TAU * hz / RATE as f32).exp()
}

/// What brings white noise through a pole of this coefficient back to an RMS of 1.
fn unity(coefficient: f32) -> f32 {
    ((2.0 - coefficient) / coefficient.max(1e-6)).sqrt() / WHITE_RMS
}

/// White noise in -1..1 (xorshift).
#[derive(Clone, Copy)]
struct Noise(u32);

impl Noise {
    fn white(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }

    /// Evenly in 0..1.
    fn share(&mut self) -> f32 {
        0.5 * (self.white() + 1.0)
    }
}

/// Slow noise with an RMS of 1, and how far it moves something about, as a share.
#[derive(Clone, Copy)]
struct Surge {
    pole: Pole,
    coefficient: f32,
    gain: f32,
    depth: f32,
}

impl Surge {
    fn new((hz, depth): (f32, f32)) -> Self {
        let coefficient = coefficient(hz);
        Self {
            pole: Pole::default(),
            coefficient,
            gain: unity(coefficient),
            depth,
        }
    }

    fn tick(&mut self, white: f32) -> f32 {
        self.gain * self.pole.tick(self.coefficient, white)
    }
}

/// A state-variable filter whose low-pass and band-pass both bring white noise to an RMS of 1
/// wherever it is tuned.
#[derive(Default, Clone, Copy)]
struct Filter {
    low: f32,
    band: f32,
    f: f32,
    gain: f32,
}

impl Filter {
    fn tune(&mut self, hz: f32, damping: f32) {
        self.f = 2.0 * (PI * hz.clamp(20.0, 5_000.0) / RATE as f32).sin();
        // White noise leaves either output with a variance of about f / 2 damping.
        self.gain = (2.0 * damping / self.f).sqrt() / WHITE_RMS;
    }

    /// The low-pass and the band-pass.
    fn tick(&mut self, input: f32, damping: f32) -> (f32, f32) {
        let high = input - self.low - damping * self.band;
        self.band += self.f * high;
        self.low += self.f * self.band;
        (self.gain * self.low, self.gain * self.band)
    }
}

/// The wind in one ear.
#[derive(Clone, Copy)]
struct Gale {
    body: Filter,
    howl: Filter,
    rumble: Pole,
    gust: Surge,
    flutter: Surge,
    /// Where the gust stands, for the pitch.
    swing: f32,
}

impl Gale {
    fn new() -> Self {
        Self {
            body: Filter::default(),
            howl: Filter::default(),
            rumble: Pole::default(),
            gust: Surge::new(WIND_GUST),
            flutter: Surge::new(WIND_FLUTTER),
            swing: 0.0,
        }
    }

    fn tune(&mut self, tone: f32) {
        let moved = tone * (1.0 + WIND_PITCH_SWING * self.swing).max(0.4);
        self.body.tune(moved, WIND_BODY_DAMPING);
        let howl = (WIND_HOWL_ABOVE * moved).min(WIND_HOWL_TOP);
        self.howl.tune(howl, WIND_HOWL_DAMPING);
    }

    /// One sample with an RMS of about 1, from three unrelated noises.
    fn tick(&mut self, fixed: &Fixed, input: f32, gust: f32, flutter: f32) -> f32 {
        self.swing = self.gust.tick(gust);
        let flutter = self.flutter.depth * self.flutter.tick(flutter);
        let (body, _) = self.body.tick(input, WIND_BODY_DAMPING);
        let (_, howl) = self.howl.tick(input, WIND_HOWL_DAMPING);
        let rumble = fixed.wind_rumble_gain * self.rumble.tick(fixed.wind_rumble, input);
        let level = (1.0 + self.gust.depth * self.swing).max(0.15) * (1.0 + flutter).max(0.2);
        level * (WIND_MIX[0] * body + WIND_MIX[1] * howl + WIND_MIX[2] * rumble)
    }
}

/// One channel's filters and its own noise.
#[derive(Clone, Copy)]
struct Channel {
    noise: Noise,
    reach: Pole,
    floor: Pole,
    presence: Pole,
    edge: Pole,
    hull: Pole,
    hull_floor: Pole,
    gale: Gale,
}

impl Channel {
    fn new(seed: u32) -> Self {
        Self {
            noise: Noise(seed),
            reach: Pole::default(),
            floor: Pole::default(),
            presence: Pole::default(),
            edge: Pole::default(),
            hull: Pole::default(),
            hull_floor: Pole::default(),
            gale: Gale::new(),
        }
    }
}

/// The filters that do not move, worked out once: this runs on the audio thread.
struct Fixed {
    floor: f32,
    presence: f32,
    edge: f32,
    hull_top: f32,
    hull_floor: f32,
    wind_rumble: f32,
    wind_rumble_gain: f32,
}

impl Fixed {
    fn new() -> Self {
        let wind_rumble = coefficient(WIND_RUMBLE_TOP);
        Self {
            floor: coefficient(ROAR_FLOOR),
            presence: coefficient(PRESENCE_EDGE),
            edge: coefficient(SOLID_EDGE),
            hull_top: coefficient(HULL_TOP),
            hull_floor: coefficient(HULL_FLOOR),
            wind_rumble,
            wind_rumble_gain: unity(wind_rumble),
        }
    }
}

/// How much of each bed is heard from `far`, 0..1: near to distant, then distant to very far.
/// Unrelated noises add by power, so each fade keeps the sum of the squares.
fn bed_weights(far: f32) -> [f32; 3] {
    let far = far.clamp(0.0, 1.0);
    let mut weights = [0.0; 3];
    if far < 0.5 {
        let (rise, fall) = (far * PI).sin_cos();
        weights[CLOSE] = fall;
        weights[DISTANT] = rise;
    } else {
        let (rise, fall) = ((far - 0.5) * PI).sin_cos();
        weights[DISTANT] = fall;
        weights[VERY_FAR] = rise;
    }
    weights
}

/// Makes the sound: stereo samples without end.
pub struct Synth {
    mix: Arc<Mix>,
    fixed: Fixed,
    channels: [Channel; 2],
    /// Noise both ears share, and the dice.
    shared: Noise,
    shake: Surge,
    /// The recordings, once there are any, the beds playing and the hull's own play head.
    bank: Option<Arc<Bank>>,
    beds: [Looper; 3],
    hull: Looper,
    tail: Option<Tail>,
    /// The levels as they stand, following the mix, and how fast they rise and fall a look.
    levels: [f32; VOICES.len()],
    follows: [(f32, f32); VOICES.len()],
    until_control: u32,
    /// The filters as they stand.
    reach_c: f32,
    /// Frames of the alert tone left, and its phase.
    beep_left: u32,
    beep_phase: f32,
    /// The other channel's sample of this frame.
    right: Option<f32>,
}

impl Synth {
    fn new(mix: Arc<Mix>) -> Self {
        let per_look = |seconds: f32| {
            1.0 - (-(CONTROL_FRAMES as f32) / (RATE as f32 * seconds.max(1e-3))).exp()
        };
        Self {
            mix,
            fixed: Fixed::new(),
            channels: [Channel::new(0x9E37_79B9), Channel::new(0x7F4A_7C15)],
            shared: Noise(0x1B87_3593),
            shake: Surge::new(BUFFET_SHAKE),
            bank: None,
            beds: Default::default(),
            hull: Looper::default(),
            tail: None,
            levels: [0.0; VOICES.len()],
            follows: VOICES.map(|v| {
                let (rise, fall) = v.follows();
                (per_look(rise), per_look(fall))
            }),
            until_control: 0,
            reach_c: coefficient(REACH_LOWEST),
            beep_left: 0,
            beep_phase: 0.0,
            right: None,
        }
    }

    fn level(&self, voice: Voice) -> f32 {
        self.levels[voice as usize]
    }

    fn control(&mut self) {
        if self.bank.is_none() {
            self.bank = self.mix.bank.get().cloned();
        }
        // From silence the engines are being lit, and a take's own attack is what that sounds like.
        let lit = self.level(Voice::Roar) < SILENT && self.mix.get(Voice::Roar) >= SILENT;
        for (i, voice) in VOICES.into_iter().enumerate() {
            let target = self.mix.get(voice);
            let level = &mut self.levels[i];
            let (rise, fall) = self.follows[i];
            *level = if (lit || *level == 0.0) && voice.is_tone() {
                target
            } else {
                *level + if target > *level { rise } else { fall } * (target - *level)
            };
        }
        let rate = f64::from(self.level(Voice::RoarRate).max(0.25));
        if let Some(bank) = self.bank.as_deref() {
            if lit {
                for (looper, bed) in self.beds.iter_mut().zip(&bank.beds) {
                    looper.ignite(bed, rate, &mut self.shared);
                }
            }
            let cut = self.mix.take_cut();
            if cut > 0.0 {
                self.tail = bank.beds[CLOSE].tail(rate, TAIL_LEVEL * cut);
            }
        }
        self.reach_c = coefficient(self.level(Voice::RoarReach).max(1.0));
        let tone = self.level(Voice::WindTone).clamp(20.0, 4_000.0);
        for channel in &mut self.channels {
            channel.gale.tune(tone);
        }
        if self.mix.take_beep() {
            self.beep_left = (2.0 * BEEP_NOTE * RATE as f32) as u32;
        }
    }

    /// This frame of the recordings: the engines, what the hull hears of them.
    fn recorded(&mut self, rate: f64) -> ([f32; 2], [f32; 2]) {
        let (mut engines, mut through_hull) = ([0.0; 2], [0.0; 2]);
        let Some(bank) = self.bank.as_deref() else {
            return (engines, through_hull);
        };
        let roar = self.level(Voice::Roar);
        if roar > SILENT {
            let weights = bed_weights(self.level(Voice::RoarFar));
            for (i, weight) in weights.into_iter().enumerate() {
                if weight > SILENT {
                    let frame = self.beds[i].frame(&bank.beds[i], rate, &mut self.shared);
                    engines[0] += roar * weight * frame[0];
                    engines[1] += roar * weight * frame[1];
                }
            }
        }
        if let Some(tail) = &mut self.tail {
            match tail.frame(&bank.beds[CLOSE], rate) {
                Some(frame) => {
                    engines[0] += frame[0];
                    engines[1] += frame[1];
                }
                None => self.tail = None,
            }
        }
        if self.level(Voice::Hull) + self.level(Voice::Buffet) > SILENT {
            let bed = &bank.beds[VERY_FAR];
            through_hull = self.hull.frame(bed, HULL_RATE, &mut self.shared);
        }
        (engines, through_hull)
    }

    /// One frame of both channels.
    fn frame(&mut self) -> (f32, f32) {
        if self.until_control == 0 {
            self.control();
            self.until_control = CONTROL_FRAMES;
        }
        self.until_control -= 1;
        let (hull, wind) = (self.level(Voice::Hull), self.level(Voice::Wind));
        let (buffet, air) = (self.level(Voice::Buffet), self.level(Voice::Air));
        let (effects, master) = (self.level(Voice::Effects), self.level(Voice::Master));
        let solid = self.level(Voice::RoarSolid);
        let reach_c = self.reach_c;
        let rate = f64::from(self.level(Voice::RoarRate).max(0.25));

        let (engines, through_hull) = self.recorded(rate);
        let beep = self.beep_sample();
        // Everything both ears hear alike.
        let fixed = &self.fixed;
        let shared = self.shared.white();
        let moved = self.shake.tick(self.shared.white());
        let shake = (1.0 + self.shake.depth * moved).max(0.0);

        let mut out = [0.0f32; 2];
        for (k, (channel, out)) in self.channels.iter_mut().zip(&mut out).enumerate() {
            let own = channel.noise.white();
            // The engines: the air takes the highs, nothing under what a speaker plays, a little
            // more of what a small one can, and a solid's crackle lifted out of the recording.
            let reached = channel.reach.tick(reach_c, engines[k]);
            let clean = reached - channel.floor.tick(fixed.floor, reached);
            let above = clean - channel.presence.tick(fixed.presence, clean);
            let crackle = clean - channel.edge.tick(fixed.edge, clean);
            let mut sum = clean + PRESENCE_LIFT * above + solid * SOLID_LIFT * crackle;
            // The hull: the jet with only what comes through metal left of it, and it shakes
            // through Mach 1.
            let muffled = channel.hull.tick(fixed.hull_top, through_hull[k]);
            let rumble = muffled - channel.hull_floor.tick(fixed.hull_floor, muffled);
            sum += (hull * HULL_LEVEL + buffet * BUFFET_LEVEL * shake) * rumble;
            // Wind, partly the same in both ears so it stays round you and not inside your head.
            let (gust, flutter) = (channel.noise.white(), channel.noise.white());
            let blown = 0.45 * shared + 0.89 * own;
            sum += wind * WIND_RMS * channel.gale.tick(fixed, blown, gust, flutter);
            *out = ((air * sum + effects * beep) * master).tanh();
        }
        (out[0], out[1])
    }

    /// This frame of the alert tone: each note fades in and out so it does not click.
    fn beep_sample(&mut self) -> f32 {
        if self.beep_left == 0 {
            return 0.0;
        }
        self.beep_left -= 1;
        let note_frames = BEEP_NOTE * RATE as f32;
        let left = self.beep_left as f32 / note_frames;
        let note = BEEP_NOTES[usize::from(left < 1.0)];
        self.beep_phase = (self.beep_phase + note / RATE as f32).fract();
        let fade = (PI * left.fract()).sin().min(0.25) * 4.0;
        BEEP_LEVEL * fade * (TAU * self.beep_phase).sin()
    }
}

impl Iterator for Synth {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if let Some(right) = self.right.take() {
            return Some(right);
        }
        let (left, right) = self.frame();
        self.right = Some(right);
        Some(left)
    }
}

impl Source for Synth {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        NonZero::new(2).expect("two is not zero")
    }

    fn sample_rate(&self) -> SampleRate {
        NonZero::new(RATE).expect("the rate is not zero")
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recordings, decoded once for all the tests.
    fn bank() -> Arc<Bank> {
        static BANK: OnceLock<Arc<Bank>> = OnceLock::new();
        BANK.get_or_init(|| Arc::new(Bank::load(&content::asset_root().join("core/sounds"))))
            .clone()
    }

    /// A mix with the master up and everything at a sensible setting.
    fn fresh_mix() -> Arc<Mix> {
        let mix = Arc::new(Mix::default());
        let _ = mix.bank.set(bank());
        mix.set(Voice::Master, 1.0);
        mix.set(Voice::Effects, 1.0);
        mix.set(Voice::Air, 1.0);
        mix.set(Voice::RoarRate, 1.0);
        mix.set(Voice::RoarReach, 12_000.0);
        mix.set(Voice::WindTone, 800.0);
        mix
    }

    /// Both channels after the levels have settled and any attack is over.
    fn play_stereo(set: impl Fn(&Mix), seconds: f32) -> Vec<[f32; 2]> {
        let mix = fresh_mix();
        set(&mix);
        let mut synth = Synth::new(mix);
        for _ in 0..(5 * RATE) {
            synth.frame();
        }
        (0..(seconds * RATE as f32) as usize)
            .map(|_| {
                let (left, right) = synth.frame();
                [left, right]
            })
            .collect()
    }

    /// One channel, the same way.
    fn play(set: impl Fn(&Mix), seconds: f32) -> Vec<f32> {
        play_stereo(set, seconds).iter().map(|f| f[0]).collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// The share of the power above `hz`.
    fn share_above(samples: &[f32], hz: f32) -> f32 {
        let c = coefficient(hz);
        let mut low = Pole::default();
        let high: Vec<f32> = samples.iter().map(|&s| s - low.tick(c, s)).collect();
        (rms(&high) / rms(samples).max(1e-9)).powi(2)
    }

    /// The share of the power well above `hz`, with hardly anything leaking through from the deep
    /// rumble that holds most of a roar's power.
    fn share_steeply_above(samples: &[f32], hz: f32) -> f32 {
        let c = coefficient(hz);
        let mut high = samples.to_vec();
        for _ in 0..3 {
            let mut low = Pole::default();
            for s in &mut high {
                *s -= low.tick(c, *s);
            }
        }
        (rms(&high) / rms(samples).max(1e-9)).powi(2)
    }

    /// The difference between the loudest and quietest of a sound's short stretches, dB.
    fn swing_db(samples: &[f32], stretch_seconds: f32) -> f32 {
        let mut levels: Vec<f32> = samples
            .chunks((stretch_seconds * RATE as f32) as usize)
            .map(rms)
            .collect();
        levels.sort_by(f32::total_cmp);
        let (low, high) = (levels[levels.len() / 20], levels[levels.len() * 19 / 20]);
        20.0 * (high / low).log10()
    }

    /// About how loud a sound is to the ear, as an RMS: little of the bass, more of the highs, as
    /// in the loudness measure of a broadcast (ITU BS.1770).
    fn loudness(samples: &[f32]) -> f32 {
        let (floor, shelf) = (coefficient(100.0), coefficient(1_500.0));
        let (mut first, mut second, mut top) = (Pole::default(), Pole::default(), Pole::default());
        let weighted: Vec<f32> = samples
            .iter()
            .map(|&s| {
                let once = s - first.tick(floor, s);
                let twice = once - second.tick(floor, once);
                twice + 0.6 * (twice - top.tick(shelf, twice))
            })
            .collect();
        rms(&weighted)
    }

    /// Where the sound of `jet` from `distance` m is heard.
    fn roar_of(m: &Mix, jet: Jet, distance: f64, solid: f32) {
        let heard = amplitude(level(jet.acoustic_power(), distance, SEA_LEVEL));
        let area = std::f64::consts::PI * jet.diameter * jet.diameter / 4.0;
        m.set(Voice::Roar, heard);
        m.set(Voice::RoarRate, roar_rate(roar_peak(jet.exhaust, area)));
        m.set(Voice::RoarFar, roar_far(distance, SEA_LEVEL));
        m.set(Voice::RoarReach, roar_reach(distance));
        m.set(Voice::RoarSolid, solid);
    }

    const SMALL_ROCKET: Jet = Jet {
        thrust: 900e3,
        exhaust: 3_000.0,
        diameter: 1.3,
    };
    const SOLIDS: Jet = Jet {
        thrust: 10e6,
        exhaust: 2_400.0,
        diameter: 3.1,
    };
    const F1_CLUSTER: Jet = Jet {
        thrust: 34e6,
        exhaust: 2_600.0,
        diameter: 8.3,
    };

    #[test]
    fn nothing_going_on_is_silence() {
        assert!(rms(&play(|_| {}, 0.2)) < 1e-6);
    }

    #[test]
    fn a_full_roar_is_loud_never_clips_and_the_master_turns_it_down() {
        let full = play(|m| m.set(Voice::Roar, 1.0), 4.0);
        assert!((0.12..0.4).contains(&rms(&full)), "{}", rms(&full));
        assert!(full.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        let quiet = play(
            |m| {
                m.set(Voice::Roar, 1.0);
                m.set(Voice::Master, 0.1);
            },
            4.0,
        );
        assert!(rms(&quiet) < 0.25 * rms(&full));
    }

    #[test]
    fn rockets_of_every_size_roar_and_none_of_them_hiss() {
        for (jet, solid) in [(F1_CLUSTER, 0.0), (SMALL_ROCKET, 0.0), (SOLIDS, 1.0)] {
            let samples = play(|m| roar_of(m, jet, 100.0, solid), 6.0);
            let above = share_above(&samples, 2_000.0);
            assert!(above < 0.1, "{jet:?}: {above} of its power is over 2 kHz");
        }
    }

    #[test]
    fn a_big_rocket_is_deeper_than_a_small_one_but_small_speakers_still_get_it() {
        let big = play(|m| roar_of(m, F1_CLUSTER, 100.0, 0.0), 12.0);
        let small = play(|m| roar_of(m, SMALL_ROCKET, 100.0, 0.0), 12.0);
        assert!(
            share_above(&small, 300.0) > 1.15 * share_above(&big, 300.0),
            "{} {}",
            share_above(&small, 300.0),
            share_above(&big, 300.0)
        );
        // A laptop plays almost nothing under about 200 Hz.
        let audible = share_above(&big, 200.0);
        assert!(audible > 0.1, "{audible} of a Saturn V is over 200 Hz");
    }

    #[test]
    fn a_roar_moves_instead_of_holding_steady() {
        let samples = play(|m| m.set(Voice::Roar, 1.0), 8.0);
        // The recordings themselves move about 4 dB, measured the same way.
        let swing = swing_db(&samples, 0.1);
        assert!(swing > 2.5, "only {swing} dB of swing");
    }

    #[test]
    fn the_stretches_fade_into_each_other_without_a_dip_or_a_click() {
        // A minute is several fades, between takes as well as inside them.
        let samples = play(|m| m.set(Voice::Roar, 1.0), 60.0);
        let swing = swing_db(&samples, 2.0);
        assert!(swing < 5.0, "{swing} dB between two-second stretches");
        let jump = samples
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(jump < 0.2, "a jump of {jump} between two samples");
    }

    #[test]
    fn a_solid_is_brighter_than_a_liquid() {
        let at = |solid: f32| {
            play(
                |m| {
                    m.set(Voice::Roar, 1.0);
                    m.set(Voice::RoarSolid, solid);
                },
                6.0,
            )
        };
        let (liquid, solid) = (at(0.0), at(1.0));
        assert!(
            share_above(&solid, 1_000.0) > 2.0 * share_above(&liquid, 1_000.0),
            "{} {}",
            share_above(&solid, 1_000.0),
            share_above(&liquid, 1_000.0)
        );
    }

    #[test]
    fn distance_and_thin_air_sound_further_away() {
        assert_eq!(roar_far(100.0, SEA_LEVEL), 0.0);
        assert!(roar_far(2_000.0, SEA_LEVEL) > 0.3);
        assert_eq!(roar_far(30_000.0, SEA_LEVEL), 1.0);
        // The same camera distance, up where a tenth of the air is left, and further up.
        assert!(roar_far(100.0, SEA_LEVEL / 10.0) > 0.3);
        assert_eq!(roar_far(100.0, 1.0), 1.0);
        let close = bed_weights(0.0);
        let far = bed_weights(1.0);
        assert!((close[CLOSE] - 1.0).abs() < 1e-3 && (far[VERY_FAR] - 1.0).abs() < 1e-3);
        for at in 0..=20 {
            let w = bed_weights(at as f32 / 20.0);
            let power: f32 = w.iter().map(|x| x * x).sum();
            assert!((power - 1.0).abs() < 1e-3);
        }
        // From further away it really is darker, once the air has had its share.
        let hear = |distance: f64| play(|m| roar_of(m, F1_CLUSTER, distance, 0.0), 8.0);
        let (close, away) = (hear(150.0), hear(15_000.0));
        assert!(
            share_steeply_above(&close, 1_500.0) > 3.0 * share_steeply_above(&away, 1_500.0),
            "{} {}",
            share_steeply_above(&close, 1_500.0),
            share_steeply_above(&away, 1_500.0)
        );
    }

    /// Stay silent a moment, set something, then one channel for each of `seconds` in a row.
    fn after(trigger: impl Fn(&Mix), seconds: &[f32]) -> Vec<Vec<f32>> {
        let mix = fresh_mix();
        let mut synth = Synth::new(mix.clone());
        for _ in 0..(2 * RATE) {
            synth.frame();
        }
        trigger(&mix);
        (seconds.iter())
            .map(|&s| {
                (0..(s * RATE as f32) as usize)
                    .map(|_| synth.frame().0)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn lighting_a_big_rocket_builds_up_through_the_recordings_own_attack() {
        let parts = after(
            |m| {
                m.set(Voice::Roar, 1.0);
                m.set(Voice::RoarRate, 0.8);
            },
            &[0.5, 3.5, 4.0],
        );
        assert!(
            rms(&parts[0]) < 0.4 * rms(&parts[2]),
            "{} {}",
            rms(&parts[0]),
            rms(&parts[2])
        );
    }

    #[test]
    fn lighting_an_engine_sets_its_tone_at_once_and_does_not_sweep_there() {
        let mix = fresh_mix();
        mix.set(Voice::RoarReach, 200.0);
        mix.set(Voice::RoarRate, 1.0);
        let mut synth = Synth::new(mix.clone());
        for _ in 0..(RATE / 4) {
            synth.frame();
        }
        mix.set(Voice::RoarReach, 12_000.0);
        mix.set(Voice::RoarRate, 0.8);
        mix.set(Voice::Roar, 1.0);
        for _ in 0..(2 * CONTROL_FRAMES) {
            synth.frame();
        }
        assert_eq!(synth.level(Voice::RoarReach), 12_000.0);
        assert_eq!(synth.level(Voice::RoarRate), 0.8);
    }

    #[test]
    fn a_small_rocket_starts_sooner_than_a_big_one() {
        let start = |rate: f32| {
            let parts = after(
                move |m| {
                    m.set(Voice::Roar, 1.0);
                    m.set(Voice::RoarRate, rate);
                },
                &[1.0, 1.0, 4.0],
            );
            rms(&parts[0]) / rms(&parts[2])
        };
        assert!(
            start(1.2) > 1.3 * start(0.8),
            "{} {}",
            start(1.2),
            start(0.8)
        );
    }

    #[test]
    fn cutting_an_engine_plays_its_tail_and_then_nothing() {
        for (rate, long) in [(0.8, true), (1.2, false)] {
            let parts = after(
                |m| {
                    m.set(Voice::RoarRate, rate);
                    m.cut(1.0);
                },
                &[0.15, 1.0, 12.0],
            );
            let (start, middle, end) = (rms(&parts[0]), rms(&parts[1]), rms(&parts[2]));
            assert!(start > 0.08, "{start}");
            assert!(middle < 0.8 * start, "{middle} {start}");
            assert!(end < 0.3 * start, "{end} {start}");
            // A big engine rolls on where a small one has stopped.
            if !long {
                assert!(middle < 0.5 * start, "{middle} {start}");
            }
            let last = &parts[2][parts[2].len() - 44_100..];
            assert!(rms(last) < 1e-3, "{}", rms(last));
        }
    }

    /// The Pathfinder on the pad with its first engines lit and up to full thrust.
    fn lit_pathfinder() -> osp_sim::propulsion::Propulsion {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/core");
        let registry = osp_mods::load(&root, None).registry;
        let catalog = content::catalog(&registry, &mut Vec::new());
        let def = registry.get("craft", "pathfinder").unwrap();
        let mut rocket = osp_sim::propulsion::Propulsion::from_craft(&def, &catalog).unwrap();
        rocket.set_throttle(1.0);
        while !rocket.engines.iter().any(|e| e.running) {
            assert!(rocket.activate_next_sequence().is_some());
            rocket.burn(SEA_LEVEL, 0.02, true, &mut Vec::new());
        }
        burn(&mut rocket, 8.0);
        rocket
    }

    fn burn(rocket: &mut osp_sim::propulsion::Propulsion, seconds: f64) {
        for _ in 0..(seconds / 0.02) as usize {
            rocket.burn(SEA_LEVEL, 0.02, true, &mut Vec::new());
        }
    }

    #[test]
    fn the_throttle_of_a_real_engine_can_be_heard() {
        let mut rocket = lit_pathfinder();
        let heard = |r: &osp_sim::propulsion::Propulsion| {
            engines_heard(&r.engines, SEA_LEVEL, 100.0).unwrap()
        };
        let full = heard(&rocket);
        assert!(full.amplitude > 0.1, "{full:?}");
        assert_eq!(full.running, full.amplitude);
        assert!((SLOWEST..=FASTEST).contains(&full.rate));
        rocket.set_throttle(0.1);
        burn(&mut rocket, 8.0);
        let low = heard(&rocket);
        // It may not go as low as it was asked to: what counts is where it got to.
        let open = rocket.engines.iter().find(|e| e.running).unwrap().level;
        assert!(open < 0.6, "{open}");
        let drop = 20.0 * f64::from(low.amplitude / full.amplitude).log10();
        let expected = 20.0 * (0.25 + THROTTLE_SWING) * open.log10();
        assert!((drop - expected).abs() < 1.0, "{drop} dB, not {expected}");
        assert!(drop < -4.0, "{drop} dB");
        assert!(low.rate < full.rate);
    }

    #[test]
    fn cutting_a_real_engine_stops_it_running_at_once_and_its_sound_dies_away() {
        let mut rocket = lit_pathfinder();
        let before = engines_heard(&rocket.engines, SEA_LEVEL, 100.0).unwrap();
        rocket.set_throttle(0.0);
        rocket.burn(SEA_LEVEL, 0.02, true, &mut Vec::new());
        // What tells the synthesiser to play a tail: nothing running, though it is still heard.
        let tailing = engines_heard(&rocket.engines, SEA_LEVEL, 100.0).unwrap();
        assert!(tailing.running < CUT_SHARE * before.running, "{tailing:?}");
        assert!(tailing.amplitude > 0.5 * before.amplitude, "{tailing:?}");
        assert!(rocket.engines.iter().all(|e| !e.running));
        burn(&mut rocket, 10.0);
        assert_eq!(engines_heard(&rocket.engines, SEA_LEVEL, 100.0), None);
    }

    #[test]
    fn separation_motors_are_part_of_the_separation_and_are_not_heard() {
        let mut rocket = lit_pathfinder();
        assert!(engines_heard(&rocket.engines, SEA_LEVEL, 100.0).is_some());
        for engine in &mut rocket.engines {
            engine.def.family = Some(osp_sim::propulsion::SEPARATION_MOTOR.into());
        }
        assert_eq!(engines_heard(&rocket.engines, SEA_LEVEL, 100.0), None);
    }

    #[test]
    fn the_air_carries_sound_down_to_a_thin_limit_and_no_further() {
        assert_eq!(air_carries(0.0), 0.0);
        assert_eq!(air_carries(AIR_SILENT), 0.0);
        assert_eq!(air_carries(AIR_FULL), 1.0);
        assert_eq!(air_carries(SEA_LEVEL), 1.0);
        // The ground on Mars: thin, and it can still be heard.
        assert!((0.3..0.9).contains(&air_carries(600.0)));
        let mut was = 0.0;
        for decade in 0..=60 {
            let carried = air_carries(10f64.powf(f64::from(decade) / 10.0));
            assert!(carried >= was);
            was = carried;
        }
    }

    #[test]
    fn a_real_engine_at_full_thrust_makes_no_sound_in_vacuum_or_very_thin_air() {
        let rocket = lit_pathfinder();
        for pressure in [0.0, 1e-3, 1.0, AIR_SILENT] {
            let heard = engines_heard(&rocket.engines, pressure, 100.0).unwrap();
            assert_eq!(heard.amplitude, 0.0, "{pressure} Pa");
            assert_eq!(heard.running, 0.0, "{pressure} Pa");
        }
        let thick = engines_heard(&rocket.engines, 20_000.0, 100.0).unwrap();
        assert!(thick.amplitude > 0.1, "{thick:?}");
    }

    #[test]
    fn without_air_there_is_nothing_at_all_but_the_alert_tone() {
        let mix = fresh_mix();
        mix.set(Voice::Air, 0.0);
        for voice in [Voice::Roar, Voice::Hull, Voice::Wind, Voice::Buffet] {
            mix.set(voice, 1.0);
        }
        mix.cut(1.0);
        let mut synth = Synth::new(mix.clone());
        for _ in 0..(4 * RATE) {
            assert_eq!(synth.frame(), (0.0, 0.0));
        }
        mix.beep();
        let tone: Vec<f32> = (0..(2.0 * BEEP_NOTE * RATE as f32) as usize)
            .map(|_| synth.frame().0)
            .collect();
        assert!(rms(&tone) > 0.03, "{}", rms(&tone));
    }

    #[test]
    fn a_whole_flight_goes_quiet_in_space_and_comes_back_with_the_air() {
        let flight = scripted_flight();
        let stretch = |from: f32, to: f32| {
            let frames = &flight[(from * RATE as f32) as usize..(to * RATE as f32) as usize];
            frames.iter().map(|f| f[0]).collect::<Vec<f32>>()
        };
        // Under the last bit of 16-bit sound, before and after the alert tone.
        for (from, to) in [(35.0, 37.9), (38.5, 44.0)] {
            let peak = stretch(from, to).iter().fold(0.0f32, |a, s| a.max(s.abs()));
            assert!(peak < 3e-5, "{peak} from {from} s to {to} s");
        }
        assert!(rms(&stretch(5.0, 10.0)) > 0.05);
        assert!(rms(&stretch(38.0, 38.22)) > 0.02);
        assert!(rms(&stretch(52.0, 56.0)) > 0.02);
    }

    /// The strongest wind there is, on its own.
    fn full_wind(tone: f32, seconds: f32) -> Vec<f32> {
        play(
            |m| {
                m.set(Voice::Wind, 1.0);
                m.set(Voice::WindTone, tone);
            },
            seconds,
        )
    }

    #[test]
    fn wind_can_be_heard_beside_the_engines_but_does_not_drown_them() {
        let roar = loudness(&play(|m| roar_of(m, F1_CLUSTER, 150.0, 0.0), 12.0));
        let wind = loudness(&full_wind(1_100.0, 12.0));
        // Close to level with a launch, which is what the first attempt was not: under the roar
        // it could not be heard at all, and the second was twice as loud as the launch.
        assert!(wind > 0.5 * roar && wind < 1.2 * roar, "{wind} {roar}");
        assert!(full_wind(1_100.0, 4.0).iter().all(|s| s.abs() < 1.0));
        // A slow plane: a third of the strength, and low.
        let breeze = play(
            |m| {
                m.set(Voice::Wind, 0.3);
                m.set(Voice::WindTone, 300.0);
            },
            12.0,
        );
        assert!(rms(&breeze) > 0.015, "{}", rms(&breeze));
    }

    #[test]
    fn wind_is_a_rush_and_not_a_hiss_however_fast_it_blows() {
        // White noise through a band-pass was a hiss centred on 4 kHz.
        let wind = full_wind(1_100.0, 12.0);
        let above = share_above(&wind, 4_000.0);
        assert!(above < 0.15, "{above} of its power is over 4 kHz");
    }

    #[test]
    fn wind_gusts_move_its_level_and_its_pitch() {
        let wind = full_wind(600.0, 30.0);
        let swing = swing_db(&wind, 0.5);
        assert!(swing > 5.0, "only {swing} dB of swing");
        // Half-second stretches differ in how bright they are, not only how loud.
        let mut bright: Vec<f32> = wind
            .chunks(RATE as usize / 2)
            .map(|chunk| share_above(chunk, 700.0))
            .collect();
        bright.sort_by(f32::total_cmp);
        let (low, high) = (bright[bright.len() / 20], bright[bright.len() * 19 / 20]);
        assert!(high > 1.3 * low, "brightness only {low} to {high}");
    }

    #[test]
    fn wind_is_different_in_each_ear_but_not_unrelated() {
        let wind = play_stereo(|m| m.set(Voice::Wind, 1.0), 8.0);
        let (mut both, mut left, mut right) = (0.0, 0.0, 0.0);
        for frame in &wind {
            both += frame[0] * frame[1];
            left += frame[0] * frame[0];
            right += frame[1] * frame[1];
        }
        let correlation = both / (left * right).sqrt();
        assert!((0.02..0.6).contains(&correlation), "{correlation}");
    }

    #[test]
    fn a_faster_wind_is_a_higher_one() {
        let (fast, slow) = (full_wind(1_000.0, 12.0), full_wind(200.0, 12.0));
        assert!(share_above(&fast, 700.0) > 1.5 * share_above(&slow, 700.0));
    }

    #[test]
    fn wind_is_brighter_than_the_hull_and_both_hold_back() {
        let hull = play(|m| m.set(Voice::Hull, 1.0), 4.0);
        let wind = play(|m| m.set(Voice::Wind, 1.0), 4.0);
        for samples in [&hull, &wind] {
            assert!((0.02..0.35).contains(&rms(samples)), "{}", rms(samples));
        }
        assert!(share_above(&wind, 400.0) > 2.0 * share_above(&hull, 400.0));
    }

    #[test]
    fn the_hull_can_be_heard_on_small_speakers() {
        let hull = play(|m| m.set(Voice::Hull, 1.0), 6.0);
        assert!(
            share_above(&hull, 150.0) > 0.15,
            "{}",
            share_above(&hull, 150.0)
        );
    }

    #[test]
    fn the_alert_tone_sounds_once_and_is_two_quiet_notes() {
        let parts = after(|m| m.beep(), &[BEEP_NOTE, BEEP_NOTE, 0.2]);
        let hz = |samples: &[f32]| {
            let count = samples.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
            count as f32 * RATE as f32 / samples.len() as f32 / 2.0
        };
        assert!(
            (hz(&parts[0]) - BEEP_NOTES[0]).abs() < 60.0,
            "{}",
            hz(&parts[0])
        );
        assert!(
            (hz(&parts[1]) - BEEP_NOTES[1]).abs() < 60.0,
            "{}",
            hz(&parts[1])
        );
        assert!((0.03..0.12).contains(&rms(&parts[0])), "{}", rms(&parts[0]));
        assert!(rms(&parts[2]) < 1e-3, "{}", rms(&parts[2]));
    }

    fn write_wav(path: &std::path::Path, frames: &[[f32; 2]]) {
        let mut wav = Vec::new();
        let bytes = (frames.len() * 4) as u32;
        wav.extend(b"RIFF");
        wav.extend((36 + bytes).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(RATE.to_le_bytes());
        wav.extend((RATE * 4).to_le_bytes());
        wav.extend(4u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(bytes.to_le_bytes());
        for frame in frames {
            for s in frame {
                wav.extend(((s.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes());
            }
        }
        std::fs::write(path, wav).unwrap();
    }

    /// A launch, an engine cut, a coast in space with an alert going off and a fall back through
    /// the air, as the game would drive it.
    fn scripted_flight() -> Vec<[f32; 2]> {
        let mix = fresh_mix();
        mix.set(Voice::Master, 0.75);
        mix.set(Voice::Air, 0.0);
        let mut synth = Synth::new(mix.clone());
        let at = |t: f32, from: f32, to: f32| ((t - from) / (to - from)).clamp(0.0, 1.0);
        let (mut was_burning, mut beeped) = (false, false);
        let seconds = 62;
        (0..seconds * RATE)
            .map(|frame| {
                let t = frame as f32 / RATE as f32;
                if frame % CONTROL_FRAMES == 0 {
                    // How many tenths of the sea-level air are gone: up, then down again.
                    let thinned = 2.2 * at(t, 0.0, 26.0) + 2.8 * at(t, 26.0, 36.0);
                    let thinned = thinned.min(5.0 * (1.0 - at(t, 42.0, 60.0)));
                    let air = SEA_LEVEL * 10f64.powf(-f64::from(thinned));
                    mix.set(Voice::Air, air_carries(air));
                    let burning = (1.0..26.0).contains(&t);
                    if burning {
                        let distance = 120.0 + 12.0 * f64::from(t);
                        let heard = amplitude(level(F1_CLUSTER.acoustic_power(), distance, air));
                        let area = std::f64::consts::PI * 8.3 * 8.3 / 4.0;
                        mix.set(Voice::Roar, air_carries(air) * heard);
                        mix.set(Voice::RoarRate, roar_rate(roar_peak(2_600.0, area)));
                        mix.set(Voice::RoarFar, roar_far(distance, air));
                        mix.set(Voice::RoarReach, roar_reach(distance));
                    } else {
                        if was_burning {
                            mix.cut(mix.get(Voice::Roar));
                        }
                        mix.set(Voice::Roar, 0.0);
                    }
                    was_burning = burning;
                    let rise = at(t, 4.0, 13.0) * (1.0 - at(t, 15.0, 24.0));
                    let fall = at(t, 42.0, 52.0) * (1.0 - at(t, 57.0, 61.0));
                    let q = rise.max(fall);
                    mix.set(Voice::Wind, 0.9 * q.sqrt());
                    mix.set(Voice::WindTone, 250.0 + 800.0 * q);
                    let through_mach = at(t, 9.0, 11.0) * (1.0 - at(t, 13.0, 15.0));
                    mix.set(Voice::Buffet, 0.7 * through_mach);
                    let hull = if burning { 0.55 } else { 0.0 };
                    mix.set(Voice::Hull, hull);
                    if t >= 38.0 && !beeped {
                        mix.beep();
                        beeped = true;
                    }
                }
                let (left, right) = synth.frame();
                [left, right]
            })
            .collect()
    }

    /// `cargo test -p osp_game render_sound_samples -- --ignored`: scenes as WAV files in
    /// `target/sound`, to listen to without flying.
    #[test]
    #[ignore = "writes files to listen to"]
    fn render_sound_samples() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sound");
        std::fs::create_dir_all(&dir).unwrap();
        let wind = |level: f32, tone: f32| {
            move |m: &Mix| {
                m.set(Voice::Wind, level);
                m.set(Voice::WindTone, tone);
            }
        };
        let scenes: [(&str, Box<dyn Fn(&Mix)>); 10] = [
            (
                "saturn_v_150m",
                Box::new(|m| roar_of(m, F1_CLUSTER, 150.0, 0.0)),
            ),
            (
                "small_rocket_60m",
                Box::new(|m| roar_of(m, SMALL_ROCKET, 60.0, 0.0)),
            ),
            (
                "solid_boosters_150m",
                Box::new(|m| roar_of(m, SOLIDS, 150.0, 1.0)),
            ),
            (
                "saturn_v_2km",
                Box::new(|m| roar_of(m, F1_CLUSTER, 2_000.0, 0.0)),
            ),
            (
                "saturn_v_15km",
                Box::new(|m| roar_of(m, F1_CLUSTER, 15_000.0, 0.0)),
            ),
            (
                "max_q",
                Box::new(|m| {
                    roar_of(m, F1_CLUSTER, 400.0, 0.0);
                    m.set(Voice::Wind, 0.75);
                    m.set(Voice::WindTone, 900.0);
                    m.set(Voice::Buffet, 0.6);
                    m.set(Voice::Hull, 0.3);
                }),
            ),
            ("wind_plane", Box::new(wind(0.3, 300.0))),
            ("wind_transonic", Box::new(wind(0.7, 700.0))),
            ("wind_reentry", Box::new(wind(1.0, 1_100.0))),
            ("hull_under_thrust", Box::new(|m| m.set(Voice::Hull, 0.45))),
        ];
        for (name, set) in &scenes {
            let frames = play_stereo(|m| set(m), 12.0);
            write_wav(&dir.join(format!("{name}.wav")), &frames);
        }
        write_wav(&dir.join("flight.wav"), &scripted_flight());
    }

    #[test]
    fn a_saturn_v_at_a_hundred_metres_is_full_scale_and_a_long_way_off_it_is_not() {
        let power = F1_CLUSTER.acoustic_power();
        let near = level(power, 100.0, SEA_LEVEL);
        assert!((145.0..160.0).contains(&near), "{near} dB");
        assert!(amplitude(near) > 0.7);
        let far = level(power, 10_000.0, SEA_LEVEL);
        assert!((near - far - 40.0).abs() < 1e-6);
        assert!((0.05..0.4).contains(&amplitude(far)), "{}", amplitude(far));
    }

    #[test]
    fn a_small_engine_is_quieter_and_vacuum_carries_nothing() {
        let small = Jet {
            thrust: 20e3,
            exhaust: 2_000.0,
            diameter: 0.3,
        };
        let big = amplitude(level(F1_CLUSTER.acoustic_power(), 100.0, SEA_LEVEL));
        let little = amplitude(level(small.acoustic_power(), 100.0, SEA_LEVEL));
        assert!(little < 0.3 * big, "{little} {big}");
        assert!(amplitude(level(F1_CLUSTER.acoustic_power(), 100.0, 0.0)) < 1e-4);
        // At 30 km the air is a hundredth as thick: 20 dB down, an amplitude a third.
        let high = amplitude(level(F1_CLUSTER.acoustic_power(), 400.0, SEA_LEVEL / 100.0));
        let low = amplitude(level(F1_CLUSTER.acoustic_power(), 400.0, SEA_LEVEL));
        assert!((0.25..0.4).contains(&(high / low)), "{}", high / low);
    }

    #[test]
    fn a_first_stage_of_ordinary_size_plays_about_as_recorded() {
        // Nine engines of about a metre: the Pathfinder, or a Falcon 9.
        let area = 9.0 * std::f64::consts::PI * 0.92 * 0.92 / 4.0;
        let rate = roar_rate(roar_peak(2_800.0, area));
        assert!((0.9..1.15).contains(&rate), "{rate}");
    }

    #[test]
    fn big_jets_play_slower_and_distance_takes_the_highs_out() {
        let area = |d: f64| std::f64::consts::PI * d * d / 4.0;
        let big = roar_rate(roar_peak(2_600.0, area(8.3)));
        let small = roar_rate(roar_peak(2_000.0, area(0.9)));
        assert!(small > 1.3 * big, "{small} {big}");
        assert!((SLOWEST..=FASTEST).contains(&big));
        assert!((SLOWEST..=FASTEST).contains(&small));
        assert!(roar_reach(5_000.0) < 0.5 * roar_reach(100.0));
        assert_eq!(roar_reach(1e9), REACH_LOWEST);
    }
}
