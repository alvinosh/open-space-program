# T300: Several engines under one tank

Said by the user on 2026-11-12: "it is very hard to add more than 1 engine to the bottom of a tank".

A tank's bottom node takes one part. Putting more there means fins-style side attachment low on the
tank, which stands an engine beside the tank, not under it. KSP does it with engine plates and with
engines surface-attached to the bottom of a tank with symmetry.

## Scope

- Find out first what a player tries and where it goes wrong, in a capture.
- Then one way that reads at once: radial symmetry on a tank's end puts the engines round its axis
  under the end, at a spacing that keeps them apart, slid out or in with the Move tool afterwards.
  If that is not enough, a procedural engine plate (a configurable part, not a special case) with N
  nodes under it.
- The engines' thrust and fuel draw work as for one engine (tank contents follow the engines, T209).

## Done when

- In a capture, four engines go under one tank with symmetry 4 in one placement, the craft flies, and
  undo takes all four off in one step.

## 2026-11-13

Found in T295's capture: a Casual career's first node has K-0s (26 kN each) to cluster and nothing
else that lifts a C-1 capsule, so without this a first rocket can carry only a 1.2 m tank. It holds
up Casual's first flights.

## Done (2026-11-13)

- An end of a part (a surface facing within 45 degrees of its axis: a tank's domed bottom, its thrust
  plate, a top) takes what would stand beside it (an engine, a tank, a pod) by its own node facing
  that end, along the axis: the engine hangs straight under the tank, and with symmetry the copies go
  round the axis at the same radius, in one placement and one undo. Where the pointer is sets the
  radius; the Move tool slides them after. Near the end's node the node still wins, for one engine
  in the middle.
- `at.axial` on a surface place (`SurfaceAtDef`, `Mount::Surface`): the part stands along the
  holder's axis out of the end the surface faces, not square to it, so engines on a domed end do not
  splay. They draw on the tank as any engine on a tank does.
- Test: `builder::tests::four_engines_go_under_a_tanks_end_along_its_axis` (four in one placement,
  upright, at one height and radius, fed, one undo). Capture: `OSP_CAPTURE_CLUSTER` (symmetry 4,
  pointed at the tank's bottom from below: four K-0s under a 2.5 m tank, undone and put back; the C-1
  on them lifts off at a TWR of 1.4 with 4,677 m/s).

## Left out

- The tank's bottom has to be looked at from below to point at it; the parts do not snap to a ring
  or keep apart by themselves.
- What the player tried first was not captured before the change (the Scope's first point).
