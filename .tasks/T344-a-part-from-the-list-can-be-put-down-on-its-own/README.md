# T344: A part from the list can be put down on its own in the hangar

Found on 2026-11-21 rebuilding the stock craft in the builder, as a player would (the playtests `rebuild_*`, crates/osp_game/src/playtest/rebuilds.rs), after the user said "I still hate the builder, i cant even mirror fairings into a payload".

Dropping a part from the parts list on empty space does nothing; only parts pulled off the craft
stay loose. In KSP any part can be put down anywhere and set up before it goes on. Here a part as the
list gives it can be too big for where it goes (a 7.5 m solid booster beside the glider's tailplane:
"overlaps wing_3"), and it cannot be put on to be made smaller, so the build has to be reordered.

## Done when

- A part dropped on empty space stays there loose, its settings can be changed, and it can then be
  put on.
