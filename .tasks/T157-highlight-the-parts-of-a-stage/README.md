# T157: Highlight the parts of a stage on the rocket

Engines and stages are not marked on the rocket. Hovering an icon in the staging window should show which parts it is, and the next stage to fire should be marked.

## Scope

- Hovering a staging row or icon outlines or tints its parts on the vessel (engines, decouplers, tanks of that stage).
- The engines about to light are outlined while the sequence is next; a running engine glows with its throttle (T120 is the heat glow, this is the selection).
- In the editor (T019) the same highlight is used for a selected part.
- It works from the part id, so it also works for a part inside a cluster or a symmetry group.

## Done when

- Hover on an icon of three identical boosters outlines all three; hover on one of a split count outlines that one (captured screenshot).
