//! The playtests: each a player's go at something, as they would go about it.

use bevy::input::keyboard::KeyCode;

use super::{Play, Playtest, Spot};

pub const PLAYS: &[Playtest] = &[FIRST_ROCKET, RESIZE_TANK, TWO_STAGES];

/// A new player's first rocket. From the title screen they start a sandbox, start a rocket, take a
/// capsule from the parts list, drag a tank under it and an engine under that, put four fins round
/// the tank with radial symmetry, launch, and fly it the way the keys say: SAS on, full throttle,
/// next sequence.
const FIRST_ROCKET: Playtest = Playtest {
    name: "first_rocket",
    about: "A new player starts a sandbox, builds a capsule, a tank, an engine and four fins from \
            the parts list, launches it and flies it straight up.",
    stuck_until: None,
    steps: &[
        Play::Until("menu.screen == title", 60.0),
        Play::Picture("title.png"),
        Play::Click(Spot::Ui("menu.open:new_game")),
        Play::Until("menu.screen == new_game", 5.0),
        Play::Picture("new_game.png"),
        // A first game is Casual (the first mode, `default: true`).
        Play::Expect("mods.modes[0].on"),
        Play::Click(Spot::Ui("ask.leave:game.new")),
        Play::Until("ui.shown.editor", 20.0),
        Play::Picture("hangar.png"),
        Play::Click(Spot::Ui("editor.new")),
        Play::Until("editor.has_vessel", 5.0),
        Play::Goal("in the editor"),
        // The capsule first: it goes on as the root.
        Play::Click(Spot::Ui("editor.group:pods")),
        Play::Click(Spot::Ui("editor.add:pod:C-1 Capsule")),
        Play::Until("editor.parts == 1", 5.0),
        Play::Picture("capsule.png"),
        // A tank dragged from the list onto the ring under the capsule.
        Play::Click(Spot::Ui("editor.group:tanks")),
        Play::Drag(Spot::Ui("palette:tank"), Spot::Ring("pod", "bottom")),
        Play::Until("editor.parts == 2", 5.0),
        // An engine under the tank.
        Play::Click(Spot::Ui("editor.group:engines")),
        Play::Drag(
            Spot::Ui("palette:engine:K-1 Booster"),
            Spot::Ring("tank", "bottom"),
        ),
        Play::Until("editor.parts == 3", 5.0),
        // The tank, still as the list gave it, grows to feed the engine for a stage's burn.
        Play::Until("editor.height > 15", 5.0),
        Play::Picture("stack.png"),
        // Four fins round the bottom of the tank: symmetry up to four, a fin taken, put on.
        Play::Click(Spot::Ui("editor.group:aero")),
        Play::Click(Spot::Ui("editor.symmetry")),
        Play::Click(Spot::Ui("editor.symmetry")),
        Play::Click(Spot::Ui("editor.symmetry")),
        Play::Expect("editor.symmetry == x4"),
        Play::Click(Spot::Ui("editor.add:fin")),
        Play::Click(Spot::Surface("tank", "barrel", 0.625, 0.12)),
        Play::Until("editor.parts == 7", 5.0),
        Play::Picture("rocket.png"),
        Play::Goal("rocket built"),
        Play::Click(Spot::Ui("editor.launch")),
        Play::Until("!ui.shown.editor", 20.0),
        Play::Picture("pad.png"),
        Play::Goal("on the pad"),
        // As the keys say: SAS, full throttle, next sequence.
        Play::Key(KeyCode::KeyT),
        Play::Key(KeyCode::KeyZ),
        Play::Key(KeyCode::Space),
        Play::Track("vessel.altitude", "height, m"),
        Play::Track("nav.pitch", "pitch, degrees"),
        Play::Track("vessel.twr", "thrust to weight"),
        Play::Track("orbit.apoapsis", "highest point ahead, m"),
        Play::Until("vessel.altitude > 100", 30.0),
        Play::Goal("liftoff"),
        Play::Hope(
            "vessel.twr < 4",
            0.0,
            "a first rocket climbs briskly, without the crew warned of high g",
        ),
        Play::Hope(
            "nav.pitch > 60",
            60.0,
            "it goes up the way it points, with SAS on, through its whole burn",
        ),
        Play::Picture("flying.png"),
        Play::Hope(
            "vessel.altitude > 5000",
            0.0,
            "it gets high enough to feel like a flight",
        ),
    ],
};

/// Making a tank the size wanted, the two ways the editor offers: its length typed into its
/// settings, its width dragged by the handle on its side; then the width taken back with Ctrl+Z.
const RESIZE_TANK: Playtest = Playtest {
    name: "resize_tank",
    about: "A player puts a tank on show, types an 8 m length into its settings, drags it wider by \
            the handle on its side, and takes that back with Ctrl+Z.",
    stuck_until: None,
    steps: &[
        Play::Until("menu.screen == title", 60.0),
        Play::Click(Spot::Ui("menu.open:new_game")),
        Play::Until("menu.screen == new_game", 5.0),
        Play::Expect("mods.modes[0].on"),
        Play::Click(Spot::Ui("ask.leave:game.new")),
        Play::Until("ui.shown.editor", 20.0),
        Play::Click(Spot::Ui("editor.new")),
        Play::Until("editor.has_vessel", 5.0),
        Play::Click(Spot::Ui("editor.group:tanks")),
        Play::Click(Spot::Ui("editor.add:tank")),
        Play::Until("editor.parts == 1", 5.0),
        Play::Track("editor.height", "height, m"),
        Play::Track("editor.mass", "mass, kg"),
        Play::Picture("tank.png"),
        Play::Goal("a tank on show"),
        // Its settings, and the length typed in.
        Play::Click(Spot::Part("tank")),
        Play::Until("editor.rail == part", 5.0),
        Play::Picture("settings.png"),
        Play::Click(Spot::Ui("editor.set:length:")),
        Play::Type("8\n"),
        Play::Until("editor.height >= 7.5", 5.0),
        Play::Picture("longer.png"),
        Play::Goal("8 m long, typed"),
        // Wider by the handle on its side.
        Play::Drag(Spot::Grip("diameter"), Spot::By(80.0, 0.0)),
        Play::Picture("wider.png"),
        Play::Goal("wider by its handle"),
        Play::Chord(KeyCode::ControlLeft, KeyCode::KeyZ),
        Play::Picture("undone.png"),
        Play::Goal("wider taken back"),
    ],
};

