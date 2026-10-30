# T051: Structure: joint strength, breakup, crash tolerance

Joint strength and breakup, crash tolerance per part, g and dynamic pressure limits, using the loads `structure::joint_loads` already computes.

## Done when

- A stack over its joint strength breaks at the weakest joint, and a part hitting the ground faster than its tolerance is destroyed.

## Notes

- Also covers: crash tolerance per part, part destruction, debris shedding, breakup with warnings. Dents and explosions are their own task.

## Where it comes from

- docs/systems-plan.md (step 26)
