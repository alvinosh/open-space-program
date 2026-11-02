//! What a flight sounds like. There are no sound files: everything is synthesised as it plays,
//! from what the simulation says is happening.
//!
//! - Engines roar through the air. A jet turns about half a percent of its power into noise, so
//!   how loud it is follows from its thrust and exhaust speed, the distance to the camera and the
//!   air around it, and in vacuum there is nothing to carry it. How deep it is follows from the
//!   size of the jet: its noise peaks at a Strouhal number of about 0.2 and falls off above that,
//!   so a cluster of big engines rumbles lower than a small one, and the air takes the highs out
//!   over distance. It is driven into a soft clip, whose overtones are what small speakers can
//!   play of a rumble they cannot, and it surges as turbulence does instead of holding steady.
//!   Above the roar are the pops of the jet's shocks, most of them from a solid.
//! - The vessel being flown also shakes under its thrust: the rumble you would hear in the hull,
//!   which is all there is in vacuum.
//! - Wind follows the air's speed and dynamic pressure, with a buffeting through Mach 1.
//! - RCS thrusters hiss, and pyros, ignitions, landings and crashes thump.
//!
//! [`Mix`] is what the game tells the synthesiser (shared levels, read on the audio thread);
//! [`Synth`] makes the sound from it. A scripted capture stays silent. To listen without flying,
//! `cargo test -p osp_game render_sound_samples -- --ignored` writes scenes to `target/sound`.

