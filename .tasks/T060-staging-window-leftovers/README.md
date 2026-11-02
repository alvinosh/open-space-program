# T060: Staging window leftovers

What is still open in the staging window, and drag and drop that has not been tried by hand.

## Scope

- Right-click 'add a sequence above / below'; the + button adds one at the end and numbers are dragged to reorder.
- One fuel bar per icon (the tanks feeding its engines), not per engine.
- Drag and drop in the panel is covered by tests of each piece and the scripted capture (`OSP_SCENARIO=leo OSP_CAPTURE_STAGING=1`), but not tried by hand yet.
- Sequence groups are found by name, so two different parts given the same name share an icon.
- Moving one part out of a group takes the last of it; which one is not chosen.

## Done when

- Each item above is done or dropped.

## Notes

- The editor's sequence window (T019) reuses this UI, so doing it first helps.
- The model: sequences are the activation groups Space fires; resource groups share propellant; the Sequence window (F8) shows part icons with live delta-v, TWR and fuel; parts and sequences are dragged around in flight.

## Done

- Add above or below is in a row's right-click menu (T158).
- One fuel bar per icon was already so (`feed_fraction` of the group).
- Trying the drag and drop by hand moved to T158, which now has a pointer script and leaves only the real mouse.
- A group is now the same kind by parts of the same stage and name, so two stages' parts of one name no longer share an icon.
- Which part leaves a group: dragging a count still takes the last of it, and Split off in a part's window takes that one (`staging.split`), so a cluster's centre engine can go first.

## Where it comes from

- old issue list, issue 3 (docs/issues.md, removed; see git history)
- docs/issues.md, issue 3 (removed)
