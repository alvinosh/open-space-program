# T163: Glider rockets off the runway

The Kestrel sailplane had no wheels and could only be released in the air. It now takes off from the runway on a dolly of three wheels with two solid rocket boosters either side of its tail boom, which put it at about 70 m/s in eleven seconds, and lets go of the boosters and then the wheels.

## Done

- Core has its first solid motor: engine `S-1` (3.3 kN, 12 s, `kind: solid`), the `HTPB` resource and the cast `Solid Grain` tank type (the Realism mod's of the same ids replace them).
- `glider` craft rebuilt by hand: two `RATO booster` radial stages on the tail boom (thrust line level with the centre of mass), a `Dolly` radial stage on the cabin's belly with the nose and main wheels; sequences: light the boosters, let them go, let the dolly go.
- `gear::legs` carries a leg out with the stage it hangs on (a stage beside the axis); before, a dolly's wheels sat on the vessel's axis.
- Scenario `glider_runway` (runway start). The air-release `glider` scenario was removed (2026-10-30), with its menu tile (the menu's 'In flight' place shows only when a scenario starts in the air, and none does now) and its capture script.
- Tests: `vessel/glider.rs` (wheels below the belly, level on the runway; takes off at 40 to 50 m/s, 65 to 78 m/s at burnout, boosters and dolly let go, glides on). `OSP_CAPTURE_GLIDER_RUNWAY=1` captures it.

## Open

- Without the dolly there is no way to land: a vessel with no gear that touches the ground faster than 10 m/s is a crash, so a glider that dropped its wheels cannot come home. A belly landing on the hull (skid) is the next step; keep the dolly to land on wheels.
- A solid still stops when the throttle goes to 0 and needs the throttle up before it lights (T154: a solid cannot be throttled or shut down).
- The booster and dolly are plain shells; no models. The S-1 draws the K-0's model at a third of its size.
- Not flown by hand in the running game (only the scripted capture).

## Where it comes from

- Raised in play on 2026-10-30: the glider should take off from the runway with two solid boosters strapped to the side that put it at 70 m/s, then be decoupled, wheels too.
