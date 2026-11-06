# T200: Alarm clock

Asked for on 2026-11-06: the transfer planner's Warp buttons should not warp. A separate alarm clock holds the times, and alarms are added to it from the transfer planner and from the other places that have a time.

## Scope

- Alarms that stop time warp at any rate, saved with the game.
- An Alarm clock window to see, warp to and delete them, and to set a timer.
- Setting one from the transfer planner (launch times, a picked departure), a manoeuvre (its panel and its menu in the map), a point in the map's menu, and the closest approach in the Target window.

## Done when

- Warp at the fastest rate stops on an alarm, without jumping past it, and says what it was for.
- A manoeuvre's alarm follows the burn when it is moved, and goes when the burn is deleted.
- Alarms survive a save and load; old saves still load.

## Done

- `osp_sim::alarm`: an `AlarmClock` on the `Universe`, in the save snapshot (`#[serde(default)]`, so older saves load with none). An alarm rings at a fixed time or `before` seconds ahead of a planned burn's start. A restart or new game empties it, and a revert puts back the clock as it was at launch.
- Planned burns have stable ids now (`plan::BurnId`), kept when a burn is moved or reshaped. Saves from before get new ids on load.
- The game never warps past the next alarm: rails stop on it, and its ringing drops warp to 1x. It comes off the clock, and the flight log (and the screen message) says `Alarm: <title>`.
- Window: `layouts/alarm_clock.json5`, F6 and the launcher, docked on the right under Orbit. The next alarm shows under the clocks, and a click there opens the window. The transfer planner's launch times have Alarm in place of Warp.
- What rings when: a launch time 60 s before it; a departure or a manoeuvre just before the burn starts (the same margin as Warp to burn); a map point or a closest approach at that time.
- Unit tests in `alarm.rs` and `plan.rs`. Scripted captures: `OSP_SCENARIO=leo OSP_CAPTURE_ALARMS=1` (a manoeuvre, a timer and a map point, three warps at the fastest rate each stopped by one), `OSP_SCENARIO=pad OSP_CAPTURE_LAUNCH_TIMES=1` (a launch time from the planner, warped to from the clock), and `OSP_SCENARIO=rendezvous OSP_CAPTURE_RENDEZVOUS=1` (closest approach).

## Left out

- Fixed times stay where they were set: a closest approach or a map point does not follow the orbit when a burn changes it. Only manoeuvre alarms follow.
- Other places keep their own Warp buttons (Orbit window, node panel, Target window): only the planner's were replaced.
- No sound when an alarm rings, no margin to choose per alarm, no notes, and no alarms for SOI changes or apsides of their own (the map's menu snaps onto apsides).
- An alarm for another vessel does not offer to switch to it.
