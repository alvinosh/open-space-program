# T197: City lights from up close

The night lights come from `BlackMarble_2016_3km_gray.jpg`, 3 km to the sample, stretched over the ground as one orange smear from about 9 km up.

## Done when

- At night from the climb, towns read as towns with dark country between them, at any graphics setting, and the view from orbit still holds.

## Done

- The map of lights alone, with a gentle unevenness up close that fades by the size of a pixel (`city_detail` in `shaders/terrain.wesl`).
- `OSP_SCENARIO=pad OSP_CAPTURE_NIGHT=1` is the check up close, and `OSP_SCENARIO=leo OSP_CAPTURE_TRANSFER=1` from orbit, as one brightness change here nearly lost the cities from up there.

## The 500 m map (same day, the user's choice)

- `osp_bake` reads NASA's eight `BlackMarble_2016_*_geo_gray.tif` tiles, checking each one's geo tags, and `--lights-only` rebakes just this layer. Night lights go to level 7, and deep tiles with nothing lit are left out: 54.5 MB, against the 125 MB first guessed.
- The map alone changed little on screen: a far chunk only had its own 129 samples, kilometres apart. So each chunk now has a lights texture of its own (`planet::lights_image`, `Dataset::finer_bytes`), finer than the chunk, with mip levels and anisotropic filtering. A chunk with nothing lit takes one texel.

## Softened (same day)

The user found it too sharp, with individual lights showing. Along the way there had been a street grid, a grain of lit lots (after EVE's detail texture, but made in the shader, as EVE's is a satellite photo of a real city of unknown origin) and a mask from the land cover, each needed only while the map was 3 km. They read as graph paper, gravel, cobblestones and flat pale patches with hard edges in turn, so all went. A chunk's lights go one level finer than it rather than two.

## Left out

- At the horizon the view is so oblique that the lights blur together along the line of sight.
