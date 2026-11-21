//! Playtests: the game played the way a person plays it, from the title screen on, and measured
//! for how easy it is to use and how soon it pays off.
//!
//! `OSP_PLAYTEST=<name>` (with `OSP_USER_DIR=<empty folder>`, so it is someone's first game)
//! plays [`plays::PLAYS`]' `<name>`. A playtest only does what a player can do: point at what is on
//! screen and click, drag or press keys. What it points at is a control by its action (found where it
//! is drawn, and only if it is drawn, in sight and not covered), or in the editor a part, a ring or a
//! spot on a surface as drawn. It only checks what the screen shows: the UI's data. Each step waits
//! for the game to have answered the last one (the editor's builds on their threads done), never
//! for a set time, so a busy machine makes it slower but not flaky.
//!
//! Every step is measured ([`report`]): how far the pointer went and how big what it went for was,
//! and from that how long a person takes over it (the keystroke-level model with Fitts's law);
//! whether it did anything at all (a dead click); how long the game took to show an answer; what
//! the game said (a refusal shows as a notice); and how long frames took meanwhile. Goals mark how
//! far the player has got, measures what they got out of it. `OSP_PLAYTEST_OUT` (default
//! `target/playtest`) gets `<name>/report.json`, the pictures, and `craft.json`, the last craft
//! launched (to fly it again in a test of the simulation); `cargo playtest` runs them all.

pub mod plays;
pub mod rebuilds;
pub mod report;

use std::{collections::VecDeque, path::PathBuf};

use bevy::{
    camera::{ImageRenderTarget, NormalizedRenderTarget},
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput, NativeKey, NativeKeyCode},
        mouse::{MouseMotion, MouseScrollUnit, MouseWheel},
        touch::TouchPhase,
    },
    picking::{
        backend::prelude::PointerId,
        hover::HoverMap,
        pointer::{Location, PointerAction, PointerButton, PointerInput},
    },
    prelude::*,
    render::{
        render_resource::TextureFormat,
        view::screenshot::{Screenshot, save_to_disk},
    },
    ui::{CalculatedClip, UiGlobalTransform},
};
use osp_ui::{
    ActionSource, DragPayload, Formatters, Template, TextFocus, TextInput, UiAction, UiData,
    UiValue,
};

use crate::{
    camera::ViewOutput,
    editing::{EditorStatus, LastLaunch},
    orbit_lines::ScriptedPointer,
    smoke_test::{Action, CaptureSet, EditorStep, EditorStepFailed, capture_size},
    space_centre::SpaceCentre,
};
use report::{Report, StepRecord};

