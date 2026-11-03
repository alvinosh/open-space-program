# Engine sounds

Not ours, and not under the licence of the rest of `assets/core`. All six are by **newlocknew**, from
the [Aircraft.Rocket](https://freesound.org/people/newlocknew/) pack on Freesound, licensed
[CC BY-NC 4.0](https://creativecommons.org/licenses/by-nc/4.0/): free to use and change with credit,
**not for commercial use**.

| File | Original | Source |
| --- | --- | --- |
| `roar_launch_slow_start.ogg` | AERORckt_Rocket launch 3.Roar.Crackle.Squawking_EM_(12lrs).wav | <https://freesound.org/s/627115/> |
| `roar_launch_fast_start.ogg` | AERORckt_Rocket launch 4.Roar.Crackle.Squawking_EM_(12lrs).wav | <https://freesound.org/s/627114/> |
| `roar_launch_deep.ogg` | AERORckt_Rocket launch 9-2.Roar.Crackle.Squawking_EM_(12lrs).wav | <https://freesound.org/s/627113/> |
| `roar_steady.ogg` | AERORckt_Rocket roar 3-3.Steady (more low end).Crackle.Squawking_EM_(11lrs).wav | <https://freesound.org/s/627117/> |
| `roar_distant.ogg` | AERORckt_Rocket roar 5.Distant.Steady.Crackle_EM_(8lrs).wav | <https://freesound.org/s/627116/> |
| `roar_very_distant.ogg` | AERORckt_Rocket roar 12.Very Distant.Steady.Crackle_EM_(7lrs).wav | <https://freesound.org/s/627112/> |

Changes: these are Freesound's 24 kHz MP3 previews, re-encoded to Vorbis. The game cuts them into
sections, plays those at varying speed, crossfades between them and filters and mixes the result
(`crates/osp_game/src/sound/recordings.rs`).

## Replacing them

The originals are 48 kHz WAVs (a free Freesound account downloads them) and have the same timeline,
so they drop straight in for a cleaner top end:

```sh
ffmpeg -i original.wav -c:a vorbis -strict experimental -q:a 8 -ac 2 roar_steady.ogg
```

A recording with another timeline, such as one under a licence that allows commercial use, needs its
times changed in `LISTINGS` in `recordings.rs`. The tests there check that they still make sense.
