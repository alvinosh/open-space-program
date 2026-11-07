# T229: Change a placed part's symmetry count

The symmetry count is chosen before a part goes on. Afterwards the copies are plain parts, so four
fins cannot become three without taking them off and putting them on again.

## Scope

- A Copies setting on a placed part that has copies (or could have): changing it puts the set on
  again round the same holder at the new count (`put_set`), keeping each part's settings.

## Done when

- Four fins become three and then six from the settings, and undo goes back a step at a time.

## Where it comes from

From the editor review of 2026-11-07: the user asked for everything wrong with the editor from a UX point of view, then for each finding to be filed as a task. The epic, T221, lists them all.
