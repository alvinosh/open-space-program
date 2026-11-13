# T304: Recover the vessel being flown once it is down

Found while building T273. Only a vessel other than the one being flown can be recovered today, from
the manifest (landed on a world with a launch site). A career's usual loop is: fly a capsule home,
land or splash down, recover it, and what its drives hold counts. That needs a Recover row for the
vessel being flown (pause menu, or a button once it is down), which then goes to the hangar.

## Done when

- A capsule that has landed or splashed down can be recovered while it is being flown, and the game
  goes on to the hangar (capture).
- What its drives hold counts in the career, as from the manifest (`Universe::recover_vessel`).

## Done (2026-11-13)

- `Universe::recoverable(id, sites)`: down (on the ground or afloat) on a world with a launch site.
  The manifest uses it too, in place of its own check.
- `ask.recover[:<id>]` asks first (`Question::confirm`, the Recover button the primary colour); no id
  is the vessel being flown. `vessel.recover:<id>` (the hangar's) brings it home with
  `Universe::recover_vessel`: another vessel just goes, with the notice in the menu; the one being
  flown goes and the hangar opens with the notice ("Recovered New vessel with 0.2 kB of data"),
  Back from it flying another vessel if there is one, or leaving for the title if not.
- The pause menu has Recover vessel (with the vessel's name) while it can be, outside tutorials
  (`menu.can_recover`, `menu.flown_name`); the Vessels screen has a Recover button beside Destroy
  for each that can be (`item.can_recover`).
- Test: `universe::tests::a_vessel_can_be_recovered_only_when_it_is_down_on_a_world_with_a_launch_site`.
  Capture: `OSP_CAPTURE_SCIENCE` now ends by recovering the capsule from the pause menu, asked, and
  the hangar saying what came home (`science_recover_menu.png`, `science_recover_asked.png`,
  `science_recovered.png`).

## Left out

- A splashdown is recoverable by the same rule (afloat is landed) but was not flown in a capture.
- The Vessels screen's Recover button was not seen in a capture.
