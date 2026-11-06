# T211: The editor in a crate of its own

Asked for on 2026-11-06: "I think the editor should be its own crate, it still needs a lot of
work", before the list of editor problems in T207 and T212 to T218.

## What was done

- `crates/osp_editor`: `hangar` (was `osp_game::editor`: opening, the hall, its camera, the list of
  vessels, launch and revert), `building`, `shaping`, `moving`, `part_window`, and `capture`.
- It reads `osp_game` and is never read by it. What the game needs of the editor is in
  `osp_game::editing`: `EditorStatus` (open, and the craft's gravity for the flight UI), the
  `EditorActions` system set the dialog goes after, `LastLaunch`, `LaunchPoint` and `LaunchPlace`.
  The editor writes the status the frame it opens or closes.
- The smoke test's editor steps (pointing at handles, rings, parts; checking the craft) are sent as
  `osp_game::smoke_test::EditorStep` messages, taken by `osp_editor::capture` in
  `CaptureSet::Editor`; the UI's pointer follows in `CaptureSet::Pointer`.
- The `osp` binary adds `osp_editor::EditorPlugin`. Commit scope `editor`.

## Done when

- The workspace builds and its tests pass, and the editor captures (`OSP_CAPTURE_EDITOR_BUILD`,
  `_MOVE`, `_SHAPE`) run to the end as before. Done 2026-11-06.
