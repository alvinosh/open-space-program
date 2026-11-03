# T092: Colliders per part and rigid-body contact

Colliders per part (box, capsule, cylinder, sphere; animated, for example legs), a compound centre of mass, and rigid-body contact with friction and rolling resistance. The docking ports and the legs use them.

## Done when

- Everything in the scope list works, and what can run headless has a test.

## Done so far, 2026-11-03

Brought forward from M6 because docking could not work without it, and redone on Rapier after plane collisions proved it was not enough (see T074).

- `collision.rs`: a part's collider starts from what is drawn of it: its tessellated surfaces, the triangles of its glTF models (`collision/model.rs`), and a strut and foot for a leg. Engines, legs, antennas, thrusters and wings all collide.
- `collision/cut.rs`: those triangles are cut into convex hulls that stand off the surface by 3 to 5 cm at most. Closed things are solid and open things are hollow, from their shape alone. It runs in the background as vessels draw near: about 1 s for a Comet, 5 s for a Saturn V.
- `contact.rs`: Rapier solves two vessels at a time in a small world of their own, kept from tick to tick so that contact persists. The faster one is swept along its path, so nothing goes through at any speed.
- Between any two vessels that have been 5 m apart, at least one more than debris. Closing over 15 m/s, both are marked as broken up. A vessel parked on wheels or a belly is shoved along by a hit and rolls to a stop on its brakes; one standing on anything else does not move.
- F9 draws the convex pieces. `tests/collision.rs` holds them to the real models, and `tests/plane_collisions.rs` flies real aircraft into each other.

## Still open

- Nothing breaks off: a hit marks the whole vessel as broken up or none of it, and it stays in one piece. Next.
- The ground. Vessels still meet it with gear legs and the old hard points (`HardPoints` in `vessel.rs`), rings of the aero outline and wing tips. They should use these pieces, and a touch should be a contact and not always a crash.
- Moving parts: legs collide at full extension, and a control surface or a hinged panel as built.
- Debris does not collide with debris.
- Three vessels touching are solved as two pairs in turn, not together.

## Where it comes from

- Early feature checklist (since removed), Phase 4
