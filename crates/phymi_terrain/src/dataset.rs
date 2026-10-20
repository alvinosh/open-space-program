//! A baked terrain dataset: a directory with [`META_FILE`] and one tile pack per layer.
//!
//! Layers are sparse quadtree pyramids: each may go deeper in some regions (e.g. around a
//! launch site) than elsewhere. Lookups always fall back to the deepest ancestor that
//! exists, so callers can ask for any key.

use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::{
    cube::{TILE_CELLS, TILE_SAMPLES, TileKey, sphere_to_cube},
    pack::{PackReader, TileData},
};

/// Version of the on-disk format; bumped on incompatible changes.
pub const FORMAT_VERSION: u32 = 1;
/// JSON metadata ([`Meta`]) next to the packs. Not `.json`, which mods read as records.
pub const META_FILE: &str = "terrain.meta";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    /// Metres above the mean radius (negative below sea level), `f32`.
    Height,
    /// Surface colour, sRGB.
    Color,
    /// Two planes: [`crate::Cover`] class (nearest) and water fraction (0..255, linear).
    Cover,
    /// Night lights, 0..255.
    Lights,
}

impl Layer {
    pub const ALL: [Layer; 4] = [Layer::Height, Layer::Color, Layer::Cover, Layer::Lights];

    pub fn file_name(self) -> &'static str {
        match self {
            Layer::Height => "height.pack",
            Layer::Color => "color.pack",
            Layer::Cover => "cover.pack",
            Layer::Lights => "lights.pack",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Contents of [`META_FILE`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Meta {
    pub format: u32,
    #[serde(default)]
    pub description: String,
    /// Credits required by the source datasets' licences.
    #[serde(default)]
    pub attribution: Vec<String>,
}

pub struct Dataset {
    pub meta: Meta,
    layers: [Option<LayerData>; 4],
}

struct LayerData {
    pack: PackReader,
    cache: TileCache,
}

/// Least-recently-used cache of decoded tiles. Lookups take a shared lock (many worker
/// threads sample at once); recency is tracked with atomics.
struct TileCache {
    tiles: RwLock<HashMap<TileKey, (Arc<TileData>, AtomicU64)>>,
    clock: AtomicU64,
    capacity: usize,
}

impl TileCache {
    fn new(capacity: usize) -> Self {
        Self {
            tiles: RwLock::new(HashMap::new()),
            clock: AtomicU64::new(0),
            capacity,
        }
    }

    fn get(&self, key: TileKey) -> Option<Arc<TileData>> {
        let tiles = self.tiles.read().unwrap_or_else(|e| e.into_inner());
        tiles.get(&key).map(|(tile, used)| {
            used.store(
                self.clock.fetch_add(1, Ordering::Relaxed),
                Ordering::Relaxed,
            );
            tile.clone()
        })
    }

    fn insert(&self, key: TileKey, tile: Arc<TileData>) {
        let mut tiles = self.tiles.write().unwrap_or_else(|e| e.into_inner());
        if tiles.len() >= self.capacity {
            // Evict the oldest quarter at once so eviction cost stays amortised.
            let mut ages: Vec<u64> = tiles
                .values()
                .map(|(_, used)| used.load(Ordering::Relaxed))
                .collect();
            let cut = ages.len() / 4;
            let (_, threshold, _) = ages.select_nth_unstable(cut);
            let threshold = *threshold;
            tiles.retain(|_, (_, used)| used.load(Ordering::Relaxed) > threshold);
        }
        let stamp = self.clock.fetch_add(1, Ordering::Relaxed);
        tiles.insert(key, (tile, AtomicU64::new(stamp)));
    }
}

impl Dataset {
    pub fn open(dir: &Path) -> io::Result<Self> {
        let meta: Meta = serde_json::from_slice(&std::fs::read(dir.join(META_FILE))?)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if meta.format != FORMAT_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "terrain format {} (expected {FORMAT_VERSION}); re-run the bake",
                    meta.format
                ),
            ));
        }
        let mut layers: [Option<LayerData>; 4] = Default::default();
        for layer in Layer::ALL {
            let path = dir.join(layer.file_name());
            if !path.is_file() {
                continue;
            }
            layers[layer.index()] = Some(LayerData {
                pack: PackReader::open(&path)?,
                cache: TileCache::new(1024),
            });
        }
        Ok(Self { meta, layers })
    }

    pub fn has(&self, layer: Layer) -> bool {
        self.layers[layer.index()].is_some()
    }

    /// Deepest level present anywhere in the layer.
    pub fn max_level(&self, layer: Layer) -> Option<u8> {
        self.layers[layer.index()]
            .as_ref()
            .map(|l| l.pack.max_level())
    }

    /// The deepest stored tile that covers `key` (the key itself or an ancestor).
    pub fn best_tile(&self, layer: Layer, key: TileKey) -> Option<(TileKey, Arc<TileData>)> {
        let data = self.layers[layer.index()].as_ref()?;
        let mut source = key.ancestor(key.level.min(data.pack.max_level()));
        while !data.pack.contains(source) {
            source = source.parent()?;
        }
        if let Some(tile) = data.cache.get(source) {
            return Some((source, tile));
        }
        // Decode outside the lock; a rare duplicate decode is cheaper than contention.
        let tile = Arc::new(data.pack.read(source).ok()??);
        data.cache.insert(source, tile.clone());
        Some((source, tile))
    }

    /// The samples of `layer` that cover `key`'s sample grid, at the best available
    /// resolution (upsampled from an ancestor tile when the layer is not that deep here).
    pub fn view(&self, layer: Layer, key: TileKey) -> Option<TileView> {
        let (source, tile) = self.best_tile(layer, key)?;
        let factor = (1u64 << (key.level - source.level)) as f64;
        Some(TileView {
            tile,
            source,
            scale: 1.0 / factor,
            offset: (
                (key.x as f64 - source.x as f64 * factor) * TILE_CELLS as f64,
                (key.y as f64 - source.y as f64 * factor) * TILE_CELLS as f64,
            ),
        })
    }

    /// View of the tile covering a direction, no deeper than `max_level`, with the sample
    /// coordinates of that direction inside it.
    pub fn locate(&self, layer: Layer, dir: DVec3, max_level: u8) -> Option<(TileView, f64, f64)> {
        let (face, s, t) = sphere_to_cube(dir);
        let key = TileKey::containing(face, s, t, max_level.min(crate::cube::MAX_LEVEL));
        let view = self.view(layer, key)?;
        let (u, v) = view.source.local_sample(s, t);
        Some((
            TileView {
                scale: 1.0,
                offset: (0.0, 0.0),
                ..view
            },
            u,
            v,
        ))
    }

    /// Terrain height in a body-fixed direction, bilinear, from data no deeper than
    /// `max_level`.
    pub fn height(&self, dir: DVec3, max_level: u8) -> Option<f64> {
        let (view, u, v) = self.locate(Layer::Height, dir, max_level)?;
        Some(view.height(u, v) as f64)
    }
}

