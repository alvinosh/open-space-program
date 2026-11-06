# T080: Warp to any point on the orbit; warp up to 100 years per second

Warp to any point on the orbit, and warp levels up to 100 years per second. Engines cannot start above 30 times.

## Done when

- Everything in the scope list works, and what can run headless has a test.

## Where it comes from

- Early feature checklist (since removed), Phase 1

## Done

- Warp to any point on the orbit: the map's menu (right-click the trajectory, Warp to here) already did it, snapping to apsides, plane crossings and closest approaches, and it can be put on the alarm clock (T200).
- Fifteen warp levels: 1, 2, 3, 4 (physics), then 10, 50, 100, 1,000, 10,000, 100,000, a million and ten million times, then 1, 10 and 100 years a second, on rails. A warp to a time or an alarm goes up to the top one too. The rate reads as times up to 100,000x, then as days or years a second (`time.warp_label`). The time panel is 264 px wide to fit the chevrons.
- Rails over a century cost little: 60 frame-sized jumps of 100 years a second took 0.2 ms in low orbit and 18 ms at 150,000 km, measured headless.
- Above 30x (`sim::ENGINE_START_WARP`) no engine starts: raising the throttle, a sequence that lights one and lighting one from a part's window are refused with "Engines cannot start above 30x: slow down first", and the warp goes on. Aborts are let through, being for emergencies. At 10x a sequence still lights it and warp drops to normal.
- `OSP_SCENARIO=leo OSP_CAPTURE_WARP=1` checks all of it.

## Left out

- The throttle cannot be raised at all above 30x, even with no engine lit, so what it does is the same every time.
