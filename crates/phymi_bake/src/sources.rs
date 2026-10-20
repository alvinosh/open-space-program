//! The public datasets the Earth terrain is baked from: where to download them and how to
//! decode them into [`Raster`]s.

use std::{
    fs::File,
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Command,
};

use phymi_terrain::Cover;
use rayon::prelude::*;
use tiff::{
    decoder::{Decoder, DecodingResult, Limits},
    tags::Tag,
};

use crate::raster::{Geo, Raster};

pub struct Source {
    pub file: String,
    pub url: String,
}

impl Source {
    fn new(file: &str, url: &str) -> Self {
        Self {
            file: file.to_owned(),
            url: url.to_owned(),
        }
    }
}

/// NASA Blue Marble Next Generation, July 2004, no relief shading.
pub fn blue_marble() -> Source {
    Source::new(
        "world.200407.3x21600x10800.jpg",
        "https://eoimages.gsfc.nasa.gov/images/imagerecords/74000/74092/world.200407.3x21600x10800.jpg",
    )
}
/// NOAA ETOPO 2022, 60 arc-second surface elevation (bathymetry and land).
pub fn etopo() -> Source {
    Source::new(
        "ETOPO_2022_v1_60s_N90W180_surface.tif",
        "https://www.ngdc.noaa.gov/mgg/global/relief/ETOPO2022/data/60s/60s_surface_elev_gtif/ETOPO_2022_v1_60s_N90W180_surface.tif",
    )
}
/// ESA CCI Land Cover 2015, 300 m.
pub fn cci_land_cover() -> Source {
    Source::new(
        "ESACCI-LC-L4-LCCS-Map-300m-P1Y-2015-v2.0.7.tif",
        "https://dap.ceda.ac.uk/neodc/esacci/land_cover/data/land_cover_maps/v2.0.7/ESACCI-LC-L4-LCCS-Map-300m-P1Y-2015-v2.0.7.tif",
    )
}
/// NASA Black Marble 2016 night lights.
pub fn black_marble() -> Source {
    Source::new(
        "BlackMarble_2016_3km_gray.jpg",
        "https://eoimages.gsfc.nasa.gov/images/imagerecords/144000/144897/BlackMarble_2016_3km_gray.jpg",
    )
}

pub const ATTRIBUTION: &[&str] = &[
    "Blue Marble: NASA Earth Observatory / Reto Stöckli, NASA GSFC (Blue Marble Next Generation)",
    "Black Marble: NASA Earth Observatory images by Joshua Stevens, using Suomi NPP VIIRS data from Miguel Román, NASA GSFC",
    "ETOPO 2022: NOAA National Centers for Environmental Information (2022), doi:10.25921/fd45-gt74",
    "Land cover: ESA Climate Change Initiative and the Land Cover CCI project (© ESA 2017 - UCLouvain)",
    "© ESA WorldCover project 2021 / Contains modified Copernicus Sentinel data (2021) processed by ESA WorldCover consortium",
    "Copernicus DEM GLO-30: produced using Copernicus WorldDEM-30 © DLR e.V. 2010-2014 and © Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights reserved",
];

pub fn copernicus_tile(lat: i32, lon: i32) -> Source {
    let name = format!(
        "Copernicus_DSM_COG_10_{}{:02}_00_{}{:03}_00_DEM",
        if lat >= 0 { 'N' } else { 'S' },
        lat.abs(),
        if lon >= 0 { 'E' } else { 'W' },
        lon.abs()
    );
    Source {
        url: format!("https://copernicus-dem-30m.s3.amazonaws.com/{name}/{name}.tif"),
        file: format!("{name}.tif"),
    }
}

/// WorldCover tiles are 3°×3°, named by their south-west corner.
pub fn worldcover_tile(lat: i32, lon: i32) -> Source {
    let name = format!(
        "ESA_WorldCover_10m_2021_v200_{}{:02}{}{:03}_Map.tif",
        if lat >= 0 { 'N' } else { 'S' },
        lat.abs(),
        if lon >= 0 { 'E' } else { 'W' },
        lon.abs()
    );
    Source {
        url: format!("https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map/{name}"),
        file: name,
    }
}

