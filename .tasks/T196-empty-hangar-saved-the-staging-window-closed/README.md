# T196: The empty hangar saved the staging window as closed

The open flight windows are saved to `windows.json` on every change. An empty hangar closes the staging window for as long as it is open, so a game closed from there came back with it closed, and every launch after that restored it closed. Only F8 brings it back: there is no button for it.

## Done when

- Every game starts with the stage stack open, whatever an earlier version left in `windows.json`.

## Done

- First fixed by not saving while the editor was open. The user still had no stages: that left the `false` already in their file, and F8 is a media key on many keyboards, so nothing could bring them back.
- The stage stack is now not remembered at all, and a saved value for it is ignored (`windows::NOT_REMEMBERED`). Closing it with F8 lasts until the game is next run.
- `OSP_CAPTURE_WINDOWS=1` with `OSP_USER_DIR=<a folder>` runs it, and was checked against a copy of the user's own `windows.json`. Scripted captures use `windows.json` when `OSP_USER_DIR` is set, as they do saves.
