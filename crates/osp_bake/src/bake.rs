//! Sampling the source rasters onto cube-sphere tiles and writing the packs.

use std::{io, path::Path};

use glam::DVec3;
use osp_terrain::{
    Cover, Layer, TILE_CELLS, TILE_SAMPLES, TileKey,
    cube::{lat_lon, sphere_to_cube},
    pack::{PackWriter, TileData},
};
use rayon::prelude::*;

use crate::raster::Pyramid;

/// Whole-planet sources.
pub struct Global {
    pub height: Pyramid<f32>,
    pub color: Pyramid<[u8; 3]>,
    pub class: Pyramid<u8>,
    pub water: Pyramid<u8>,
    pub lights: Pyramid<u8>,
    /// Deepest level baked everywhere.
    pub level: u8,
}

/// A high-resolution patch (elevation and land cover) blended over the global data.
pub struct Region {
    pub name: String,
    pub lat: (f64, f64),
    pub lon: (f64, f64),
    /// Width of the blend into the global data inside the region's edge, degrees.
    pub feather: f64,
    pub height: Pyramid<f32>,
    /// Raw ESA WorldCover codes.
    pub cover: Pyramid<u8>,
    /// WorldCover water fraction, 0..255.
    pub water: Pyramid<u8>,
    pub height_level: u8,
    pub cover_level: u8,
}

impl Region {
    /// 0 outside, rising to 1 over `feather` inside the edge.
    fn weight(&self, lat: f64, lon: f64) -> f64 {
        let inset = (lat - self.lat.0)
            .min(self.lat.1 - lat)
            .min(lon - self.lon.0)
            .min(self.lon.1 - lon);
        (inset / self.feather).clamp(0.0, 1.0)
    }

    fn contains(&self, lat: f64, lon: f64, margin: f64) -> bool {
        (self.lat.0 - margin..=self.lat.1 + margin).contains(&lat)
            && (self.lon.0 - margin..=self.lon.1 + margin).contains(&lon)
    }

    /// True if the tile overlaps the region.
    fn intersects(&self, key: TileKey) -> bool {
        let step = TILE_CELLS as f64 / 8.0;
        for j in 0..=8 {
            for i in 0..=8 {
                let (lat, lon) = degrees(key.sample_dir(i as f64 * step, j as f64 * step));
                if self.contains(lat, lon, 0.01) {
                    return true;
                }
            }
        }
        // A region smaller than the grid spacing can sit wholly inside the tile.
        let corners = [
            (self.lat.0, self.lon.0),
            (self.lat.0, self.lon.1),
            (self.lat.1, self.lon.0),
            (self.lat.1, self.lon.1),
        ];
        corners.iter().any(|&(lat, lon)| {
            let dir = osp_terrain::cube::from_lat_lon(lat.to_radians(), lon.to_radians());
            let (face, s, t) = sphere_to_cube(dir);
            TileKey::containing(face, s, t, key.level) == key
        })
    }
}

pub struct Earth {
    pub global: Global,
    pub regions: Vec<Region>,
}

fn degrees(dir: DVec3) -> (f64, f64) {
    let (lat, lon) = lat_lon(dir);
    (lat.to_degrees(), lon.to_degrees())
}

/// Angular size of a texel at `level`, degrees (a face spans about 90°).
fn texel_degrees(level: u8) -> f64 {
    90.0 / (1u64 << level) as f64 / TILE_CELLS as f64
}

/// Deterministic noise in 0..1 used to dither class boundaries where data sets meet.
fn hash01(lat: f64, lon: f64) -> f64 {
    let x = (lat * 3000.0).floor() as i64;
    let y = (lon * 3000.0).floor() as i64;
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h >> 11) as f64 / (1u64 << 53) as f64
}

const SUPERSAMPLE: [(f64, f64); 4] = [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)];

impl Earth {
    fn region_at(&self, lat: f64, lon: f64) -> Option<(&Region, f64)> {
        self.regions
            .iter()
            .map(|r| (r, r.weight(lat, lon)))
            .find(|(_, w)| *w > 0.0)
    }