/// A rocket in two stages: a capsule, a decoupler, an upper tank and engine, another decoupler,
/// a lower tank and engine, and fins; launched, the lower stage dropped when it burns out.
const TWO_STAGES: Playtest = Playtest {
    name: "two_stages",
    about: "A player builds a two-stage rocket from the top down (capsule, decoupler, tank, engine, \
            decoupler, tank, engine, four fins), launches it, and drops the first stage when it \
            burns out.",
    stuck_until: None,
    steps: &[
        Play::Until("menu.screen == title", 60.0),
        Play::Click(Spot::Ui("menu.open:new_game")),
        Play::Until("menu.screen == new_game", 5.0),
        Play::Expect("mods.modes[0].on"),
        Play::Click(Spot::Ui("ask.leave:game.new")),
        Play::Until("ui.shown.editor", 20.0),
        Play::Click(Spot::Ui("editor.new")),
        Play::Until("editor.has_vessel", 5.0),
        Play::Click(Spot::Ui("editor.group:pods")),
        Play::Click(Spot::Ui("editor.add:pod:C-1 Capsule")),
        Play::Until("editor.parts == 1", 5.0),
        // The upper stage.
        Play::Click(Spot::Ui("editor.group:structure")),
        Play::Drag(Spot::Ui("palette:decoupler"), Spot::Ring("pod", "bottom")),
        Play::Until("editor.parts == 2", 5.0),
        Play::Click(Spot::Ui("editor.group:tanks")),
        Play::Drag(Spot::Ui("palette:tank"), Spot::Ring("decoupler", "bottom")),
        Play::Until("editor.parts == 3", 5.0),
        Play::Click(Spot::Ui("editor.group:engines")),
        Play::Drag(
            Spot::Ui("palette:engine:K-1 Booster"),
            Spot::Ring("tank", "bottom"),
        ),
        Play::Until("editor.parts == 4", 5.0),
        Play::Picture("upper.png"),
        Play::Goal("upper stage"),
        // The lower stage. The view has come out to keep the grown craft in sight (T308).
        Play::Click(Spot::Ui("editor.group:structure")),
        Play::Drag(
            Spot::Ui("palette:decoupler"),
            Spot::Ring("engine", "bottom"),
        ),
        Play::Until("editor.parts == 5", 5.0),
        Play::Click(Spot::Ui("editor.group:tanks")),
        Play::Drag(
            Spot::Ui("palette:tank"),
            Spot::Ring("decoupler_2", "bottom"),
        ),
        Play::Until("editor.parts == 6", 5.0),
        Play::Click(Spot::Ui("editor.group:engines")),
        Play::Drag(
            Spot::Ui("palette:engine:K-1 Booster"),
            Spot::Ring("tank_2", "bottom"),
        ),
        Play::Until("editor.parts == 7", 5.0),
        Play::Click(Spot::Ui("editor.group:aero")),
        Play::Click(Spot::Ui("editor.symmetry")),
        Play::Click(Spot::Ui("editor.symmetry")),
        Play::Click(Spot::Ui("editor.symmetry")),
        Play::Click(Spot::Ui("editor.add:fin")),
        Play::Click(Spot::Surface("tank_2", "barrel", 0.625, 0.12)),
        Play::Until("editor.parts == 11", 5.0),
        Play::Picture("rocket.png"),
        Play::Goal("two stages built"),
        Play::Click(Spot::Ui("editor.launch")),
        Play::Until("!ui.shown.editor", 20.0),
        Play::Picture("pad.png"),
        Play::Key(KeyCode::KeyT),
        Play::Key(KeyCode::KeyZ),
        Play::Key(KeyCode::Space),
        Play::Track("vessel.altitude", "height, m"),
        Play::Track("nav.pitch", "pitch, degrees"),
        Play::Track("nav.speed", "speed, m/s"),
        Play::Track("orbit.apoapsis", "highest point ahead, m"),
        Play::Until("vessel.altitude > 100", 30.0),
        Play::Goal("liftoff"),
        Play::Hope(
            "nav.pitch > 60",
            15.0,
            "it goes up the way it points, with SAS on",
        ),
        // The first stage burns out: the next sequence drops it and lights the second, together.
        Play::Until("vessel.engine_level < 0.01", 240.0),
        Play::Picture("burnout.png"),
        Play::Goal("first stage burnt out"),
        Play::Key(KeyCode::Space),
        Play::Wait(5.0),
        Play::Picture("second_stage.png"),
        Play::Hope(
            "vessel.engine_level > 0.5",
            0.0,
            "the second stage lights when the player stages",
        ),
        // Faster time while the second stage burns, until it burns out.
        Play::Key(KeyCode::Period),
        Play::Key(KeyCode::Period),
        Play::Until("vessel.engine_level < 0.01", 180.0),
        Play::Key(KeyCode::Slash),
        Play::Picture("second_burnout.png"),
        Play::Goal("second stage burnt out"),
        Play::Hope(
            "orbit.apoapsis > 100000",
            0.0,
            "with its second stage it gets to space: its highest point is past 100 km",
        ),
    ],
};
