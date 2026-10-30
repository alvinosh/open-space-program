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

## Where it comes from

- old issue list, issue 3 (docs/issues.md, removed; see git history)
- docs/issues.md, issue 3 (removed)