/// What a player can point at.
#[derive(Clone, Copy, Debug)]
pub enum Spot {
    /// The control with this action (or drag payload) as it reads now; one that starts with it if it
    /// ends in `:`. It must be drawn, in sight (not scrolled away) and not covered.
    Ui(&'static str),
    /// In the editor: the ring of this free node of this part; with something in hand, where its
    /// own node that goes on that ring is on it.
    Ring(&'static str, &'static str),
    /// In the editor: this surface of this part, this far round it and this far along it (0 to 1).
    Surface(&'static str, &'static str, f64, f64),
    /// In the editor: the middle of this part.
    Part(&'static str),
    /// In the editor: the handle for this size of the part picked.
    Grip(&'static str),
    /// This many px right and down of where the drag began (where a drag goes to).
    By(f32, f32),
    /// This share of the screen across and down: somewhere with nothing on it.
    Screen(f32, f32),
    /// In the space centre's view: this building, by id, where its front is drawn.
    Building(&'static str),
}

/// One thing a player does, or looks for.
#[derive(Clone, Copy, Debug)]
pub enum Play {
    Click(Spot),
    /// Press on the first, carry it to the second, let go.
    Drag(Spot, Spot),
    /// Tap a key.
    Key(KeyCode),
    /// Hold the first key and tap the second (Ctrl+Z).
    Chord(KeyCode, KeyCode),
    /// Turn the mouse wheel this many notches over a spot: up (+) or down (-).
    Wheel(Spot, i32),
    /// Type this (`\n` is Enter).
    Type(&'static str),
    /// Wait for the screen to show this (a condition on the UI's data, with `<`, `>`, `<=` and
    /// `>=` on numbers as well as `==`, `!=` and `!`), at most this many seconds.
    Until(&'static str, f64),
    /// Stop with a failure unless the screen shows this now.
    Expect(&'static str),
    /// Keep a picture of the screen.
    Picture(&'static str),
    /// The player has got this far: the report says how long it took and how much it asked of them.
    Goal(&'static str),
    /// From now on keep the lowest, highest and last of this number of the UI's data, under this
    /// label: what the player got out of it.
    Track(&'static str, &'static str),
    /// Let the game run this many seconds.
    Wait(f64),
    /// What a player hopes for: that this holds for the next so many seconds (or now, for 0). Not
    /// a failure if it does not, but a letdown in the report, with why it mattered.
    Hope(&'static str, f64, &'static str),
}

/// A playtest: a player's go at something.
pub struct Playtest {
    pub name: &'static str,
    /// What the player sets out to do, for the report.
    pub about: &'static str,
    pub steps: &'static [Play],
    /// Known to get stuck until this task is done: the run says so and goes on, and once the player
    /// gets through it fails until this is taken out (as smoke tests' known issues).
    pub stuck_until: Option<&'static str>,
}

/// What the UI's data shows that a player would see change: a step that changes none of these and
/// fires no action did nothing they could see.
const WATCHED: &[&str] = &[
    "menu.screen",
    "menu.playing",
    "menu.career",
    "dialog.title",
    "editor.rail",
    "editor.parts",
    "editor.part.picked",
    "editor.part.rows",
    "editor.group",
    "editor.palette",
    "editor.symmetry",
    "editor.mirror",
    "editor.tool",
    "editor.holding",
    "editor.notice",
    "editor.list_open",
    "editor.has_vessel",
    "editor.name",
    "editor.height",
    "editor.mass",
    "editor.place",
    "editor.loose",
    "centre.open",
    "centre.pointed",
    "centre.notice",
    "ui.shown.editor",
    "vessel.name",
    "vessel.throttle",
    "vessel.sas",
    "vessel.stage",
];

/// Frames the game must have been quiet for before a step goes on.
const QUIET_FRAMES: u32 = 3;
/// The longest the game may take to settle after a step, s.
const SETTLE_LIMIT: f64 = 30.0;

pub struct PlaytestPlugin;

impl Plugin for PlaytestPlugin {
    fn build(&self, app: &mut App) {
        let Ok(name) = std::env::var("OSP_PLAYTEST") else {
            return;
        };
        let Some(play) = plays::PLAYS.iter().find(|p| p.name == name) else {
            let names: Vec<_> = plays::PLAYS.iter().map(|p| p.name).collect();
            panic!("no playtest called `{name}`; there are {names:?}");
        };
        let out = std::env::var_os("OSP_PLAYTEST_OUT")
            .map_or_else(|| PathBuf::from("target/playtest"), PathBuf::from)
            .join(play.name);
        // Last time's pictures go, so the folder shows this run only.
        let _ = std::fs::remove_dir_all(&out);
        if let Err(e) = std::fs::create_dir_all(&out) {
            panic!("cannot make {}: {e}", out.display());
        }
        let (width, height) = capture_size();
        app.insert_resource(Runner {
            play,
            out,
            target: Handle::default(),
            screen: Vec2::new(width as f32, height as f32),
            ops: VecDeque::new(),
            next: 0,
            mouse: Vec2::new(width as f32, height as f32) * 0.5,
            pressed: None,
            aim: None,
            asked: false,
            on_mouse: true,
            op_frames: 0,
            op_started: 0.0,
            quiet: 0,
            input_at: None,
            before: Vec::new(),
            at_begin: Vec::new(),
            current: None,
            report: Report::new(play, (width, height)),
            started: None,
            ending: None,
            tracked: Vec::new(),
            busy_at: 0.0,
            changed_at: 0.0,
            stable: 0,
            last_seen: Vec::new(),
            moved_at: None,
        })
        .init_resource::<EditorStepFailed>()
        .add_systems(PostStartup, render_offscreen)
        .add_systems(Update, run.in_set(CaptureSet::Script));
    }
}

/// Where a step points: the spot, how big it is across (px), and the control, if it is one.
#[derive(Clone)]
struct Aim {
    at: Vec2,
    width: f32,
    entity: Option<Entity>,
}

/// The small things a step is done with, one or a few frames each.
#[derive(Clone, Debug)]
enum Op {
    /// Open the record of step `i`.
    Begin(usize),
    Find(Spot),
    /// The pointer goes to what was found (a second leg of the same pointing if `true`).
    Move,
    /// Partway from where it is to the middle of the screen: a drag on its way.
    Via,
    /// Wait for the game to be quiet.
    Settle,
    /// The control aimed at is what the pointer is over.
    Hover,
    Press,
    Release,
    Frames(u32),
    /// One notch of the wheel, up or down; the count left after it.
    Notch(i32),
    Key(KeyCode, bool),
    Type(&'static str),
    Until(&'static str, f64),
    Expect(&'static str),
    Picture(&'static str),
    Goal(&'static str),
    Track(&'static str, &'static str),
    Wait(f64),
    Hope(&'static str, f64, &'static str),
    /// Close the step's record.
    End,
}

#[derive(Resource)]
struct Runner {
    play: &'static Playtest,
    out: PathBuf,
    target: Handle<Image>,
    screen: Vec2,
    ops: VecDeque<Op>,
    /// The next step to turn into ops.
    next: usize,
    mouse: Vec2,
    /// Where the button went down, while it is held: what a drag's [`Spot::By`] counts from.
    pressed: Option<Vec2>,
    aim: Option<Aim>,
    /// The editor has been asked where a spot is and answers by the next frame.
    asked: bool,
    /// The player's hand is on the mouse, not the keys.
    on_mouse: bool,
    op_frames: u32,
    op_started: f64,
    /// Frames in a row the editor has had nothing building.
    quiet: u32,
    /// When the last input went in, real s, until the screen answers it.
    input_at: Option<f64>,
    /// What the watched data said before the step's input, and when the step began.
    before: Vec<String>,
    at_begin: Vec<String>,
    current: Option<StepRecord>,
    report: Report,
    /// Real s at the first step.
    started: Option<f64>,
    /// The run is over (failed or not), and frames left for pictures to be written.
    ending: Option<(bool, u32)>,
    /// The numbers tracked: key, label, and the lowest, highest and last.
    tracked: Vec<(&'static str, &'static str, Option<[f64; 3]>)>,
    /// The last frame the editor had something building, and the last the watched data changed,
    /// real s; and frames since it last changed.
    busy_at: f64,
    changed_at: f64,
    stable: u32,
    /// What the watched data said last frame.
    last_seen: Vec<String>,
    /// When the pointer last moved onto what a step goes for, real s, till the game has answered.
    moved_at: Option<f64>,
}

impl Runner {
    fn expand(&mut self, i: usize) {
        let ops: Vec<Op> = match self.play.steps[i] {
            // As a player does, a second look where it is before the press: what is on show may have
            // moved while the pointer went there.
            Play::Click(spot) => vec![
                Op::Begin(i),
                Op::Find(spot),
                Op::Move,
                Op::Settle,
                Op::Find(spot),
                Op::Move,
                Op::Settle,
                Op::Hover,
                Op::Press,
                Op::Frames(2),
                Op::Release,
                Op::Settle,
                Op::End,
            ],
            Play::Drag(from, to) => vec![
                Op::Begin(i),
                Op::Find(from),
                Op::Move,
                Op::Settle,
                Op::Hover,
                Op::Press,
                Op::Frames(2),
                Op::Via,
                Op::Settle,
                Op::Find(to),
                Op::Move,
                Op::Settle,
                // Where something in hand goes by its own node, that is drawn now where the pointer
                // brought it: look again and put it right.
                Op::Find(to),
                Op::Move,
                Op::Settle,
                Op::Release,
                Op::Settle,
                Op::End,
            ],
            Play::Key(key) => vec![
                Op::Begin(i),
                Op::Key(key, true),
                Op::Frames(2),
                Op::Key(key, false),
                Op::Settle,
                Op::End,
            ],
            Play::Chord(held, key) => vec![
                Op::Begin(i),
                Op::Key(held, true),
                Op::Frames(1),
                Op::Key(key, true),
                Op::Frames(2),
                Op::Key(key, false),
                Op::Key(held, false),
                Op::Settle,
                Op::End,
            ],
            Play::Type(text) => vec![Op::Begin(i), Op::Type(text), Op::Settle, Op::End],
            Play::Wheel(spot, notches) => vec![
                Op::Begin(i),
                Op::Find(spot),
                Op::Move,
                Op::Settle,
                Op::Notch(notches),
                Op::Settle,
                Op::End,
            ],
            Play::Until(condition, limit) => vec![Op::Until(condition, limit)],
            Play::Expect(condition) => vec![Op::Expect(condition)],
            Play::Picture(name) => vec![Op::Settle, Op::Picture(name)],
            Play::Goal(name) => vec![Op::Goal(name)],
            Play::Track(key, label) => vec![Op::Track(key, label)],
            Play::Wait(seconds) => vec![Op::Wait(seconds)],
            Play::Hope(condition, seconds, why) => vec![Op::Hope(condition, seconds, why)],
        };
        self.ops.extend(ops);
    }

    fn fail(&mut self, reason: String, commands: &mut Commands) {
        error!("playtest {}: {reason}", self.play.name);
        let step = self.current.as_ref().map(|s| s.index);
        self.report.fail(step, reason);
        self.picture("failed.png", commands);
        self.ops.clear();
        self.ending = Some((false, 0));
    }

    fn picture(&mut self, name: &str, commands: &mut Commands) {
        let path = self.out.join(name);
        commands
            .spawn(Screenshot::image(self.target.clone()))
            .observe(save_to_disk(path));
        self.report.pictures.push(name.to_string());
        if let Some(step) = &mut self.current {
            step.picture = Some(name.to_string());
        }
    }

    /// The pointer goes to `to`: the view's, and the UI's.
    fn move_to(&mut self, to: Vec2, pointer: &mut ScriptedPointer, input: &mut Input) {
        let delta = to - self.mouse;
        self.mouse = to;
        pointer.0 = Some(to);
        input.pointer(&self.target, to, PointerAction::Move { delta });
        // As a mouse does: the view turns by how far it went (a drag on nothing).
        input.motion.write(MouseMotion { delta });
    }
}

/// What the runner sends in: the pointer and keys, as the devices would.
#[derive(bevy::ecs::system::SystemParam)]
struct Input<'w> {
    pointer: MessageWriter<'w, PointerInput>,
    keys: MessageWriter<'w, KeyboardInput>,
    wheel: MessageWriter<'w, MouseWheel>,
    motion: MessageWriter<'w, MouseMotion>,
    buttons: ResMut<'w, ButtonInput<MouseButton>>,
}

impl Input<'_> {
    fn pointer(&mut self, target: &Handle<Image>, position: Vec2, action: PointerAction) {
        let target = NormalizedRenderTarget::Image(ImageRenderTarget {
            handle: target.clone(),
            scale_factor: 1.0,
        });
        self.pointer.write(PointerInput::new(
            PointerId::Mouse,
            Location { target, position },
            action,
        ));
    }

    fn key(&mut self, key_code: KeyCode, logical_key: Key, text: Option<&str>, pressed: bool) {
        self.keys.write(KeyboardInput {
            key_code,
            logical_key,
            state: if pressed {
                ButtonState::Pressed
            } else {
                ButtonState::Released
            },
            text: text.filter(|_| pressed).map(Into::into),
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
    }
}

/// The UI's controls, to find one where it is drawn.
#[derive(bevy::ecs::system::SystemParam)]
struct Controls<'w, 's> {
    nodes: Query<
        'w,
        's,
        (
            Entity,
            &'static ComputedNode,
            &'static UiGlobalTransform,
            &'static InheritedVisibility,
            Option<&'static ActionSource>,
            Option<&'static DragPayload>,
            Option<&'static TextInput>,
            Option<&'static CalculatedClip>,
        ),
    >,
    parents: Query<'w, 's, &'static ChildOf>,
    hover: Res<'w, HoverMap>,
    data: Res<'w, UiData>,
    formatters: Res<'w, Formatters>,
    focus: Res<'w, TextFocus>,
}

impl Controls<'_, '_> {
    /// The control that says `what`: where it is, and why a player could not get at it if they
    /// could not.
    fn find(&self, what: &str, screen: Vec2) -> Result<Aim, String> {
        let says = |template: Option<&Template>| {
            template.is_some_and(|t| {
                let said = t.render(&self.data, &self.formatters);
                said == what || (what.ends_with(':') && said.starts_with(what))
            })
        };
        let mut hidden = None;
        for (entity, node, transform, visible, click, drag, input, clip) in &self.nodes {
            let named = says(click.map(|c| &c.0))
                || says(drag.map(|d| &d.0))
                || says(input.map(TextInput::action));
            if !named || !visible.get() || node.size().min_element() <= 0.0 {
                continue;
            }
            let at = transform.translation;
            let on_screen = at.x >= 0.0 && at.y >= 0.0 && at.x < screen.x && at.y < screen.y;
            if !on_screen || clip.is_some_and(|c| !c.contains_point(at)) {
                hidden = Some(format!(
                    "`{what}` is there but out of sight (scrolled away)"
                ));
                continue;
            }
            let size = node.size();
            return Ok(Aim {
                at,
                width: size.min_element(),
                entity: Some(entity),
            });
        }
        Err(hidden.unwrap_or_else(|| format!("nothing on screen says `{what}`")))
    }

    /// Whether the pointer is over `entity` (or what is in it), as picking sees it; if not, what it
    /// is over instead.
    fn over(&self, entity: Entity) -> Result<(), String> {
        let Some(hits) = self.hover.get(&PointerId::Mouse) else {
            return Err("the pointer is over nothing".into());
        };
        let mut found = Vec::new();
        for &hit in hits.keys() {
            let mut at = Some(hit);
            while let Some(e) = at {
                if e == entity {
                    return Ok(());
                }
                at = self.parents.get(e).ok().map(ChildOf::parent);
            }
            found.push(hit);
        }
        let names: Vec<String> = found.iter().map(|e| self.describe(*e)).collect();
        Err(format!(
            "it is covered: the pointer is over {}",
            names.join(", ")
        ))
    }

    /// What a node is, for a message: the action of it or the nearest control round it.
    fn describe(&self, entity: Entity) -> String {
        let mut at = Some(entity);
        while let Some(e) = at {
            if let Ok((.., Some(click), _, _, _)) = self.nodes.get(e) {
                return format!("`{}`", click.0.render(&self.data, &self.formatters));
            }
            at = self.parents.get(e).ok().map(ChildOf::parent);
        }
        format!("{entity}")
    }
}

/// Whether `condition` holds on the UI's data: [`UiData::condition`], and comparisons of numbers.
pub fn holds(data: &UiData, condition: &str) -> bool {
    for operator in ["<=", ">=", "<", ">"] {
        if let Some((key, wanted)) = condition.split_once(operator) {
            let Some(UiValue::Number(found)) = data.get(key.trim()) else {
                return false;
            };
            let Ok(wanted) = wanted.trim().parse::<f64>() else {
                return false;
            };
            return match operator {
                "<=" => *found <= wanted,
                ">=" => *found >= wanted,
                "<" => *found < wanted,
                _ => *found > wanted,
            };
        }
    }
    data.condition(condition)
}

/// What the key on the left of `condition` holds, to say what was there instead.
fn found(data: &UiData, condition: &str) -> String {
    let key = condition
        .trim_start_matches('!')
        .split(['=', '!', '<', '>'])
        .next()
        .unwrap_or("")
        .trim();
    match data.get(key) {
        Some(value) => format!("{key} is {}", osp_ui::format::plain(value)),
        None => format!("{key} is not set"),
    }
}

/// What the screen shows that a player would see change: the watched data, and whether a text box
/// has the keys (its caret).
fn watched(data: &UiData, focus: &TextFocus) -> Vec<String> {
    (WATCHED.iter())
        .map(|key| data.get(key).map(|v| format!("{v:?}")).unwrap_or_default())
        .chain([focus.typing().to_string()])
        .collect()
}

fn render_offscreen(
    mut commands: Commands,
    mut runner: ResMut<Runner>,
    mut images: ResMut<Assets<Image>>,
) {
    let (width, height) = capture_size();
    let image = Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
    runner.target = images.add(image);
    commands.insert_resource(ViewOutput(Some(runner.target.clone())));
}

#[allow(clippy::too_many_arguments)]
fn run(
    mut commands: Commands,
    mut runner: ResMut<Runner>,
    time: Res<Time<Real>>,
    status: Res<EditorStatus>,
    controls: Controls,
    mut input: Input,
    mut pointer: ResMut<ScriptedPointer>,
    mut actions: MessageReader<UiAction>,
    mut editor_steps: MessageWriter<EditorStep>,
    mut editor_failed: ResMut<EditorStepFailed>,
    (last_launch, centre): (Res<LastLaunch>, Res<SpaceCentre>),
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs_f64();
    let frame_ms = time.delta_secs_f64() * 1000.0;
    let runner = &mut *runner;
    let data = &*controls.data;
    let focus = &*controls.focus;

    if let Some((passed, frames)) = runner.ending.as_mut() {
        // Pictures take a few frames to be read back and written.
        *frames += 1;
        if *frames == 20 {
            let passed = *passed;
            for (key, label, range) in std::mem::take(&mut runner.tracked) {
                runner.report.measure(label, key, range);
            }
            runner.report.finish(now - runner.started.unwrap_or(now));
            // What was launched, to fly it again in a test of the simulation.
            if let Some(point) = last_launch.point()
                && let Ok(text) = serde_json::to_string_pretty(&point.craft)
            {
                let _ = std::fs::write(runner.out.join("craft.json"), text);
            }
            let path = runner.out.join("report.json");
            match serde_json::to_string_pretty(&runner.report) {
                Ok(text) => {
                    if let Err(e) = std::fs::write(&path, text) {
                        error!("playtest: cannot write {}: {e}", path.display());
                    }
                }
                Err(e) => error!("playtest: cannot write the report: {e}"),
            }
            info!(
                "playtest {}: {}",
                runner.play.name,
                runner.report.headline()
            );
            exit.write(if passed {
                AppExit::Success
            } else {
                AppExit::error()
            });
        }
        return;
    }

    // What the editor has building, and what the UI said it did.
    runner.quiet = if status.is_busy() {
        0
    } else {
        runner.quiet + 1
    };
    if status.is_busy() {
        runner.busy_at = now;
    }
    let seen = watched(data, focus);
    if seen != runner.last_seen {
        runner.last_seen = seen;
        runner.changed_at = now;
        runner.stable = 0;
    } else {
        runner.stable += 1;
    }
    for (key, _, range) in &mut runner.tracked {
        if let Some(UiValue::Number(value)) = data.get(key) {
            let [low, high, _] = range.unwrap_or([*value; 3]);
            *range = Some([low.min(*value), high.max(*value), *value]);
        }
    }
    let fired: Vec<String> = actions.read().map(|a| a.0.clone()).collect();
    if let Some(step) = &mut runner.current {
        step.frames.push(frame_ms);
        for action in fired {
            if !step.actions.contains(&action) {
                step.actions.push(action);
            }
        }
    }
    if runner.started.is_some() {
        runner.report.frame(frame_ms);
    }
    // The screen answered the input: the first frame the watched data or an action says so.
    if let (Some(at), Some(step)) = (runner.input_at, &mut runner.current)
        && (!step.actions.is_empty() || watched(data, focus) != runner.before)
    {
        step.answer_ms = Some((now - at) * 1000.0);
        runner.input_at = None;
    }

    // Give the game a moment to open before the player starts.
    if runner.started.is_none() {
        if now < 2.0 || runner.quiet < 10 {
            return;
        }
        runner.started = Some(now);
    }

    loop {
        if runner.ops.is_empty() {
            if runner.next >= runner.play.steps.len() {
                runner.ending = Some((true, 0));
                return;
            }
            let next = runner.next;
            runner.next += 1;
            runner.expand(next);
        }
        let Some(op) = runner.ops.front().cloned() else {
            return;
        };
        if runner.op_frames == 0 {
            runner.op_started = now;
        }
        runner.op_frames += 1;
        let waited = now - runner.op_started;
        // Whether the op is done, and whether the next can go on in the same frame.
        let (done, same_frame) = match op {
            Op::Begin(i) => {
                let play = runner.play.steps[i];
                runner.current = Some(StepRecord::new(i, play));
                runner.before = watched(data, focus);
                runner.at_begin = runner.before.clone();
                (true, true)
            }
            Op::Find(spot) => match spot {
                Spot::Ui(what) => match controls.find(what, runner.screen) {
                    Ok(aim) => {
                        runner.aim = Some(aim);
                        (true, true)
                    }
                    Err(e) => {
                        runner.fail(e, &mut commands);
                        return;
                    }
                },
                Spot::Screen(x, y) => {
                    runner.aim = Some(Aim {
                        at: Vec2::new(x, y) * runner.screen,
                        width: 200.0,
                        entity: None,
                    });
                    (true, true)
                }
                Spot::Building(id) => {
                    let Some(at) = centre.spot(id) else {
                        runner.fail(format!("the space centre shows no `{id}`"), &mut commands);
                        return;
                    };
                    // A building is drawn at least this big.
                    runner.aim = Some(Aim {
                        at,
                        width: 60.0,
                        entity: None,
                    });
                    (true, true)
                }
                Spot::By(x, y) => {
                    // From where a drag began, however often it is looked for on the way.
                    let from = runner.pressed.unwrap_or(runner.mouse);
                    runner.aim = Some(Aim {
                        at: from + Vec2::new(x, y),
                        width: 30.0,
                        entity: None,
                    });
                    (true, true)
                }
                Spot::Ring(..) | Spot::Surface(..) | Spot::Part(..) | Spot::Grip(..)
                    if !runner.asked =>
                {
                    // The editor knows where it drew them: it puts the view's pointer there.
                    let action = match spot {
                        Spot::Ring(part, node) => Action::EditorPointAtNode(part, node),
                        Spot::Surface(part, surface, u, v) => {
                            Action::EditorPointAtSurface(part, surface, u, v)
                        }
                        Spot::Part(part) => Action::EditorPointAtPart(part),
                        Spot::Grip(size) => Action::EditorPointAtGrip(size, 0.0, 0.0),
                        _ => unreachable!(),
                    };
                    editor_steps.write(EditorStep(action));
                    runner.asked = true;
                    (false, false)
                }
                Spot::Ring(..) | Spot::Surface(..) | Spot::Part(..) | Spot::Grip(..) => {
                    runner.asked = false;
                    if let Some(e) = editor_failed.0.take() {
                        runner.fail(e, &mut commands);
                        return;
                    }
                    let Some(at) = pointer.0 else {
                        runner.fail(format!("the editor did not find {spot:?}"), &mut commands);
                        return;
                    };
                    // A ring snaps from 24 px round it; a part is about this big on screen.
                    let width = match spot {
                        Spot::Ring(..) => 48.0,
                        Spot::Part(..) => 40.0,
                        // Handles are drawn about this big.
                        Spot::Grip(..) => 16.0,
                        _ => 30.0,
                    };
                    runner.aim = Some(Aim {
                        at,
                        width,
                        entity: None,
                    });
                    (true, true)
                }
            },
            Op::Move => {
                let Some(aim) = runner.aim.clone() else {
                    runner.fail("nothing to point at".into(), &mut commands);
                    return;
                };
                let from = runner.mouse;
                runner.move_to(aim.at, &mut pointer, &mut input);
                let homed = !runner.on_mouse;
                if let Some(step) = &mut runner.current {
                    step.point(from, aim.at, aim.width, homed);
                }
                runner.on_mouse = true;
                runner.moved_at = Some(now);
                (true, false)
            }
            Op::Via => {
                let to = runner.mouse.lerp(runner.screen * 0.5, 0.5);
                runner.move_to(to, &mut pointer, &mut input);
                (true, false)
            }
            Op::Settle => {
                if runner.op_frames >= 2
                    && runner.quiet >= QUIET_FRAMES
                    && runner.stable >= QUIET_FRAMES
                {
                    // Done when the last build it started was, and the screen last changed.
                    let done_at = runner.busy_at.max(runner.changed_at) + frame_ms / 1000.0;
                    let moved_at = runner.moved_at.take();
                    if let Some(step) = &mut runner.current {
                        if let Some(at) = step.input_at {
                            if step.settled_ms.is_none() {
                                step.settled_ms = Some((done_at - at).max(0.0) * 1000.0);
                            }
                        } else if let Some(moved) = moved_at
                            && runner.busy_at > moved
                        {
                            // The pointer on a spot made the editor build what would go there.
                            step.preview_ms = Some((done_at - moved) * 1000.0);
                        }
                    }
                    (true, false)
                } else if waited > SETTLE_LIMIT {
                    runner.fail(
                        format!("the editor was still building after {SETTLE_LIMIT} s"),
                        &mut commands,
                    );
                    return;
                } else {
                    (false, false)
                }
            }
            // Picking may take a frame or two to see the pointer where it went: it is told again,
            // and only a control still not under it after a second is covered.
            Op::Hover => match runner.aim.as_ref().and_then(|a| a.entity) {
                Some(entity) => match controls.over(entity) {
                    Ok(()) => (true, true),
                    Err(_) if waited < 1.0 => {
                        let (target, at) = (runner.target.clone(), runner.mouse);
                        input.pointer(&target, at, PointerAction::Move { delta: Vec2::ZERO });
                        (false, false)
                    }
                    Err(e) => {
                        runner.fail(e, &mut commands);
                        return;
                    }
                },
                None => (true, true),
            },
            Op::Press => {
                let at = runner.mouse;
                runner.pressed = Some(at);
                input.buttons.press(MouseButton::Left);
                let target = runner.target.clone();
                input.pointer(&target, at, PointerAction::Press(PointerButton::Primary));
                (true, false)
            }
            Op::Release => {
                let at = runner.mouse;
                runner.pressed = None;
                input.buttons.release(MouseButton::Left);
                let target = runner.target.clone();
                input.pointer(&target, at, PointerAction::Release(PointerButton::Primary));
                runner.mark_input(now, data, focus);
                (true, false)
            }
            Op::Frames(n) => (runner.op_frames > n, false),
            Op::Notch(left) => {
                // A notch a frame, as a hand turns it.
                input.wheel.write(MouseWheel {
                    unit: MouseScrollUnit::Line,
                    x: 0.0,
                    y: left.signum() as f32,
                    window: Entity::PLACEHOLDER,
                    phase: TouchPhase::Moved,
                });
                if let Some(step) = &mut runner.current
                    && runner.op_frames == 1
                {
                    step.notches = left.unsigned_abs() as usize;
                }
                if runner.op_frames == left.unsigned_abs() {
                    runner.mark_input(now, data, focus);
                }
                (runner.op_frames >= left.unsigned_abs(), false)
            }
            Op::Key(key, pressed) => {
                input.key(
                    key,
                    Key::Unidentified(NativeKey::Unidentified),
                    None,
                    pressed,
                );
                if pressed {
                    if let Some(step) = &mut runner.current {
                        step.keys(1, runner.on_mouse);
                    }
                    runner.on_mouse = false;
                    runner.mark_input(now, data, focus);
                }
                (true, false)
            }
            Op::Type(text) => {
                for c in text.chars() {
                    let typed = c.to_string();
                    let (code, key, text) = match c {
                        '\n' => (KeyCode::Enter, Key::Enter, None),
                        _ => (
                            KeyCode::Unidentified(NativeKeyCode::Unidentified),
                            Key::Character(typed.as_str().into()),
                            Some(typed.as_str()),
                        ),
                    };
                    input.key(code, key.clone(), text, true);
                    input.key(code, key, text, false);
                }
                if let Some(step) = &mut runner.current {
                    step.keys(text.chars().count(), runner.on_mouse);
                }
                runner.on_mouse = false;
                runner.mark_input(now, data, focus);
                (true, false)
            }
            Op::Until(condition, limit) => {
                if holds(data, condition) {
                    (true, true)
                } else if waited > limit {
                    let found = found(data, condition);
                    runner.fail(
                        format!("waited {limit} s for `{condition}`, but {found}"),
                        &mut commands,
                    );
                    return;
                } else {
                    (false, false)
                }
            }
            Op::Expect(condition) => {
                if !holds(data, condition) {
                    let found = found(data, condition);
                    runner.fail(
                        format!("`{condition}` does not hold: {found}"),
                        &mut commands,
                    );
                    return;
                }
                (true, true)
            }
            Op::Picture(name) => {
                runner.picture(name, &mut commands);
                (true, false)
            }
            Op::Goal(name) => {
                let at = now - runner.started.unwrap_or(now);
                runner.report.goal(name, at);
                (true, true)
            }
            Op::Track(key, label) => {
                runner.tracked.push((key, label, None));
                (true, true)
            }
            Op::Wait(seconds) => (waited >= seconds, false),
            Op::Hope(condition, seconds, why) => {
                let at = now - runner.started.unwrap_or(now);
                if !holds(data, condition) {
                    let found = found(data, condition);
                    runner.report.letdown(why, condition, at, found);
                    // Waited out all the same, so the rest of the go is as long.
                    runner.ops.pop_front();
                    runner.ops.push_front(Op::Wait((seconds - waited).max(0.0)));
                    runner.op_frames = 0;
                    return;
                }
                if waited < seconds {
                    (false, false)
                } else {
                    runner.report.hope_met(why, condition);
                    (true, seconds == 0.0)
                }
            }
            Op::End => {
                if let Some(mut step) = runner.current.take() {
                    let after = watched(data, focus);
                    let keys = WATCHED.iter().copied().chain(["text box focus"]);
                    step.changed = (keys.zip(runner.at_begin.iter().zip(&after)))
                        .filter(|(_, (a, b))| a != b)
                        .map(|(key, _)| key.to_string())
                        .collect();
                    if let Some(UiValue::Text(notice)) = data.get("editor.notice")
                        && step.changed.iter().any(|k| k == "editor.notice")
                    {
                        step.notice = Some(notice.clone());
                    }
                    runner.report.step(step);
                }
                runner.input_at = None;
                (true, true)
            }
        };
        if !done {
            return;
        }
        runner.ops.pop_front();
        runner.op_frames = 0;
        if !same_frame {
            return;
        }
    }
}

impl Runner {
    /// An input went in now: time how long the screen takes to answer it.
    fn mark_input(&mut self, now: f64, data: &UiData, focus: &TextFocus) {
        self.input_at = Some(now);
        if let Some(step) = &mut self.current {
            step.input_at = Some(now);
            // A press already answered (a drag shows what is in hand before it is let go) does not
            // count: the answer is to the input that finishes the step.
            step.actions.clear();
        }
        self.before = watched(data, focus);
    }
}