    fn height(&self, lat: f64, lon: f64, texel: f64) -> f32 {
        let g = &self.global.height;
        let global = g.bilinear(g.level_for(texel), lat, lon, |v| [v])[0];
        match self.region_at(lat, lon) {
            Some((region, w)) => {
                let r = &region.height;
                let mut local = r.bilinear(r.level_for(texel), lat, lon, |v| [v])[0];
                // The regional data is a surface model, which over the sea is the sea
                // surface: keep the global data's depth there.
                if local <= 0.0 {
                    local = local.min(global);
                }
                global + (local - global) * w as f32
            }
            None => global,
        }
    }

    /// Cover class and water fraction (0..1).
    fn cover(&self, lat: f64, lon: f64, texel: f64) -> (u8, f32) {
        let g = &self.global;
        let level = g.class.level_for(texel);
        let global_class = g.class.nearest(level, lat, lon);
        let global_water = g.water.bilinear(level, lat, lon, |v| [v as f32])[0] / 255.0;
        let Some((region, w)) = self.region_at(lat, lon) else {
            return (global_class, global_water);
        };
        let level = region.cover.level_for(texel);
        let code = region.cover.nearest(level, lat, lon);
        let water = region.water.bilinear(level, lat, lon, |v| [v as f32])[0] / 255.0;
        let height = region.height.bilinear(0, lat, lon, |v| [v])[0] as f64;
        // Any open water within ~150 m (WorldCover mip level 4 has 160 m pixels).
        let near_water = region.water.bilinear(4, lat, lon, |v| [v as f32])[0] > 2.0;
        let class =
            Cover::from_worldcover(code, Cover::from_u8(global_class), height, near_water, lat)
                .map_or(global_class, |c| c as u8);
        let class = if w > hash01(lat, lon) {
            class
        } else {
            global_class
        };
        (class, global_water + (water - global_water) * w as f32)
    }

    pub fn height_tile(&self, key: TileKey) -> TileData {
        let texel = texel_degrees(key.level);
        TileData::Height(grid(key, |dir| {
            let mut sum = 0.0;
            for (di, dj) in SUPERSAMPLE {
                let (lat, lon) = degrees(dir(di, dj));
                sum += self.height(lat, lon, texel);
            }
            sum / 4.0
        }))
    }

    pub fn color_tile(&self, key: TileKey) -> TileData {
        let texel = texel_degrees(key.level);
        let color = &self.global.color;
        let level = color.level_for(texel);
        TileData::Rgb(grid(key, |dir| {
            let mut sum = [0f32; 3];
            for (di, dj) in SUPERSAMPLE {
                let (lat, lon) = degrees(dir(di, dj));
                let c = color.bilinear(level, lat, lon, |v| v.map(|x| x as f32));
                for k in 0..3 {
                    sum[k] += c[k];
                }
            }
            sum.map(|v| (v / 4.0).round().clamp(0.0, 255.0) as u8)
        }))
    }

    pub fn lights_tile(&self, key: TileKey) -> TileData {
        let texel = texel_degrees(key.level);
        let lights = &self.global.lights;
        let level = lights.level_for(texel);
        TileData::Bytes {
            channels: 1,
            data: grid(key, |dir| {
                let mut sum = 0.0;
                for (di, dj) in SUPERSAMPLE {
                    let (lat, lon) = degrees(dir(di, dj));
                    sum += lights.bilinear(level, lat, lon, |v| [v as f32])[0];
                }
                (sum / 4.0).round().clamp(0.0, 255.0) as u8
            }),
        }
    }

    pub fn cover_tile(&self, key: TileKey) -> TileData {
        let texel = texel_degrees(key.level);
        let samples = grid(key, |dir| {
            let mut counts = [0u8; Cover::COUNT];
            let mut water = 0.0;
            for (di, dj) in SUPERSAMPLE {
                let (lat, lon) = degrees(dir(di, dj));
                let (class, wet) = self.cover(lat, lon, texel);
                counts[class as usize] += 1;
                water += wet;
            }
            // Most common class; the centre-most sample breaks ties.
            let class = (0..Cover::COUNT).max_by_key(|&c| counts[c]).unwrap() as u8;
            (class, (water / 4.0 * 255.0).round() as u8)
        });
        let mut data = Vec::with_capacity(samples.len() * 2);
        data.extend(samples.iter().map(|s| s.0));
        data.extend(samples.iter().map(|s| s.1));
        TileData::Bytes { channels: 2, data }
    }

