# Star background

`starmap.png` is the main camera's skybox: six 2048² cube faces stacked top to bottom. It
is generated, not checked in. Bake it with:

```sh
cargo run -p osp_bake --release -- --sky
```

This downloads NASA's Deep Star Maps 2020 (`starmap_2020_8k.exr`, 130 MB) into
`.bake-cache/` and writes about 45 MB here. Without it space is black.

Credit: NASA/Goddard Space Flight Center Scientific Visualization Studio, from the
Hipparcos-2, Tycho-2 and Gaia DR2 (ESA/Gaia/DPAC) catalogues.
