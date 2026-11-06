# T017: The palette

A list of everything a player can place, with tags, size class, default parameters, where it wants to go, and the date's unlocks.

## Scope

- Catalogue records: engines by family, pods, RCS, wheels.
- Procedural presets: cylinder, cone and sphere tanks, shells and adapters, skirts and interstages, fins and wings, gear legs, decouplers, beams and trusses.
- The assemblies (`mods/realism/assemblies`).
- Respect the tech tree and the game's year (`tech.rs`); sandbox shows all.

## Done when

- Every part of every shipped craft can be found in the palette.

## Where it comes from

- docs/build-editor-plan.md (E5)

## Notes

- Every sizer built before the editor (T032, T175, the battery, solar, avionics and antenna sizers) gets its palette entry here, with a slider per requirement field it lists. T032's and T175's old "palette entry" lines moved to this task.

## Done so far

- 2026-11-06, a first palette (`osp_sim::palette`), KSP-with-procedural-parts style: six procedural
  parts (tank, structural tube, nose cone, decoupler, fin, wing) whose sizes the player sets, then
  the catalogue's pods, engines, RCS blocks and reaction wheels. Each is fitted to what it goes on
  (`palette::fit`: a tank as wide as the node and three times as long, fins sized for the hull) and
  says the node and turn it goes on a surface by. Shown in the editor's Parts list by group, with a
  search.

## Still open

- The Saturn V's own parts (the assemblies, interstages, the other sizers), the tech tree's dates,
  and the procedural parts' other shapes (cone, sphere tank), which come with the part menu (T203).
