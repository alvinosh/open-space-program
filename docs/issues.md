# Issues

Open problems found while playing, oldest first. Delete an entry when it is fixed.

## 1. Atmosphere stops rendering above 100 km

Reported 2026-10-20, not looked into yet. The atmosphere stops rendering once the vessel is
above 100 km.

100 km is exactly the top of the scattering shell (`THICKNESS` in
`crates/osp_game/src/shaders/atmosphere.wesl` and `crates/osp_game/src/atmosphere.rs`), so
the switch from inside to outside the shell is the first suspect: the from-outside branch of
`crates/osp_game/src/shaders/sky.wesl`, and `dim_sunlight`, which changes lighting and
exposure at the same height. Repro: `OSP_SCENARIO=leo cargo run` (300 km), flight view, or
fly the default launch past 100 km.

## 2. Plume swings with the engine gimbal

Reported 2026-10-20. The whole plume is a child of the engine's gimbal pivot (`EnginePivot`
in `crates/osp_game/src/vessel_render.rs`), so when the engine gimbals the plume swings like
a rigid stick, by metres at its far end. Only the exhaust at the nozzle should follow the
gimbal; gas already downstream keeps going where it was going.

Idea: hang the plume off the vessel instead of the pivot and bend it. The start follows the
nozzle axis, the far part follows the flow (the vessel axis in vacuum, trailing the airflow
in atmosphere, which is also the "crossflow bending" in `docs/ksa-parity.md`), and the far
direction is smoothed over time so quick gimbal moves do not show in it. The shader already
marches in plume coordinates (`crates/osp_game/src/shaders/plume.wesl`), so the bend can be
an offset of the axis that grows with distance from the nozzle.

## 3. Staging should work like KSA's

Reported 2026-10-20. Today a stage is one unit: Space arms its engines, the next press drops
it and arms the one above (`Propulsion::activate_next_stage` in
`crates/osp_sim/src/propulsion.rs`). Problems:

- The stage stack (`assets/core/layouts/staging.json5`) shows too much text.
- The fairing cannot be jettisoned; it is only drawn (`crates/osp_game/src/hull.rs`).
- RCS cannot be put in a stage (only toggled with R).
- Stages cannot be reordered or regrouped, and decoupling cannot be staged apart from
  ignition.

Wanted: staging done the way KSA does it, with icons and numbers instead of text.

How KSA does it, from the community wiki's build notes and a fan feature list (KSA has no
official docs on this):

- Two separate things. Sequences are the activation groups that Space fires one after
  another. Resource groups (called stages until build 2026.6.9.4750) are the parts that
  share propellant. Decoupling and engine ignition are separate entries, so they can sit in
  different sequences.
- The Sequence window (F8) lists the sequences as rows of part icons instead of text (first
  pass in 2026.6.9.4750, icons in flight from 2026.7.4.4860). Symmetric parts share one
  icon and can be expanded.
- In flight it shows live delta-v, TWR and a fuel bar per engine for the active sequence;
  the editor shows delta-v and TWR per sequence (2026.7.4.4860).
- Editable in flight and in the editor (2026.5.12.4510): drag a part from one sequence or
  stage to another, drag a whole sequence or stage to reorder it, right-click a sequence or
  stage to add a new one above or below it.
- Decouplers, and interstage shrouds with their own decouplers, are staged like engines. A
  part's decoupler can be switched off, which turns it into a plain shroud and removes it
  from the Sequence window. A launch escape tower is staged like any other motor.

What it means here: split our stage into a resource group (the propellant pool it is now)
and an ordered list of sequences, each holding activations (light these engines, fire this
decoupler, jettison the fairing, switch RCS on) that can be moved between sequences. The sim
needs decouplers and the fairing as things it knows about (Phase 2 of `docs/ksa-parity.md`:
decouplers, activation sequences separate from resource groups, drag and drop in flight).
The staging panel becomes a column of numbered sequences with one icon per item and a fuel
bar under the engines, plus delta-v and TWR for the active one.

Sources:

- https://kittenspaceagency.wiki.gg/wiki/Version_2026.5.12.4510
- https://kittenspaceagency.wiki.gg/wiki/Version_2026.6.9.4750
- https://kittenspaceagency.wiki.gg/wiki/Version_2026.7.4.4860
- https://kittenspaceagency.wiki.gg/wiki/Controls
- https://areweksayet.com/ (unofficial feature list)
