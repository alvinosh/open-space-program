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
