# Build editor, and the Saturn V built in it

Plan from 2026-10-30. The game has no way to build a craft: every craft is a JSON5 file written
by hand or by a script, and `docs/systems-plan.md` adds a dozen kinds of part (batteries,
antennas, legs, heat shields, parachutes) that a player must be able to pick and place. So the
editor comes first, and the first full test of it is the one you asked for: build the Saturn V
in the editor, save it, launch it, and fly Apollo 11 from the pad to the entry with the smoke
test of `docs/smoke-tests.md`.

Written for: you, and the later sessions that build each step. Each step is one turn: build it,
verify it, report back. Sizes (S, M, L, XL) are defined in `.tasks/README.md`. This covers the
editor items of the early feature checklist (now in `.tasks/`) and the editor sections of
`docs/vessel-overhaul.md`.

## What there is

- A part model that can describe any craft: geometry, connectors, node and surface joints,
  components, assemblies, templates, and `CraftDef::place` and `fit_fairings` as build tools
  (`osp_sim::craft`, `assembly`, `component`).
- A UI that is data: layouts in `assets/core/layouts`, values published under keys, buttons that
  fire action strings, drag and drop between nodes (`osp_ui`, `docs/modding.md`).
- An orbit camera (`CameraRig`), a vessel renderer that rebuilds when the vessel changes
  (`vessel_render.rs`), and picking of a part by ray cast (`part_info.rs`, right-click in
  flight).
- A scripted harness that drives the whole game with action strings and the pointer, and takes
  screenshots (`osp_game/src/smoke_test.rs`, `OSP_CAPTURE_*`). It needs a GPU, and is separate
  from the mission suite of `cargo smoke`.
- Per-sequence delta-v, burn time and thrust to weight (`Propulsion::stage_info`), the voxel aero
  for stability, and the structure loads, so the editor's numbers are the flight's numbers.

What is missing: any editing, a place to look at a craft that is not flying, craft files a
player can save and open, flying from a file, and (from `docs/systems-plan.md`) physics that
accepts any part at any pose. Today `PartTree::from_craft` refuses a tilted engine and a part
with two components, so a player could place things the game would then reject.

## Design

Three layers. Only the top one needs Bevy, so the lower two run in tests without a GPU.

1. The builder (`osp_sim::builder`). Operations on a draft craft: place a part at a node, on a
   surface at (u, v) with a standoff, inside a container, or radially; remove; move; rotate;
   pattern (n round the axis, a mirror); reroot. Each returns what changed, or refuses with the
   rule that stops it (overlap, node size or type, outside the fairing, a joint that cannot carry
   its load). Every operation writes plain parts, and nothing remembers a pattern afterwards, as
   in the overhaul plan. An undo log.
2. The editor session (`osp_sim::editor`). The draft (a `CraftDef`), the selection, the symmetry
   mode and rotation step, undo and redo, the numbers (mass, delta-v and burn time per sequence,
   thrust to weight, centre of mass and of pressure and the margin between them), and a list of
   checks (no engine, a stage that cannot light, an unattached part, thrust to weight under 1).
   It is driven by commands, and each command is an action string: `editor.place:<palette id>`,
   `editor.select:<part>`, `editor.rotate:<axis>:<degrees>`, `editor.symmetry:<n>`,
   `editor.param:<name>:<value>`, `editor.undo`, `editor.save`, and so on. The UI fires them, a
   recipe is a list of them, and the tests call them. Components describe their editable
   parameters (name, unit, range, step, choices) so the property panel is generated, and a new
   kind of part needs no editor code.
3. The editor in the game (`osp_game::editor`). A scene (a hangar, a floor, light), a camera
   (orbit, and the orthographic front, side and top), the draft drawn by the vessel renderer, a
   ghost under the pointer, picking, snapping, handles, and the layouts that publish the
   session's numbers and fire its commands.

Choices:

- The draft is drawn by building a `Vessel` from it after each edit and handing that to the
  existing renderer, which already rebuilds on change. It costs a rebuild (the voxel aero is
  about 12 ms for the Saturn V) and gives the real aero, mass and stability readouts for free.
  A lighter path can come if it is slow.
- Placement snaps. A part goes on a node or a surface, so a craft is always joined; the checks
  warn about balance and use, they do not stop a launch.
