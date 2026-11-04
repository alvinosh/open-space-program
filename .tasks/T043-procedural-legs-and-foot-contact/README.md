# T043: Procedural legs and foot contact

A procedural leg (strut length and stroke from mass, touchdown speed and slope; crush core that absorbs energy once; footpad area from ground pressure; deployed and stowed), and a contact solver that generalises `gear.rs` from a wheel to a foot. Tip-over comes out of the contact solver.

## Done when

- Four legs go on a descent stage with one radial pattern from the builder.
- A lander touches down at 1 m/s unharmed, crushes its struts at 5, and breaks at 12.

## Notes

- The foot contact uses the colliders of the colliders task; animated colliders for legs are also wanted.

## Notes

- 2026-11-04, from T041: on the Saturn V the legs are folded up the descent stage to fit in the adapter, by the direction of their nodes, and nothing unfolds them. This has to do that. Legs of a carried stage do not count (`gear::legs`).
- T179 added the foot and four legs on the lunar module; the crush core, stowing and the sizer are still to do.
- 2026-11-02: moved up for the Moon mission. It no longer waits for colliders (T092), actuated joints (T030) or lunar terrain (T042): the foot is the gear contact without rolling, on the smooth Moon first.

## Done

- A leg with `stowed` stands on nothing and is drawn folded until a `deploy_leg` action, on any craft (`systems/leg.rs`). Legs of a carried stage do not count either.
- Coming down faster than 5 m/s the crush cores take it, faster than 12 m/s the legs break and the vessel is lost.

## Left out

The limits are constants and not sized from the leg, the legs unfold at once, and nothing sizes a leg from mass and slope. T194.

## Where it comes from

- docs/systems-plan.md (step 18)