use std::{
    f32::consts::{PI, TAU},
    num::NonZero,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use bevy::{
    audio::{AddAudioSource, ChannelCount, Decodable, SampleRate, Source},
    prelude::*,
};
use osp_sim::{
    constants::G0,
    vessel::{Vessel, VesselEventKind, VesselId},
};

use crate::{
    FrameSet,
    alerts::ActiveAlerts,
    camera::{CameraRig, MainCamera},
    frame::RenderFrame,
    menu::Menu,
    settings::{AudioSettings, InterfaceSettings},
    sim::{Sim, SimEventMessage},
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
/// How far away the map view listens from, m, and how much quieter it is on top.
const MAP_DISTANCE: f64 = 300.0;
const MAP_QUIET: f32 = 0.35;
/// A vessel's hull is heard from this close, m.
const HULL_REACH: f64 = 500.0;
/// The acceleration at which the hull shakes at its loudest, m/s².
const HULL_FULL: f64 = 30.0;
/// The dynamic pressure at which the wind is at its loudest, Pa.
const WIND_FULL: f64 = 40_000.0;
/// Where a jet's noise peaks, as a Strouhal number, and the lowest and highest that is let go, Hz:
/// lower than this no speaker plays it, higher and a small rocket hisses.
const STROUHAL: f64 = 0.2;
const PEAK_LOWEST: f32 = 55.0;
const PEAK_HIGHEST: f32 = 220.0;
/// How far above the peak the roar reaches, close up, and the least and most that can be, Hz.
const REACH_ABOVE_PEAK: f64 = 10.0;
const REACH_LOWEST: f32 = 200.0;
const REACH_HIGHEST: f32 = 5_000.0;
/// The distance over which the air takes the roar's reach down by half, m.
const ABSORPTION: f64 = 500.0;
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

/// How high a roar that peaks at `peak` Hz is still heard from `distance` m, Hz.
fn roar_reach(peak: f32, distance: f64) -> f32 {
    let reach = REACH_ABOVE_PEAK * f64::from(peak) / (1.0 + distance / ABSORPTION);
    (reach as f32).clamp(REACH_LOWEST, REACH_HIGHEST)
}

/// What is heard of a vessel's lit engines from `distance` m away.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Heard {
    amplitude: f32,
    peak: f32,
    reach: f32,
    /// How much of it is a solid's, 0..1.
    solid: f32,
}

fn engines_heard(vessel: &Vessel, distance: f64) -> Option<Heard> {
    let pressure = vessel.flight.static_pressure;
    let (mut power, mut area, mut flow, mut solid) = (0.0, 0.0, 0.0, 0.0);
    for engine in vessel.propulsion.engines.iter().filter(|e| e.level > 0.0) {
        let jet = Jet {
            thrust: engine.thrust,
            exhaust: engine.isp(pressure) * G0,
            diameter: engine.def.exit_diameter.max(0.2),
        };
        let noise = jet.acoustic_power();
        power += noise;
        area += std::f64::consts::PI * jet.diameter * jet.diameter / 4.0;
        flow += noise * jet.exhaust;
        if engine.is_solid() {
            solid += noise;
        }
    }
    (power > 0.0).then(|| {
        let peak = roar_peak(flow / power, area);
        Heard {
            amplitude: amplitude(level(power, distance, pressure)),
            peak,
            reach: roar_reach(peak, distance),
            solid: (solid / power) as f32,
        }
    })
}

/// What the game last told the synthesiser about the vessel being flown, to tell what changed.
#[derive(Default)]
struct Memory {
    flown: Option<(VesselId, usize, usize)>,
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
    mut events: MessageReader<SimEventMessage>,
    mut memory: Local<Memory>,
) {
    let mix = &mix.0;
    let system = &sim.universe.system;
    let map = if rig.map_view { MAP_QUIET } else { 1.0 };
    let (mut roar, mut crackle) = (0.0f32, 0.0f32);
    let (mut peak, mut reach) = (PEAK_LOWEST, REACH_LOWEST);
    let (mut hull, mut wind, mut wind_tone, mut buffet, mut hiss) = (0.0f32, 0.0, 0.0, 0.0, 0.0);
    let flown = sim.active_id();
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
        if let Some(heard) = engines_heard(vessel, distance) {
            // Noises add by power, and the louder says what it sounds like.
            if heard.amplitude >= roar {
                (peak, reach) = (heard.peak, heard.reach);
            }
            crackle = crackle.max(heard.amplitude * (0.2 + 0.8 * heard.solid));
            roar = (roar * roar + heard.amplitude * heard.amplitude).sqrt();
        }
        if vessel.id() != flown {
            continue;
        }
        let near = 1.0 / (1.0 + distance / HULL_REACH);
        if vessel.flight.mass > 0.0 {
            let shake = (vessel.flight.thrust / vessel.flight.mass / HULL_FULL).sqrt();
            hull = 0.5 * shake.min(1.0) as f32 * near as f32;
        }
        let pressure = (vessel.flight.dynamic_pressure / WIND_FULL).max(0.0);
        wind = 0.8 * pressure.sqrt().min(1.0) as f32 * near as f32;
        let speed = 340.0 * vessel.flight.mach;
        wind_tone = (120.0 + 0.8 * speed).min(1_100.0) as f32;
        let through = ((vessel.flight.mach - 1.0) / 0.25).powi(2);
        buffet = (-through).exp() as f32 * wind;
        if !vessel.propulsion.firing().is_empty() {
            hiss = 0.25 * near as f32;
        }
        let parts = vessel.propulsion.parts().parts().len();
        let lit = (vessel.propulsion.engines.iter())
            .filter(|e| e.level > 0.0)
            .count();
        if let Some((id, were_parts, were_lit)) = memory.flown
            && id == vessel.id()
        {
            if parts < were_parts {
                mix.bang(0.55 * near as f32);
            } else if lit > were_lit {
                mix.bang(0.35 * near as f32);
            }
        }
        memory.flown = Some((vessel.id(), parts, lit));
    }
    for message in events.read() {
        if message.0.vessel != flown {
            continue;
        }
        match message.0.event.kind {
            VesselEventKind::Crashed { .. } | VesselEventKind::Destroyed => mix.bang(1.0),
            VesselEventKind::Landed { speed } => mix.bang((0.1 * speed as f32).clamp(0.05, 0.6)),
            _ => {}
        }
    }
    let now = time.elapsed_secs_f64();
    let rested = memory.alert_tone.is_none_or(|last| now - last >= ALERT_GAP);
    if alerts.0.len() > memory.alerts && interface.alert_tone && rested {
        mix.beep();
        memory.alert_tone = Some(now);
    }
    memory.alerts = alerts.0.len();
    let share = |percent: u32| percent as f32 / 100.0;
    let engines = share(settings.engines) * map;
    let air = share(settings.wind) * map;
    let effects = share(settings.effects);
    let master = if sim.paused || menu.open {
        0.0
    } else {
        share(settings.master)
    };
    mix.set(Voice::Roar, roar * engines);
    mix.set(Voice::RoarPeak, peak);
    mix.set(Voice::RoarReach, reach);
    mix.set(Voice::Crackle, crackle * engines);
    mix.set(Voice::Hull, hull * engines);
    mix.set(Voice::Wind, wind * air);
    mix.set(Voice::WindTone, wind_tone.max(100.0));
    mix.set(Voice::Buffet, buffet * air);
    mix.set(Voice::Hiss, hiss * effects);
    mix.set(Voice::Effects, effects);
    mix.set(Voice::Master, master);
}

