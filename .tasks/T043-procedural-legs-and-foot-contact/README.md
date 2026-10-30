# T043: Procedural legs and foot contact

A procedural leg (strut length and stroke from mass, touchdown speed and slope; crush core that absorbs energy once; footpad area from ground pressure; deployed and stowed), and a contact solver that generalises `gear.rs` from a wheel to a foot. Tip-over comes out of the contact solver.

## Done when

- Four legs go on a descent stage with one radial pattern from the builder.
- A lander touches down at 1 m/s unharmed, crushes its struts at 5, and breaks at 12.

## Notes

- The foot contact uses the colliders of the colliders task; animated colliders for legs are also wanted.

## Where it comes from

- docs/systems-plan.md (step 18)
