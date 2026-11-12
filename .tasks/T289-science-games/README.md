# T289: Career games: science and unlocked nodes kept per save, the parts list shows what is unlocked

The New game screen offers a sandbox (every part, no limits) or a career: the tree's roots only,
science to unlock the rest, and later money for the launch pad and the builder (T293, T294). A
career keeps its science and its unlocked nodes in the save; the editor's parts list shows what is
unlocked, an engine only its unlocked configurations and a tank only its unlocked tank types, and a
craft with a locked part says so. Historical scenarios keep the date rule.

## Done when

- A new career has only the root's parts, and a save brings back its science and unlocks (test).
- The sandbox is unchanged.

## 2026-11-12

The user chose money, not science, for facility upgrades ("Money (new career)"), so a game that
locks parts is a career, not a science game.

## Done (2026-11-12)

- `osp_sim::career::Career`: the tree it is played on, its science, its unlocked nodes (the roots at
  the start); `unlock` pays a node's cost once what it builds on is unlocked; `locked_parts` names
  the parts of a craft it cannot build: a part whose every possible parts-list entry is locked, or an
  engine in a locked configuration. `palette::sources` tells which entries a placed part could have
  come from (its template, or how it is made, then its name); every entry's own part traces back to
  it alone (test).
- `Universe::career`, in the snapshot and so in saves and in the revert point; a sandbox has none.
- New game screen: Sandbox or Career (`game.kind:<kind>`, `menu.career`), above the modes; Start now
  sits under the modes so it shows however many mods there are. A new career starts at the new
  game's first launch with the rules' tree's roots.
- Editor: in a career the parts list shows only unlocked entries and says how many are still locked
  (`editor.locked`); an engine's configuration choice offers only unlocked configurations (and the
  one it is in). A craft with locked parts is not launched: "Still locked in the tech tree: …".
- `osp_game::career::CareerTree`: the rules' tree, rebuilt when the records change.
- Seen in the game (`OSP_CAPTURE_CAREER=1`): the New game rows, a Casual career's engines tab with
  only the K-0 and the S-1 and "33 parts are still locked in the tech tree", and Pathfinder Crew
  refused naming the C-4 Capsule, K-1 Vacuum, Attitude Control Unit, Cold Gas Quad and K-1 Booster.
- `OSP_CAPTURE_MODS` had gone stale (it counted six installed mods; there are seven now): its counts
  are fixed and it passes, and a failed data check now says what the value was.
- Tests: a new career has its roots and unlocks with science, once; a craft's locked parts are
  named; a save keeps a career's science and unlocks and a sandbox stays one; every entry traces back.
  820 tests across the sim, game, mods and editor crates.

## Left out

- Nothing unlocks yet in play: there is no science to earn (T284) and no screen to spend it (T290).
- Sandbox stays the default on the New game screen until a career can be played through (T291).
- Tank sizes and tank types are not locked yet (T292).
- A part made by hand, like no parts-list entry (stock craft's own shells and containers), is not
  held against a career.
