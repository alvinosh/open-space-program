# T324: Prune the unit tests that playtests cover

On 2026-11-16 the user said many of the game's unit tests are not needed and the focus should be e2e
tests that play like a real user. About 980 `#[test]`s: osp_sim 713, osp_game 111, osp_ui 51,
osp_render 36, osp_editor 31, others 34.

A first rule to agree with the user before deleting anything:

- Keep the simulation's oracles and golden figures (orbits, physics, craft numbers, saves) and the
  smoke missions: they say whether the sim is right, which a playtest cannot.
- Drop tests that only check UI wiring, layouts or editor plumbing that a playtest now walks through,
  and tests that restate the code.

## Done when

- The user has agreed the rule, the covered tests are gone, and `cargo test` is faster.

## Done (2026-11-16)

The user agreed the rule. Four reviews, one per group of crates, each test judged by it, "when
unsure, keep". 90 of 978 `#[test]`s removed, test code only (1,381 lines deleted, none added);
877 left pass with no warnings.

| Crate | Before | After | What went |
|---|---|---|---|
| osp_game | 111 | 74 | menu and dialog flows, part highlight and marker state, UI id parsing, labels |
| osp_editor | 31 | 20 | rail and list wiring, labels, hangar visuals the captures show |
| osp_ui | 51 | 30 | how components expand into nodes, dock/popup/tab/hover wiring |
| osp_sim (lib) | 630 | 609 | restated constants (fin ratios, labels, defaults), pass-throughs, duplicates |
| osp_render, osp_mods, osp_bake, osp_terrain | 65 | 65 | nothing: all maths, physics or mod-data logic |
| osp_sim/tests, osp_smoke | | | kept whole: integration, golden figures, missions |

The simulation keeps nearly all its tests: they check physics against known answers. A stricter
pass there would need a looser rule from the user.
