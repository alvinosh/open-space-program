# T125: Engine sounds, wind, layered sounds

Engine sounds driven by throttle and distance, clusters; wind; layered sounds.

## Done when

- Everything in the scope list works, and what can run headless has a test.

## Done

- `sound.rs` synthesises everything as it plays (no sound files, so nothing to license): the roar of every lit engine in the universe, how loud from its thrust and exhaust speed, the distance to the camera and the air around it (none in vacuum), how deep from the size of the cluster's jets, with the highs taken out by distance; the jet's crackle, strongest from a solid; the hull's rumble under thrust for the vessel being flown, which is all there is in vacuum; wind from dynamic pressure and airspeed, buffeting through Mach 1; RCS hiss; thumps for parts falling away, ignition, landing and crashing. It goes quiet behind the menu and in scripted captures.
- Tests: `sound::tests` measure the levels (a Saturn V at 100 m is full scale, 40 dB down at 10 km, nothing in vacuum, deeper for bigger jets) and what the synthesiser renders (silent with nothing going on, never clips, crackle is pops, the tones are where they should be, a bang rings out).
- Volumes for all of it, the engines, the wind and the effects are on the menu's Sound page (T128).

## Still open

- It sounded very bad to the user on 2026-11-02, so this task is open again. Rendering it and measuring found why: a small rocket and a solid had their power centred near 2 kHz, like a detuned radio; the roar ran at -13 dBFS RMS without letting up; a Saturn V and the hull were centred under 100 Hz, which laptop speakers barely play; the level hardly moved; and crackle was bursts of white noise. The voices were rebuilt against those measurements, and `sound::tests` now holds them there.
- Nobody has judged the rebuilt voices by ear, which is what closes this. `cargo test -p osp_game render_sound_samples -- --ignored` writes scenes and a whole flight to `target/sound` to listen to.
- No delay for the distance, no panning, and one roar per listener rather than one per engine.

## Where it comes from

- Early feature checklist (since removed), Phase 9
