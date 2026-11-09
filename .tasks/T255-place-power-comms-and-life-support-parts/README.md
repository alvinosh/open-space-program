# T255: Batteries, solar panels, antennas, fuel cells and life support in the parts list

The simulation has them, the parts list does not, so only hand-written craft can use them.

## Done when

- Each can be placed and sized in the editor, and a probe built only from the list has power and a link.

## Done so far

Built 2026-11-09. A Power and comms tab in the parts list with solar panel, RTG, antenna, dish and
battery, plus a fuel cell (only with Realistic Fuels) and an avionics ring (only with Realism's records).
A probe core is under Pods. Each is sized in the part settings and its model follows the size.

Life support is not here: it is built into a crew pod (`Component::LifeSupport` on the part with the
crew), so a part of its own would do nothing. T258 takes it.