    /// Keys to bake for `layer`: every tile down to the global level, plus the tiles over
    /// each region down to that region's level for the layer.
    pub fn keys(&self, layer: Layer) -> Vec<TileKey> {
        let mut keys = Vec::new();
        let mut stack: Vec<TileKey> = (0..6).map(TileKey::root).collect();
        while let Some(key) = stack.pop() {
            keys.push(key);
            let deeper = key.level < self.global.level
                || self.regions.iter().any(|r| {
                    let max = match layer {
                        Layer::Height => r.height_level,
                        Layer::Cover => r.cover_level,
                        Layer::Color | Layer::Lights => 0,
                    };
                    key.level < max && r.intersects(key)
                });
            if deeper {
                stack.extend(key.children());
            }
        }
        keys.sort();
        keys
    }
}

/// Evaluate `sample` at every grid sample of `key`. The closure gets a function from a
/// sub-sample offset (in samples) to a direction.
fn grid<T: Send>(key: TileKey, sample: impl Fn(&dyn Fn(f64, f64) -> DVec3) -> T + Sync) -> Vec<T> {
    (0..TILE_SAMPLES * TILE_SAMPLES)
        .into_par_iter()
        .map(|k| {
            let (i, j) = ((k % TILE_SAMPLES) as f64, (k / TILE_SAMPLES) as f64);
            sample(&|di, dj| key.sample_dir(i + di, j + dj))
        })
        .collect()
}

/// Build and write one layer's pack.
pub fn write_layer(
    earth: &Earth,
    layer: Layer,
    out_dir: &Path,
    build: impl Fn(&Earth, TileKey) -> TileData + Sync,
) -> io::Result<()> {
    let keys = earth.keys(layer);
    let path = out_dir.join(layer.file_name());
    let mut writer = PackWriter::create(&path)?;
    let total = keys.len();
    let mut done = 0;
    for batch in keys.chunks(256) {
        let encoded: io::Result<Vec<(TileKey, osp_terrain::pack::Encoding, Vec<u8>)>> = batch
            .par_iter()
            .map(|&key| {
                let tile = build(earth, key);
                let blob = match &tile {
                    // Global data has metre-level detail at best; regional data is finer.
                    TileData::Height(h) => osp_terrain::pack::encode_height(
                        h,
                        if key.level <= earth.global.level {
                            0.5
                        } else {
                            0.05
                        },
                    )?,
                    other => other.encode()?,
                };
                let encoding = match &tile {
                    TileData::Height(_) => osp_terrain::pack::Encoding::Height,
                    TileData::Rgb(_) => osp_terrain::pack::Encoding::Rgb,
                    TileData::Bytes { channels, .. } => osp_terrain::pack::Encoding::Bytes {
                        channels: *channels,
                    },
                };
                Ok((key, encoding, blob))
            })
            .collect();
        for (key, encoding, blob) in encoded? {
            writer.write_encoded(key, encoding, &blob)?;
        }
        done += batch.len();
        print!("\r  {:<8} {done}/{total} tiles", format!("{layer:?}"));
        use std::io::Write;
        io::stdout().flush().ok();
    }
    writer.finish()?;
    let size = std::fs::metadata(&path)?.len();
    println!("  ({:.1} MB)", size as f64 / 1e6);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texel_size_matches_face_span() {
        assert!((texel_degrees(0) * TILE_CELLS as f64 - 90.0).abs() < 1e-12);
        assert!((texel_degrees(13) * 111_000.0 - 9.5).abs() < 0.5);
    }

    #[test]
    fn hash_is_uniformish() {
        let mean: f64 = (0..10_000)
            .map(|k| hash01(k as f64 * 0.001, k as f64 * 0.0007))
            .sum::<f64>()
            / 10_000.0;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
    }
}
