# T316: Playtests: the game played as a person, measured for ease of use and payoff

Asked 2026-11-16: fewer unit tests, more end-to-end tests that play the game as a real user does,
with figures for how usable the builder is and how much fun the game is.

Built (`osp_game::playtest`, `cargo playtest`, guide in `docs/playtests.md`):

- A playtest starts on the title screen with an empty user folder and only clicks, drags, turns the
  wheel and presses keys on what is drawn. A control must be on screen, not scrolled away and not
  covered (checked against picking's hover map) or the player is stuck. Checks read the UI's data.
- Each step waits for the game (the editor's builds on their threads, the screen to stop changing),
  never for a set time.
- Per step: pointer distance, target size, Fitts bits, a person's time (keystroke-level model), time
  to the screen's answer, preview build time, worst frame, what changed, dead inputs, the editor's
  notices. Per playtest: goals (pace), hopes (a letdown is a fun problem, not a failure), tracked
  numbers, frame stats.
- `stuck_until: Some("T…")` keeps a playtest that gets stuck from failing the run until that task is
  done, and fails once it gets through.
- Report: `target/playtest/report.html`, plus each playtest's `report.json` and pictures.

First three: `first_rocket`, `resize_tank`, `two_stages` (stuck on T317). Their findings are T317 to
T323.
