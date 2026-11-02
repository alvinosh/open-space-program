//! What a flight sounds like. There are no sound files: everything is synthesised as it plays,
//! from what the simulation says is happening.
//!
//! - Engines roar through the air. A jet turns about half a percent of its power into noise, so
//!   how loud it is follows from its thrust and exhaust speed, the distance to the camera and the
//!   air around it, and in vacuum there is nothing to carry it. How deep it is follows from the
//!   size of the jet: its noise peaks at a Strouhal number of about 0.2, so a cluster of big
//!   engines rumbles far lower than a small one, and the air takes the highs out over distance.
//!   Above the roar is the crackle of the jet's shocks, strongest from a solid.
//! - The vessel being flown also shakes under its thrust: the rumble you would hear in the hull,
//!   which is all there is in vacuum.
//! - Wind follows the air's speed and dynamic pressure, with a buffeting through Mach 1.
//! - RCS thrusters hiss, and pyros, ignitions, landings and crashes thump.
//!
//! [`Mix`] is what the game tells the synthesiser (shared levels, read on the audio thread);
//! [`Synth`] makes the sound from it. A scripted capture stays silent.

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
/// The lowest and highest the roar's filter goes, Hz, and how far above the jet's peak it opens.
const ROAR_LOWEST: f32 = 90.0;
const ROAR_HIGHEST: f32 = 3_500.0;
const ROAR_ABOVE_PEAK: f64 = 6.0;
/// The distance over which the air takes the roar's highs down by half, m.
const ABSORPTION: f64 = 500.0;

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

/// Where the roar's filter stands, Hz, for jets of `area` m² of exit in all leaving at `exhaust`
/// m/s, heard from `distance` m.
fn roar_tone(exhaust: f64, area: f64, distance: f64) -> f32 {
    let diameter = (4.0 * area / std::f64::consts::PI).sqrt().max(0.05);
    let peak = 0.2 * exhaust / diameter;
    let tone = ROAR_ABOVE_PEAK * peak / (1.0 + distance / ABSORPTION);
    (tone as f32).clamp(ROAR_LOWEST, ROAR_HIGHEST)
}

/// What is heard from a vessel's lit engines `distance` m away: the amplitude of the roar, its
/// tone, and how much of it is a solid's.
fn engines_heard(vessel: &Vessel, distance: f64) -> (f32, f32, f32) {
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
    if power <= 0.0 {
        return (0.0, ROAR_LOWEST, 0.0);
    }
    let heard = amplitude(level(power, distance, pressure));
    (
        heard,
        roar_tone(flow / power, area, distance),
        (solid / power) as f32,
    )
}

