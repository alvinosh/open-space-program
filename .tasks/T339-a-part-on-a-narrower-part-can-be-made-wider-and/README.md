# T339: A part on a narrower part can be made wider, and a size that is not taken says why

Found on 2026-11-21 rebuilding the stock craft in the builder, as a player would (the playtests `rebuild_*`, crates/osp_game/src/playtest/rebuilds.rs), after the user said "I still hate the builder, i cant even mirror fairings into a payload".

A part's end joined to another part keeps that part's width, and a size typed in that does
not match is dropped without a word:

- A tank under the 0.6 m probe core stays 0.6 m when 3.7 is typed into its diameter (the whole
  Pathfinder came out 0.6 m wide, and its 3 m engine was refused under it).
- A tube's top under a 0.5 m reaction wheel: 1.3 typed, 0.50 shown.
- A nose cone on a 0.6 m part stays 0.6 m.
- The glider's tail taper: its joined end typed to 0.3, nothing happened.

So a payload cannot start from a probe core, an inline reaction wheel narrows everything under it,
and the wide body has to be the root. KSP lets any size join any size. A tube also has a bottom and
a top diameter and no single diameter, and typing one turns it into a taper.

## Done when

- A part can be made wider or narrower than what it is joined to (the joint shows the step), or the
  builder says why not, next to the field.
- A tube's diameter is one field until it is made a taper.
