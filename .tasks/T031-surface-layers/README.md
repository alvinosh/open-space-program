# T031: Surface layers

A layer on a wall (an ablator, insulation, solar cells, a radiator panel) with its own thickness, material and area, drawn like paint and weighed from area, thickness and density. A wall can carry more than one.

## Done when

- A layer's mass is area times thickness times density, and it sits under a fairing without showing.

## Notes

- Decision recorded in the plan: coatings are layers on a wall; things with their own shape (dish, leg, hinged panel, parachute pack) are parts.

## Done (2026-11-02)

- `surface_layer` component (`SurfaceLayerDef`, `Component::SurfaceLayer`): material, thickness, the wall it covers (every wall if left out) and a look. It weighs area x thickness x density where the wall lies, changes no shape the air sees (a test compares the outlines), so one under a fairing shows nothing, and a wall can carry any number.
- Named `surface_layer` because `layers` in a part's paint already means layers of paint; the look it gives goes into the part's paint for that wall.

## Where it comes from

- docs/systems-plan.md (step 6)
