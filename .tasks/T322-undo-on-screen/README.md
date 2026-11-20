# T322: Undo and redo on screen, and the editor's keys do not fly the rocket

Found by the `resize_tank` playtest (2026-11-16):

- Undo and redo are Ctrl+Z and Ctrl+Shift+Z only; nothing on screen shows them.
- Ctrl+Z in the editor also fires `vessel.throttle_full` (the Z binding), as flight keys are not held
  back while building.

## Done when

- Undo and redo buttons in the editor bar (greyed when there is nothing to undo), with their keys in
  the tooltip and the change they would undo or redo ("Undo: move fin"; this was T241, merged here).
- Flight bindings do nothing while the editor is open; `resize_tank` shows only the undo firing.