/// What the game last told the synthesiser about the vessel being flown, to tell what changed.
#[derive(Default)]
struct Memory {
    flown: Option<(VesselId, usize, usize)>,
    alerts: usize,
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
    mix: Res<FlightMix>,
    mut events: MessageReader<SimEventMessage>,
    mut memory: Local<Memory>,
) {
    let mix = &mix.0;
    let system = &sim.universe.system;
    let map = if rig.map_view { MAP_QUIET } else { 1.0 };
    let (mut roar, mut tone, mut crackle) = (0.0f32, 0.0f32, 0.0f32);
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
        let (heard, jet_tone, solid) = engines_heard(vessel, distance);
        if heard > 0.0 {
            // Noises add by power; the deeper of two roars as loud is the one felt.
            let louder = (roar * roar + heard * heard).sqrt();
            tone = if heard >= roar {
                jet_tone
            } else {
                tone.max(jet_tone)
            };
            crackle = crackle.max(heard * (0.25 + 0.75 * solid));
            roar = louder;
        }
        if vessel.id() != flown {
            continue;
        }
        let near = 1.0 / (1.0 + distance / HULL_REACH);
        if vessel.flight.mass > 0.0 {
            let shake = (vessel.flight.thrust / vessel.flight.mass / HULL_FULL).sqrt();
            hull = 0.3 * shake.min(1.0) as f32 * near as f32;
        }
        let pressure = (vessel.flight.dynamic_pressure / WIND_FULL).max(0.0);
        wind = 0.8 * pressure.sqrt().min(1.0) as f32 * near as f32;
        let speed = 340.0 * vessel.flight.mach;
        wind_tone = (200.0 + 2.5 * speed).min(4_000.0) as f32;
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
    if alerts.0.len() > memory.alerts && interface.alert_tone {
        mix.beep();
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
    mix.set(Voice::RoarTone, tone.max(ROAR_LOWEST));
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
    RoarTone,
    Crackle,
    Hull,
    Wind,
    WindTone,
    Buffet,
    Hiss,
    Effects,
    Master,
}
const VOICES: usize = 10;

/// What the game has the synthesiser play: levels, and thumps waiting to go off.
#[derive(Default)]
pub struct Mix {
    levels: [AtomicU32; VOICES],
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
/// Seconds over which a level follows the mix, so that nothing clicks.
const FOLLOW: f32 = 0.05;
/// Pops a second of the jet's crackle, at full crackle, and how long one rings, seconds.
const CRACKLE_RATE: f32 = 90.0;
const CRACKLE_RING: f32 = 0.004;
/// The alert tone: two notes, Hz, each for this long, seconds, at this level.
const BEEP_NOTES: [f32; 2] = [880.0, 660.0];
const BEEP_NOTE: f32 = 0.13;
const BEEP_LEVEL: f32 = 0.3;

/// A two-pole low-pass filter.
#[derive(Default, Clone, Copy)]
struct Lowpass(f32, f32);

impl Lowpass {
    fn tick(&mut self, coefficient: f32, input: f32) -> f32 {
        self.0 += coefficient * (input - self.0);
        self.1 += coefficient * (self.0 - self.1);
        self.1
    }
}

/// The filter coefficient for a cutoff of `hz`.
fn coefficient(hz: f32) -> f32 {
    1.0 - (-TAU * hz / RATE as f32).exp()
}

/// What keeps a filtered voice about as loud whatever its cutoff.
fn loudness(coefficient: f32) -> f32 {
    (2.0 / coefficient.max(1e-5)).sqrt()
}

/// One channel's filters. The channels hear different noise, so the sound is wide.
#[derive(Default, Clone, Copy)]
struct Channel {
    noise: u32,
    roar: Lowpass,
    flutter: Lowpass,
    hull: Lowpass,
    wind_low: f32,
    wind_band: f32,
    gust: Lowpass,
    buffet: Lowpass,
    hiss: Lowpass,
    pop: f32,
}

impl Channel {
    /// White noise in -1..1 (xorshift).
    fn white(&mut self) -> f32 {
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        (self.noise as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Makes the sound: stereo samples without end.
pub struct Synth {
    mix: Arc<Mix>,
    channels: [Channel; 2],
    /// The levels as they stand, following the mix.
    levels: [f32; VOICES],
    follow: f32,
    until_control: u32,
    /// The roar's and the wind's filter as they stand.
    roar_c: f32,
    roar_gain: f32,
    wind_f: f32,
    /// The thump ringing now: its level, what it is multiplied by each frame, and its low note.
    bang: f32,
    bang_decay: f32,
    bang_phase: f32,
    /// Frames of the alert tone left, and its phase.
    beep_left: u32,
    beep_phase: f32,
    /// The other channel's sample of this frame.
    right: Option<f32>,
}

impl Synth {
    fn new(mix: Arc<Mix>) -> Self {
        let channel = |seed: u32| Channel {
            noise: seed,
            ..default()
        };
        Self {
            mix,
            channels: [channel(0x9E37_79B9), channel(0x7F4A_7C15)],
            levels: [0.0; VOICES],
            follow: 1.0 - (-(CONTROL_FRAMES as f32) / (RATE as f32 * FOLLOW)).exp(),
            until_control: 0,
            roar_c: coefficient(ROAR_LOWEST),
            roar_gain: 1.0,
            wind_f: 0.1,
            bang: 0.0,
            bang_decay: 0.0,
            bang_phase: 0.0,
            beep_left: 0,
            beep_phase: 0.0,
            right: None,
        }
    }

    fn control(&mut self) {
        for (i, voice) in [
            Voice::Roar,
            Voice::RoarTone,
            Voice::Crackle,
            Voice::Hull,
            Voice::Wind,
            Voice::WindTone,
            Voice::Buffet,
            Voice::Hiss,
            Voice::Effects,
            Voice::Master,
        ]
        .into_iter()
        .enumerate()
        {
            let target = self.mix.get(voice);
            // A filter that has not been told where to stand starts there.
            let level = &mut self.levels[i];
            *level = if *level == 0.0 && matches!(voice, Voice::RoarTone | Voice::WindTone) {
                target
            } else {
                *level + self.follow * (target - *level)
            };
        }
        self.roar_c = coefficient(self.levels[Voice::RoarTone as usize].max(1.0));
        self.roar_gain = loudness(self.roar_c);
        let wind = self.levels[Voice::WindTone as usize].clamp(20.0, 6_000.0);
        self.wind_f = 2.0 * (PI * wind / RATE as f32).sin();
        if self.mix.take_beep() {
            self.beep_left = (2.0 * BEEP_NOTE * RATE as f32) as u32;
        }
        let bang = self.mix.take_bang();
        if bang > 0.0 && bang >= self.bang {
            // The harder it is, the longer it rings.
            self.bang = bang;
            let ring = 0.12 + 0.9 * bang;
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
        let level = |voice: Voice| self.levels[voice as usize];
        let (roar, crackle, hull) = (
            level(Voice::Roar),
            level(Voice::Crackle),
            level(Voice::Hull),
        );
        let (wind, buffet, hiss) = (level(Voice::Wind), level(Voice::Buffet), level(Voice::Hiss));
        let (effects, master) = (level(Voice::Effects), level(Voice::Master));
        let (roar_c, roar_gain, wind_f) = (self.roar_c, self.roar_gain, self.wind_f);
        let flutter_c = coefficient(3.0);
        let hull_c = coefficient(85.0);
        let gust_c = coefficient(0.8);
        let buffet_c = coefficient(45.0);
        let hiss_c = coefficient(2_500.0);
        let pop_ring = (-1.0 / (CRACKLE_RING * RATE as f32)).exp();
        let pop_chance = CRACKLE_RATE * crackle.min(1.0) / RATE as f32;
        let bang_note = (TAU * self.bang_phase).sin();
        self.bang_phase = (self.bang_phase + 52.0 / RATE as f32).fract();
        let bang = self.bang;
        self.bang *= self.bang_decay;
        let beep = self.beep();
        let mut out = [0.0f32; 2];
        for (channel, out) in self.channels.iter_mut().zip(&mut out) {
            let white = channel.white();
            let flutter = 1.0 + 0.4 * loudness(flutter_c) * channel.flutter.tick(flutter_c, white);
            let mut sum = roar * 0.6 * roar_gain * channel.roar.tick(roar_c, white) * flutter;
            // Pops: rare, and most of them small.
            let chance = 0.5 * (channel.white() + 1.0);
            if chance < pop_chance {
                let size = 0.5 * (channel.white() + 1.0);
                channel.pop = channel.pop.max(size * size * size * crackle.min(1.0));
            }
            sum += 1.6 * channel.pop * channel.white();
            channel.pop *= pop_ring;
            sum += hull * 0.7 * loudness(hull_c) * channel.hull.tick(hull_c, white);
            // Wind: a band of noise round its tone, in gusts.
            let high = white - channel.wind_low - 0.8 * channel.wind_band;
            channel.wind_band += wind_f * high;
            channel.wind_low += wind_f * channel.wind_band;
            let gust = 1.0 + 0.5 * loudness(gust_c) * channel.gust.tick(gust_c, white);
            sum += wind * 0.5 * channel.wind_band * gust;
            sum += buffet * 0.8 * loudness(buffet_c) * channel.buffet.tick(buffet_c, white);
            // Hiss: what the low-pass filter takes out.
            sum += hiss * 0.5 * (white - channel.hiss.tick(hiss_c, white));
            sum += effects * bang * (0.7 * bang_note + 0.4 * loudness(buffet_c) * channel.buffet.1);
            sum += effects * beep;
            *out = (sum * master).tanh();
        }
        (out[0], out[1])
    }

    /// This frame of the alert tone: each note fades in and out so it does not click.
    fn beep(&mut self) -> f32 {
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
        mix.set(Voice::RoarTone, 300.0);
        mix.set(Voice::WindTone, 800.0);
        set(&mix);
        let mut synth = Synth::new(mix);
        for _ in 0..(RATE / 2) {
            synth.frame();
        }
        (0..(seconds * RATE as f32) as usize)
            .map(|_| synth.frame().0)
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// How often it crosses zero, per second: higher is brighter.
    fn crossings(samples: &[f32]) -> f32 {
        let count = samples.windows(2).filter(|w| w[0] * w[1] < 0.0).count();
        count as f32 * RATE as f32 / samples.len() as f32
    }

    #[test]
    fn nothing_going_on_is_silence() {
        assert!(rms(&play(|_| {}, 0.2)) < 1e-6);
    }

    #[test]
    fn a_roar_is_loud_and_never_clips_and_the_master_turns_it_down() {
        let full = play(|m| m.set(Voice::Roar, 1.0), 1.0);
        assert!(rms(&full) > 0.25, "{}", rms(&full));
        assert!(full.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        let quiet = play(
            |m| {
                m.set(Voice::Roar, 1.0);
                m.set(Voice::Master, 0.1);
            },
            1.0,
        );
        assert!(rms(&quiet) < 0.25 * rms(&full));
    }

    #[test]
    fn its_loudness_stays_with_its_tone_and_its_tone_shows() {
        let deep = play(
            |m| {
                m.set(Voice::Roar, 0.3);
                m.set(Voice::RoarTone, 100.0);
            },
            1.0,
        );
        let bright = play(
            |m| {
                m.set(Voice::Roar, 0.3);
                m.set(Voice::RoarTone, 2_000.0);
            },
            1.0,
        );
        assert!(crossings(&bright) > 3.0 * crossings(&deep));
        let ratio = rms(&bright) / rms(&deep);
        assert!((0.4..2.5).contains(&ratio), "{ratio}");
    }

    #[test]
    fn crackle_is_pops_not_a_steady_noise() {
        let samples = play(|m| m.set(Voice::Crackle, 1.0), 1.0);
        let peak = samples.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        assert!(peak > 5.0 * rms(&samples), "{peak} {}", rms(&samples));
    }

    #[test]
    fn wind_and_hiss_are_brighter_than_the_hull() {
        let hull = play(|m| m.set(Voice::Hull, 1.0), 1.0);
        let wind = play(|m| m.set(Voice::Wind, 1.0), 1.0);
        let hiss = play(|m| m.set(Voice::Hiss, 1.0), 1.0);
        assert!(rms(&hull) > 0.1 && rms(&wind) > 0.05 && rms(&hiss) > 0.05);
        assert!(crossings(&wind) > 3.0 * crossings(&hull));
        assert!(crossings(&hiss) > crossings(&wind));
    }

    #[test]
    fn a_bang_rings_out_and_goes_off_once() {
        let mix = Arc::new(Mix::default());
        mix.set(Voice::Master, 1.0);
        mix.set(Voice::Effects, 1.0);
        let mut synth = Synth::new(mix.clone());
        for _ in 0..(RATE / 2) {
            synth.frame();
        }
        mix.bang(0.3);
        mix.bang(1.0);
        mix.bang(0.5);
        let mut play = |seconds: f32| -> Vec<f32> {
            (0..(seconds * RATE as f32) as usize)
                .map(|_| synth.frame().0)
                .collect()
        };
        let first = play(0.2);
        let later = play(1.8);
        let last = play(4.0);
        assert!(rms(&first) > 0.2, "{}", rms(&first));
        assert!(rms(&later) < rms(&first));
        assert!(rms(&last) < 0.1 * rms(&first), "{}", rms(&last));
    }

    #[test]
    fn the_alert_tone_sounds_once_and_is_two_notes() {
        let mix = Arc::new(Mix::default());
        mix.set(Voice::Master, 1.0);
        mix.set(Voice::Effects, 1.0);
        let mut synth = Synth::new(mix.clone());
        for _ in 0..(RATE / 2) {
            synth.frame();
        }
        mix.beep();
        let mut play = |seconds: f32| -> Vec<f32> {
            (0..(seconds * RATE as f32) as usize)
                .map(|_| synth.frame().0)
                .collect()
        };
        let first = play(BEEP_NOTE);
        let second = play(BEEP_NOTE);
        let after = play(0.2);
        assert!(rms(&first) > 0.1 && rms(&second) > 0.1);
        let (high, low) = (crossings(&first) / 2.0, crossings(&second) / 2.0);
        assert!((high - 880.0).abs() < 60.0, "{high}");
        assert!((low - 660.0).abs() < 60.0, "{low}");
        assert!(rms(&after) < 1e-3, "{}", rms(&after));
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
        let big = roar_tone(2_600.0, area(8.3), 100.0);
        let small = roar_tone(2_000.0, area(0.3), 100.0);
        let far = roar_tone(2_000.0, area(0.3), 5_000.0);
        assert!(small > 4.0 * big, "{small} {big}");
        assert!(far < 0.5 * small, "{far} {small}");
        assert!((ROAR_LOWEST..=ROAR_HIGHEST).contains(&big));
    }
}
