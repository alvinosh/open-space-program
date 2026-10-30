# T024: UI editor smoke test: build the Saturn V with the pointer

In the capture harness (`osp_game/src/smoke_test.rs`): `OSP_CAPTURE_EDITOR=1 cargo run` replays the recipe through `Action::Ui("editor.…")` strings and the scripted pointer as a player would. Needs a GPU, run by hand.

## Scope

- Drag from the palette to a node, click to place, rotate with keys, a pattern, a tank's length with its slider, a sequence reordered.
- A screenshot after each stage of the build, the Launch button, a capture at liftoff.
- Writes the craft file and checks it is identical to the headless build.

## Done when

- The screenshots show a Saturn V growing on the screen and lifting off, and the saved craft equals the headless one.

## Where it comes from

- docs/build-editor-plan.md (E12)