/// Download `source` into `cache` unless it is already there. Returns `None` when the
/// server has no such file (e.g. Copernicus tiles over open ocean).
pub fn fetch(cache: &Path, source: &Source, optional: bool) -> io::Result<Option<PathBuf>> {
    let path = cache.join(&source.file);
    if path.is_file() {
        return Ok(Some(path));
    }
    std::fs::create_dir_all(cache)?;
    let partial = cache.join(format!("{}.part", source.file));
    println!("  downloading {}", source.url);
    let status = Command::new("curl")
        .args(["-L", "--fail", "--retry", "3", "-C", "-", "-#", "-o"])
        .arg(&partial)
        .arg(&source.url)
        .status()
        .map_err(|e| io::Error::other(format!("cannot run curl: {e}")))?;
    if !status.success() {
        std::fs::remove_file(&partial).ok();
        if optional {
            println!("  (not available, skipped)");
            return Ok(None);
        }
        return Err(io::Error::other(format!("download failed: {}", source.url)));
    }
    std::fs::rename(&partial, &path)?;
    Ok(Some(path))
}

fn tiff_decoder(path: &Path) -> io::Result<Decoder<BufReader<File>>> {
    Ok(Decoder::new(BufReader::new(File::open(path)?))
        .map_err(io::Error::other)?
        .with_limits(Limits::unlimited()))
}

/// Decode tiles of a tiled GeoTIFF in parallel. `wanted` gets `(tile_x, tile_y)`; results
/// are `(tile_x, tile_y, width, height, samples)` with `width × height` samples per tile.
fn read_tiles<T: Send>(
    path: &Path,
    wanted: impl Fn(u32, u32) -> bool + Sync,
    convert: impl Fn(DecodingResult) -> Vec<T> + Sync,
) -> io::Result<Vec<(u32, u32, usize, usize, Vec<T>)>> {
    let mut decoder = tiff_decoder(path)?;
    let (width, _height) = decoder.dimensions().map_err(io::Error::other)?;
    let (tile_w, _tile_h) = decoder.chunk_dimensions();
    let across = width.div_ceil(tile_w);
    let count = decoder.tile_count().map_err(io::Error::other)?;
    let indices: Vec<u32> = (0..count)
        .filter(|&i| wanted(i % across, i / across))
        .collect();
    indices
        .par_iter()
        .map_init(
            || tiff_decoder(path),
            |decoder, &index| {
                let decoder = decoder
                    .as_mut()
                    .map_err(|e| io::Error::other(e.to_string()))?;
                let (w, h) = decoder.chunk_data_dimensions(index);
                let samples = convert(decoder.read_chunk(index).map_err(io::Error::other)?);
                // Some encoders return padded tiles; keep the data part only.
                let samples = if samples.len() == (w * h) as usize {
                    samples
                } else {
                    let padded_w = samples.len() / decoder.chunk_dimensions().1 as usize;
                    samples
                        .into_iter()
                        .enumerate()
                        .filter(|(k, _)| k % padded_w < w as usize && k / padded_w < h as usize)
                        .map(|(_, v)| v)
                        .collect()
                };
                Ok((
                    index % across,
                    index / across,
                    w as usize,
                    h as usize,
                    samples,
                ))
            },
        )
        .collect()
}

fn as_f32(result: DecodingResult) -> Vec<f32> {
    match result {
        DecodingResult::F32(v) => v,
        DecodingResult::I16(v) => v.into_iter().map(|x| x as f32).collect(),
        DecodingResult::F64(v) => v.into_iter().map(|x| x as f32).collect(),
        other => panic!(
            "unexpected sample type {:?}",
            std::mem::discriminant(&other)
        ),
    }
}

