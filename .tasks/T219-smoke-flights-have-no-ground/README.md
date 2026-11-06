# T219: Smoke flights have no ground: the command module sank through land 200 m up

Seen watching Apollo 11 in the game (`cargo smoke-watch saturn_v`, 2026-11-06). The command module
came down on ground drawn about 200 m up, went on through it to sea level and "splashed down" there.
It landed at 0.81°S 91.22°W, on Isabela in the Galápagos. Apollo 11 came down at 13.30°N 169.15°W,
about 8,700 km away.

Why: the mission harness builds its universe without terrain. `World::new` passes no height sources to
`Universe::new_game`, where the game's `content::new_game` passes the baked datasets. Only the launch
sites' earthworks are there, so the Earth is a smooth sphere at its mean radius. The sim calls a place
sea when the body has an ocean and the terrain there is at 0 m or below, so in the smoke flights the
whole Earth is sea, and "It comes down on the sea and floats" passes wherever the capsule lands. A
watched flight is the same universe drawn over the game's terrain, so the capsule sinks into the land.

In normal play the ground is solid; this is the harness only.

## Done when

- Smoke missions fly on the terrain the game flies on: the heights from the baked datasets, installed
  as `content::new_game` installs them. The baked Earth data is not in git (`cargo run -p osp_bake
  --release`); without it the report says the flight had no terrain, as a finding, and it flies the
  smooth sphere as now.
- The command module stands on land where it comes down on land and floats where it comes down at
  sea, and a watched flight shows it on the drawn ground, not under it.
- "It comes down on the sea and floats" fails when it comes down on land.
- The report gives where it came down (latitude, longitude, the ground's height) next to Apollo 11's
  point.
- The 8,700 km miss is looked at: the entry aiming for the Pacific is fixed, or it gets a task of its
  own and the report marks it known.
