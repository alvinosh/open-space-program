# Issues

Open problems found while playing, oldest first. Delete an entry when it is fixed.

## 3. Staging: what is still different from KSA

Reported 2026-10-20; the main request (KSA-style sequences, icons and numbers, fairings and
RCS stageable, reorder and regroup, decoupling apart from ignition) landed 2026-10-22, see
"Staging" in `docs/modding.md`. Left over:

- A stage's engines are one activation, so a cluster cannot be split (light the centre
  engine first, say). KSA moves single parts and groups symmetric ones under one icon.
- No right-click "add a sequence above / below"; + adds one at the end and numbers are
  dragged to reorder.
- One fuel bar per engine group, not per engine.
- Stages and fairings that are released just vanish (no debris yet).
- Drag and drop in the panel is covered by tests of each piece and the scripted capture
  (`OSP_SCENARIO=leo OSP_CAPTURE_STAGING=1`), but not tried by hand yet.

KSA's model, from the community wiki's build notes: sequences are the activation groups
Space fires; resource groups (stages until 2026.6.9.4750) share propellant; the Sequence
window (F8) shows part icons with live delta-v, TWR and fuel; parts and sequences are
dragged around in flight (2026.5.12.4510). Sources:
https://kittenspaceagency.wiki.gg/wiki/Version_2026.5.12.4510,
https://kittenspaceagency.wiki.gg/wiki/Version_2026.6.9.4750,
https://kittenspaceagency.wiki.gg/wiki/Version_2026.7.4.4860,
https://kittenspaceagency.wiki.gg/wiki/Controls, https://areweksayet.com/ (unofficial).

## 4. Trees glitch near the ground

Reported 2026-10-22 (seen before the aerodynamics work too), most visible flying low and
fast, e.g. the rocket plane's takeoff roll. Fixed 2026-10-22: trees loading and
unloading as the camera orbits (no hysteresis on the radii, and cells dropped with their
old mesh when their detail flipped mid-build). Fixed again 2026-10-22 (reported as trees
appearing and disappearing with camera angle and position): cells were found by probing a
grid one cell apart around the camera, which missed ~12% of them at the Cape, and a
different set as the camera moved; they are now found by walking the quadtree.
Remaining suspects, from reading `crates/osp_game/src/trees.rs`:

- When a rebuilt cell lands, the old mesh stays for three more frames (`retiring`) on top
  of the new one, so detailed and cheap trees overlap for those frames.
- Trees stand on the finest terrain height (`surface.terrain.height`), while the terrain
  under them is drawn at its current LOD, so they float or sink until the chunk splits.

To look at it: `OSP_SCENARIO=runway OSP_CAPTURE_RUNWAY=1 OSP_CAPTURE=<dir> cargo run`
saves six frames 0.1 s apart during the takeoff roll.
