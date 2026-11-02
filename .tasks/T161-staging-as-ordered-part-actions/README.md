# T161: Staging as ordered part actions

A stage is a list of actions on parts, done in the order listed when Space is pressed (ignite these five engines; then decouple that decoupler). Every action can also be done by hand on its part: select the decoupler and press Decouple, select an engine and press Ignite. A stage only groups them.

## Done

- Actions run in the listed order. The old sort (decouplers first, then engines, fairings, RCS) is gone, so a sequence that ignites before it decouples now does that. No craft file depended on the sort; `Activation::rank` is removed (this changes T159's starting point).
- `Propulsion::part_actions` says what a part offers now (Ignite if the engine is not lit; Decouple for the joint it hangs from and for joints a decoupler on it holds; Jettison for a fairing panel; RCS on while RCS is off). `Propulsion::perform` does one, and it leaves the sequence that listed it. `Vessel::perform`, `Universe::perform` and the `part.do:<vessel>:<kind>:<part>` action carry it to the game; debris and the centre of mass shift the same way as for a stage.
- The part window (right-click a part) has a button per action (`self.actions[]` in `part_info`).
- Staging captions use the same words (Ignite, Decouple, Jettison, RCS on).
- Tests: `an_activation_can_be_done_by_hand_and_leaves_its_sequence`, `a_sequence_does_its_activations_in_the_order_listed`, `a_part_button_names_the_vessel_the_kind_and_the_part`.

## Open

- A tile dropped on another goes just before it (`staging.drop_before`), within its row or from another, which is how to change the order inside a row. The buttons in a part's window are tried by the pointer script of T158 (`OSP_CAPTURE_STAGING_POINTER=1`): a right-click on a tile opens the window, its Decouple button does it and takes it out of its sequence.
- Cut off by hand and in a sequence came with T026. Parachutes have no system yet, so there is nothing to offer.
- "Sequence" stays the name in code and craft files and on the part window's row: Section is already called stage in the sim, and renaming either is not worth it now.
- Not tried with a real mouse.
