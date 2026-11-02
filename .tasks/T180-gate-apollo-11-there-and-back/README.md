# T180: Gate: Apollo 11 from the pad to the Moon and back to splashdown

What "the core game works" means for the Moon: one flight, from LC-39A, of the Saturn V with its real lunar module.

## Done when

- `cargo smoke saturn_v` flies: launch, translunar injection, transposition and docking, lunar orbit, undocking, landing, ascent, rendezvous and docking, crew back in the command module, trans-Earth injection, entry and a splashdown the capsule survives.
- The script has no `bridged` finding, and no `gap` or `known` finding for any of those steps.