- Craft files go to a user folder (`OSP_USER_DIR`, a `user/` folder in a development checkout, the
  platform's data folder otherwise), loaded as one more mod layer above the others, so the menu
  lists them with the shipped craft and the modding rules hold. A saved craft is the draft's
  `CraftDef`, its sequences included.
- Parts unlock by the game's date and tech tree, as everything else does (`tech.rs`); sandbox
  shows all of them.
- Flat masses and hand-shaped parts stay possible (a part can carry a `structure.mass`, and a
  shape can be a model or a raw compound), and the editor marks them. The Saturn V's first
  build has some; the procedural sizers of the systems plan replace them, and the end of that
  plan forbids them.

## Steps

Milestone 1, the base:

E1. (M) The build smoke tests, first version, while most of it fails. A second suite next to the
    missions (`crates/osp_sim/tests/build/`, `cargo smoke build`, a page in
    `target/smoke/build/index.html`). The placement matrix first: rows are every component the
    sim has today (storage, engine, RCS, reaction wheel, battery, crew, decoupler, lifting
    surface, container, gear), columns are the ways to put a part (on a top or bottom node,
    radially on a surface at some (u, v) and standoff, in a ring of 2, 3, 4 or 6, inside a
    container, inside a fairing, canted 15° and 90°, mirrored). Each cell builds a small craft
    and checks, never just "it loaded":
    - it loads, or refuses with a reason that names the rule; it is never silently dropped
    - mass is the sum of the parts, and the centre of mass and inertia match an independent sum
    - outside the skin it is in the aero outline, and inside a fairing it is shielded
    - saving and loading gives the same craft
    - five seconds on the pad give no NaN, and the weight is in the load on the pad

    The page draws the matrix as a grid, green, red or grey (the rules say no, and which rule). A
    red cell carries the step that is meant to fix it; when that step lands the expectation
    flips, and a red cell then fails the test. Later steps add sizer checks, reference builds
    and property tests to the same suite. Done when the page exists and every cell that fails
    names its step.
E2. (L) Physics on any part at any pose. Replace the sorting of parts into eleven kinds in
    `parts/gather.rs` with a direct reading of the assembly: every part at its own pose with its
    own geometry and any number of components. `Hull` and the stage layout go; release groups
    stay derived, as `Section`s already are. Done when the golden figures do not move, "until
    the physics runs on parts" is gone from the code, and the matrix's cells for tilted and
    sideways engines, RCS blocks, pods and tanks, and parts with two components, are green.
E3. (L) The builder. The operations above, the attach rules and fit checks, patterns, undo.
    Done when the Saturn V is rebuilt by calls to it and weighs within 1% of the hand-built
    craft, a place then a remove restores the craft exactly, a pattern of n is n placements by
    hand, mirroring twice is the identity, and a seeded random walk of valid operations never
    makes an invalid craft.
E4. (M) The editor session, headless. The state, the commands (action strings), the numbers and
    the checks, the parameter schema, save and load in the user folder, and the user folder as a
    content layer. Done when a test drives it with commands to build the Aerobee and the Titan II
    GLV, compares them with the hand-built files, saves, reloads, and gets the same craft.
E5. (M) The palette. A list of everything a player can place: engines by family, pods, RCS and
    wheels from the catalogue records; procedural presets (cylinder, cone and sphere tanks,
    shells and adapters, skirts and interstages, fins and wings, gear legs, decouplers, beams
    and trusses); and the assemblies. Each entry has tags, a size class, default parameters and
    where it wants to go, and respects the date's unlocks. Done when every part of every shipped
    craft can be found in it.

Milestone 2, the editor in the game:

E6. (L) The editor scene. A "Build" entry in the main menu and a toolbar button to come back to
    it; the hangar scene; the camera and its ortho views; the draft drawn by the vessel
    renderer; a ghost under the pointer; picking and selection with an outline; connectors lit
    when a part could go there; snapping to nodes and surfaces (with (u, v) from where the ray
    hit); handles to move, rotate and offset in 90°, 15° and 5° steps; the symmetry preview;
    delete, duplicate, reroot. Done when a person places an engine under a tank, a pair of fins
    on its side, and a payload on top, with the mouse.
E7. (L) The editor UI. Layouts for the palette (search, tags, a size filter, drag a part onto the
    scene), the part's properties (generated from the schema: sliders with units and ranges, a
    list of engines, of tank types, of what a tank holds), the numbers, the checks (click one to
    select the part), the file menu, the sequences (the staging window's drag and drop on the
    draft's sequences), and the keys. Done when a person builds a two-stage rocket with the mouse
    and every number on the screen is the one the flight uses.
E8. (M) Launch and files. New, open, save, save as; the craft list with shipped and user craft;
    Launch picks a site (pad, runway, orbit), as the main menu's "place then spacecraft" steps
    already do, and starts the scenario with the draft or a file; the menu lists user craft;
    Revert to the editor from flight. Done when you can build, launch, fly, revert, change a
    tank, and launch again.
E9. (S) Thumbnails and tooltips. Each palette entry rendered offscreen once and kept, and
    tooltips with the part's data, from the `part_info` keys. Done when the palette shows
    pictures.

Milestone 3, the Saturn V built in the editor, and flown:

E10. (M) Recipes, and the Saturn V's. A recipe is a record: a craft id and a list of editor
     commands. A tool (`examples/craft_recipe.rs`) turns any craft file into a recipe, which
     starts the Saturn V's and keeps the recipe honest. The Saturn V recipe builds the S-IC (a
     tank, five F-1s, four fins by a pattern), the interstage, the S-II and S-IVB, the ullage
     motors and the APS (by patterns), the adapter, the `apollo_csm` assembly, the lunar module
     drum and the sequences, from palette entries. Done when the recipe-built craft is within 1%
     of the hand-built one in mass, thrust, delta-v per sequence, centre of mass and inertia,
     every difference is listed, and every flat mass or hand-shaped part is flagged.
