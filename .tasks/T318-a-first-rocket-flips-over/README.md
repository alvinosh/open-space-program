# T318: A first rocket flips over within 20 s with SAS on

Found by the `first_rocket` playtest (2026-11-16), Casual sandbox. The most obvious first rocket
(C-1 Capsule, the default tank under it, a K-1 Booster, four fins round the bottom of the tank),
SAS on, full throttle, staged: pitch fell under 60° about 17 s after liftoff, reached -88° and the
rocket was nose down at 7.5 km.

## What it was

Flown again outside the game (`cargo run -p osp_sim --example fly_craft --release --
target/playtest/first_rocket/craft.json`, `--stability` for the aero split): straight up to Mach
1.5, then the angle of attack grew from 0.1° to 47° in four seconds with SAS at full deflection, and
without SAS it went at Mach 1.4. The static margin was +0.2 m on the pad and negative from Mach 0.93
as the tank drained (the propellant settles low, so the centre of mass sinks towards the fins).

The aero model was right about the shape it was given. The body pushed with 1.76 per radian at 8.9 m
up, above the top of the rocket: a couple, from the tank's bare domes. The top dome made a waist
under the capsule's flat base and the bottom one a tail tapering to a point, both pushing the nose
away from the airflow. On a 2.8 m tank the bare domes cost about 2 m of margin.

## Done (2026-11-16)

- A cylinder tank from the parts list has skirts down to its domes' tips (a stage tank's shape: a
  plain cylinder outside), and so does a tank switched to a cylinder. A skirt that reaches its dome's
  tip keeps reaching it when the width or the dome changes (`CylinderTankDef::keep_skirts`, in
  `shaping::set` and the dome setting). The trim bands sit at the domes' rims.
- Fins from the parts list are 1.5 times as big for the hull they go on (root 1.8 x the hull's width,
  span 1.2 x): the first rocket keeps its centre of pressure behind its centre of mass to Mach 8.
- `first_rocket` now hopes for straight flight through the whole burn (60 s): met, pitch never under
  89.8°, past 124 km.

Still a letdown: thrust to weight 4.9 at liftoff (the crew get High g). The tank was sized for the
capsule before the engine went under it; sizing it for the engine is T309's first idea, which waits
on the user.
