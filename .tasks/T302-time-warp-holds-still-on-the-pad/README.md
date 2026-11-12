# T302: Time warp above physics warp holds still on the pad

Found on 2026-11-12 while capturing T272: a craft just launched from the editor stands on the pad, and
at 50x the warp shows 50x but the clock does not move (it stayed at 18:00:05 for ten seconds of
capture). At 4x, physics warp, it runs. The same with a bare capsule and tank, so it is not the
experiments. Instruments that take ten minutes on the pad (Realistic) need it, as does waiting for a
launch window there.

## Done when

- A craft standing on the pad warps at every rate the bar offers, its clock moving at that rate, and
  launches afterwards as it would have (capture), or the bar offers no more than works there and says
  why.

## Done (2026-11-12)

The cause was the radiation work (T267): for a crewed vessel that is not coasting, the time to the
next dose that matters is the dose over the rate, and on the ground under the air the rate is next to
nothing. The time was astronomically far off; adding it to the clock overflowed, and in a release
build wrapped round to a time in the past, which every rails jump was then cut to. A probe warped;
anything with a crew stood still at 50x and up.

- `Vessel::next_system_event` counts that dose only if it comes before the jump's end.
- `SimTime::add_seconds` and `add_seconds_ceil` saturate at the end of time instead of wrapping.
- The game stops warp with a notice ("something needs attention now") if a rails jump ever makes no
  progress without saying why, instead of showing a rate that does nothing.
- Test: `tests/pad_warp.rs` puts Pathfinder and Pathfinder Crew on the Cape's pad with every mod on,
  warps an hour in ten-minute jumps (exactly an hour), then lights the engine and lifts off. With the
  old code the crewed one moved 0 s.
- Capture: `OSP_CAPTURE_SCIENCE` warps at 50x on the pad, and the thermometer finishes its set (75 B,
  one whole set). `OSP_CAPTURE_WARP`, `OSP_CAPTURE_ALARMS` (in `leo`), `OSP_CAPTURE_LAUNCH_TIMES` and
  `OSP_CAPTURE_REVERT` pass.
