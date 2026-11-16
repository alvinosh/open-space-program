# Playtests

A playtest plays the game the way a person does, from the title screen, and measures how easy it was
and what the player got out of it. It is the end-to-end test for anything a player touches: the
menus, the editor, flying. The smoke tests (`docs/smoke-tests.md`) check that the simulation is right;
playtests check that a person can get through the game and enjoy it.

```sh
cargo playtest                                  # every playtest, then target/playtest/report.html
OSP_PLAYTESTS=first_rocket cargo playtest        # only these (comma separated)
OSP_PLAYTEST=first_rocket OSP_USER_DIR=/tmp/empty cargo run -p osp   # one, in the game, to watch
```

Each runs in the game built for the tests, in an empty user folder (a first game: its settings,
saves and windows live there, never the player's own), rendering offscreen at 1600 x 900. A run of all three takes under a minute.

## What a playtest may do

Only what a player can (`Play` in `crates/osp_game/src/playtest.rs`):

- `Click`, `Drag`, `Wheel` on a `Spot`: a control by its action (`Spot::Ui("editor.launch")`), or in
  the editor a ring, a part, a spot on a surface or a handle, as drawn. A control must be drawn, on
  screen, not scrolled out of sight and not covered (the pointer has to be over it as picking sees
  it), or the player is stuck and the playtest fails there.
- `Key`, `Chord` (Ctrl+Z), `Type`.
- `Until` and `Expect`: conditions on the UI's data, the same as layouts use (`==`, `!=`, `!`) plus
  `<`, `>`, `<=`, `>=` on numbers. Nothing reads the game's insides.

Steps wait for the game to answer (the editor's builds on their threads, the screen to stop
changing), never for a set time, so a busy machine makes a playtest slower but not flaky.

## What it measures

For each step: how far the pointer went and how big its target was (Fitts's index of difficulty),
an estimate of a person's time (the keystroke-level model: a moment to think, pointing, buttons,
keys, moving a hand between mouse and keys), the time until the screen answered, how long a preview
took to show, the worst frame, what changed on screen, and the editor's notices. A step that changed
nothing on screen is a dead input.

For the playtest: inputs, a person's time, pointer travel, the share answered within 0.1 s, targets
under 24 px, frame times and stalls, and:

- `Goal`: how far the player has got, with the time and inputs it took (the pace).
- `Track`: the lowest, highest and last of a number (height, pitch, thrust to weight).
- `Hope`: what a player wants out of it, held for some seconds ("it goes up the way it points").
  A broken hope is a letdown, reported with when and what the screen showed, but not a failure:
  it is a fun problem to file.

The person times are rough on purpose. What matters is how they move from one run to the next and
which steps cost most (`costliest` in the summary).

## Looking into a flight

Each playtest also writes `craft.json`, the last craft it launched. To see why a flight went wrong,
fly it again outside the game, SAS on and full throttle as a player's first go, with its height,
Mach, pitch, angle of attack, controls, thrust to weight, dynamic pressure and static margin each
second (`--no-sas`, `--throttle=0.5`, `--realistic`), or with `--stability` the air's push on the body
and on the fins by Mach, and where each acts:

```sh
cargo run -p osp_sim --example fly_craft --release -- target/playtest/first_rocket/craft.json 60
cargo run -p osp_sim --example fly_craft --release -- target/playtest/first_rocket/craft.json --stability
```

## Stuck on a task

A playtest that gets stuck because the game cannot do it yet has `stuck_until: Some("T317")`. The
run reports it as known and stays green; once the player gets through, it fails until the mark is
taken out.

## Adding one

Add a `Playtest` to `PLAYS` in `crates/osp_game/src/playtest/plays.rs`: what the player sets out to
do (`about`), and the steps as they would go about it. Start with
`Play::Until("menu.screen == title", 60.0)`. Take pictures where a person would look. Put a `Hope`
wherever a player would be disappointed if it did not hold, and file what the report finds in
`.tasks/`.