/// Copy decoded tiles into one raster covering the window starting at tile pixel
/// `(x0, y0)` with the given size.
fn assemble<T: Copy + Default>(
    tiles: Vec<(u32, u32, usize, usize, Vec<T>)>,
    tile_size: (usize, usize),
    window: (usize, usize, usize, usize),
) -> Vec<T> {
    let (x0, y0, width, height) = window;
    let mut out = vec![T::default(); width * height];
    for (tx, ty, w, h, samples) in tiles {
        for row in 0..h {
            let y = ty as usize * tile_size.1 + row;
            if y < y0 || y >= y0 + height {
                continue;
            }
            for col in 0..w {
                let x = tx as usize * tile_size.0 + col;
                if x < x0 || x >= x0 + width {
                    continue;
                }
                out[(y - y0) * width + (x - x0)] = samples[row * w + col];
            }
        }
    }
    out
}

/// ETOPO 2022 60″: 21600×10800 float metres, pixel-is-area, edges at ±180°/±90°.
pub fn load_etopo(path: &Path) -> io::Result<Raster<f32>> {
    let mut decoder = tiff_decoder(path)?;
    let (width, height) = decoder.dimensions().map_err(io::Error::other)?;
    let tile = decoder.chunk_dimensions();
    let tiles = read_tiles(path, |_, _| true, as_f32)?;
    let (w, h) = (width as usize, height as usize);
    let mut data = assemble(tiles, (tile.0 as usize, tile.1 as usize), (0, 0, w, h));
    for v in &mut data {
        if !v.is_finite() || *v < -12_000.0 {
            *v = 0.0;
        }
    }
    Ok(Raster::new(w, h, data))
}

pub fn load_jpeg_rgb(path: &Path) -> io::Result<Raster<[u8; 3]>> {
    let mut reader = image::ImageReader::open(path)?;
    reader.no_limits();
    let image = reader.decode().map_err(io::Error::other)?.into_rgb8();
    let (w, h) = (image.width() as usize, image.height() as usize);
    let data: Vec<[u8; 3]> = image.pixels().map(|p| p.0).collect();
    Ok(Raster::new(w, h, data))
}

pub fn load_jpeg_gray(path: &Path) -> io::Result<Raster<u8>> {
    let mut reader = image::ImageReader::open(path)?;
    reader.no_limits();
    let image = reader.decode().map_err(io::Error::other)?.into_luma8();
    let (w, h) = (image.width() as usize, image.height() as usize);
    Ok(Raster::new(w, h, image.into_raw()))
}

