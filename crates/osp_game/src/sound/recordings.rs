//! The engines as recorded: takes of a rocket's roar in `assets/core/sounds`, which are not ours
//! (see `CREDITS.md` there), sorted into beds by how far away they were made to sound.
//!
//! A take is an attack, a long steady part and sometimes the engine being cut. A [`Looper`] plays
//! a bed without ever being heard to repeat: it starts at a take's own attack, then every few
//! seconds fades into some other stretch of steady roar, in any take of the bed. Roar is noise, so
//! a fade that keeps the power level is inaudible and no take needs a tuned loop.

use std::{f32::consts::FRAC_PI_2, path::Path};

use bevy::{
    audio::{AudioSource, Decodable, Source},
    prelude::*,
};

use super::{Noise, RATE};

/// The beds, nearest first.
pub const CLOSE: usize = 0;
pub const DISTANT: usize = 1;
pub const VERY_FAR: usize = 2;
const BEDS: usize = 3;

/// What is on disk. In a bed the first take has the slow start and long tail of a big rocket and
/// the second the sharp ones of a small rocket. Times are seconds into the file, measured.
struct Listing {
    file: &'static str,
    bed: usize,
    /// The part that holds steady.
    steady: (f32, f32),
    /// Where the level starts to fall as the engine is cut.
    cut: Option<f32>,
}

const LISTINGS: [Listing; 6] = [
    Listing {
        file: "roar_launch_slow_start.ogg",
        bed: CLOSE,
        steady: (4.0, 17.3),
        cut: Some(17.5),
    },
    Listing {
        file: "roar_launch_fast_start.ogg",
        bed: CLOSE,
        steady: (1.0, 12.3),
        cut: Some(12.5),
    },
    Listing {
        file: "roar_steady.ogg",
        bed: CLOSE,
        steady: (2.5, 28.2),
        cut: None,
    },
    Listing {
        file: "roar_launch_deep.ogg",
        bed: CLOSE,
        steady: (1.0, 8.6),
        cut: None,
    },
    Listing {
        file: "roar_distant.ogg",
        bed: DISTANT,
        steady: (4.5, 18.3),
        cut: None,
    },
    Listing {
        file: "roar_very_distant.ogg",
        bed: VERY_FAR,
        steady: (1.0, 25.1),
        cut: None,
    },
];

/// A take's steady part is brought to this RMS, as measured above `SPEAKER_FLOOR`: the takes hold
/// very different amounts of deep rumble, and level should mean how loud it is, not how much sub
/// there is. So a level of 1 from any bed sounds alike; how far away it is takes the level down.
const HEARD_RMS: f32 = 0.105;
const SPEAKER_FLOOR: f32 = 120.0;
/// A jet playing this fast starts and stops like a small rocket.
const SMALL_ROCKET_RATE: f64 = 1.2;
/// How long a stretch plays before the next one comes in, s, and how long that takes.
const STRETCH: (f32, f32) = (5.0, 11.0);
const FADE: f32 = 1.5;

/// One recording in memory.
pub struct Take {
    frames: Vec<[f32; 2]>,
    /// Frames of the take to an output frame at a playback rate of 1.
    step: f64,
    /// The steady part, in frames.
    steady: (f64, f64),
    /// Where the level starts to fall as the engine is cut, in frames.
    cut: Option<f64>,
    gain: f32,
}

impl Take {
    fn new(listing: &Listing, bytes: Vec<u8>) -> Self {
        let decoder = AudioSource {
            bytes: bytes.into(),
        }
        .decoder();
        let channels = usize::from(decoder.channels().get());
        let rate = f64::from(decoder.sample_rate().get());
        let samples: Vec<f32> = decoder.collect();
        let frames: Vec<[f32; 2]> = samples
            .chunks_exact(channels)
            .map(|frame| [frame[0], frame[1.min(channels - 1)]])
            .collect();
        let last = frames.len().saturating_sub(1) as f64;
        let at = |seconds: f32| (f64::from(seconds) * rate).min(last);
        let steady = (at(listing.steady.0), at(listing.steady.1));
        let gain = HEARD_RMS / heard_rms(&frames, steady, rate as f32).max(1e-6);
        Self {
            frames,
            step: rate / f64::from(RATE),
            steady,
            cut: listing.cut.map(at),
            gain,
        }
    }

