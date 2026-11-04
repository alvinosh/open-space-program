# T193: A launch needs a clear pad or runway; vessels can be destroyed from the menu

Asked for by the user on 2026-11-04: a craft launched onto an occupied pad used to overlap what was there.

## What it does

- `Universe::on_launch_ground`: the vessels on a site's pad or anywhere on its runway, including debris, up to 150 m above it and 20 m past its edge.
- The editor refuses to launch from there and offers to clear it (`editor.clear`, asked once). A new game is exempt: everything goes when it starts. An orbit stands on nothing.
- The pause menu's Vessels screen (it was Switch vessel) destroys any vessel (`vessel.destroy`, asked once). Destroying the one being flown flies another.
- A game always has a vessel in it. With the last one destroyed the hangar is what is left, and going back from it leaves the game for the title screen.
- The Mission results window was not in the flight group, so it stayed up over the menu and the hangar. It is now.

## Done when

- `OSP_CAPTURE_CLEAR=1` passes: a refused launch, clearing, launching; a plane on the runway blocking the next; destroying one not flown, one flown, and the last.
- The sim tests cover the pad, the whole runway, and what is not on them.
