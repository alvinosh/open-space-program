//! The playtests: each a player's go at something, as they would go about it.

use bevy::input::keyboard::KeyCode;

use super::{Play, Playtest, Spot};

pub const PLAYS: &[Playtest] = &[
    SPACE_CENTRE,
    CAREER_CENTRE,
    FIRST_ROCKET,
    RESIZE_TANK,
    TWO_STAGES,
    MIRRORED_WINGS,
    SOLID_BOOSTER,
    BOOSTER_CURVE,
];

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
        // A game starts at the space centre: rockets are built in the Vehicle Assembly.
        Play::Until("centre.open", 20.0),
        Play::Picture("space_centre.png"),
        Play::Click(Spot::Building("vehicle_assembly")),
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
        // To the space centre from the pause menu, and back to flying the rocket from there.
        Play::Key(KeyCode::Escape),
        Play::Until("menu.screen == pause", 5.0),
        Play::Click(Spot::Ui("centre.open")),
        Play::Until("centre.open", 10.0),
        Play::Expect("centre.can_fly"),
        Play::Picture("back_at_the_centre.png"),
        Play::Click(Spot::Ui("centre.fly")),
        Play::Until("!centre.open", 10.0),
        Play::Goal("to the space centre and back"),
        // Saved on the way out to the title screen; Continue carries on at the space centre.
        Play::Key(KeyCode::Escape),
        Play::Until("menu.screen == pause", 5.0),
        Play::Click(Spot::Ui("ask.leave:game.exit")),
        Play::Click(Spot::Ui("dialog.answer:0")),
        Play::Until("menu.screen == title", 10.0),
        Play::Click(Spot::Ui("game.continue")),
        Play::Until("centre.open", 20.0),
        Play::Expect("centre.can_fly"),
        Play::Picture("continued.png"),
        Play::Goal("carried on at the space centre"),
        // The Tracking Station: the rocket on the map, and flown again from there.
        Play::Click(Spot::Building("tracking")),
        Play::Until("centre.inside == tracking", 5.0),
        Play::Until("centre.has_tracked", 5.0),
        Play::Picture("tracking.png"),
        Play::Click(Spot::Ui("vessel.fly:")),
        Play::Until("!centre.open", 10.0),
        Play::Picture("flown_from_tracking.png"),
        Play::Goal("flying again from the Tracking Station"),
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
        Play::Until("centre.open", 20.0),
        Play::Click(Spot::Building("vehicle_assembly")),
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
        Play::Until("centre.open", 20.0),
        Play::Click(Spot::Building("vehicle_assembly")),
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