/// A level the synthesiser reads.
#[derive(Clone, Copy)]
enum Voice {
    Roar,
    RoarPeak,
    RoarReach,
    Crackle,
    Hull,
    Wind,
    WindTone,
    Buffet,
    Hiss,
    Effects,
    Master,
}

const VOICES: [Voice; 11] = [
    Voice::Roar,
    Voice::RoarPeak,
    Voice::RoarReach,
    Voice::Crackle,
    Voice::Hull,
    Voice::Wind,
    Voice::WindTone,
    Voice::Buffet,
    Voice::Hiss,
    Voice::Effects,
    Voice::Master,
];

impl Voice {
    /// Seconds it takes to rise and to fall: a jet builds and dies away, a thruster spits.
    fn follows(self) -> (f32, f32) {
        match self {
            Voice::Roar | Voice::Crackle => (0.3, 0.9),
            Voice::RoarPeak | Voice::RoarReach | Voice::WindTone => (0.5, 0.5),
            Voice::Hull | Voice::Wind | Voice::Buffet => (0.3, 0.5),
            Voice::Hiss => (0.02, 0.08),
            Voice::Effects | Voice::Master => (0.05, 0.05),
        }
    }

    /// A filter's setting stands where it is first told to, rather than sweeping there.
    fn is_tone(self) -> bool {
        matches!(self, Voice::RoarPeak | Voice::RoarReach | Voice::WindTone)
    }
}

/// What the game has the synthesiser play: levels, and thumps waiting to go off.
#[derive(Default)]
pub struct Mix {
    levels: [AtomicU32; VOICES.len()],
    bang: AtomicU32,
    beep: AtomicU32,
}

impl Mix {
    fn set(&self, voice: Voice, value: f32) {
        self.levels[voice as usize].store(value.to_bits(), Ordering::Relaxed);
    }

    fn get(&self, voice: Voice) -> f32 {
        f32::from_bits(self.levels[voice as usize].load(Ordering::Relaxed))
    }

    /// A thump of this strength, 0..1: the strongest of those since the synthesiser last looked.
    fn bang(&self, strength: f32) {
        let was = f32::from_bits(self.bang.load(Ordering::Relaxed));
        self.bang
            .store(was.max(strength).to_bits(), Ordering::Relaxed);
    }

    fn take_bang(&self) -> f32 {
        f32::from_bits(self.bang.swap(0, Ordering::Relaxed))
    }

    /// The alert tone, once.
    fn beep(&self) {
        self.beep.store(1, Ordering::Relaxed);
    }

    fn take_beep(&self) -> bool {
        self.beep.swap(0, Ordering::Relaxed) != 0
    }
}

const RATE: u32 = 44_100;
/// Frames between looks at the mix.
const CONTROL_FRAMES: u32 = 32;
/// How loud each voice is at a level of 1, RMS: a full roar sits about 20 dB under full scale, as
/// a sound that goes on for minutes should, which leaves room for what stands out from it.
const ROAR_RMS: f32 = 0.14;
const HULL_RMS: f32 = 0.12;
const WIND_RMS: f32 = 0.07;
const BUFFET_RMS: f32 = 0.08;
const HISS_RMS: f32 = 0.4;
/// How hard the rumbles are driven: the overtones of the soft clip are what small speakers play.
const ROAR_DRIVE: f32 = 1.6;
const HULL_DRIVE: f32 = 1.5;
/// Highest and lowest turbulence, Hz, and how far each moves the roar about, as shares of it.
const SURGES: [(f32, f32); 3] = [(0.6, 0.17), (2.7, 0.19), (11.0, 0.12)];
/// Pops a second at full crackle. A pop goes through these, Hz: it is the difference of two
/// low-passes, a sharp push and its slow recovery, as a shock front is.
const CRACKLE_RATE: f32 = 45.0;
const POP_EDGE: f32 = 3_000.0;
const POP_RECOVERY: f32 = 450.0;
/// How strong a pop is against the rest, before the crackle's own level.
const POP_LEVEL: f32 = 1.9;
/// A thump's note glides down from the first to the second, Hz, as its body rings below this.
const BANG_NOTE: [f32; 2] = [120.0, 42.0];
const BANG_BODY: f32 = 320.0;
const BANG_LEVEL: f32 = 0.5;
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

