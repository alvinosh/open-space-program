//! What a playtest found: each step as the player did it, and the figures for the whole go.
//!
//! How long a person takes is estimated with the keystroke-level model (Card, Moran and Newell):
//! a moment to think before each step, pointing by Fitts's law (the smaller of the target's sides,
//! as for two-dimensional targets), a tenth of a second per button press or release, a keystroke
//! for each key, and moving a hand between the mouse and the keys. The figures are rough; what
//! matters is how they move between runs, and which steps cost most.

use bevy::math::Vec2;
use serde::Serialize;

use super::{Play, Playtest};

/// A moment to think what to do next, s.
const THINK: f64 = 1.35;
/// Pressing or letting go of a mouse button, s.
const BUTTON: f64 = 0.1;
/// A notch of the mouse wheel, s.
const NOTCH: f64 = 0.15;
/// A keystroke, an average player's, s.
const KEY: f64 = 0.28;
/// A hand going between the mouse and the keys, s.
const HOME: f64 = 0.4;
/// Fitts's law for a mouse, rough: s, and s per bit.
const FITTS_A: f64 = 0.1;
const FITTS_B: f64 = 0.15;
/// Controls smaller than this across are hard to hit (WCAG 2.5.8 asks for 24 px), px.
const SMALL: f32 = 24.0;
/// A frame longer than this is a stall a player sees, ms.
const STALL: f64 = 100.0;

/// The pointer going to something.
#[derive(Serialize, Clone)]
pub struct Pointing {
    pub from: [f32; 2],
    pub to: [f32; 2],
    pub distance: f32,
    /// How big what it went for is across, px.
    pub width: f32,
    /// Fitts's index of difficulty, bits.
    pub bits: f64,
    /// The hand came off the keys first.
    pub homed: bool,
}

impl Pointing {
    pub fn new(from: Vec2, to: Vec2, width: f32, homed: bool) -> Self {
        let distance = from.distance(to);
        let bits = (distance as f64 / width.max(1.0) as f64 + 1.0).log2();
        Self {
            from: from.to_array(),
            to: to.to_array(),
            distance,
            width,
            bits,
            homed,
        }
    }

    fn seconds(&self) -> f64 {
        // Already there: no pointing at all.
        let point = if self.distance < 1.0 {
            0.0
        } else {
            FITTS_A + FITTS_B * self.bits
        };
        point + if self.homed { HOME } else { 0.0 }
    }
}

/// One step as the player did it.
#[derive(Serialize)]
pub struct StepRecord {
    pub index: usize,
    pub play: String,
    pub kind: &'static str,
    pub pointings: Vec<Pointing>,
    /// Keys pressed, and whether the hand came off the mouse for them.
    pub keys: usize,
    pub homed_keys: bool,
    /// Notches of the mouse wheel.
    pub notches: usize,
    /// How long a person takes over it, s.
    pub person_s: f64,
    #[serde(skip)]
    pub input_at: Option<f64>,
    /// From the input that finishes it to the screen showing an answer, and to the game being done
    /// with it (what it built on show), ms.
    pub answer_ms: Option<f64>,
    pub settled_ms: Option<f64>,
    /// From the pointer reaching where something in hand goes to the editor showing it there, ms.
    pub preview_ms: Option<f64>,
    #[serde(skip)]
    pub frames: Vec<f64>,
    pub worst_frame_ms: f64,
    /// The UI's actions it fired, and what the screen shows that it changed.
    pub actions: Vec<String>,
    pub changed: Vec<String>,
    /// It was an input and nothing a player could see happened.
    pub dead: bool,
    /// What the editor said after it.
    pub notice: Option<String>,
    pub picture: Option<String>,
}

