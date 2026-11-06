# T016: Editor session, headless

`osp_sim::editor`: the draft craft, selection, symmetry mode and rotation step, undo and redo, the numbers and the checks, driven by commands that are action strings. No Bevy, so it runs in tests.

## Scope

- Commands as action strings: `editor.place:<palette id>`, `editor.select:<part>`, `editor.rotate:<axis>:<degrees>`, `editor.symmetry:<n>`, `editor.param:<name>:<value>`, `editor.undo`, `editor.save`, and so on.
- Numbers: mass, delta-v and burn time per sequence, thrust to weight (`Propulsion::stage_info`), centre of mass and of pressure and the margin.
- Checks: no engine, a stage that cannot light, an unattached part, thrust to weight under 1.
- A parameter schema trait: each component describes its editable parameters (name, unit, range, step, choices), so a new kind of part needs no editor code.
- Save and load in the user folder (`OSP_USER_DIR`), loaded as one more content layer above the mods.

## Done when

- A test drives the session with commands to build the Aerobee and the Titan II GLV, compares them with the hand-built files, saves, reloads, and gets the same craft.

## Notes

- Per-sequence delta-v and thrust to weight in vacuum and in atmosphere, from a fuel-flow time simulation, is also wanted; `Propulsion::stage_info` has the vacuum figures.

## Where it comes from

- docs/build-editor-plan.md (E4, Design)

## Done so far

- 2026-11-06, the parameter schema for shapes: `osp_sim::shaping` lists each shape's sizes (name,
  label, value, least and most, whether it is a width), where each is taken hold of (grips, with
  the way and rate they move) and `resize`, which carries a part's own nodes along (on the wall
  stays on the wall, near an end stays as far from it). Cylinder and cone tanks, spheres, whole
  shells, rings, prisms, boxes and wings (mirrored too). Sizers, choices (a shell's profile, what a
  tank holds) and other components are not in it yet.
