# T354: Decouplers carry what stands on them

Found on 2026-11-21 flying the Pathfinder rebuilt in the builder to orbit (smoke `first_orbit`):
at 6 g near booster burnout the joint under the upper stage broke and the vessel was lost. A decoupler
as the parts list gives it "Carries" 1 MN whatever stands on it (the upper stage and payload weigh
110 t: 7 MN at that g), and the field is typed in newtons. The builder sizes a tank for its engines and
a booster for what it lifts (T306, T332), but not a decoupler for what it carries. The rebuild
playtests now type each decoupler's load in by hand.

## Done when

- A decoupler put under a stack carries what stands on it at the most the craft below it can pull,
  as a tank is sized for its engines, and its field reads in tonnes or kN.
