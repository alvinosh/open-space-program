//! Bakes Earth terrain for Open Space Program from public datasets.
//!
//! ```sh
//! cargo run -p osp_bake --release            # download (~900 MB, cached) and bake
//! cargo run -p osp_bake --release -- --help
//! ```
//!
//! Global layers (elevation, colour, land cover, night lights) are baked to cube-sphere
//! level 5 (~2.4 km per texel). Around Cape Canaveral, 30 m elevation and 10 m land cover
//! go much deeper. Output: `assets/core/terrain/earth/`.
//!
//! `--sky` instead bakes the star background, `assets/core/sky/starmap.png` (see `sky`).

mod bake;
mod raster;
mod sky;
mod sources;

use std::{
    io,
    path::{Path, PathBuf},
    time::Instant,
};

use osp_terrain::{
    Layer, Meta,
    dataset::{FORMAT_VERSION, META_FILE},
};

use crate::{
    bake::{Earth, Global, Region},
    raster::{Geo, Pyramid, Raster, mean_f32, mean_rgb, mean_u8, mode_u8},
    sources::*,
};

/// Global quadtree depth: level 5 tiles are ~312 km wide with ~2.4 km texels, matching the
/// ~1.9 km sources.
const GLOBAL_LEVEL: u8 = 5;

struct RegionSpec {
    name: &'static str,
    lat: (f64, f64),
    lon: (f64, f64),
    /// Deepest elevation level (12 ≈ 19 m texels from 30 m data).
    height_level: u8,
    /// Deepest land-cover level (13 ≈ 9.5 m texels from 10 m data).
    cover_level: u8,
}

const REGIONS: &[RegionSpec] = &[RegionSpec {
    name: "Cape Canaveral",
    lat: (28.15, 29.05),
    lon: (-81.0, -80.1),
    height_level: 12,
    cover_level: 13,
}];

