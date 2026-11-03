# T074: Decide the contact solver

Rigid-body contact for the colliders: Avian, Rapier, or our own solver on top of the current integrator. It must keep the exact-on-rails guarantees.

## Done when

- The choice and its reason are written into the README of the colliders task, and the colliders task is unblocked.

## Decision, 2026-11-02 (replaced above)

Our own solver on the current integrator, with our own narrow phase over triangles (`contact.rs`, `collision.rs`).

Rapier and Avian would each own the rigid bodies and step them, and our vessels are conics and Encke states that have to stay exact on rails and be identical at any warp. A contact is a short event between two rigid bodies we already integrate, so a solver only has to change their velocities on a physics tick, and rails stop short of a meeting. Neither library's mesh contact fits either: they want solids or convex pieces, and our parts are thin shells with things inside them, so a convex collider would fill in a nozzle or an adapter. `gltf`, already in the tree through Bevy, reads the model meshes.

## Changed, 2026-11-03

Rapier (`rapier3d-f64`) solves contact, on convex pieces we cut ourselves. The decision of 2026-11-02 below turned out wrong in practice: a one-shot impulse over triangle pairs left two Comets that clipped each other flying on welded together for seconds, logging a bump every tick, and a parked one was an immovable wall.

Neither reason against a library held up:

- "They would own the rigid bodies." Not if they are only lent. Two vessels near enough to touch are copied into a small world of their own each tick, with one of them as the origin and at rest, and only what the step changed is taken back. A vessel nothing touches comes out bit for bit the same, and on rails none of this runs, so what is exact stays exact.
- "A convex collider would fill in a nozzle." Not if it is cut up. Rapier's own decomposition (V-HACD) does fill them, in a way no setting fixes: it measures error by volume, so a thin disc across a hollow counts for nothing. `collision/cut.rs` measures how far a hull stands off the surface in metres instead, which keeps open parts open and closed parts solid, from the triangles alone, so procedural parts need nothing special.

Box2D is 2D, so it was never an option. Jolt through bindings would mean a C++ build for the same thing.

## Where it comes from

- Early feature checklist (since removed), Decisions to make
