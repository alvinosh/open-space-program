# T125: Engine sounds, wind, layered sounds

Engine sounds driven by throttle and distance, clusters; wind; layered sounds.

## Done when

- Everything in the scope list works, and what can run headless has a test.

## Done

- Engines are recordings since 2026-11-03: the synthesised roar sounded horrible to the user twice, so `sound/recordings.rs` plays six takes the user found (`assets/core/sounds`, CC BY-NC, see T177). A looper fades between random stretches of a bed so it never repeats; a take's own attack plays on ignition and its own tail on a cut. How loud still comes from `sound.rs`'s physics (thrust, exhaust speed, distance, air, none in vacuum), plus a throttle swing the physics alone hardly has. The size of the jets sets the playback speed, and distance and thin air move it to the distant takes. The hull is the most distant take, muffled; it also shakes through Mach 1.
- Space is silent, on the user's word of 2026-11-03. Air carries everything but the alert tone (`air_carries`: nothing below 10 Pa, about 65 km on Earth, all of it from 5 kPa, about 20 km), both where a jet is and where the vessel being flown is, so vacuum is silent by construction and not only by the levels, which would never reach silence on their own.
- Wind is still synthesised but rebuilt: a low-passed rush, a quiet howl and a rumble, with gusts that move the pitch and differ in each ear. It follows dynamic pressure and airspeed. At full strength it is about level with a launch; the old one could not be heard under the roar.
- No thumps at all, on the user's word of 2026-11-03: collisions (there are no explosions to go with them) and decoupling make no sound, and separation motors do not count as engines to hear, for the roar or the hull. The engines' own tail still plays when a stage's engines stop. Docking also lost its thump, as it only happens in space.
- The RCS hiss is gone. It was never RCS: it was on whenever `propulsion.firing()` was not empty, which means armed engines, so it played at about -20 dB from the first stage on, throttle or not, in space too. Nothing in the simulation says when RCS thrusters fire; a hiss in the air would need that first. It goes quiet behind the menu and in scripted captures.
- Tests: `sound::tests` measure the levels and what is rendered (silent with nothing going on, never clips, fades between stretches without a dip or a click, attack and tail play, wind is a rush and level with the launch, real engines throttled and cut, nothing but the alert tone without air, a whole flight silent in space). Loudness of the renders was also checked with `ffmpeg`'s `ebur128`: a Saturn V at 150 m -15.5 LUFS (the old synth: -23), Max Q about -14, re-entry -16, a slow plane -28.
- The wiring was checked by running the game with temporary logging (taken out again): the recordings load in under a second, levels follow the flight, staging gives one tail and no thump, and a vessel in orbit reports 0 Pa, which silences everything but the alert tone. That run did not get an engine lit in orbit, and the scripted ascent only reached about 38 km, so a burn in vacuum is covered by the tests (real engines at 0 Pa, the synthesiser without air) and not by a live burn.
- Volumes for all of it, the engines, the wind and the effects are on the menu's Sound page (T128).

The user closed this task on 2026-11-04. `cargo test -p osp_game render_sound_samples -- --ignored` writes scenes and a whole flight to `target/sound` to listen to.

## Left out

- The takes are Freesound's 24 kHz previews. The 48 kHz originals have the same timeline and drop in (`assets/core/sounds/CREDITS.md`).
- No delay for the distance, no panning, and one roar per listener rather than one per engine.

## Where it comes from

- Early feature checklist (since removed), Phase 9