/// Wings put on with mirror symmetry stay a pair: whatever is done to one, the wing moved along the
/// body, its settings typed in, its handle dragged or the wing taken off, is done to the other as
/// its mirror image.
const MIRRORED_WINGS: Playtest = Playtest {
    name: "mirrored_wings",
    about: "A player starts a plane (a capsule and a tank), puts a pair of wings on the tank with \
            mirror symmetry, then moves one wing along the body, types its span, drags the other's \
            handle and takes one off, then undoes that; the other wing follows each time.",
    stuck_until: None,
    steps: &[
        Play::Until("menu.screen == title", 60.0),
        Play::Click(Spot::Ui("menu.open:new_game")),
        Play::Until("menu.screen == new_game", 5.0),
        Play::Click(Spot::Ui("ask.leave:game.new")),
        // Planes are built in the Hangar, for the runway.
        Play::Until("centre.open", 20.0),
        Play::Click(Spot::Building("hangar")),
        Play::Until("ui.shown.editor", 20.0),
        Play::Expect("editor.place == runway"),
        Play::Click(Spot::Ui("editor.new:plane")),
        Play::Until("editor.has_vessel", 5.0),
        // A capsule for its nose, a tank behind it for its body.
        Play::Click(Spot::Ui("editor.group:pods")),
        Play::Click(Spot::Ui("editor.add:pod:C-1 Capsule")),
        Play::Until("editor.parts == 1", 5.0),
        Play::Click(Spot::Ui("editor.group:tanks")),
        Play::Drag(Spot::Ui("palette:tank"), Spot::Ring("pod", "bottom")),
        Play::Until("editor.parts == 2", 5.0),
        // A plane starts with mirror symmetry on.
        Play::Expect("editor.mirror"),
        Play::Click(Spot::Ui("editor.group:aero")),
        Play::Click(Spot::Ui("editor.add:wing")),
        Play::Click(Spot::Surface("tank", "barrel", 0.0, 0.4)),
        Play::Until("editor.parts == 4", 5.0),
        Play::Picture("wings.png"),
        Play::Goal("a pair of wings"),
        // Taken hold of and put further along the body: both go.
        Play::Drag(
            Spot::Part("wing"),
            Spot::Surface("tank", "barrel", 0.0, 0.7),
        ),
        Play::Until("!editor.holding", 5.0),
        Play::Expect("editor.parts == 4"),
        Play::Picture("moved.png"),
        Play::Goal("moved along the body, both"),
        // One wing's settings: they say they change both, and its span is typed in.
        Play::Click(Spot::Part("wing")),
        Play::Until("editor.rail == part", 5.0),
        Play::Expect("editor.part.name == Wing x2"),
        Play::Click(Spot::Ui("editor.set:span:")),
        Play::Type("6\n"),
        Play::Until("editor.part.rows[2].value == 6.00 m", 5.0),
        Play::Picture("span_typed.png"),
        // The other wing, behind the body from here: the view turned round to it, it has it too.
        Play::Drag(Spot::Screen(0.5, 0.2), Spot::By(500.0, 0.0)),
        Play::Picture("other_side.png"),
        Play::Click(Spot::Part("wing_2")),
        Play::Until("editor.part.rows[2].value == 6.00 m", 5.0),
        Play::Expect("editor.part.name == Wing x2"),
        Play::Goal("span typed into one, both 6 m"),
        // Its tip dragged by the handle: both change.
        Play::Drag(Spot::Grip("span"), Spot::By(0.0, 60.0)),
        Play::Until("editor.part.rows[2].value != 6.00 m", 5.0),
        Play::Picture("span_dragged.png"),
        Play::Click(Spot::Part("wing")),
        Play::Until("editor.part.rows[2].value != 6.00 m", 5.0),
        Play::Expect("editor.part.name == Wing x2"),
        Play::Goal("handle dragged on one, both changed"),
        // Taken off: both go.
        Play::Key(KeyCode::Delete),
        Play::Until("editor.parts == 2", 5.0),
        Play::Picture("removed.png"),
        Play::Goal("both taken off"),
        // And back, both, with one undo.
        Play::Chord(KeyCode::ControlLeft, KeyCode::KeyZ),
        Play::Until("editor.parts == 4", 5.0),
        Play::Goal("both back with one undo"),
    ],
};

/// The space centre, where a game starts: a player looks round it, goes into the Vehicle Assembly
/// and back out, into the Hangar and back out, tries a building not built yet, and leaves for the
/// title screen, as nothing has been launched.
const SPACE_CENTRE: Playtest = Playtest {
    name: "space_centre",
    about: "A player starts a sandbox and lands at the space centre, looks round it, goes into the \
            Vehicle Assembly and back, into the Hangar and back, into the Tracking Station and back, \
            tries Research, and goes back to \
            the title screen.",
    stuck_until: None,
    steps: &[
        Play::Until("menu.screen == title", 60.0),
        Play::Click(Spot::Ui("menu.open:new_game")),
        Play::Until("menu.screen == new_game", 5.0),
        Play::Click(Spot::Ui("ask.leave:game.new")),
        Play::Until("centre.open", 20.0),
        Play::Expect("!ui.shown.editor"),
        Play::Picture("arrived.png"),
        Play::Goal("at the space centre"),
        // Looked round: the view turns with a drag on the sky.
        Play::Drag(Spot::Screen(0.6, 0.15), Spot::By(-260.0, 0.0)),
        Play::Picture("looked_round.png"),
        // Into the Vehicle Assembly by clicking the building, and back out.
        Play::Click(Spot::Building("vehicle_assembly")),
        Play::Until("ui.shown.editor", 20.0),
        Play::Expect("editor.place == pad"),
        Play::Picture("vehicle_assembly.png"),
        Play::Goal("in the Vehicle Assembly"),
        Play::Key(KeyCode::Escape),
        Play::Until("centre.open", 20.0),
        Play::Expect("!ui.shown.editor"),
        Play::Goal("back at the space centre"),
        // Into the Hangar, from its row in the panel this time, and back out by its Back.
        Play::Click(Spot::Ui("centre.enter:hangar")),
        Play::Until("ui.shown.editor", 20.0),
        Play::Expect("editor.place == runway"),
        Play::Picture("hangar.png"),
        Play::Click(Spot::Ui("menu.back")),
        Play::Until("centre.open", 20.0),
        Play::Goal("in the Hangar and back"),
        // The Tracking Station: the map, with nothing in flight yet; Esc back to the Cape.
        Play::Click(Spot::Building("tracking")),
        Play::Until("centre.inside == tracking", 5.0),
        Play::Expect("!centre.has_vessels"),
        Play::Picture("tracking_empty.png"),
        Play::Key(KeyCode::Escape),
        Play::Until("!centre.inside", 5.0),
        Play::Expect("centre.open"),
        Play::Goal("in the Tracking Station and back"),
        // A building only for a career says so.
        Play::Click(Spot::Building("research")),
        Play::Until("centre.notice", 5.0),
        Play::Expect("centre.open"),
        Play::Picture("research.png"),
        // Nothing launched yet: Esc goes back to the title screen.
        Play::Key(KeyCode::Escape),
        Play::Until("menu.screen == title", 20.0),
        Play::Expect("!centre.open"),
        Play::Goal("back to the title screen"),
    ],
};