    /// The take at a position in frames, between frames by a cubic.
    fn read(&self, at: f64) -> [f32; 2] {
        let whole = at as usize;
        let t = (at - whole as f64) as f32;
        let frame = |i: usize| {
            self.frames
                .get(i)
                .or(self.frames.last())
                .copied()
                .unwrap_or_default()
        };
        let (a, b) = (frame(whole.saturating_sub(1)), frame(whole));
        let (c, d) = (frame(whole + 1), frame(whole + 2));
        let channel = |k: usize| {
            let (p0, p1, p2, p3) = (a[k], b[k], c[k], d[k]);
            let w = -0.5 * p0 + 1.5 * p1 - 1.5 * p2 + 0.5 * p3;
            let x = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
            let y = -0.5 * p0 + 0.5 * p2;
            self.gain * (((w * t + x) * t + y) * t + p1)
        };
        [channel(0), channel(1)]
    }

    fn steady_length(&self) -> f64 {
        self.steady.1 - self.steady.0
    }
}

/// The RMS of the part of `frames` a small speaker can play.
fn heard_rms(frames: &[[f32; 2]], (from, to): (f64, f64), rate: f32) -> f32 {
    let coefficient = 1.0 - (-std::f32::consts::TAU * SPEAKER_FLOOR / rate).exp();
    let mut low = 0.0f32;
    let mut sum = 0.0f64;
    let part = frames.get(from as usize..to as usize).unwrap_or_default();
    for frame in part {
        let mono = 0.5 * (frame[0] + frame[1]);
        low += coefficient * (mono - low);
        sum += f64::from((mono - low) * (mono - low));
    }
    (sum / part.len().max(1) as f64).sqrt() as f32
}

/// Takes that all sound as if from about the same distance.
#[derive(Default)]
pub struct Bed {
    takes: Vec<Take>,
}

impl Bed {
    /// The take to start from, or to be cut in: a big rocket is slower at both.
    fn opening(&self, rate: f64) -> usize {
        usize::from(rate >= SMALL_ROCKET_RATE).min(self.takes.len().saturating_sub(1))
    }

    /// Some stretch of steady roar: a take as likely as it is long, then anywhere in it that
    /// leaves room for a fade in and a fade out at this rate.
    fn somewhere(&self, rate: f64, dice: &mut Noise) -> Head {
        let total: f64 = self.takes.iter().map(Take::steady_length).sum();
        let mut pick = f64::from(dice.share()) * total;
        let mut chosen = 0;
        for (i, take) in self.takes.iter().enumerate() {
            chosen = i;
            pick -= take.steady_length();
            if pick < 0.0 {
                break;
            }
        }
        let take = &self.takes[chosen];
        let room = 2.0 * f64::from(FADE) * f64::from(RATE) * take.step * rate;
        let last = (take.steady.1 - room).max(take.steady.0);
        Head {
            take: chosen,
            at: take.steady.0 + f64::from(dice.share()) * (last - take.steady.0),
        }
    }

    /// The engine being cut, from where its level starts to fall, if a take has that.
    pub fn tail(&self, rate: f64, level: f32) -> Option<Tail> {
        let take = self.opening(rate);
        Some(Tail {
            take,
            at: self.takes.get(take)?.cut?,
            level,
        })
    }
}

/// Everything recorded.
#[derive(Default)]
pub struct Bank {
    pub beds: [Bed; BEDS],
}

impl Bank {
    /// Read and decode the takes in `dir`. One that is missing leaves a gap and a warning.
    pub fn load(dir: &Path) -> Self {
        let mut bank = Self::default();
        for listing in &LISTINGS {
            let path = dir.join(listing.file);
            match std::fs::read(&path) {
                Ok(bytes) => bank.beds[listing.bed].takes.push(Take::new(listing, bytes)),
                Err(error) => warn!("no engine sound {}: {error}", path.display()),
            }
        }
        bank
    }
}

/// Where in a bed a play head is.
#[derive(Clone, Copy, Default)]
struct Head {
    take: usize,
    /// In frames of the take.
    at: f64,
}

/// Plays a bed, without end.
#[derive(Default)]
pub struct Looper {
    on: Head,
    next: Option<Head>,
    fade: f32,
    /// Seconds until the next stretch comes in.
    left: f32,
}