/// ESA CCI land cover (129600×64800, 300 m) reduced to the ETOPO grid (21600×10800, 6×6
/// source pixels per output pixel): the most common [`Cover`] class and the water fraction.
/// Water whose ETOPO surface is at or below sea level becomes ocean.
pub fn load_cci(path: &Path, etopo: &Raster<f32>) -> io::Result<(Raster<u8>, Raster<u8>)> {
    const FACTOR: usize = 6;
    let tiles = RawTiles::open(path)?;
    let (width, height, tile_h) = (tiles.width, tiles.height, tiles.tile_h);
    assert_eq!(
        (width / FACTOR, height / FACTOR),
        (etopo.width, etopo.height),
        "land cover grid does not line up with ETOPO"
    );
    let (out_w, out_h) = (width / FACTOR, height / FACTOR);
    let mut classes = vec![0u8; out_w * out_h];
    let mut water = vec![0u8; out_w * out_h];
    let lut: Vec<u8> = (0..=255u8).map(|c| Cover::from_cci(c) as u8).collect();

    // Strips of FACTOR tile rows map onto whole output rows.
    let strip_rows = tile_h * FACTOR;
    for strip in 0..height.div_ceil(strip_rows) {
        let y0 = strip * strip_rows;
        let rows = strip_rows.min(height - y0);
        let buffer = tiles.read_window((0, y0, width, rows))?;
        let out_y0 = y0 / FACTOR;
        let out_rows = rows / FACTOR;
        let results: Vec<(u8, u8)> = (0..out_rows * out_w)
            .into_par_iter()
            .map(|k| {
                let (ox, oy) = (k % out_w, k / out_w);
                let mut counts = [0u16; Cover::COUNT];
                let mut wet = 0u32;
                for dy in 0..FACTOR {
                    let row = &buffer[(oy * FACTOR + dy) * width..];
                    for dx in 0..FACTOR {
                        let code = row[ox * FACTOR + dx];
                        counts[lut[code as usize] as usize] += 1;
                        wet += matches!(code, 0 | 210) as u32;
                    }
                }
                let class = (0..Cover::COUNT).max_by_key(|&c| counts[c]).unwrap() as u8;
                let fraction = (wet * 255 / (FACTOR * FACTOR) as u32) as u8;
                (class, fraction)
            })
            .collect();
        for (k, (class, fraction)) in results.into_iter().enumerate() {
            let index = out_y0 * out_w + k;
            let mut class = class;
            if class == Cover::Lake as u8 && etopo.data[index] <= 0.0 {
                class = Cover::Ocean as u8;
            }
            classes[index] = class;
            water[index] = fraction;
        }
        print!(
            "\r  land cover {}%",
            (strip + 1) * 100 / height.div_ceil(strip_rows)
        );
        use std::io::Write;
        io::stdout().flush().ok();
    }
    println!();
    Ok((
        Raster::new(out_w, out_h, classes),
        Raster::new(out_w, out_h, water),
    ))
}

/// Copernicus GLO-30 tiles (1″, pixel-is-point) merged over whole degrees
/// `lat_range` × `lon_range`; missing tiles (open ocean) are sea level. Buildings and tree
/// canopy (this is a surface model) are removed with a morphological opening.
pub fn load_copernicus(
    cache: &Path,
    lat_range: (i32, i32),
    lon_range: (i32, i32),
) -> io::Result<(Geo, Raster<f32>)> {
    const N: usize = 3600;
    let (lat_tiles, lon_tiles) = (
        (lat_range.1 - lat_range.0) as usize,
        (lon_range.1 - lon_range.0) as usize,
    );
    let (width, height) = (lon_tiles * N, lat_tiles * N);
    let mut data = vec![0f32; width * height];
    for lat in lat_range.0..lat_range.1 {
        for lon in lon_range.0..lon_range.1 {
            let Some(path) = fetch(cache, &copernicus_tile(lat, lon), true)? else {
                continue;
            };
            let mut decoder = tiff_decoder(&path)?;
            let (w, h) = decoder.dimensions().map_err(io::Error::other)?;
            if (w as usize, h as usize) != (N, N) {
                // Tiles above 50° are narrower; not needed for the regions baked here.
                return Err(io::Error::other(format!(
                    "{}: unexpected size {w}×{h}",
                    path.display()
                )));
            }
            let tile = decoder.chunk_dimensions();
            let tiles = read_tiles(&path, |_, _| true, as_f32)?;
            let samples = assemble(tiles, (tile.0 as usize, tile.1 as usize), (0, 0, N, N));
            // Row 0 of the tile is its northern edge (lat + 1).
            let row0 = (lat_range.1 - 1 - lat) as usize * N;
            let col0 = (lon - lon_range.0) as usize * N;
            for (row, line) in samples.chunks_exact(N).enumerate() {
                let start = (row0 + row) * width + col0;
                data[start..start + N].copy_from_slice(line);
            }
        }
    }
    let mut raster = Raster::new(width, height, data);
    for v in &mut raster.data {
        if !v.is_finite() || *v < -500.0 {
            *v = 0.0;
        }
    }
    // Opening (erosion then dilation) over ~270 m removes buildings and tree canopy while
    // keeping landforms wider than that; a light blur then softens the result.
    raster = filter(&raster, 4, |a, b| a.min(b));
    raster = filter(&raster, 4, |a, b| a.max(b));
    raster = box_blur(&raster, 1);
    let geo = Geo {
        lon0: lon_range.0 as f64,
        lat0: lat_range.1 as f64,
        dlon: 1.0 / N as f64,
        dlat: 1.0 / N as f64,
        center: 0.0,
        wrap: false,
    };
    Ok((geo, raster))
}