E11. (M) The headless editor smoke test: `cargo smoke editor`. It builds the Saturn V through the
     session, with the stage by stage numbers as it goes (the mass, delta-v and thrust to weight
     growing, and the checks coming and going, such as "no engine" until the F-1s are in); then
     the editor's own refusals (an engine above a tank is refused, a part outside its node's size
     is refused); then save, load, compare; then launch from the file and fly the whole Apollo 11
     script of `tests/smoke/saturn_v.rs` on that craft. The report is one page: the build
     timeline, then the mission. Done when the mission's results match those on the hand-built
     craft (the same phases pass, the same findings).
E12. (M) The UI editor smoke test, in the capture harness: `OSP_CAPTURE_EDITOR=1 cargo run`. The
     same recipe, driven by `Action::Ui("editor.…")` strings and the scripted pointer as a
     player would (drag from the palette to a node, click to place, rotate with keys, a pattern,
     a tank's length with its slider, a sequence reordered), a screenshot after each stage of
     the build, the Launch button, a capture at liftoff. It writes the craft and checks it is
     identical to the headless one. It needs a GPU, so it is run by hand like the other
     captures. Done when the screenshots show a Saturn V growing on the screen and lifting off.
E13. The gate. `cargo smoke` builds the Saturn V in the editor and flies Apollo 11 from it, and
     the GPU run does the same with the mouse. Nothing in `docs/systems-plan.md` after
     milestone 1 starts before this passes.

## What each system adds after that

A system step in `docs/systems-plan.md` is done when its parts are in the palette with their
sliders (from the schema, so with no editor code), the Saturn V recipe places them (the
instrument unit, the batteries, the antennas, the legs on the lunar module), the placement matrix
has their row and it is green, and the mission test checks what they do. The recipe grows with
the game. The last step of that plan rebuilds the Saturn V from procedural parts only, in the
editor, with no flat mass left.

## Left out for now

- Subassemblies saved by a player, and sharing craft between players.
- Editing a shape's curve (revolved walls) by hand; shapes come from presets and models.
- Action groups beyond sequences, and the fuel flow view.
- Importing KSP craft files.

## Tracker

Each step is a task in `.tasks/`, with its dependencies: E1 is T013, E2 T014, E3 T015, E4 T016, E5
T017, E6 T018, E7 T019, E8 T020, E9 T021, E10 T022, E11 T023, E12 T024 and E13 T025, all under the
epic T002. Status lives there, not here.

