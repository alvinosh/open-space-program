# T223: Esc never leaves the editor

Esc does three things in turn: it puts back a part in hand, lets go of the picked part, and then
leaves the editor. One press too many and the editor is gone (T222 makes that ask first, but it
should not be one key away).

## Scope

- With nothing to put back or let go of, Esc opens a small menu over the editor (go on building,
  leave the editor) in place of leaving.
- Back still leaves, asking as T222 does.

## Done when

- Esc never closes the editor on its own; a capture presses it until nothing is left to let go of
  and the editor is still open.

## Where it comes from

From the editor review of 2026-11-07: the user asked for everything wrong with the editor from a UX point of view, then for each finding to be filed as a task. The epic, T221, lists them all.