/// A requested sample grid mapped onto a stored tile.
pub struct TileView {
    pub tile: Arc<TileData>,
    /// The stored tile the samples come from.
    pub source: TileKey,
    scale: f64,
    offset: (f64, f64),
}

impl TileView {
    /// Level of the stored data (lower than the requested key when upsampling).
    pub fn data_level(&self) -> u8 {
        self.source.level
    }

    /// Requested-grid sample coordinates to stored-tile sample coordinates.
    pub fn to_source(&self, i: f64, j: f64) -> (f64, f64) {
        (
            (self.offset.0 + i) * self.scale,
            (self.offset.1 + j) * self.scale,
        )
    }

    pub fn height(&self, i: f64, j: f64) -> f32 {
        let heights = self.tile.heights();
        let (u, v) = self.to_source(i, j);
        bilinear(u, v, |x, y| heights[y * TILE_SAMPLES + x])
    }

    pub fn rgb(&self, i: f64, j: f64) -> [f32; 3] {
        let rgb = self.tile.rgb();
        let (u, v) = self.to_source(i, j);
        [0, 1, 2].map(|c| bilinear(u, v, |x, y| rgb[y * TILE_SAMPLES + x][c] as f32))
    }

    pub fn byte_linear(&self, channel: usize, i: f64, j: f64) -> f32 {
        let plane = self.tile.channel(channel);
        let (u, v) = self.to_source(i, j);
        bilinear(u, v, |x, y| plane[y * TILE_SAMPLES + x] as f32)
    }

