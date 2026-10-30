//! Render the layers of a baked dataset around a point into PNGs, for checking a bake.
//!
//! `cargo run -p osp_terrain --example preview -- <dataset dir> <lat> <lon> <level> <out dir>`

use std::path::PathBuf;

use osp_terrain::{
    Cover, Dataset, Layer, TILE_SAMPLES, TileKey,
    cube::{from_lat_lon, sphere_to_cube},
};

const PALETTE: [[u8; 3]; Cover::COUNT] = [
    [20, 40, 90],    // ocean
    [40, 90, 160],   // lake
    [240, 240, 250], // ice
    [150, 130, 110], // barren
    [230, 215, 170], // sand
    [140, 150, 120], // tundra
    [150, 190, 90],  // grassland
    [120, 140, 70],  // shrubland
    [170, 170, 80],  // savanna
    [220, 200, 110], // cropland
    [80, 150, 140],  // wetland
    [30, 110, 70],   // mangrove
    [20, 80, 40],    // needleleaf
    [10, 100, 20],   // tropical
    [60, 130, 40],   // deciduous
    [40, 110, 50],   // mixed
    [200, 60, 60],   // urban
    [128, 128, 128], // concrete
    [40, 40, 44],    // asphalt
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [dir, lat, lon, level, out] = &args[..] else {
        eprintln!("usage: preview <dataset dir> <lat> <lon> <level> <out dir>");
        std::process::exit(2);
    };
    let data = Dataset::open(&PathBuf::from(dir)).expect("open dataset");
    let (lat, lon, level): (f64, f64, u8) = (
        lat.parse().unwrap(),
        lon.parse().unwrap(),
        level.parse().unwrap(),
    );
    let (face, s, t) = sphere_to_cube(from_lat_lon(lat.to_radians(), lon.to_radians()));
    let key = TileKey::containing(face, s, t, level);
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let n = TILE_SAMPLES as u32;
    let save = |name: &str, pixel: &dyn Fn(f64, f64) -> [u8; 3]| {
        let image = image::RgbImage::from_fn(n, n, |i, j| {
            // Row 0 at the top is the far edge in t.
            image::Rgb(pixel(i as f64, (n - 1 - j) as f64))
        });
        image.save(out.join(format!("{name}.png"))).unwrap();
    };
    for layer in Layer::ALL {
        let Some(view) = data.view(layer, key) else {
            continue;
        };
        println!("{layer:?}: data level {}", view.data_level());
        match layer {
            Layer::Height => {
                let (lo, hi) =
                    (0..TILE_SAMPLES * TILE_SAMPLES).fold((f32::MAX, f32::MIN), |(a, b), k| {
                        let h = view.height((k % TILE_SAMPLES) as f64, (k / TILE_SAMPLES) as f64);
                        (a.min(h), b.max(h))
                    });
                println!("  height {lo:.1} .. {hi:.1} m");
                save("height", &|i, j| {
                    let v = ((view.height(i, j) - lo) / (hi - lo).max(1.0) * 255.0) as u8;
                    [v, v, v]
                });
            }
            Layer::Color => save("color", &|i, j| view.rgb(i, j).map(|c| c as u8)),
            Layer::Cover => {
                save("cover", &|i, j| {
                    PALETTE[view.byte_nearest(0, i, j) as usize % Cover::COUNT]
                });
                save("water", &|i, j| {
                    let v = view.byte_linear(1, i, j) as u8;
                    [v, v, v]
                });
            }
            Layer::Lights => save("lights", &|i, j| {
                let v = view.byte_linear(0, i, j) as u8;
                [v, v, v]
            }),
        }
    }
}