/// A career's space centre: its funds and science in sight, the tech tree in Research, and the
/// contracts in Mission Control, one taken on there; each comes back to the space centre.
const CAREER_CENTRE: Playtest = Playtest {
    name: "career_centre",
    about: "A player starts a career and lands at the space centre, upgrades the builder, opens the \
            tech tree in Research \
            and comes back, takes on a contract in Mission Control and comes back, and goes back to \
            the title screen.",
    stuck_until: None,
    steps: &[
        Play::Until("menu.screen == title", 60.0),
        Play::Click(Spot::Ui("menu.open:new_game")),
        Play::Until("menu.screen == new_game", 5.0),
        Play::Click(Spot::Ui("game.kind:career")),
        Play::Until("menu.career", 5.0),
        Play::Click(Spot::Ui("ask.leave:game.new")),
        Play::Until("centre.open", 20.0),
        // What the career has to spend shows at once, and its facilities to upgrade, not in the
        // hangar any more.
        Play::Expect("career.active"),
        Play::Until("career.facilities", 5.0),
        Play::Picture("career_centre.png"),
        Play::Goal("at a career's space centre"),
        // The builder upgraded from there: the Vehicle Assembly and the Hangar grow at once.
        Play::Click(Spot::Ui("career.upgrade:builder")),
        Play::Until("career.facilities[1].level == Level 2 of 4", 5.0),
        Play::Picture("builder_upgraded.png"),
        Play::Goal("the builder upgraded at the space centre"),
        // Research: the tech tree, over the view; Esc back.
        Play::Click(Spot::Building("research")),
        Play::Until("ui.shown.tech_tree", 10.0),
        Play::Picture("research.png"),
        Play::Goal("the tech tree in Research"),
        Play::Key(KeyCode::Escape),
        Play::Until("!ui.shown.tech_tree", 5.0),
        Play::Expect("centre.open"),
        // Mission Control: the contracts in the panel, one taken on; Back to the buildings.
        Play::Click(Spot::Building("mission_control")),
        Play::Until("centre.inside == missions", 5.0),
        Play::Expect("career.has_offers"),
        Play::Click(Spot::Ui("career.accept:")),
        Play::Until("career.has_taken", 5.0),
        Play::Picture("mission_control.png"),
        Play::Goal("a contract taken on in Mission Control"),
        Play::Click(Spot::Ui("menu.back")),
        Play::Until("!centre.inside", 5.0),
        Play::Expect("centre.open"),
        // Nothing launched yet: Esc goes back to the title screen.
        Play::Key(KeyCode::Escape),
        Play::Until("menu.screen == title", 20.0),
        Play::Goal("back to the title screen"),
    ],
};