/// One channel's filters. The lows are mostly the same in both ears and the highs differ, so the
/// sound is wide and the rumble stays in the middle.
#[derive(Clone, Copy)]
struct Channel {
    noise: Noise,
    roar: Pole,
    roar_reach: Pole,
    roar_floor: Pole,
    hull: Pole,
    hull_second: Pole,
    wind_low: f32,
    wind_band: f32,
    buffet: Pole,
    hiss_floor: Pole,
    hiss_top: Pole,
    hiss_top_second: Pole,
    pop_edge: Pole,
    pop_recovery: Pole,
    bang: Pole,
}

impl Channel {
    fn new(seed: u32) -> Self {
        Self {
            noise: Noise(seed),
            roar: Pole::default(),
            roar_reach: Pole::default(),
            roar_floor: Pole::default(),
            hull: Pole::default(),
            hull_second: Pole::default(),
            wind_low: 0.0,
            wind_band: 0.0,
            buffet: Pole::default(),
            hiss_floor: Pole::default(),
            hiss_top: Pole::default(),
            hiss_top_second: Pole::default(),
            pop_edge: Pole::default(),
            pop_recovery: Pole::default(),
            bang: Pole::default(),
        }
    }
}

/// Turbulence: slow noise with an RMS of 1.
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

/// The filters that do not move, worked out once: this runs on the audio thread.
struct Fixed {
    hull: f32,
    hull_gain: f32,
    hull_overtones: f32,
    buffet: f32,
    buffet_gain: f32,
    hiss_floor: f32,
    hiss_top: f32,
    pop_edge: f32,
    pop_recovery: f32,
    roar_floor: f32,
    bang_body: f32,
    bang_gain: f32,
}

impl Fixed {
    fn new() -> Self {
        let (hull, buffet) = (coefficient(70.0), coefficient(40.0));
        let bang_body = coefficient(BANG_BODY);
        Self {
            hull,
            hull_gain: unity(hull),
            hull_overtones: coefficient(420.0),
            buffet,
            buffet_gain: unity(buffet),
            hiss_floor: coefficient(900.0),
            hiss_top: coefficient(3_000.0),
            pop_edge: coefficient(POP_EDGE),
            pop_recovery: coefficient(POP_RECOVERY),
            roar_floor: coefficient(28.0),
            bang_body,
            bang_gain: unity(bang_body),
        }
    }
}