fn stretch(dice: &mut Noise) -> f32 {
    STRETCH.0 + dice.share() * (STRETCH.1 - STRETCH.0)
}

impl Looper {
    /// Start over from the top of a take, through its own attack.
    pub fn ignite(&mut self, bed: &Bed, rate: f64, dice: &mut Noise) {
        let take = bed.opening(rate);
        let Some(opening) = bed.takes.get(take) else {
            return;
        };
        self.on = Head { take, at: 0.0 };
        self.next = None;
        let frames_per_second = opening.step * rate * f64::from(RATE);
        self.left = (opening.steady.0 / frames_per_second) as f32 + stretch(dice);
    }

    /// The next frame at a playback rate of `rate`.
    pub fn frame(&mut self, bed: &Bed, rate: f64, dice: &mut Noise) -> [f32; 2] {
        let Some(take) = bed.takes.get(self.on.take) else {
            return [0.0; 2];
        };
        let moved = take.step * rate;
        let frames_left = (take.steady.1 - self.on.at) / moved;
        self.left -= 1.0 / RATE as f32;
        if self.next.is_none() && (self.left <= 0.0 || frames_left <= f64::from(FADE * RATE as f32))
        {
            self.next = Some(bed.somewhere(rate, dice));
            self.fade = 0.0;
            self.left = stretch(dice);
        }
        let here = take.read(self.on.at);
        self.on.at += moved;
        let Some(mut next) = self.next else {
            return here;
        };
        let coming = &bed.takes[next.take];
        let there = coming.read(next.at);
        next.at += coming.step * rate;
        // Unrelated noises add by power, so the fade keeps the sum of the squares.
        let (rise, fall) = (self.fade * FRAC_PI_2).sin_cos();
        self.fade += 1.0 / (FADE * RATE as f32);
        if self.fade >= 1.0 {
            self.on = next;
            self.next = None;
        } else {
            self.next = Some(next);
        }
        [
            fall * here[0] + rise * there[0],
            fall * here[1] + rise * there[1],
        ]
    }
}

/// An engine's cut playing out, once.
pub struct Tail {
    take: usize,
    at: f64,
    level: f32,
}

impl Tail {
    /// The next frame, or `None` at the end of the take.
    pub fn frame(&mut self, bed: &Bed, rate: f64) -> Option<[f32; 2]> {
        let take = bed.takes.get(self.take)?;
        if self.at + 2.0 >= take.frames.len() as f64 {
            return None;
        }
        let [left, right] = take.read(self.at);
        self.at += take.step * rate;
        Some([self.level * left, self.level * right])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bank() -> Bank {
        Bank::load(&crate::content::asset_root().join("core/sounds"))
    }

    #[test]
    fn every_take_loads_and_its_times_are_inside_it() {
        let bank = bank();
        let takes: usize = bank.beds.iter().map(|bed| bed.takes.len()).sum();
        assert_eq!(takes, LISTINGS.len());
        for take in bank.beds.iter().flat_map(|bed| &bed.takes) {
            let length = take.frames.len() as f64;
            assert!(take.steady.0 < take.steady.1 && take.steady.1 < length);
            assert!(take.steady_length() > 5.0 * 24_000.0);
            assert!(
                take.cut
                    .is_none_or(|cut| cut > take.steady.1 && cut < length)
            );
            assert!((0.2..12.0).contains(&take.gain), "{}", take.gain);
        }
    }

    #[test]
    fn the_steady_parts_are_steady() {
        // If a file is swapped for one with another timeline, the listing has to change with it.
        for take in bank().beds.iter().flat_map(|bed| &bed.takes) {
            let (from, to) = (take.steady.0 as usize, take.steady.1 as usize);
            let second = 24_000;
            let levels: Vec<f32> = take.frames[from..to]
                .chunks(second)
                .filter(|chunk| chunk.len() == second)
                .map(|chunk| {
                    let sum: f32 = chunk.iter().map(|f| f[0] * f[0] + f[1] * f[1]).sum();
                    (sum / chunk.len() as f32).sqrt()
                })
                .collect();
            let high = levels.iter().copied().fold(0.0, f32::max);
            let low = levels.iter().copied().fold(1.0, f32::min);
            let swing = 20.0 * (high / low).log10();
            assert!(swing < 6.0, "{swing} dB");
        }
    }
}
