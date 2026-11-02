# T029: Actions and sequences, generic

A component declares its actions (deploy, arm, cut, jettison, undock, cutoff, throttle); a sequence entry, a key binding and a staging icon are a part and an action. `Activation` is a list of four today.

## Done when

- `Activation` is not a list of four.
- The cutoff from T026 is one action among them.
- The editor's sequence window shows an action it has never heard of.

## Done (2026-11-02)

- `Activation` is a part and an `Action`: a kind, a verb and an optional setting, not an enum of four. `Action::IGNITE`, `CUTOFF`, `DECOUPLE`, `JETTISON`, `RCS_ON` and `Action::throttle` are the built-in ones.
- A system offers its own (`System::actions`, `System::act`; `Kind::actions` so a craft file's sequence can name them). `Propulsion::run` hands what it does not know to the part's system. The part window's buttons, a craft file's `<kind>:<part>[:<setting>]` and the staging panel's tiles all carry them by name, a tile using `icons/part.svg` where none was drawn and the action's verb for its caption. A test goes through all of it with a system and an action nobody has heard of.
- Key bindings for a part's action, and the editor's sequence window (T019), still to come; both read the same `Activation`.

## Where it comes from

- docs/systems-plan.md (step 4)