/// Makes the sound: stereo samples without end.
pub struct Synth {
    mix: Arc<Mix>,
    fixed: Fixed,
    channels: [Channel; 2],
    /// Noise both ears share, and what it drives.
    shared: Noise,
    surges: [Surge; 3],
    gust: Surge,
    shake: Surge,
    /// The levels as they stand, following the mix, and how fast they rise and fall a look.
    levels: [f32; VOICES.len()],
    follows: [(f32, f32); VOICES.len()],
    until_control: u32,
    /// The filters as they stand.
    roar_c: f32,
    roar_gain: f32,
    reach_c: f32,
    wind_f: f32,
    wind_gain: f32,
    /// The chance of a pop this frame.
    pop_chance: f32,
    /// The thump ringing now: its level, what it is multiplied by each frame, and its note.
    bang: f32,
    bang_decay: f32,
    bang_phase: f32,
    bang_hz: f32,
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
            surges: SURGES.map(Surge::new),
            gust: Surge::new((0.4, 0.45)),
            shake: Surge::new((9.0, 0.6)),
            levels: [0.0; VOICES.len()],
            follows: VOICES.map(|v| {
                let (rise, fall) = v.follows();
                (per_look(rise), per_look(fall))
            }),
            until_control: 0,
            roar_c: coefficient(PEAK_LOWEST),
            roar_gain: 1.0,
            reach_c: coefficient(REACH_LOWEST),
            wind_f: 0.1,
            wind_gain: 1.0,
            pop_chance: 0.0,
            bang: 0.0,
            bang_decay: 0.0,
            bang_phase: 0.0,
            bang_hz: BANG_NOTE[1],
            beep_left: 0,
            beep_phase: 0.0,
            right: None,
        }
    }

    fn level(&self, voice: Voice) -> f32 {
        self.levels[voice as usize]
    }

    fn control(&mut self) {
        for (i, voice) in VOICES.into_iter().enumerate() {
            let target = self.mix.get(voice);
            let level = &mut self.levels[i];
            let (rise, fall) = self.follows[i];
            *level = if *level == 0.0 && voice.is_tone() {
                target
            } else {
                *level + if target > *level { rise } else { fall } * (target - *level)
            };
        }
        self.roar_c = coefficient(self.level(Voice::RoarPeak).max(1.0));
        self.roar_gain = unity(self.roar_c);
        self.reach_c = coefficient(self.level(Voice::RoarReach).max(1.0));
        let wind = self.level(Voice::WindTone).clamp(20.0, 4_000.0);
        self.wind_f = 2.0 * (PI * wind / RATE as f32).sin();
        // A band of noise as wide as its tone holds about as much as the tone is high.
        self.wind_gain = 1.0 / (WHITE_RMS * self.wind_f.max(1e-4).sqrt());
        self.pop_chance = CRACKLE_RATE * self.level(Voice::Crackle).min(1.0).sqrt() / RATE as f32;
        if self.mix.take_beep() {
            self.beep_left = (2.0 * BEEP_NOTE * RATE as f32) as u32;
        }
        let bang = self.mix.take_bang();
        if bang > 0.0 && bang >= self.bang {
            // The harder it is, the longer it rings.
            self.bang = bang;
            self.bang_hz = BANG_NOTE[0];
            let ring = 0.1 + 0.7 * bang;
            self.bang_decay = (-1.0 / (ring * RATE as f32)).exp();
        }
    }

    /// One frame of both channels.
    fn frame(&mut self) -> (f32, f32) {
        if self.until_control == 0 {
            self.control();
            self.until_control = CONTROL_FRAMES;
        }
        self.until_control -= 1;
        let (roar, crackle) = (self.level(Voice::Roar), self.level(Voice::Crackle));
        let (hull, wind) = (self.level(Voice::Hull), self.level(Voice::Wind));
        let (buffet, hiss) = (self.level(Voice::Buffet), self.level(Voice::Hiss));
        let (effects, master) = (self.level(Voice::Effects), self.level(Voice::Master));
        let (roar_c, roar_gain, reach_c) = (self.roar_c, self.roar_gain, self.reach_c);
        let (wind_f, wind_gain) = (self.wind_f, self.wind_gain);

        let beep = self.beep_sample();
        // Everything both ears hear alike.
        let fixed = &self.fixed;
        let shared = self.shared.white();
        let mut surge = 1.0;
        for turbulence in &mut self.surges {
            let moved = turbulence.tick(self.shared.white());
            surge *= (1.0 + turbulence.depth * moved).max(0.1);
        }
        let gust = (1.0 + self.gust.depth * self.gust.tick(self.shared.white())).max(0.1);
        let shake = (1.0 + self.shake.depth * self.shake.tick(self.shared.white())).max(0.0);
        // Pops come in bursts as the turbulence surges.
        let pop_chance = self.pop_chance * (surge * surge * surge).min(4.0);
        let pop = (self.shared.share() < pop_chance).then(|| {
            let size = self.shared.share();
            (size * size * size, self.shared.share())
        });
        // The thump's note glides down as it rings.
        let bang_note = (TAU * self.bang_phase).sin();
        self.bang_phase = (self.bang_phase + self.bang_hz / RATE as f32).fract();
        self.bang_hz += (BANG_NOTE[1] - self.bang_hz) * 0.0004;
        let bang = self.bang;
        self.bang *= self.bang_decay;

        let mut out = [0.0f32; 2];
        for (k, (channel, out)) in self.channels.iter_mut().zip(&mut out).enumerate() {
            let own = channel.noise.white();
            let low = 0.85 * shared + 0.53 * own;
            // The roar: falling off above its peak, up to where it reaches, with nothing under
            // what a speaker plays, then driven for the overtones.
            let body = roar_gain * channel.roar.tick(roar_c, low);
            let reached = channel.roar_reach.tick(reach_c, body);
            let clean = reached - channel.roar_floor.tick(fixed.roar_floor, reached);
            let driven = (ROAR_DRIVE * clean).tanh() / ROAR_DRIVE.tanh();
            let mut sum = roar * ROAR_RMS * surge * driven;
            // Pops: rare, most of them small, a sharp push and a slow recovery.
            if let Some((size, pan)) = pop {
                let share = if k == 0 { 1.0 - pan } else { pan };
                channel.pop_edge.0 += POP_LEVEL * size * share.sqrt();
            }
            let front = channel.pop_edge.tick(fixed.pop_edge, 0.0);
            let recovery = channel.pop_recovery.tick(fixed.pop_recovery, front);
            sum += crackle.min(1.0) * (front - recovery);
            // The hull: a rumble, driven for overtones small speakers can play.
            let rumble = fixed.hull_gain * channel.hull.tick(fixed.hull, shared);
            let hull_driven = (HULL_DRIVE * rumble).tanh() / HULL_DRIVE.tanh();
            let overtones = channel.hull_second.tick(fixed.hull_overtones, hull_driven);
            sum += hull * HULL_RMS * overtones * surge.sqrt();
            // Wind: a broad band of noise round its tone, in gusts.
            let high = own - channel.wind_low - 1.2 * channel.wind_band;
            channel.wind_band += wind_f * high;
            channel.wind_low += wind_f * channel.wind_band;
            sum += wind * WIND_RMS * wind_gain * channel.wind_band * gust * WHITE_RMS;
            let buffeting = fixed.buffet_gain * channel.buffet.tick(fixed.buffet, low);
            sum += buffet * BUFFET_RMS * (2.0 * buffeting).tanh() * shake;
            // Hiss: a soft band in the middle of the ear's range, not the top of it.
            let top = channel.hiss_top.tick(fixed.hiss_top, own);
            let top = channel.hiss_top_second.tick(fixed.hiss_top, top);
            let hissing = top - channel.hiss_floor.tick(fixed.hiss_floor, top);
            sum += hiss * HISS_RMS * hissing / WHITE_RMS;
            // A thump: its note, and a burst of noise below its body's cutoff.
            let thud = fixed.bang_gain * channel.bang.tick(fixed.bang_body, low);
            sum += effects * bang * BANG_LEVEL * (0.8 * bang_note + 0.35 * thud);
            sum += effects * beep;
            *out = (sum * master).tanh();
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

    /// Samples of one channel after the levels have settled.
    fn play(set: impl Fn(&Mix), seconds: f32) -> Vec<f32> {
        let mix = Arc::new(Mix::default());
        mix.set(Voice::Master, 1.0);
        mix.set(Voice::Effects, 1.0);
        mix.set(Voice::RoarPeak, 120.0);
        mix.set(Voice::RoarReach, 1_200.0);
        mix.set(Voice::WindTone, 800.0);
        set(&mix);
        let mut synth = Synth::new(mix);
        for _ in 0..(3 * RATE) {
            synth.frame();
        }
        (0..(seconds * RATE as f32) as usize)
            .map(|_| synth.frame().0)
            .collect()
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

    /// Where the sound of `jet` from `distance` m is heard.
    fn roar_of(m: &Mix, jet: Jet, distance: f64, solid: f32) {
        let heard = amplitude(level(jet.acoustic_power(), distance, SEA_LEVEL));
        let area = std::f64::consts::PI * jet.diameter * jet.diameter / 4.0;
        let peak = roar_peak(jet.exhaust, area);
        m.set(Voice::Roar, heard);
        m.set(Voice::RoarPeak, peak);
        m.set(Voice::RoarReach, roar_reach(peak, distance));
        m.set(Voice::Crackle, heard * (0.2 + 0.8 * solid));
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

    #[test]
    fn nothing_going_on_is_silence() {
        assert!(rms(&play(|_| {}, 0.2)) < 1e-6);
    }

    #[test]
    fn a_full_roar_leaves_room_and_never_clips_and_the_master_turns_it_down() {
        let full = play(|m| m.set(Voice::Roar, 1.0), 2.0);
        // About 20 dB under full scale.
        assert!((0.06..0.16).contains(&rms(&full)), "{}", rms(&full));
        assert!(full.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        let quiet = play(
            |m| {
                m.set(Voice::Roar, 1.0);
                m.set(Voice::Master, 0.1);
            },
            2.0,
        );
        assert!(rms(&quiet) < 0.25 * rms(&full));
    }

    #[test]
    fn rockets_of_every_size_roar_and_none_of_them_hiss() {
        // This is what the first attempt got wrong: a small rocket and a solid had most of their
        // power above 1 kHz, which sounds like a detuned radio.
        for (jet, solid) in [(F1_CLUSTER, 0.0), (SMALL_ROCKET, 0.0), (SOLIDS, 1.0)] {
            let samples = play(|m| roar_of(m, jet, 100.0, solid), 4.0);
            let above = share_above(&samples, 2_000.0);
            assert!(above < 0.1, "{jet:?}: {above} of its power is over 2 kHz");
        }
    }

    #[test]
    fn a_big_rocket_is_deeper_than_a_small_one_but_small_speakers_still_get_it() {
        let big = play(|m| roar_of(m, F1_CLUSTER, 100.0, 0.0), 4.0);
        let small = play(|m| roar_of(m, SMALL_ROCKET, 100.0, 0.0), 4.0);
        assert!(share_above(&small, 300.0) > 1.5 * share_above(&big, 300.0));
        // A laptop plays almost nothing under about 200 Hz.
        let audible = share_above(&big, 200.0);
        assert!(
            audible > 0.15,
            "{audible} of a Saturn V's power is over 200 Hz"
        );
    }

    #[test]
    fn a_roar_surges_instead_of_holding_steady() {
        let samples = play(|m| m.set(Voice::Roar, 1.0), 8.0);
        let mut levels: Vec<f32> = samples.chunks((RATE / 10) as usize).map(rms).collect();
        levels.sort_by(f32::total_cmp);
        let (low, high) = (levels[levels.len() / 20], levels[levels.len() * 19 / 20]);
        let swing = 20.0 * (high / low).log10();
        assert!(swing > 5.0, "only {swing} dB of swing");
    }

    #[test]
    fn crackle_is_pops_pushing_outwards_and_not_a_hiss() {
        let samples = play(|m| m.set(Voice::Crackle, 1.0), 4.0);
        let peak = samples.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(peak > 5.0 * rms(&samples), "{peak} {}", rms(&samples));
        // A shock front is a sharp push and a slow recovery.
        let cube: f32 = samples.iter().map(|s| s * s * s).sum();
        assert!(cube > 0.0);
        assert!(share_above(&samples, 5_000.0) < 0.25);
    }

    #[test]
    fn wind_and_hiss_are_brighter_than_the_hull_and_hold_back() {
        let hull = play(|m| m.set(Voice::Hull, 1.0), 2.0);
        let wind = play(|m| m.set(Voice::Wind, 1.0), 2.0);
        let hiss = play(|m| m.set(Voice::Hiss, 1.0), 2.0);
        for (samples, under) in [(&hull, 0.2), (&wind, 0.2), (&hiss, 0.15)] {
            assert!((0.02..under).contains(&rms(samples)), "{}", rms(samples));
        }
        assert!(share_above(&wind, 400.0) > 2.0 * share_above(&hull, 400.0));
        assert!(share_above(&hiss, 1_500.0) > 0.4);
        assert!(
            share_above(&hiss, 6_000.0) < 0.1,
            "{}",
            share_above(&hiss, 6_000.0)
        );
    }

    #[test]
    fn the_hull_can_be_heard_on_small_speakers() {
        let hull = play(|m| m.set(Voice::Hull, 1.0), 2.0);
        assert!(
            share_above(&hull, 150.0) > 0.15,
            "{}",
            share_above(&hull, 150.0)
        );
    }

    /// One channel after a thump or a tone, for `seconds`, in a row.
    fn after(trigger: impl Fn(&Mix), seconds: &[f32]) -> Vec<Vec<f32>> {
        let mix = Arc::new(Mix::default());
        mix.set(Voice::Master, 1.0);
        mix.set(Voice::Effects, 1.0);
        let mut synth = Synth::new(mix.clone());
        for _ in 0..(RATE / 2) {
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
    fn a_bang_rings_out_and_goes_off_once() {
        let parts = after(
            |m| {
                m.bang(0.3);
                m.bang(1.0);
                m.bang(0.5);
            },
            &[0.2, 1.8, 4.0],
        );
        assert!(rms(&parts[0]) > 0.15, "{}", rms(&parts[0]));
        assert!(rms(&parts[1]) < rms(&parts[0]));
        assert!(rms(&parts[2]) < 0.1 * rms(&parts[0]), "{}", rms(&parts[2]));
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

    /// `cargo test -p osp_game render_sound_samples -- --ignored`: scenes as WAV files in
    /// `target/sound`, to listen to without flying.
    #[test]
    #[ignore = "writes files to listen to"]
    fn render_sound_samples() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sound");
        std::fs::create_dir_all(&dir).unwrap();
        let scenes: [(&str, Box<dyn Fn(&Mix)>); 7] = [
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
                "saturn_v_8km",
                Box::new(|m| roar_of(m, F1_CLUSTER, 8_000.0, 0.0)),
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
            ("vacuum_hull", Box::new(|m| m.set(Voice::Hull, 0.35))),
            ("rcs", Box::new(|m| m.set(Voice::Hiss, 0.25))),
        ];
        // A whole flight, as the game would drive it.
        let mix = Arc::new(Mix::default());
        mix.set(Voice::Master, 0.75);
        mix.set(Voice::Effects, 1.0);
        let mut synth = Synth::new(mix.clone());
        let mut flight = Vec::new();
        let at = |t: f32, from: f32, to: f32| ((t - from) / (to - from)).clamp(0.0, 1.0);
        let mut said = [false; 5];
        for frame in 0..(30 * RATE) {
            let t = frame as f32 / RATE as f32;
            for (done, (when, strength)) in said.iter_mut().zip([
                (1.0, 0.35),
                (14.0, 0.55),
                (15.0, 0.35),
                (24.0, 1.0),
                (0.0, 0.0),
            ]) {
                if t >= when && strength > 0.0 && !*done {
                    mix.bang(strength);
                    *done = true;
                }
            }
            if frame % CONTROL_FRAMES == 0 {
                let burning = (1.0..14.0).contains(&t);
                if burning {
                    roar_of(&mix, F1_CLUSTER, 150.0 + 200.0 * f64::from(t), 0.0);
                } else {
                    mix.set(Voice::Roar, 0.0);
                    mix.set(Voice::Crackle, 0.0);
                }
                let q = at(t, 3.0, 11.0) * (1.0 - at(t, 12.0, 15.0));
                mix.set(Voice::Wind, 0.8 * q);
                mix.set(Voice::WindTone, 300.0 + 700.0 * q);
                mix.set(
                    Voice::Buffet,
                    0.7 * at(t, 8.0, 10.0) * (1.0 - at(t, 11.0, 13.0)),
                );
                mix.set(
                    Voice::Hull,
                    if (1.0..23.0).contains(&t) { 0.4 } else { 0.0 },
                );
                let puff = (19.0..19.4).contains(&t) || (20.5..20.8).contains(&t);
                mix.set(Voice::Hiss, if puff { 0.25 } else { 0.0 });
                if (21.99..22.0).contains(&t) {
                    mix.beep();
                }
            }
            flight.push(synth.frame().0);
        }
        let rendered = scenes
            .iter()
            .map(|(name, set)| (name.to_string(), play(|m| set(m), 6.0)))
            .chain([("flight".to_owned(), flight)]);
        for (name, samples) in rendered {
            let mut wav = Vec::new();
            let bytes = (samples.len() * 2) as u32;
            wav.extend(b"RIFF");
            wav.extend((36 + bytes).to_le_bytes());
            wav.extend(b"WAVEfmt ");
            wav.extend(16u32.to_le_bytes());
            wav.extend(1u16.to_le_bytes());
            wav.extend(1u16.to_le_bytes());
            wav.extend(RATE.to_le_bytes());
            wav.extend((RATE * 2).to_le_bytes());
            wav.extend(2u16.to_le_bytes());
            wav.extend(16u16.to_le_bytes());
            wav.extend(b"data");
            wav.extend(bytes.to_le_bytes());
            for s in &samples {
                wav.extend(((s.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes());
            }
            std::fs::write(dir.join(format!("{name}.wav")), wav).unwrap();
        }
    }

    const F1_CLUSTER: Jet = Jet {
        thrust: 34e6,
        exhaust: 2_600.0,
        diameter: 8.3,
    };

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
    fn big_jets_rumble_lower_and_distance_takes_the_highs_out() {
        let area = |d: f64| std::f64::consts::PI * d * d / 4.0;
        let big = roar_peak(2_600.0, area(8.3));
        let small = roar_peak(2_000.0, area(0.9));
        assert!(small > 2.0 * big, "{small} {big}");
        assert!((PEAK_LOWEST..=PEAK_HIGHEST).contains(&big));
        assert!(roar_reach(big, 5_000.0) < 0.5 * roar_reach(big, 100.0));
        assert!((REACH_LOWEST..=REACH_HIGHEST).contains(&roar_reach(small, 100.0)));
    }
}