/// Separable square-window filter with an associative reducer (min or max).
fn filter(
    raster: &Raster<f32>,
    radius: usize,
    reduce: impl Fn(f32, f32) -> f32 + Sync,
) -> Raster<f32> {
    let (w, h) = (raster.width, raster.height);
    let pass = |src: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![0f32; w * h];
        out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                let mut acc = src[y * w + x];
                for d in 1..=radius {
                    let (a, b) = if horizontal {
                        (y * w + x.saturating_sub(d), y * w + (x + d).min(w - 1))
                    } else {
                        (y.saturating_sub(d) * w + x, (y + d).min(h - 1) * w + x)
                    };
                    acc = reduce(acc, reduce(src[a], src[b]));
                }
                *value = acc;
            }
        });
        out
    };
    let horizontal = pass(&raster.data, true);
    Raster::new(w, h, pass(&horizontal, false))
}

fn box_blur(raster: &Raster<f32>, radius: usize) -> Raster<f32> {
    let (w, h) = (raster.width, raster.height);
    let n = (2 * radius + 1) as f32;
    let pass = |src: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![0f32; w * h];
        out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                let mut sum = 0.0;
                for d in -(radius as isize)..=radius as isize {
                    let k = if horizontal {
                        y * w + (x as isize + d).clamp(0, w as isize - 1) as usize
                    } else {
                        (y as isize + d).clamp(0, h as isize - 1) as usize * w + x
                    };
                    sum += src[k];
                }
                *value = sum / n;
            }
        });
        out
    };
    let horizontal = pass(&raster.data, true);
    Raster::new(w, h, pass(&horizontal, false))
}

/// Tiles of an 8-bit, single-band tiled TIFF read directly: deflate or LZW, no predictor.
/// The TIFF decoder rejects palette images (WorldCover) and LZW tiles without an end code
/// (CCI land cover), both of which are fine to decode by hand.
struct RawTiles {
    path: PathBuf,
    compression: u32,
    offsets: Vec<u64>,
    counts: Vec<u64>,
    width: usize,
    height: usize,
    tile_w: usize,
    tile_h: usize,
}

impl RawTiles {
    fn open(path: &Path) -> io::Result<Self> {
        let mut decoder = tiff_decoder(path)?;
        let (width, height) = decoder.dimensions().map_err(io::Error::other)?;
        let (tile_w, tile_h) = decoder.chunk_dimensions();
        let tag = |decoder: &mut Decoder<BufReader<File>>, tag| {
            decoder.get_tag_u64_vec(tag).map_err(io::Error::other)
        };
        let compression = decoder
            .get_tag_u32(Tag::Compression)
            .map_err(io::Error::other)?;
        let predictor = decoder
            .find_tag_unsigned::<u32>(Tag::Predictor)
            .map_err(io::Error::other)?
            .unwrap_or(1);
        let bits = decoder
            .get_tag_u32_vec(Tag::BitsPerSample)
            .map_err(io::Error::other)?;
        if !matches!(compression, 5 | 8) || predictor != 1 || bits != [8] {
            return Err(io::Error::other(format!(
                "{}: unsupported layout (compression {compression}, predictor {predictor}, bits {bits:?})",
                path.display()
            )));
        }
        Ok(Self {
            path: path.to_owned(),
            compression,
            offsets: tag(&mut decoder, Tag::TileOffsets)?,
            counts: tag(&mut decoder, Tag::TileByteCounts)?,
            width: width as usize,
            height: height as usize,
            tile_w: tile_w as usize,
            tile_h: tile_h as usize,
        })
    }