impl StepRecord {
    pub fn new(index: usize, play: Play) -> Self {
        let kind = match play {
            Play::Click(_) => "click",
            Play::Drag(..) => "drag",
            Play::Key(_) | Play::Chord(..) => "key",
            Play::Type(_) => "type",
            Play::Wheel(..) => "wheel",
            _ => "look",
        };
        Self {
            index,
            play: format!("{play:?}"),
            kind,
            pointings: Vec::new(),
            keys: 0,
            homed_keys: false,
            notches: 0,
            person_s: 0.0,
            input_at: None,
            answer_ms: None,
            settled_ms: None,
            preview_ms: None,
            frames: Vec::new(),
            worst_frame_ms: 0.0,
            actions: Vec::new(),
            changed: Vec::new(),
            dead: false,
            notice: None,
            picture: None,
        }
    }

    /// The pointer went from `from` to `to`, for something `width` across.
    pub fn point(&mut self, from: Vec2, to: Vec2, width: f32, homed: bool) {
        // A click's second look is the same pointing, put right.
        if self.kind == "click"
            && let Some(last) = self.pointings.last_mut()
        {
            let (first, homed) = (Vec2::from_array(last.from), last.homed);
            *last = Pointing::new(first, to, width, homed);
            return;
        }
        if self.kind == "drag" {
            match self.pointings.len() {
                // What is in hand is carried from where it was taken, whichever way it went.
                1 => {
                    let from = Vec2::from_array(self.pointings[0].to);
                    self.pointings.push(Pointing::new(from, to, width, false));
                    return;
                }
                // A second look at where it goes is the same pointing put right.
                n if n >= 2 => {
                    let from = Vec2::from_array(self.pointings[n - 1].from);
                    self.pointings[n - 1] = Pointing::new(from, to, width, false);
                    return;
                }
                _ => {}
            }
        }
        self.pointings.push(Pointing::new(from, to, width, homed));
    }

    pub fn keys(&mut self, count: usize, hand_on_mouse: bool) {
        self.homed_keys |= hand_on_mouse && self.keys == 0;
        self.keys += count;
    }

    fn close(&mut self) {
        let buttons = match self.kind {
            "click" | "drag" => 2.0 * BUTTON,
            _ => 0.0,
        } + self.notches as f64 * NOTCH;
        let pointing: f64 = self.pointings.iter().map(Pointing::seconds).sum();
        let keys = self.keys as f64 * KEY + if self.homed_keys { HOME } else { 0.0 };
        self.person_s = THINK + pointing + buttons + keys;
        self.worst_frame_ms = self.frames.iter().copied().fold(0.0, f64::max);
        // How far the view turned or zoomed is not on the UI's data: a wheel is not judged.
        self.dead = self.kind != "wheel" && self.actions.is_empty() && self.changed.is_empty();
    }

    fn inputs(&self) -> usize {
        match self.kind {
            "click" | "drag" | "wheel" => 1,
            _ => self.keys,
        }
    }
}

#[derive(Serialize)]
pub struct Goal {
    pub name: String,
    /// Real time the playtest took to get there, s.
    pub real_s: f64,
    pub steps: usize,
    pub inputs: usize,
    /// How long a person takes to get there, s.
    pub person_s: f64,
}

/// A number the player got out of it: the lowest, highest and last it was.
#[derive(Serialize)]
pub struct Measure {
    pub label: String,
    pub key: String,
    pub lowest: Option<f64>,
    pub highest: Option<f64>,
    pub last: Option<f64>,
}

/// What a player hoped for, and whether they got it.
#[derive(Serialize)]
pub struct Hope {
    pub why: String,
    pub condition: String,
    pub met: bool,
    /// When it stopped holding, s into the go, and what the screen showed then.
    pub broke_at: Option<f64>,
    pub found: Option<String>,
}

#[derive(Serialize)]
pub struct Failure {
    pub step: Option<usize>,
    pub reason: String,
}

#[derive(Serialize, Default)]
pub struct Frames {
    pub count: usize,
    pub mean_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub over_50ms: usize,
    pub over_100ms: usize,
}

