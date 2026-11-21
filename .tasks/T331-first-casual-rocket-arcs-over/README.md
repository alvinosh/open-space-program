# T331: A new Casual career's first rocket, as the parts list sizes it, arcs over into a 3 km hop

Found by the career smoke test (T291, `cargo smoke career_casual`) on 2026-11-20.

A C-1 Capsule with its parachute, a decoupler and the solid booster from the parts list under it,
four fins: what First Steps offers for a first rocket. The booster takes the decoupler's width
(1.9 m) and three times that in length, 23 t of grain, and keeps the list's 400 kN: 28 t on the pad,
1.3 to 1.5 times its weight. Nothing on First Steps can steer (no reaction wheel, the C-1 has none
and the booster no gimbal), so SAS does nothing and the rocket turns into its own flight path. At
1.5 g that path bends over: pitch 90 to 0 degrees in 48 s, still burning, 2.7 km up at 360 m/s. A
player who stages the capsule off there tears the parachute (63 kPa) and lands at 100 m/s. The
flight earns 4.8 science, short of the 5 the cheapest node costs.

With the booster's thrust set to 800 kN in its settings (2.6 g) the same rocket goes to 124 km and
earns 11.2, which is what the smoke test flies now.

## To decide (ask the user)

- The C-1 gets a small reaction wheel, as KSP's first pod has, so SAS holds it straight; or
- a solid booster put under a part is sized for an unguided climb (2.5 g or more, so a shorter
  booster or more thrust), where `fitting::BOOSTER_TWR` (1.4) is for liquid stages that steer; or
- both, or neither and the tutorial says to raise the thrust.

## Done when

- `career_casual` flies the booster as the parts list gives it (no `BOOSTER_THRUST`) and earns a node.

## Decided and built (2026-11-20)

The user chose the reaction wheel. Pod records have `torque` (N·m about each axis, counted in the
pod's mass), added to the stage's wheel torque (`propulsion.rs`); the C-1 has 5,000 N·m, about what
KSP's first pod has. The Realistic Avionics mod sets it to 0 (`mods/realistic_avionics/patches/
pods.json5`): a capsule there turns by thrusters, as Mercury did. `career_casual` flies the booster
as the parts list gives it: 89 degrees up at burnout, 19.2 science, Aerodynamics unlocked. It goes
far too high now and the crew die coming down: T332.
