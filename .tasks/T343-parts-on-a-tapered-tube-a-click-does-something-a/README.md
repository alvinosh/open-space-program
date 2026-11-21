# T343: Parts on a tapered tube: a click does something, and the preview settles

Found on 2026-11-21 rebuilding the stock craft in the builder, as a player would (the playtests `rebuild_*`, crates/osp_game/src/playtest/rebuilds.rs), after the user said "I still hate the builder, i cant even mirror fairings into a payload".

Two things on a taper (a Structural with different top and bottom diameters):

- A nose wheel clicked onto it does nothing: no part, no notice (a dead click).
- A wing held over it near its narrow end (the glider's fin): the editor kept rebuilding the
  preview for 30 s without settling. T329 fixed this for straight hulls.

The glider rebuild uses a short taper and a straight 0.3 m boom to get round both.

## Done when

- Gear goes on a taper, or a notice says why not; a part held over a taper settles at once.
