# T234: Show the root and let any part become it

The root decides a lot (a drag on it carries the craft, its rings turn the craft, it cannot be
removed) and nothing shows which part it is. `Builder::reroot` exists and nothing calls it.

## Scope

- Mark the root when a part is picked or pointed at.
- Make root in a part's settings.

## Done when

- The root can be seen and changed from the editor, and undo puts it back.

## Where it comes from

From the editor review of 2026-11-07: the user asked for everything wrong with the editor from a UX point of view, then for each finding to be filed as a task. The epic, T221, lists them all.
