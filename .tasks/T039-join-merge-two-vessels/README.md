# T039: Join: merge two vessels

`Assembly::release` and `split` exist; nothing joins. A join takes two vessels and a pair of connectors, merges the part trees, re-roots, adds mass and inertia, conserves linear and angular momentum, merges runtime state and sequences, and removes one vessel from the universe.

## Done when

- A property test joins and releases any two craft without changing either.

## Notes

- 2026-11-02: moved up for the Moon mission. It no longer waits for the builder (T015): a join works on two vessels in flight.

## Done

- `Universe::join` makes one vessel of two in flight, at two free nodes turned any way about the line they meet along (`Vessel::join`, `Propulsion::join`, `parts/join.rs`). The other vessel's tree is re-rooted at its node, its parts move into this vessel's frame and get ids of their own, and its tanks, systems and sequences come along; mass, inertia, plumbing and power follow from the parts. The pair's centre of mass, linear momentum and angular momentum go on exactly.
- Letting go of the joint (`Activation::decoupler` on the part `Propulsion::join` returned) gives the other vessel back: its numbers, root, frame, sequences, triggers, armed engines, name, plan, target and log, its parts where they stood to the bit. A vessel joined to it goes with it.
- A piece that comes off elsewhere (the S-IVB from a turned-round stack) goes on in the frame it was built in, the right way up to itself.
- Sections keep the relations they were built with whichever way a joint runs, so neither vessel blocks the other's exhaust. The burn executor and readouts use the bottom of the stack of the vessel it is flown from; the other's engines are off until lit by hand, but for a solid already burning.
- `tests/join.rs` joins and releases every ordered pair of craft (random free nodes and roll, up to two sequences fired first) and holds both to the bit: parts, joints, engines, tanks, sections, sequences, mass, inertia, delta-v, networks, part actions. The smoke reports were byte for byte what they were before this.

## Still open

- Nothing in the game calls it yet, so no one has seen a joined stack drawn: T040.
- What an ordinary decoupler sheds still loses its sequences (`from_released`). The lunar module coming off the CSM with the S-IVB must keep its own: T041 or T044.
- A joined vessel's rows in `stage_info` left out the mass of the vessel it joined: fixed with T040.
- A fin set on a section turned round is placed as if upright (`Shape::new`): wrong only for a joined stack with fins in air.
- The push at letting go adds no turn when it does not pass through the centres of mass, as in all staging.
- An empty sequence the player made on the vessel that joined stays with the other one when they part.

## Where it comes from

- docs/systems-plan.md (step 14, Join)
