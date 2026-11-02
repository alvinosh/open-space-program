# T074: Decide the contact solver

Rigid-body contact for the colliders: Avian, Rapier, or our own solver on top of the current integrator. It must keep the exact-on-rails guarantees.

## Done when

- The choice and its reason are written into the README of the colliders task, and the colliders task is unblocked.

## Decision, 2026-11-02

Our own solver on the current integrator, with our own narrow phase over triangles (`contact.rs`, `collision.rs`).

Rapier and Avian would each own the rigid bodies and step them, and our vessels are conics and Encke states that have to stay exact on rails and be identical at any warp. A contact is a short event between two rigid bodies we already integrate, so a solver only has to change their velocities on a physics tick, and rails stop short of a meeting. Neither library's mesh contact fits either: they want solids or convex pieces, and our parts are thin shells with things inside them, so a convex collider would fill in a nozzle or an adapter. `gltf`, already in the tree through Bevy, reads the model meshes.

## Where it comes from

- Early feature checklist (since removed), Decisions to make
