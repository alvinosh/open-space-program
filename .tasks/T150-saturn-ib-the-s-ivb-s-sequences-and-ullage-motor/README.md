# T150: Saturn IB: the S-IVB's sequences and ullage motors

The Saturn IB's S-IVB (J-2-225K, no sequences of its own) most likely fails to light after the S-IB the way the Saturn V's did, and has no ullage motors either. Not looked at yet (was part of issue 8 of the old list).

## Scope

- Give it sequences and ullage motors as the Saturn V has, and fly it with a test (as `tests/saturn_v.rs` does for the Saturn V).

## Done when

- The Saturn IB flies its sequences to orbit in a test.

## Done (2026-11-02)

- The S-IVB has two sized ullage motors, and the craft its own sequences. A new smoke mission, `saturn_ib` (`cargo smoke saturn_ib`), flies Apollo 7 from LC-34 with the stage key: S-IB 148 s (Apollo 7: 141 s), the J-2 lights on the motors' push, a 230 km orbit in 9.5 minutes (10 min 27 s).
- It could not have reached orbit with the lunar service module it was carrying: 30.5 t of CSM against the real 14.8 t. Its SPS tanks are now 1.27 m long, under 5 t of propellant, so it no longer carries the assembly `apollo_csm` part for part (`tests/assemblies.rs` checks the ids only).
- Flying it found a bug in `attitude::actuate`: an engine a few millimetres off the centre of mass's line counted as able to roll, and with so little roll authority SAS asked for all of it, which took the whole gimbal and left none for pitch (`rolls`, `ROLL_LEVER_SHARE`).

## Where it comes from

- docs/issues.md, issue 8 (removed)
