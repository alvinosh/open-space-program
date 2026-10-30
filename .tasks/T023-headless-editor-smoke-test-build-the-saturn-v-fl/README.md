# T023: Headless editor smoke test: build the Saturn V, fly Apollo 11

`cargo smoke editor`: builds the Saturn V through the editor session, saves, reloads, launches from the file and flies the whole Apollo 11 script on that craft.

## Scope

- Stage by stage numbers as it builds (mass, delta-v and thrust to weight growing; checks such as 'no engine' coming and going).
- The editor's own refusals (an engine above a tank, a part outside its node's size).
- Save, load, compare; launch from the file; run `tests/smoke/saturn_v.rs` on it (the mission harness takes a craft override).
- One report page: the build timeline, then the mission.

## Done when

- The mission's results match those on the hand-built craft: the same phases pass, the same findings.

## Where it comes from

- docs/build-editor-plan.md (E11)
- docs/smoke-tests.md
