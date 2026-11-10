# T285: Engine families: one part each, with its configuration picked in the part settings

Asked on 2026-11-10: many engines in the parts list are the same engine in a different configuration,
and for simplicity they should be one part whose configuration is picked, mostly for Realistic.

## Where it stands

- The Realism mod has 899 engine records, every one naming its `family`, and 348 `engine_families`
  records (`configs`, `default_config`, `model`): 171 families have one configuration, 40 have five or
  more, the most has 36. The base game has 14 engines and no families.
- The parts list makes one entry per engine record (`palette.rs`), so Realistic lists all 899.
- The families are loaded only into the game's `Library`, and only the sounds read them. The physics
  and the editor never see them.

## The shape of it

- The parts list shows one entry per family (and an engine with no family as itself), with the
  family's title, and the default configuration's figures.
- The part settings offer the family's configurations, each with its thrust, Isp, propellants, mass,
  ignitions and year. Picking one swaps the part's engine record in place: same position, same
  attachments, its tanks' contents following the engine (T209).
- Configurations not yet available by date or tech are shown but cannot be picked (T284 later adds
  science unlocks to the same rule). Not done: see Left out.
- A craft file keeps naming the configuration (`template: "<engine id>"`), so nothing saved changes.
- Search finds a family by any of its configurations' names.

## Done when

- Under Realistic the parts list shows one engine a family (348, not 899), in a test.
- Picking another configuration of a placed engine changes its thrust, Isp and mass in the editor and
  in flight, and the saved craft names it, in a test.
- The Saturn V still builds and flies.

## Note

T203 (part menu) and T207 (part settings in the left panel) are in progress in another session; the
picker goes where those put part settings.

## Done (2026-11-10)

- The part catalogue holds the `engine_families` (`PartCatalog::engine_families`), with
  `configurations`, `family_of` and `first_configuration` to look them up. A family counts only for
  engines it lists.
- The parts list makes one entry a family: its default configuration (or its first), named by the
  family's title, saying how many configurations it has, with their names in its description so a
  search finds any of them. An engine of no family is listed as before. Under all mods: 916 engine
  records, 348 families and 17 engines of none, 365 engines in the list.
- Part settings: an engine of a family of several has a Configuration choice under Engine, each
  option with its thrust, Isp, propellants, mass and year. Picking one sets the part's `template` to
  it (`properties::set`), checked like any setting; tanks still as the parts list gave them follow
  what it burns (`Builder::set_field`). The editor's settings panel shows it with no code of its own.
- The craft keeps naming the configuration, so saved craft and the Saturn V are as they were.
- Tests: `an_engine_family_is_one_part_with_its_configuration_picked` (properties.rs: one entry, found
  by the other's name, the choice lists both, picking builds the other engine, a foreign one is
  refused, a lone engine has no choice), `realistic_lists_one_engine_a_family` (tests/engine_families.rs).

## Left out

- Configurations are not held back by date or tech: the editor does not know the game's date, so
  each shows its year, and a craft from before its parts is flagged at launch as now. T284 adds
  unlocks.
- A configuration with another model or size than the one placed is checked like any change and
  refused if its attachments no longer fit.
- 811 tests pass across the sim, game, mods, smoke and editor crates, every smoke mission among them;
  `cargo smoke saturn_v`: 143 passed, 0 gaps.