#[derive(Serialize, Default)]
pub struct Summary {
    pub steps: usize,
    pub clicks: usize,
    pub drags: usize,
    pub keys: usize,
    pub inputs: usize,
    pub pointer_travel_px: f64,
    /// How long a person takes over the whole go, s.
    pub person_s: f64,
    /// How long the game took to be done with the inputs, all told, s.
    pub game_s: f64,
    /// The share of inputs the screen answered within a tenth of a second and within a second.
    pub answered_100ms: f64,
    pub answered_1s: f64,
    /// Steps that did nothing a player could see.
    pub dead: Vec<usize>,
    /// What the editor said, by step.
    pub notices: Vec<(usize, String)>,
    /// Controls smaller than 24 px across, by step.
    pub small_targets: Vec<(usize, f32)>,
    /// The step the game took longest over, ms.
    pub slowest: Option<(usize, f64)>,
    /// The step whose preview took longest to show, ms.
    pub slowest_preview: Option<(usize, f64)>,
    /// Steps with a frame over 100 ms, and the worst.
    pub stalls: Vec<(usize, f64)>,
    /// The steps that cost a person most time, s, most first.
    pub costliest: Vec<(usize, f64)>,
}

#[derive(Serialize)]
pub struct Report {
    pub name: String,
    pub about: String,
    /// Known to get stuck until this task is done.
    pub stuck_until: Option<String>,
    pub screen: [u32; 2],
    pub passed: bool,
    pub failure: Option<Failure>,
    pub real_s: f64,
    pub summary: Summary,
    pub goals: Vec<Goal>,
    pub measures: Vec<Measure>,
    pub hopes: Vec<Hope>,
    pub frames: Frames,
    pub steps: Vec<StepRecord>,
    pub pictures: Vec<String>,
    #[serde(skip)]
    frame_times: Vec<f64>,
}

impl Report {
    pub fn new(play: &Playtest, screen: (u32, u32)) -> Self {
        Self {
            name: play.name.into(),
            about: play.about.into(),
            stuck_until: play.stuck_until.map(Into::into),
            screen: [screen.0, screen.1],
            passed: true,
            failure: None,
            real_s: 0.0,
            summary: Summary::default(),
            goals: Vec::new(),
            measures: Vec::new(),
            hopes: Vec::new(),
            frames: Frames::default(),
            steps: Vec::new(),
            pictures: Vec::new(),
            frame_times: Vec::new(),
        }
    }

    pub fn frame(&mut self, ms: f64) {
        self.frame_times.push(ms);
    }

    pub fn step(&mut self, mut step: StepRecord) {
        step.close();
        self.steps.push(step);
    }

    pub fn goal(&mut self, name: &str, real_s: f64) {
        self.goals.push(Goal {
            name: name.into(),
            real_s,
            steps: self.steps.len(),
            inputs: self.steps.iter().map(StepRecord::inputs).sum(),
            person_s: self.steps.iter().map(|s| s.person_s).sum(),
        });
    }

    pub fn measure(&mut self, label: &str, key: &str, range: Option<[f64; 3]>) {
        self.measures.push(Measure {
            label: label.into(),
            key: key.into(),
            lowest: range.map(|r| r[0]),
            highest: range.map(|r| r[1]),
            last: range.map(|r| r[2]),
        });
    }

    pub fn letdown(&mut self, why: &str, condition: &str, at: f64, found: String) {
        self.hopes.push(Hope {
            why: why.into(),
            condition: condition.into(),
            met: false,
            broke_at: Some(at),
            found: Some(found),
        });
    }

    pub fn hope_met(&mut self, why: &str, condition: &str) {
        self.hopes.push(Hope {
            why: why.into(),
            condition: condition.into(),
            met: true,
            broke_at: None,
            found: None,
        });
    }

    pub fn fail(&mut self, step: Option<usize>, reason: String) {
        self.passed = false;
        self.failure = Some(Failure { step, reason });
    }

