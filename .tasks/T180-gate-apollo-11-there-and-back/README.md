# T180: Gate: Apollo 11 from the pad to the Moon and back to splashdown

What "the core game works" means for the Moon: one flight, from LC-39A, of the Saturn V with its real lunar module.

## Done when

- `cargo smoke saturn_v` flies: launch, translunar injection, transposition and docking, lunar orbit, undocking, landing, ascent, rendezvous and docking, crew back in the command module, trans-Earth injection, entry and a splashdown the capsule survives.
- The script has no `bridged` finding, and no `gap` or `known` finding for any of those steps.
- Nothing is hard-coded for Apollo (the user's word, 2026-11-04: someone can make their own Apollo 11 type mission). No Apollo, Saturn or Eagle names or special cases in `crates/` outside tests: all of this flight is general parts and data. The editor is not needed for this gate.
- The entry is part of it (the user, 2026-11-04): T045, T046 and T047 moved here from M5.
- A completed Saturn V (the user, 2026-11-04): the launch escape system (T059) and the parachutes (T048) are on it.

## Done (2026-11-04)

`cargo smoke saturn_v`: 143 passed, 0 failed, 0 gaps, 0 known, 0 bridged. A pilot script flew it, not a person: nobody has flown it by hand yet. What each task simplified is listed in it and gathered in T194, as the user asked not to chase the values for now.