    fn read_tile(&self, file: &mut File, index: usize) -> io::Result<Vec<u8>> {
        let size = self.tile_w * self.tile_h;
        if self.counts[index] == 0 {
            return Ok(vec![0; size]);
        }
        file.seek(SeekFrom::Start(self.offsets[index]))?;
        let mut blob = vec![0u8; self.counts[index] as usize];
        file.read_exact(&mut blob)?;
        let mut samples = Vec::with_capacity(size);
        if self.compression == 8 {
            flate2::read::ZlibDecoder::new(&blob[..]).read_to_end(&mut samples)?;
        } else {
            let mut decoder =
                weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
            // Some writers omit the end code and leave junk after the tile's data; anything
            // after a complete tile is ignored.
            let result = decoder.into_vec(&mut samples).decode(&blob);
            if let (Err(e), true) = (result.status, samples.len() < size) {
                return Err(io::Error::other(format!(
                    "{}: tile {index} (offset {}, {} bytes, starts {:02x?}): LZW: {e}",
                    self.path.display(),
                    self.offsets[index],
                    blob.len(),
                    &blob[..blob.len().min(12)]
                )));
            }
        }
        samples.resize(size, 0);
        Ok(samples)
    }

    /// Decode the window `(x0, y0, width, height)` in parallel.
    fn read_window(&self, window: (usize, usize, usize, usize)) -> io::Result<Vec<u8>> {
        let (x0, y0, w, h) = window;
        let across = self.width.div_ceil(self.tile_w);
        let mut wanted = Vec::new();
        for ty in y0 / self.tile_h..=(y0 + h - 1) / self.tile_h {
            for tx in x0 / self.tile_w..=(x0 + w - 1) / self.tile_w {
                wanted.push((tx, ty));
            }
        }
        let tiles: io::Result<Vec<_>> = wanted
            .par_iter()
            .map_init(
                || File::open(&self.path),
                |file, &(tx, ty)| {
                    let file = file
                        .as_mut()
                        .map_err(|e| io::Error::new(e.kind(), e.to_string()))?;
                    let samples = self.read_tile(file, ty * across + tx)?;
                    Ok((tx as u32, ty as u32, self.tile_w, self.tile_h, samples))
                },
            )
            .collect();
        Ok(assemble(tiles?, (self.tile_w, self.tile_h), window))
    }
}

/// A window of an ESA WorldCover tile (3°×3°, 1/12000°, pixel-is-area): raw class codes.
pub fn load_worldcover(
    path: &Path,
    tile_corner: (i32, i32),
    lat_range: (f64, f64),
    lon_range: (f64, f64),
) -> io::Result<(Geo, Raster<u8>)> {
    const PER_DEGREE: f64 = 12_000.0;
    let tiles = RawTiles::open(path)?;
    let north = (tile_corner.0 + 3) as f64;
    let west = tile_corner.1 as f64;
    let x0 = ((lon_range.0 - west) * PER_DEGREE).floor().max(0.0) as usize;
    let x1 = (((lon_range.1 - west) * PER_DEGREE).ceil() as usize).min(tiles.width);
    let y0 = ((north - lat_range.1) * PER_DEGREE).floor().max(0.0) as usize;
    let y1 = (((north - lat_range.0) * PER_DEGREE).ceil() as usize).min(tiles.height);
    let data = tiles.read_window((x0, y0, x1 - x0, y1 - y0))?;
    let geo = Geo {
        lon0: west + x0 as f64 / PER_DEGREE,
        lat0: north - y0 as f64 / PER_DEGREE,
        dlon: 1.0 / PER_DEGREE,
        dlat: 1.0 / PER_DEGREE,
        center: 0.5,
        wrap: false,
    };
    Ok((geo, Raster::new(x1 - x0, y1 - y0, data)))
}
