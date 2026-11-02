# T092: Colliders per part and rigid-body contact

Colliders per part (box, capsule, cylinder, sphere; animated, for example legs), a compound centre of mass, and rigid-body contact with friction and rolling resistance. The docking ports and the legs use them.

## Done when

- Everything in the scope list works, and what can run headless has a test.

## Done so far, 2026-11-02

Brought forward from M6 because docking could not work without it.

- `collision.rs`: a part's collider is what is drawn of it: its tessellated surfaces, the triangles of its glTF models (`collision/model.rs`), and a strut and foot for a leg, with a bounding volume hierarchy over each. Engines, legs, antennas, thrusters and wings all collide. Hollow things are hollow.
- `contact.rs`: sequential impulses with restitution and Coulomb friction, conserving linear and angular momentum. The universe bisects to the moment of first touch and goes on from just before it, so surfaces do not sink in.
- Between any two vessels that have been 5 m apart, at least one more than debris. Closing over 15 m/s, both break up. A landed vessel does not move when hit.
- F9 draws the colliders. `tests/collision.rs` holds them to the real models.

## Still open

- The ground. Vessels still meet it with gear legs and the old hard points (`HardPoints` in `vessel.rs`), rings of the aero outline and wing tips. They should use these meshes, and a touch should be a contact and not always a crash.
- Moving parts: legs collide at full extension, and a control surface or a hinged panel as built.
- Nothing breaks off: a hit breaks up the whole vessel or none of it.
- Debris does not collide with debris.

## Where it comes from

- Early feature checklist (since removed), Phase 4