    pub fn finish(&mut self, real_s: f64) {
        self.real_s = real_s;
        let mut times = self.frame_times.clone();
        times.sort_by(f64::total_cmp);
        if !times.is_empty() {
            let count = times.len();
            self.frames = Frames {
                count,
                mean_ms: times.iter().sum::<f64>() / count as f64,
                p95_ms: times[(count * 95 / 100).min(count - 1)],
                max_ms: times[count - 1],
                over_50ms: times.iter().filter(|&&t| t > 50.0).count(),
                over_100ms: times.iter().filter(|&&t| t > STALL).count(),
            };
        }
        let inputs: Vec<&StepRecord> = (self.steps.iter()).filter(|s| s.kind != "look").collect();
        let answered = |limit: f64| {
            let n = (inputs.iter())
                .filter(|s| s.answer_ms.is_some_and(|ms| ms <= limit))
                .count();
            n as f64 / inputs.len().max(1) as f64
        };
        let mut costliest: Vec<(usize, f64)> =
            self.steps.iter().map(|s| (s.index, s.person_s)).collect();
        costliest.sort_by(|a, b| b.1.total_cmp(&a.1));
        costliest.truncate(5);
        self.summary = Summary {
            steps: self.steps.len(),
            clicks: inputs.iter().filter(|s| s.kind == "click").count(),
            drags: inputs.iter().filter(|s| s.kind == "drag").count(),
            keys: inputs.iter().map(|s| s.keys).sum(),
            inputs: inputs.iter().map(|s| s.inputs()).sum(),
            pointer_travel_px: (self.steps.iter())
                .flat_map(|s| &s.pointings)
                .map(|p| p.distance as f64)
                .sum(),
            person_s: self.steps.iter().map(|s| s.person_s).sum(),
            game_s: inputs.iter().filter_map(|s| s.settled_ms).sum::<f64>() / 1000.0,
            answered_100ms: answered(100.0),
            answered_1s: answered(1000.0),
            dead: inputs.iter().filter(|s| s.dead).map(|s| s.index).collect(),
            notices: (self.steps.iter())
                .filter_map(|s| Some((s.index, s.notice.clone()?)))
                .filter(|(_, notice)| !notice.is_empty())
                .collect(),
            slowest_preview: (self.steps.iter())
                .filter_map(|s| Some((s.index, s.preview_ms?)))
                .max_by(|a, b| a.1.total_cmp(&b.1)),
            small_targets: (self.steps.iter())
                .flat_map(|s| s.pointings.iter().map(move |p| (s.index, p.width)))
                .filter(|(_, w)| *w < SMALL)
                .collect(),
            slowest: (inputs.iter())
                .filter_map(|s| Some((s.index, s.settled_ms?)))
                .max_by(|a, b| a.1.total_cmp(&b.1)),
            stalls: (self.steps.iter())
                .filter(|s| s.worst_frame_ms > STALL)
                .map(|s| (s.index, s.worst_frame_ms))
                .collect(),
            costliest,
        };
    }

    /// One line for the log.
    pub fn headline(&self) -> String {
        let s = &self.summary;
        let result = match &self.failure {
            None => "passed".to_string(),
            Some(f) => format!("FAILED at step {:?}: {}", f.step, f.reason),
        };
        let letdowns = self.hopes.iter().filter(|h| !h.met).count();
        format!(
            "{result}; {} inputs ({} clicks, {} drags, {} keys), a person takes about {:.0} s, \
             {} dead, {:.0}% answered within 0.1 s, worst frame {:.0} ms, {letdowns} of {} hopes \
             let down",
            s.inputs,
            s.clicks,
            s.drags,
            s.keys,
            s.person_s,
            s.dead.len(),
            s.answered_100ms * 100.0,
            self.frames.max_ms,
            self.hopes.len(),
        )
    }
}
