# T333: Smoke: a Casual career from the first launch to the first orbit

Part of T301. The career smoke harness grows from one flight (T291) to many in one game: between
flights the career takes on what it is offered, unlocks the nodes the next craft needs with its
science and upgrades the facilities it needs with its money, as a player does; each craft is built
in the builder from what is unlocked and within the facilities.

Casual, from the first launch to the first orbit: sounding flights for the 1 km, 10 km and 30 km
contracts, space, a person to space and back, then a rocket to orbit (a K-1 cluster under one tank,
an A-10 upper stage) once Basic and General Rocketry are unlocked and the pad upgraded.

## Done when

- `cargo smoke career_casual_orbit` passes with no gaps, each flight's contract met.
- The report gives per flight: the craft, which tech nodes its parts come from, the science and
  money it earned, what was unlocked or upgraded after it, and the game date.

## Built (2026-11-20)

`crates/osp_smoke/src/career_orbit.rs`, mission `career_casual_orbit` (`cargo smoke
career_casual_orbit`); the shared pieces stay in `career.rs` (`first_rocket`, `capsule_hop`,
`recover`, `Hangar::under_end` for clusters; `Hangar` now puts parts on with `put_set`, as the
editor does, so tanks follow their engines). 111 checks, no gaps. In 0.1 days of game time:

| Flight | Craft | Contract | Earned | Bought after |
| --- | --- | --- | --- | --- |
| 1 | First Rocket, the list's booster (22 km, 7.8 g) | First launch | 11.2 science, 8,000 | Basic Rocketry |
| 2 | the same | 10 km | 5.6, 12,000 | Launch pad level 2 |
| 3 | Sky Rocket, 1.9 m by 2.4 m booster at 300 kN typed in (242 km, 12.3 g) | 30 km | 10.8, 18,000 | General Rocketry |
| 4 | the same | Space | 5.4, 30,000 | Builder level 2 |
| 5 | the same | A person to space and back | 2.7, 45,000 | |
| 6 | Orbiter: C-1, A-10 stage, three K-1s under a 3 m tank, 132 t | Orbit (165 by 903 km) | 0, 90,000 | |

Found on the way:
- A typed 1.25 m by 6 m booster at 2.9 g flipped at Mach 3.5 and 21 km: too fast too low for its
  fins. One as wide as the capsule, slower off the pad, flies straight.
- From 343 km and more the crew pull 15 to 20 g coming back (dead at 20 g, the contract's limit is
  15); 242 km gives 12.3 g.
- A tank the parts list gave too heavy for the engine put under it stayed so (pod, tank, A-10: a 16 t
  stage on 44 kN). `fitting::size_for_engines` now makes it smaller.
- A tank with a cluster round its end is not sized for it: T337.
