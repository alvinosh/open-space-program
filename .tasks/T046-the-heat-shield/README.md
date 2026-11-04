# T046: The heat shield

An ablator layer sized from the heat load the trajectory gives, with recession, charring and a failure when it is gone. It covers the outer surface that faces the flow (the voxels know) and takes its nose radius from the geometry.

## Done when

- The Apollo entry survives, and the same entry burns through a half-thickness shield.

## Done

- `heat_shield` on a part: what falls on the surfaces it covers is radiated from a glowing surface or burns ablator, and goes into the part once there is none left. Saturn V's entry burns 37 kg of 850 kg.

## Left out

The ablator's mass does not come off the part, it does not char, and it is not sized from the heat load, so half a shield is not shown to burn through. T194.

## Where it comes from

- docs/systems-plan.md (step 21)
- old issue list, issue 13 (docs/issues.md, removed; see git history)
