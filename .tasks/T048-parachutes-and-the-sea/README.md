# T048: Parachutes and the sea

A procedural parachute and the sea as physics. A capsule hits the ground today at 93 m/s: no parachute part, and the sea is just the terrain's zero.

## Scope

- Canopy area from the vehicle mass and the descent speed it must reach; a type (ringsail, ribbon, cruciform) for drag and opening shock; pack mass from area and material.
- Arm, deploy by altitude or dynamic pressure as an action, reefing and disreef, cut; drag along the velocity; opening load that breaks a part or a chute if too fast. Drogues and mains are the same part with different requirements.
- Retro rockets and airbags use the same triggers.
- Buoyancy from the displaced volume (the geometry integration exists), splash loads, floating attitude and flotation bags, recovery.
- Also wanted: max g for a canopy, and radial parachutes.

## Done when

- The CM lands at about 9 m/s and floats upright; the smoke test's known issue 11 flips to a pass.

## Where it comes from

- docs/systems-plan.md (step 23)
- old issue list, issue 11 (docs/issues.md, removed; see git history)