/// A rocket on a solid booster: a capsule on a booster from the parts list, its thrust looked at
/// in its settings, four fins; launched, and once lit nothing stops it until its grain is gone.
const SOLID_BOOSTER: Playtest = Playtest {
    name: "solid_booster",
    about: "A player builds a capsule on a solid booster with four fins, launches it, tries to cut \
            the throttle, and watches it burn until it is empty.",
    stuck_until: None,
    steps: &[
        Play::Until("menu.screen == title", 60.0),
        Play::Click(Spot::Ui("menu.open:new_game")),
        Play::Until("menu.screen == new_game", 5.0),
        Play::Click(Spot::Ui("ask.leave:game.new")),
        Play::Until("centre.open", 20.0),
        Play::Click(Spot::Building("vehicle_assembly")),
        Play::Until("ui.shown.editor", 20.0),
        Play::Click(Spot::Ui("editor.new")),
        Play::Until("editor.has_vessel", 5.0),
        Play::Click(Spot::Ui("editor.group:pods")),
        Play::Click(Spot::Ui("editor.add:pod:C-1 Capsule")),
        Play::Until("editor.parts == 1", 5.0),
        // The booster, from the engines: one part, the grain and its nozzle.
        Play::Click(Spot::Ui("editor.group:engines")),
        Play::Drag(
            Spot::Ui("palette:solid_booster"),
            Spot::Ring("pod", "bottom"),
        ),
        Play::Until("editor.parts == 2", 5.0),
        Play::Click(Spot::Part("solid_booster")),
        Play::Until("editor.rail == part", 5.0),
        Play::Picture("booster_settings.png"),
        Play::Goal("a capsule on a solid booster"),
        // Four fins round its base, from the parts list again.
        Play::Click(Spot::Ui("editor.parts")),
        Play::Until("editor.rail == parts", 5.0),
        Play::Click(Spot::Ui("editor.group:aero")),
        Play::Click(Spot::Ui("editor.symmetry")),
        Play::Click(Spot::Ui("editor.symmetry")),
        Play::Click(Spot::Ui("editor.symmetry")),
        Play::Click(Spot::Ui("editor.add:fin")),
        Play::Click(Spot::Surface("solid_booster", "barrel", 0.625, 0.12)),
        Play::Until("editor.parts == 6", 5.0),
        Play::Picture("booster_rocket.png"),
        Play::Click(Spot::Ui("editor.launch")),
        Play::Until("!ui.shown.editor", 20.0),
        Play::Picture("booster_pad.png"),
        // SAS on, and the next sequence lights it.
        Play::Key(KeyCode::KeyT),
        Play::Key(KeyCode::Space),
        Play::Track("vessel.altitude", "height, m"),
        Play::Track("vessel.twr", "thrust to weight"),
        Play::Until("vessel.altitude > 100", 30.0),
        Play::Goal("liftoff on a solid"),
        // The throttle cut: a solid takes no notice.
        Play::Key(KeyCode::KeyX),
        Play::Wait(3.0),
        Play::Expect("vessel.engines_running"),
        Play::Picture("booster_burning.png"),
        Play::Goal("it cannot be stopped"),
        // Faster time until it burns out.
        Play::Key(KeyCode::Period),
        Play::Key(KeyCode::Period),
        Play::Until("vessel.engine_level < 0.01", 240.0),
        Play::Key(KeyCode::Slash),
        Play::Goal("burnt out"),
        Play::Hope(
            "vessel.altitude > 5000",
            0.0,
            "a booster as the parts list gives it takes a capsule well up",
        ),
    ],
};

/// In a Realistic game a solid booster's thrust curve, the shape its grain is cast in, is set in
/// its settings: a preset, then one of its points by hand.
const BOOSTER_CURVE: Playtest = Playtest {
    name: "booster_curve",
    about: "A player starts a Realistic game, puts a solid booster under a capsule, picks the burn \
            with a dip for max-Q, and lowers its thrust at the end by hand.",
    stuck_until: None,
    steps: &[
        Play::Until("menu.screen == title", 60.0),
        Play::Click(Spot::Ui("menu.open:new_game")),
        Play::Until("menu.screen == new_game", 5.0),
        Play::Click(Spot::Ui("mods.mode:realistic")),
        Play::Until("!mods.modes[0].on", 5.0),
        Play::Click(Spot::Ui("ask.leave:game.new")),
        Play::Until("centre.open", 60.0),
        Play::Click(Spot::Building("vehicle_assembly")),
        Play::Until("ui.shown.editor", 20.0),
        Play::Click(Spot::Ui("editor.new")),
        Play::Until("editor.has_vessel", 5.0),
        Play::Click(Spot::Ui("editor.group:pods")),
        Play::Click(Spot::Ui("editor.add:pod:C-1 Capsule")),
        Play::Until("editor.parts == 1", 5.0),
        Play::Click(Spot::Ui("editor.group:engines")),
        Play::Drag(
            Spot::Ui("palette:solid_booster"),
            Spot::Ring("pod", "bottom"),
        ),
        Play::Until("editor.parts == 2", 5.0),
        Play::Click(Spot::Part("solid_booster")),
        Play::Until("editor.rail == part", 5.0),
        Play::Goal("a booster in a Realistic game"),
        // Its burn: a preset first.
        Play::Click(Spot::Ui("editor.expand:motor_curve")),
        Play::Click(Spot::Ui("editor.set:motor_curve:max_q")),
        Play::Until("editor.part.problem == ", 5.0),
        Play::Picture("max_q.png"),
        Play::Goal("a burn with a dip for max-Q"),
        // Then less thrust at the end, by hand: its own curve.
        Play::Click(Spot::Ui("editor.set:motor_curve_4:")),
        Play::Type("50\n"),
        Play::Until("editor.part.rows[12].value == Your own", 5.0),
        Play::Expect("editor.part.rows[17].value == 50%"),
        Play::Picture("own_curve.png"),
        Play::Goal("its own curve"),
    ],
};