    pub fn byte_nearest(&self, channel: usize, i: f64, j: f64) -> u8 {
        let (u, v) = self.to_source(i, j);
        self.byte_nearest_source(channel, u, v)
    }

    /// Nearest sample at stored-tile coordinates (clamped to the tile).
    pub fn byte_nearest_source(&self, channel: usize, u: f64, v: f64) -> u8 {
        let plane = self.tile.channel(channel);
        let x = (u.round() as isize).clamp(0, TILE_CELLS as isize) as usize;
        let y = (v.round() as isize).clamp(0, TILE_CELLS as isize) as usize;
        plane[y * TILE_SAMPLES + x]
    }
}

/// Bilinear interpolation over a `TILE_SAMPLES²` grid; coordinates are clamped to it.
pub fn bilinear(u: f64, v: f64, fetch: impl Fn(usize, usize) -> f32) -> f32 {
    let max = TILE_CELLS as f64;
    let (u, v) = (u.clamp(0.0, max), v.clamp(0.0, max));
    let (x0, y0) = (
        (u.floor() as usize).min(TILE_CELLS - 1),
        (v.floor() as usize).min(TILE_CELLS - 1),
    );
    let (fx, fy) = ((u - x0 as f64) as f32, (v - y0 as f64) as f32);
    let top = fetch(x0, y0) * (1.0 - fx) + fetch(x0 + 1, y0) * fx;
    let bottom = fetch(x0, y0 + 1) * (1.0 - fx) + fetch(x0 + 1, y0 + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::PackWriter;

    /// A dataset whose height is a smooth function of direction, stored to level 3 on
    /// face 0 and level 1 elsewhere.
    fn build(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        let meta = Meta {
            format: FORMAT_VERSION,
            ..Default::default()
        };
        std::fs::write(dir.join(META_FILE), serde_json::to_vec(&meta).unwrap()).unwrap();
        let f = |d: DVec3| (d.x * 1000.0 + d.y * 300.0 - d.z * 700.0) as f32;
        let mut writer = PackWriter::create(&dir.join(Layer::Height.file_name())).unwrap();
        let mut stack: Vec<TileKey> = (0..6).map(TileKey::root).collect();
        while let Some(key) = stack.pop() {
            let mut heights = Vec::new();
            for j in 0..TILE_SAMPLES {
                for i in 0..TILE_SAMPLES {
                    heights.push(f(key.sample_dir(i as f64, j as f64)));
                }
            }
            writer.write(key, &TileData::Height(heights)).unwrap();
            if key.level < if key.face == 0 { 3 } else { 1 } {
                stack.extend(key.children());
            }
        }
        writer.finish().unwrap();
    }

    #[test]
    fn falls_back_to_ancestors_and_agrees_between_paths() {
        let dir = std::env::temp_dir().join(format!("phymi_dataset_{}", std::process::id()));
        build(&dir);
        let data = Dataset::open(&dir).unwrap();
        assert_eq!(data.max_level(Layer::Height), Some(3));
        assert!(!data.has(Layer::Color));

        // Deep request on face 2 falls back to level 1.
        let key = TileKey::containing(2, 0.3, 0.6, 9);
        let view = data.view(Layer::Height, key).unwrap();
        assert_eq!(view.data_level(), 1);

        // The chunk-grid path and the direction path give the same value.
        for (i, j) in [(0.0, 0.0), (17.0, 99.0), (128.0, 64.0)] {
            let by_grid = view.height(i, j) as f64;
            let by_dir = data.height(key.sample_dir(i, j), 9).unwrap();
            assert!((by_grid - by_dir).abs() < 1e-3, "{by_grid} {by_dir}");
        }

        // Interpolated values approximate the source function.
        let probe = TileKey::containing(0, 0.41, 0.52, 3).sample_dir(10.5, 20.25);
        let exact = probe.x * 1000.0 + probe.y * 300.0 - probe.z * 700.0;
        assert!((data.height(probe, 20).unwrap() - exact).abs() < 0.1);
        std::fs::remove_dir_all(dir).ok();
    }
}