struct Options {
    out: PathBuf,
    cache: PathBuf,
    regions: bool,
    sky: bool,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn parse_args() -> Options {
    let root = workspace_root();
    let mut options = Options {
        out: root.join("assets/core/terrain/earth"),
        cache: root.join(".bake-cache"),
        regions: true,
        sky: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => options.out = args.next().expect("--out needs a directory").into(),
            "--cache" => options.cache = args.next().expect("--cache needs a directory").into(),
            "--no-regions" => options.regions = false,
            "--sky" => options.sky = true,
            "-h" | "--help" => {
                println!(
                    "osp_bake: bake Earth terrain tiles from public datasets\n\n\
                     Options:\n  \
                     --out <dir>     output (default assets/core/terrain/earth)\n  \
                     --cache <dir>   downloads (default .bake-cache)\n  \
                     --no-regions    skip the high-resolution regions\n  \
                     --sky           bake the star background instead"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument `{other}` (try --help)");
                std::process::exit(2);
            }
        }
    }
    options
}

fn main() -> io::Result<()> {
    let options = parse_args();
    let start = Instant::now();
    let cache = &options.cache;
    if options.sky {
        println!("{}", sky::ATTRIBUTION);
        return sky::bake(cache, &workspace_root().join("assets/core/sky/starmap.png"));
    }

    println!("Fetching sources into {}", cache.display());
    let fetch_required = |source: Source| -> io::Result<PathBuf> {
        Ok(fetch(cache, &source, false)?.expect("required download"))
    };
    let etopo_path = fetch_required(etopo())?;
    let color_path = fetch_required(blue_marble())?;
    let cover_path = fetch_required(cci_land_cover())?;
    let lights_path = fetch_required(black_marble())?;

    println!("Loading elevation");
    let etopo = load_etopo(&etopo_path)?;
    println!("Loading land cover");
    let (class, water) = load_cci(&cover_path, &etopo)?;
    println!("Loading colour and night lights");
    let color = load_jpeg_rgb(&color_path)?;
    let lights = load_jpeg_gray(&lights_path)?;

    println!("Building mip levels");
    let global = Global {
        height: Pyramid::new(Geo::global(etopo.width, etopo.height), etopo, mean_f32),
        color: Pyramid::new(Geo::global(color.width, color.height), color, mean_rgb),
        class: Pyramid::new(Geo::global(class.width, class.height), class, mode_u8),
        water: Pyramid::new(Geo::global(water.width, water.height), water, mean_u8),
        lights: Pyramid::new(Geo::global(lights.width, lights.height), lights, mean_u8),
        level: GLOBAL_LEVEL,
    };

    let mut regions = Vec::new();
    if options.regions {
        for spec in REGIONS {
            println!("Loading region {}", spec.name);
            regions.push(load_region(cache, spec)?);
        }
    }
    let earth = Earth { global, regions };

    std::fs::create_dir_all(&options.out)?;
    println!("Baking tiles into {}", options.out.display());
    bake::write_layer(&earth, Layer::Height, &options.out, Earth::height_tile)?;
    bake::write_layer(&earth, Layer::Cover, &options.out, Earth::cover_tile)?;
    bake::write_layer(&earth, Layer::Color, &options.out, Earth::color_tile)?;
    bake::write_layer(&earth, Layer::Lights, &options.out, Earth::lights_tile)?;

    let meta = Meta {
        format: FORMAT_VERSION,
        description: format!(
            "Earth: global level {GLOBAL_LEVEL}{}",
            earth
                .regions
                .iter()
                .map(|r| format!("; {} to level {}/{}", r.name, r.height_level, r.cover_level))
                .collect::<String>()
        ),
        attribution: ATTRIBUTION.iter().map(|s| s.to_string()).collect(),
    };
    std::fs::write(
        options.out.join(META_FILE),
        serde_json::to_vec_pretty(&meta).map_err(io::Error::other)?,
    )?;
    println!("Done in {:.0} s", start.elapsed().as_secs_f64());
    Ok(())
}

fn load_region(cache: &Path, spec: &RegionSpec) -> io::Result<Region> {
    let lat_tiles = (spec.lat.0.floor() as i32, spec.lat.1.ceil() as i32);
    let lon_tiles = (spec.lon.0.floor() as i32, spec.lon.1.ceil() as i32);
    let (height_geo, height) = load_copernicus(cache, lat_tiles, lon_tiles)?;

    let corner = (
        (spec.lat.0 / 3.0).floor() as i32 * 3,
        (spec.lon.0 / 3.0).floor() as i32 * 3,
    );
    let (corner_hi_lat, corner_hi_lon) = (
        (spec.lat.1 / 3.0).floor() as i32 * 3,
        (spec.lon.1 / 3.0).floor() as i32 * 3,
    );
    if (corner_hi_lat, corner_hi_lon) != corner {
        return Err(io::Error::other(format!(
            "region {} spans several WorldCover tiles; split it",
            spec.name
        )));
    }
    let worldcover_path =
        fetch(cache, &worldcover_tile(corner.0, corner.1), false)?.expect("required download");
    let (cover_geo, cover) = load_worldcover(&worldcover_path, corner, spec.lat, spec.lon)?;
    // Water bodies (80), and the open sea: WorldCover maps all land and leaves the sea far
    // from it as no data (0).
    let water = Raster::new(
        cover.width,
        cover.height,
        cover
            .data
            .iter()
            .map(|&c| if c == 80 || c == 0 { 255 } else { 0 })
            .collect(),
    );
    Ok(Region {
        name: spec.name.to_owned(),
        lat: spec.lat,
        lon: spec.lon,
        feather: 0.04,
        height: Pyramid::new(height_geo, height, mean_f32),
        water: Pyramid::new(cover_geo, water, mean_u8),
        cover: Pyramid::new(cover_geo, cover, mode_u8),
        height_level: spec.height_level,
        cover_level: spec.cover_level,
    })
}
